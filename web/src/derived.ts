import type { DerivedDef, DerivedSource, SignalSeries } from "./types";

export type { DerivedDef, DerivedSource };

type Node =
  | { kind: "number"; value: number }
  | { kind: "ref"; name: string }
  | { kind: "unary"; op: "-"; value: Node }
  | {
      kind: "binary";
      op: "+" | "-" | "*" | "/" | ">" | "<" | ">=" | "<=" | "==" | "!=";
      left: Node;
      right: Node;
    }
  | { kind: "call"; name: string; args: Node[] };

interface Token {
  kind: "number" | "ref" | "name" | "op" | "paren" | "comma" | "end";
  text: string;
  value?: number;
}

function tokenize(source: string): Token[] {
  const tokens: Token[] = [];
  let i = 0;
  while (i < source.length) {
    const ch = source[i];
    if (/\s/.test(ch)) {
      i += 1;
      continue;
    }
    const pair = source.slice(i, i + 2);
    if ([">=", "<=", "==", "!="].includes(pair)) {
      tokens.push({ kind: "op", text: pair });
      i += 2;
      continue;
    }
    if ("+-*/<>".includes(ch)) {
      tokens.push({ kind: "op", text: ch });
      i += 1;
      continue;
    }
    if (ch === "(" || ch === ")") {
      tokens.push({ kind: "paren", text: ch });
      i += 1;
      continue;
    }
    if (ch === ",") {
      tokens.push({ kind: "comma", text: ch });
      i += 1;
      continue;
    }
    if (ch === "[") {
      const end = source.indexOf("]", i + 1);
      if (end < 0) throw new Error("missing ]");
      const name = source.slice(i + 1, end).trim();
      if (!name) throw new Error("empty [signal]");
      tokens.push({ kind: "ref", text: name });
      i = end + 1;
      continue;
    }
    const number = /^\d+(?:\.\d*)?|^\.\d+/.exec(source.slice(i));
    if (number) {
      tokens.push({ kind: "number", text: number[0], value: Number(number[0]) });
      i += number[0].length;
      continue;
    }
    const name = /^[A-Za-z_]\w*/.exec(source.slice(i));
    if (name) {
      tokens.push({ kind: "name", text: name[0] });
      i += name[0].length;
      continue;
    }
    throw new Error(`unexpected '${ch}'`);
  }
  tokens.push({ kind: "end", text: "" });
  return tokens;
}

class Parser {
  private index = 0;

  constructor(private tokens: Token[]) {}

  private peek(): Token {
    return this.tokens[this.index];
  }

  private take(kind: Token["kind"], text?: string): Token {
    const token = this.peek();
    if (token.kind !== kind || (text !== undefined && token.text !== text)) {
      throw new Error(`expected ${text ?? kind}`);
    }
    this.index += 1;
    return token;
  }

  parse(): Node {
    const node = this.expression();
    if (this.peek().kind !== "end") throw new Error("unexpected trailing input");
    return node;
  }

  private expression(): Node {
    let node = this.additive();
    while (
      this.peek().kind === "op" &&
      [">", "<", ">=", "<=", "==", "!="].includes(this.peek().text)
    ) {
      const op = this.take("op").text as
        | ">"
        | "<"
        | ">="
        | "<="
        | "=="
        | "!=";
      node = { kind: "binary", op, left: node, right: this.additive() };
    }
    return node;
  }

  private additive(): Node {
    let node = this.term();
    while (
      this.peek().kind === "op" &&
      (this.peek().text === "+" || this.peek().text === "-")
    ) {
      const op = this.take("op").text as "+" | "-";
      node = { kind: "binary", op, left: node, right: this.term() };
    }
    return node;
  }

  private term(): Node {
    let node = this.factor();
    while (
      this.peek().kind === "op" &&
      (this.peek().text === "*" || this.peek().text === "/")
    ) {
      const op = this.take("op").text as "*" | "/";
      node = { kind: "binary", op, left: node, right: this.factor() };
    }
    return node;
  }

  private factor(): Node {
    const token = this.peek();
    if (token.kind === "op" && token.text === "-") {
      this.take("op");
      return { kind: "unary", op: "-", value: this.factor() };
    }
    if (token.kind === "number") {
      this.take("number");
      return { kind: "number", value: token.value ?? 0 };
    }
    if (token.kind === "ref") {
      this.take("ref");
      return { kind: "ref", name: token.text };
    }
    if (token.kind === "paren" && token.text === "(") {
      this.take("paren", "(");
      const node = this.expression();
      this.take("paren", ")");
      return node;
    }
    if (token.kind === "name") {
      const name = this.take("name").text;
      this.take("paren", "(");
      const args: Node[] = [];
      if (!(this.peek().kind === "paren" && this.peek().text === ")")) {
        args.push(this.expression());
        while (this.peek().kind === "comma") {
          this.take("comma");
          args.push(this.expression());
        }
      }
      this.take("paren", ")");
      return { kind: "call", name, args };
    }
    throw new Error("expected a value");
  }
}

function collectRefs(node: Node, out: Set<string>) {
  if (node.kind === "ref") out.add(node.name);
  else if (node.kind === "unary") collectRefs(node.value, out);
  else if (node.kind === "binary") {
    collectRefs(node.left, out);
    collectRefs(node.right, out);
  } else if (node.kind === "call") {
    for (const arg of node.args) collectRefs(arg, out);
  }
}

/// The minimum an expression needs to resolve a `[reference]`: a stable key,
/// a display name and the bus label used in `[Bus:name]`. Chart lanes and
/// catalog entries both satisfy this.
export interface DerivedReference {
  key: string;
  name: string;
  bus: string;
  /// Optional frame/name qualifier for `[Bus:Frame:Signal]` when one bus
  /// carries the same signal name in two frames.
  frame?: string;
  /// The hex id (CAN) or slot token accepted as the same frame qualifier.
  frameId?: string;
}

function resolveRefs<T extends DerivedReference>(
  names: Set<string>,
  series: T[],
): { refs: Map<string, T>; error?: string } {
  const refs = new Map<string, T>();
  for (const name of names) {
    const exact = series.filter(
      (signal) =>
        signal.name === name ||
        `${signal.bus}:${signal.name}` === name ||
        (signal.frame !== undefined &&
          `${signal.bus}:${signal.frame}:${signal.name}` === name) ||
        (signal.frameId !== undefined &&
          `${signal.bus}:${signal.frameId}:${signal.name}` === name) ||
        signal.key === name,
    );
    if (exact.length === 0) {
      return { refs, error: `unknown signal [${name}]` };
    }
    if (exact.length > 1) {
      return {
        refs,
        error: `[${name}] is ambiguous; use [Bus:Message:Signal]`,
      };
    }
    refs.set(name, exact[0]);
  }
  return { refs };
}

function valueAt(points: [number, number][], t: number): number | null {
  if (points.length === 0) return null;
  if (t <= points[0][0]) return points[0][1];
  const last = points[points.length - 1];
  if (t >= last[0]) return last[1];
  let lo = 0;
  let hi = points.length - 1;
  while (lo + 1 < hi) {
    const mid = (lo + hi) >> 1;
    if (points[mid][0] <= t) lo = mid;
    else hi = mid;
  }
  const [t0, v0] = points[lo];
  const [t1, v1] = points[hi];
  return t1 > t0 ? v0 + ((v1 - v0) * (t - t0)) / (t1 - t0) : v0;
}

function evaluate(
  node: Node,
  t: number,
  refs: Map<string, SignalSeries>,
): number | null {
  switch (node.kind) {
    case "number":
      return node.value;
    case "ref": {
      const signal = refs.get(node.name);
      return signal ? valueAt(signal.points, t) : null;
    }
    case "unary": {
      const value = evaluate(node.value, t, refs);
      return value === null ? null : -value;
    }
    case "binary": {
      const left = evaluate(node.left, t, refs);
      const right = evaluate(node.right, t, refs);
      if (left === null || right === null) return null;
      switch (node.op) {
        case "+":
          return left + right;
        case "-":
          return left - right;
        case "*":
          return left * right;
        case "/":
          return right === 0 ? null : left / right;
        case ">":
          return left > right ? 1 : 0;
        case "<":
          return left < right ? 1 : 0;
        case ">=":
          return left >= right ? 1 : 0;
        case "<=":
          return left <= right ? 1 : 0;
        case "==":
          return left === right ? 1 : 0;
        case "!=":
          return left !== right ? 1 : 0;
      }
      return null;
    }
    case "call": {
      const values = node.args.map((arg) => evaluate(arg, t, refs));
      if (values.some((value) => value === null)) return null;
      const nums = values as number[];
      switch (node.name) {
        case "abs":
          return Math.abs(nums[0]);
        case "sqrt":
          return nums[0] >= 0 ? Math.sqrt(nums[0]) : null;
        case "min":
          return Math.min(...nums);
        case "max":
          return Math.max(...nums);
        default:
          return null;
      }
    }
  }
}

function invalidCall(node: Node): string | null {
  if (node.kind === "unary") return invalidCall(node.value);
  if (node.kind === "binary") {
    return invalidCall(node.left) ?? invalidCall(node.right);
  }
  if (node.kind === "call") {
    if (!["abs", "sqrt", "min", "max"].includes(node.name)) {
      return `unknown function ${node.name}()`;
    }
    if (node.name !== "min" && node.name !== "max" && node.args.length !== 1) {
      return `${node.name}() takes one argument`;
    }
    if (node.args.length === 0) return `${node.name}() takes arguments`;
    for (const arg of node.args) {
      const error = invalidCall(arg);
      if (error) return error;
    }
  }
  return null;
}

export function validateDerivedExpression(
  expression: string,
  series: DerivedReference[],
): string | null {
  try {
    const ast = new Parser(tokenize(expression)).parse();
    const callError = invalidCall(ast);
    if (callError) return callError;
    const names = new Set<string>();
    collectRefs(ast, names);
    if (names.size === 0) return "reference at least one [signal]";
    const resolved = resolveRefs(names, series);
    if (resolved.error) return resolved.error;
    return null;
  } catch (e) {
    return e instanceof Error ? e.message : String(e);
  }
}

/// The exact lanes the editor resolved for each `[reference]`. The core uses
/// these instead of guessing by name, so an observer that later watches a
/// same-named signal on another bus cannot turn a working lane ambiguous.
export function resolveDerivedSources(
  expression: string,
  series: DerivedReference[],
): DerivedSource[] {
  try {
    const ast = new Parser(tokenize(expression)).parse();
    const names = new Set<string>();
    collectRefs(ast, names);
    const resolved = resolveRefs(names, series);
    // Keep whatever resolved even when one reference did not: the core falls
    // back to name resolution only for the missing ones.
    return [...resolved.refs.entries()].map(([name, signal]) => ({
      name,
      key: signal.key,
    }));
  } catch {
    return [];
  }
}

/// Rising-edge times where the condition becomes true. Used by the event
/// jump dialog; the condition uses the same grammar as derived channels.
export function conditionEvents(
  expression: string,
  series: SignalSeries[],
): { times: number[]; error: string | null } {
  const error = validateDerivedExpression(expression, series);
  if (error) return { times: [], error };
  try {
    const ast = new Parser(tokenize(expression)).parse();
    const names = new Set<string>();
    collectRefs(ast, names);
    const resolved = resolveRefs(names, series);
    if (resolved.error) return { times: [], error: resolved.error };
    const base = [...resolved.refs.values()][0];
    const times: number[] = [];
    let previous = false;
    for (const [t] of base.points) {
      const value = evaluate(ast, t, resolved.refs);
      const truth = value !== null && value > 0;
      if (truth && !previous) times.push(t);
      previous = truth;
    }
    return { times, error: null };
  } catch (e) {
    return { times: [], error: e instanceof Error ? e.message : String(e) };
  }
}

export function statsOf(
  points: [number, number][],
): { min: number; max: number; avg: number } | null {
  if (points.length === 0) return null;
  let min = Number.POSITIVE_INFINITY;
  let max = Number.NEGATIVE_INFINITY;
  let sum = 0;
  for (const [, value] of points) {
    if (value < min) min = value;
    if (value > max) max = value;
    sum += value;
  }
  return { min, max, avg: sum / points.length };
}

function csvNumber(value: number): string {
  let text = value.toFixed(6);
  if (text.includes(".")) {
    while (text.endsWith("0")) text = text.slice(0, -1);
    if (text.endsWith(".")) text = text.slice(0, -1);
  }
  return text;
}

/// Wide, time-aligned CSV of everything on the chart. Derived channels are
/// core series by now, so they arrive here like any other lane.
/// Sample-and-hold at a 10 ms floor.
export function buildAlignedCsv(
  series: SignalSeries[],
  fromS: number,
  toS: number,
): string {
  if (!(fromS < toS) || series.length === 0) return "time_s\n";
  const step = Math.max(0.01, (toS - fromS) / 20_000);
  const rows = series.map((signal) => ({
    points: signal.points,
    index: 0,
    last: null as number | null,
  }));
  let out = "time_s";
  for (const signal of series) out += `,${signal.bus}:${signal.name}`;
  out += "\n";
  for (let t = fromS; t <= toS + 1e-9; t += step) {
    out += csvNumber(t);
    for (const row of rows) {
      while (row.index < row.points.length && row.points[row.index][0] <= t) {
        row.last = row.points[row.index][1];
        row.index += 1;
      }
      out += ",";
      if (row.last !== null) out += csvNumber(row.last);
    }
    out += "\n";
  }
  return out;
}
