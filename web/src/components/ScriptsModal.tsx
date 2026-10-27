import { useEffect, useLayoutEffect, useRef, useState } from "preact/hooks";
import {
  fetchDebug,
  fetchScriptHelp,
  fetchScripts,
  sendDebug,
} from "../api";
import type {
  Bus,
  Catalog,
  CatalogMessage,
  Command,
  DebugView,
  DerivedDef,
  Script,
  ScriptFunctionHelp,
  SysVar,
} from "../types";
import { FilePick } from "./FilePick";

interface Props {
  buses: Bus[];
  catalog: Catalog | null;
  derived: DerivedDef[];
  sysvars: SysVar[];
  onCommand: (command: Command) => void;
  onClose: () => void;
}

const INDENT = "    ";
const BRACKET_PAIRS: Record<string, string> = { "(": ")", "[": "]", "{": "}" };
const BRACKET_CLOSERS = new Set(Object.values(BRACKET_PAIRS));

export interface CompletionItem {
  label: string;
  detail: string;
  insert: string;
  /// Caret offset from the start of the inserted text.
  caret: number;
  rank: number;
}

interface Completion {
  start: number;
  end: number;
  items: CompletionItem[];
  active: number;
  left: number;
  top: number;
  /// Alt+/ or arrow-key navigation: Enter/Tab may accept the selection.
  manual: boolean;
}

interface SignatureHint {
  name: string;
  args: string[];
  active: number;
  left: number;
  top: number;
}

function lineStartOf(text: string, position: number): number {
  return text.lastIndexOf("\n", Math.max(0, position - 1)) + 1;
}

function lineEndOf(text: string, position: number): number {
  const end = text.indexOf("\n", position);
  return end < 0 ? text.length : end;
}

function lineOffset(text: string, line: number): number {
  let offset = 0;
  for (let current = 1; current < line; current += 1) {
    const next = text.indexOf("\n", offset);
    if (next < 0) return text.length;
    offset = next + 1;
  }
  return offset;
}

function revealPosition(textarea: HTMLTextAreaElement, position: number) {
  const style = getComputedStyle(textarea);
  const lineHeight =
    Number.parseFloat(style.lineHeight) ||
    (Number.parseFloat(style.fontSize) || 11.5) * 1.45;
  const padTop = Number.parseFloat(style.paddingTop) || 0;
  const line = textarea.value.slice(0, position).split("\n").length - 1;
  const top = padTop + line * lineHeight;
  if (top < textarea.scrollTop) {
    textarea.scrollTop = Math.max(0, top - lineHeight);
  } else if (top + lineHeight > textarea.scrollTop + textarea.clientHeight) {
    textarea.scrollTop = top + lineHeight - textarea.clientHeight + lineHeight;
  }
}

export function findMatches(text: string, query: string): number[] {
  if (!query) return [];
  const haystack = text.toLowerCase();
  const needle = query.toLowerCase();
  const hits: number[] = [];
  let at = 0;
  while (at <= haystack.length) {
    const found = haystack.indexOf(needle, at);
    if (found < 0) break;
    hits.push(found);
    at = found + needle.length;
  }
  return hits;
}

function escapeRegExp(text: string): string {
  return text.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}

export interface LocalSymbol {
  name: string;
  kind: "let" | "fn" | "param";
  /// Declared parameters for a top-level function.
  params?: string[];
}

/// Declarations visible at the caret: top-level `fn` names and `let`
/// variables whose enclosing block is still open. Comments and strings are
/// skipped so their text cannot look like a declaration.
export function localSymbols(source: string, caret: number): LocalSymbol[] {
  const isStart = (c: string) => /[A-Za-z_]/.test(c);
  const isPart = (c: string) => /[A-Za-z0-9_]/.test(c);
  const blocks: number[] = [];
  const lets: { name: string; block: number }[] = [];
  const functions: LocalSymbol[] = [];
  const bodies: { start: number; end: number; params: string[] }[] = [];
  let pending: { params: string[] } | null = null;
  let body: { start: number; params: string[] } | null = null;
  let snapshot: number[] | null = null;
  let i = 0;
  while (i < source.length) {
    if (snapshot === null && i >= caret) snapshot = blocks.slice();
    const c = source[i];
    if (c === "/" && source[i + 1] === "/") {
      i = lineEndOf(source, i);
      continue;
    }
    if (c === "/" && source[i + 1] === "*") {
      const end = source.indexOf("*/", i + 2);
      i = end < 0 ? source.length : end + 2;
      continue;
    }
    if (c === '"') {
      i += 1;
      while (i < source.length && source[i] !== '"') {
        if (source[i] === "\\") i += 1;
        i += 1;
      }
      i += 1;
      continue;
    }
    if (c === "{") {
      blocks.push(i);
      if (pending) {
        body = { start: i, params: pending.params };
        pending = null;
      }
      i += 1;
      continue;
    }
    if (c === "}") {
      blocks.pop();
      if (body && blocks.length === 0) {
        bodies.push({ ...body, end: i });
        body = null;
      }
      i += 1;
      continue;
    }
    if (!isStart(c)) {
      i += 1;
      continue;
    }
    let end = i + 1;
    while (end < source.length && isPart(source[end])) end += 1;
    const word = source.slice(i, end);
    if (word === "let" && i < caret) {
      let at = end;
      while (at < source.length && /\s/.test(source[at])) at += 1;
      if (isStart(source[at] ?? "")) {
        let nameEnd = at + 1;
        while (nameEnd < source.length && isPart(source[nameEnd])) nameEnd += 1;
        lets.push({
          name: source.slice(at, nameEnd),
          block: blocks[blocks.length - 1] ?? -1,
        });
      }
    } else if (word === "fn" && blocks.length === 0) {
      let at = end;
      while (at < source.length && /\s/.test(source[at])) at += 1;
      if (isStart(source[at] ?? "")) {
        let nameEnd = at + 1;
        while (nameEnd < source.length && isPart(source[nameEnd])) nameEnd += 1;
        const name = source.slice(at, nameEnd);
        let params: string[] = [];
        let paren = nameEnd;
        while (paren < source.length && /\s/.test(source[paren])) paren += 1;
        if (source[paren] === "(") {
          let p = paren + 1;
          while (p < source.length && source[p] !== ")") {
            if (isStart(source[p])) {
              let pe = p + 1;
              while (pe < source.length && isPart(source[pe])) pe += 1;
              params.push(source.slice(p, pe));
              p = pe;
            } else {
              p += 1;
            }
          }
          pending = { params };
        }
        functions.push({ name, kind: "fn", params });
      }
    }
    i = end;
  }
  if (snapshot === null) snapshot = blocks.slice();

  const open = new Set(snapshot);
  const activeParams: string[] = [];
  for (const candidate of bodies) {
    if (caret > candidate.start && caret <= candidate.end) {
      activeParams.push(...candidate.params);
    }
  }
  if (body && caret > body.start) activeParams.push(...body.params);

  const seen = new Set<string>();
  const out: LocalSymbol[] = [];
  const push = (symbol: LocalSymbol) => {
    if (seen.has(symbol.name)) return;
    seen.add(symbol.name);
    out.push(symbol);
  };
  for (const name of activeParams) push({ name, kind: "param" });
  for (const fn of functions) push(fn);
  for (const variable of lets) {
    if (variable.block !== -1 && !open.has(variable.block)) continue;
    push({ name: variable.name, kind: "let" });
  }
  return out;
}

/// Strings cannot span lines (the lexer rejects a newline inside one), so the
/// quote state only needs the text from the current line start.
function inStringAt(text: string, position: number): boolean {
  let inside = false;
  for (let i = lineStartOf(text, position); i < position; i += 1) {
    if (text[i] === '"' && text[i - 1] !== "\\") inside = !inside;
  }
  return inside;
}

export interface CallInfo {
  name: string;
  active: number;
}

/// Blank strings and comments while keeping offsets, so a bracket or comma
/// inside them cannot affect the call shape.
function maskCode(text: string): string {
  const out = text.split("");
  let quote = false;
  let block = false;
  for (let i = 0; i < text.length; i += 1) {
    const char = text[i];
    if (block) {
      if (char === "*" && text[i + 1] === "/") {
        out[i] = " ";
        out[i + 1] = " ";
        i += 1;
        block = false;
      } else if (char !== "\n") {
        out[i] = " ";
      }
      continue;
    }
    if (quote) {
      if (char === '"' && text[i - 1] !== "\\") quote = false;
      if (char !== "\n") out[i] = " ";
      continue;
    }
    if (char === "/" && text[i + 1] === "*") {
      out[i] = " ";
      out[i + 1] = " ";
      i += 1;
      block = true;
    } else if (char === "/" && text[i + 1] === "/") {
      while (i < text.length && text[i] !== "\n") {
        out[i] = " ";
        i += 1;
      }
      i -= 1;
    } else if (char === '"') {
      quote = true;
      out[i] = " ";
    }
  }
  return out.join("");
}

/// The innermost call still open at the caret, looking back far enough for
/// multi-line argument lists but bounded so typing stays cheap.
export function callAtCaret(value: string, caret: number): CallInfo | null {
  if (inStringAt(value, caret)) return null;
  const floor = Math.max(0, caret - 4000);
  let start = lineStartOf(value, caret);
  for (let lines = 0; lines < 40 && start > floor; lines += 1) {
    start = lineStartOf(value, start - 1);
  }
  const masked = maskCode(value.slice(start, caret));

  let depth = 0;
  let open = -1;
  for (let i = masked.length - 1; i >= 0; i -= 1) {
    const char = masked[i];
    if (char === ")") {
      depth += 1;
    } else if (char === "(") {
      if (depth === 0) {
        open = i;
        break;
      }
      depth -= 1;
    }
  }
  if (open < 0) return null;

  const lineStart = masked.lastIndexOf("\n", open - 1) + 1;
  const name = /([A-Za-z_]\w*)\s*$/.exec(masked.slice(lineStart, open))?.[1];
  if (!name) return null;

  let active = 0;
  let nested = 0;
  for (let i = open + 1; i < masked.length; i += 1) {
    const char = masked[i];
    if (char === "(" || char === "[" || char === "{") {
      nested += 1;
    } else if (char === ")" || char === "]" || char === "}") {
      nested = Math.max(0, nested - 1);
    } else if (char === "," && nested === 0) {
      active += 1;
    }
  }
  return { name, active };
}

/// Split a usage string's argument list for the signature hint. Quoted
/// commas and nested calls stay in one argument.
export function signatureArgs(usage: string): string[] {
  const open = usage.indexOf("(");
  const close = usage.lastIndexOf(")");
  if (open < 0 || close <= open) return [];
  const body = usage.slice(open + 1, close);
  if (!body.trim()) return [];
  const args: string[] = [];
  let start = 0;
  let quote = false;
  let depth = 0;
  for (let i = 0; i < body.length; i += 1) {
    const char = body[i];
    if (quote) {
      if (char === '"' && body[i - 1] !== "\\") quote = false;
      continue;
    }
    if (char === '"') {
      quote = true;
    } else if (char === "(" || char === "[" || char === "{") {
      depth += 1;
    } else if (char === ")" || char === "]" || char === "}") {
      depth = Math.max(0, depth - 1);
    } else if (char === "," && depth === 0) {
      args.push(body.slice(start, i).trim());
      start = i + 1;
    }
  }
  args.push(body.slice(start).trim());
  return args;
}

function leadingSpace(text: string): string {
  return /^[ \t]*/.exec(text)?.[0] ?? "";
}

/// Replaces a range through the browser's own insertion command so typing
/// remains one undo step. Falls back to `setRangeText` if the command is
/// unavailable (e.g. an embedded webview).
function replaceRange(
  textarea: HTMLTextAreaElement,
  start: number,
  end: number,
  text: string,
  caret: number,
  selectionEnd = caret,
) {
  textarea.focus();
  textarea.setSelectionRange(start, end);
  let inserted = false;
  try {
    inserted = document.execCommand("insertText", false, text);
  } catch {
    inserted = false;
  }
  if (!inserted) {
    textarea.setRangeText(text, start, end, "end");
    textarea.dispatchEvent(new Event("input", { bubbles: true }));
  }
  textarea.setSelectionRange(caret, selectionEnd);
}

function quoted(text: string): string {
  return text.replace(/\\/g, "\\\\").replace(/"/g, '\\"');
}

let measureCanvas: HTMLCanvasElement | null = null;

function characterWidth(style: CSSStyleDeclaration): number {
  measureCanvas ??= document.createElement("canvas");
  const context = measureCanvas.getContext("2d");
  if (!context) return 7;
  context.font = `${style.fontWeight} ${style.fontSize} ${style.fontFamily}`;
  return context.measureText("M").width || 7;
}

/// The editor uses a monospace font and `white-space: pre`, so the caret's
/// pixel position is line/column arithmetic plus a character-width measure.
/// The popup is clamped inside the editor and flips above near the bottom.
function caretPlacement(
  textarea: HTMLTextAreaElement,
  position: number,
  boxWidth: number,
  boxHeight: number,
): { left: number; top: number } {
  const style = getComputedStyle(textarea);
  const padLeft = Number.parseFloat(style.paddingLeft) || 0;
  const padTop = Number.parseFloat(style.paddingTop) || 0;
  const lineHeight =
    Number.parseFloat(style.lineHeight) ||
    (Number.parseFloat(style.fontSize) || 11.5) * 1.45;
  const before = textarea.value.slice(0, position);
  const line = before.split("\n").length - 1;
  const lineText = before.slice(before.lastIndexOf("\n") + 1);
  let column = 0;
  for (const char of lineText) {
    column += char === "\t" ? 4 - (column % 4) : 1;
  }
  const charWidth = characterWidth(style);
  const editor = textarea.parentElement;
  const width = editor?.clientWidth ?? 0;
  const height = editor?.clientHeight ?? 0;
  const x = padLeft + column * charWidth - textarea.scrollLeft;
  const y = padTop + (line + 1) * lineHeight - textarea.scrollTop;
  const left = Math.max(4, Math.min(x, Math.max(4, width - boxWidth - 6)));
  const top =
    y + 2 + boxHeight <= height - 4 ? y + 2 : Math.max(4, y - boxHeight - 2);
  return { left, top };
}

function completionPlacement(
  textarea: HTMLTextAreaElement,
  position: number,
  itemCount: number,
): { left: number; top: number } {
  return caretPlacement(
    textarea,
    position,
    340,
    Math.min(190, 8 + Math.min(itemCount, 8) * 22),
  );
}

function catalogMessages(
  catalog: Catalog | null,
  channel: number,
) {
  return (
    catalog?.channels.find(
      (candidate) => candidate.kind === "can" && candidate.index === channel,
    )?.messages ?? []
  );
}

function catalogMessage(
  catalog: Catalog | null,
  channel: number,
  name: string,
) {
  return (
    catalogMessages(catalog, channel).find(
      (candidate) => candidate.name === name,
    ) ?? null
  );
}

function catalogBus(catalog: Catalog | null, kind: string, index: number) {
  return (
    catalog?.channels.find(
      (candidate) => candidate.kind === kind && candidate.index === index,
    ) ?? null
  );
}

/// A channel addressed by a description name (`FR.PowerTrain`,
/// `ETH.Body`). The display name (`FR0`) also works, but only an
/// unambiguous match resolves -- the same rule the script host applies.
function catalogAliasBus(
  catalog: Catalog | null,
  kind: string,
  alias: string,
) {
  const hits = (catalog?.channels ?? []).filter(
    (candidate) =>
      candidate.kind === kind &&
      (candidate.name === alias || candidate.aliases?.includes(alias)),
  );
  return hits.length === 1 ? hits[0] : null;
}

function aliasItems(
  catalog: Catalog | null,
  kind: string,
  detail: string,
): CompletionItem[] {
  const items: CompletionItem[] = [];
  const seen = new Set<string>();
  for (const channel of catalog?.channels ?? []) {
    if (channel.kind !== kind) continue;
    for (const alias of channel.aliases ?? []) {
      if (seen.has(alias)) continue;
      seen.add(alias);
      items.push({
        label: alias,
        detail: `${detail} · ${channel.name}`,
        insert: alias,
        caret: alias.length,
        rank: 0,
      });
    }
  }
  return items;
}

function frSignalItems(
  channel: { index: number; messages: CatalogMessage[] },
  partial: string,
): CompletionItem[] {
  const seen = new Set<string>();
  const query = partial.toLowerCase();
  const items: CompletionItem[] = [];
  for (const frame of channel.messages) {
    for (const signal of frame.signals) {
      if (seen.has(signal.name)) continue;
      seen.add(signal.name);
      if (query !== "" && !signal.name.toLowerCase().includes(query)) continue;
      items.push(
        signalItem(
          signal.name,
          `FR${channel.index} · ${frame.name}${signal.unit ? ` · ${signal.unit}` : ""}`,
        ),
      );
    }
  }
  return items.slice(0, 12);
}

/// `FR3` / `ETH0` → 3 / 0. A name that merely starts with the letters
/// (`Frame`) is an ordinary identifier, matching the parser's rule.
function busIndex(name: string, prefix: string): number | null {
  if (!name.startsWith(prefix)) return null;
  const rest = name.slice(prefix.length);
  return /^\d+$/.test(rest) ? Number(rest) : null;
}

function signalItem(name: string, detail: string): CompletionItem {
  return {
    label: name,
    detail,
    insert: name,
    caret: name.length,
    rank: 0,
  };
}

function messageDetail(message: { id: number; extended: boolean }) {
  return `0x${message.id.toString(16).toUpperCase()}${message.extended ? "x" : ""}`;
}

/// Completion is deliberately context-light: a word prefix matches keywords,
/// builtins and derived/sysvar call templates; `Message.` and `Message["…"]`
/// list that message's DBC signals; inside `derived("…")` (or
/// `sys_get`/`sys_set`) only the matching names are offered.
export function buildCandidates(
  value: string,
  caret: number,
  allowEmpty: boolean,
  functions: ScriptFunctionHelp[],
  derived: DerivedDef[],
  sysvars: SysVar[],
  catalog: Catalog | null,
  channel: number,
): { start: number; end: number; items: CompletionItem[] } | null {
  const lineStart = lineStartOf(value, caret);
  const lineBefore = value.slice(lineStart, caret);
  if (lineBefore.includes("//")) return null;
  // Appending after a finished token needs the space the user was about to
  // type (`on` + start → `on start`), but never before punctuation.
  const separator = caret > 0 && /\w/.test(value[caret - 1]) ? " " : "";

  let inString = false;
  let quoteAt = -1;
  for (let i = 0; i < lineBefore.length; i += 1) {
    if (lineBefore[i] === '"' && (i === 0 || lineBefore[i - 1] !== "\\")) {
      if (!inString) {
        inString = true;
        quoteAt = i;
      } else {
        inString = false;
      }
    }
  }

  // A handler header has a fixed grammar, so only the next token of that
  // clause is useful there; once `{` is passed, expressions take over and
  // the general function/keyword candidates below apply again.
  if (
    !inString &&
    /^\s*on(?:\s|$)/.test(lineBefore) &&
    !lineBefore.includes("{")
  ) {
    const target = /^\s*on\s+message(?:\s+([A-Za-z_]\w*)?)?$/.exec(lineBefore);
    if (target) {
      const partial = target[1] ?? "";
      const query = partial.toLowerCase();
      const lead = partial === "" ? separator : "";
      const items = catalogMessages(catalog, channel)
        .filter(
          (message) =>
            query === "" || message.name.toLowerCase().includes(query),
        )
        .slice(0, 12)
        .map((message) => ({
          label: message.name,
          detail: messageDetail(message),
          insert: `${lead}${message.name}`,
          caret: lead.length + message.name.length,
          rank: 0,
        }));
      if (query === "") {
        items.push({
          label: "*",
          detail: "every frame",
          insert: `${lead}*`,
          caret: lead.length + 1,
          rank: 0,
        });
      }
      if (items.length === 0) return null;
      return { start: caret - partial.length, end: caret, items };
    }

    const clause =
      /^\s*on\s+(extended|fr)(?:\s+(?:\d+\s*)?([A-Za-z_]\w*)?)?$/.exec(
        lineBefore,
      );
    if (clause) {
      const [, name, partial = ""] = clause;
      const word = name === "extended" ? "message" : "slot";
      const detail =
        name === "extended" ? "29-bit frame by id" : "FlexRay slot number";
      if (partial === "" || word.startsWith(partial.toLowerCase())) {
        const lead = partial === "" ? separator : "";
        return {
          start: caret - partial.length,
          end: caret,
          items: [
            {
              label: word,
              detail: `event · ${name} ${detail}`,
              insert: `${lead}${word}`,
              caret: lead.length + word.length,
              rank: 0,
            },
          ],
        };
      }
      return null;
    }

    const event = /^\s*on(?:\s+([A-Za-z_]\w*)?)?$/.exec(lineBefore);
    if (event) {
      const partial = event[1] ?? "";
      const query = partial.toLowerCase();
      // A complete event word means the next token is its value or `{`.
      if (EVENTS.some(([word]) => word.toLowerCase() === query)) return null;
      const lead = partial === "" ? separator : "";
      const items = EVENTS.filter(
        ([word]) => query === "" || word.toLowerCase().startsWith(query),
      ).map(([word, detail]) => ({
        label: word,
        detail: `event · ${detail}`,
        insert: `${lead}${word}`,
        caret: lead.length + word.length,
        rank: 0,
      }));
      if (items.length === 0) return null;
      return { start: caret - partial.length, end: caret, items };
    }

    return null;
  }

  // `this.` / `node.` are handler contexts, not catalog names.
  if (!inString) {
    const context = /(this|node)\.([A-Za-z_]\w*)?$/.exec(lineBefore);
    if (context) {
      const fields: {
        label: string;
        detail: string;
        insert: string;
        caret?: number;
      }[] =
        context[1] === "this"
          ? [
              { label: "bus", detail: "trigger bus", insert: "bus" },
              { label: "id", detail: "CAN frame id", insert: "id" },
              { label: "slot", detail: "FlexRay slot", insert: "slot" },
              { label: "len", detail: "payload length", insert: "len" },
              { label: "cycle", detail: "FlexRay cycle", insert: "cycle" },
              {
                label: "signal",
                detail: 'triggering frame signal("Name")',
                insert: 'signal("")',
                caret: 8,
              },
            ]
          : [
              {
                label: "bus",
                detail: "the script's own CAN channel",
                insert: "bus",
              },
            ];
      const partial = context[2] ?? "";
      const query = partial.toLowerCase();
      const items = fields
        .filter(
          (field) => query === "" || field.label.toLowerCase().includes(query),
        )
        .map((field) => ({
          label: field.label,
          detail: field.detail,
          insert: field.insert,
          caret: field.caret ?? field.insert.length,
          rank: 0,
        }));
      if (items.length === 0) return null;
      return { start: caret - partial.length, end: caret, items };
    }
  }

  // Named-cluster spellings: `FR.PowerTrain.`, `ETH.Body.`,
  // `ETH.Body.Frame.`. They run before the numeric forms because the
  // same regex shape would otherwise read the name as a CAN message.
  if (!inString) {
    const ethSignal =
      /(?:^|[^\w.])ETH\.([A-Za-z_]\w*)\.([A-Za-z_]\w*)\.([A-Za-z_]\w*)?$/.exec(
        lineBefore,
      );
    if (ethSignal) {
      const [, alias, frameName, partial = ""] = ethSignal;
      const busChannel = catalogAliasBus(catalog, "eth", alias);
      const frame = busChannel?.messages.find(
        (message) => message.name === frameName || message.frame === frameName,
      );
      if (!busChannel || !frame) return null;
      const query = partial.toLowerCase();
      const items = frame.signals
        .filter(
          (signal) => query === "" || signal.name.toLowerCase().includes(query),
        )
        .slice(0, 12)
        .map((signal) =>
          signalItem(
            signal.name,
            `ETH.${alias} · ${frame.name}${signal.unit ? ` · ${signal.unit}` : ""}`,
          ),
        );
      if (items.length === 0) return null;
      return { start: caret - partial.length, end: caret, items };
    }
    const ethFrame =
      /(?:^|[^\w.])ETH\.([A-Za-z_]\w*)\.([A-Za-z_]\w*)?$/.exec(lineBefore);
    if (ethFrame) {
      const [, alias, partial = ""] = ethFrame;
      const busChannel = catalogAliasBus(catalog, "eth", alias);
      if (!busChannel) return null;
      const query = partial.toLowerCase();
      const items = busChannel.messages
        .filter(
          (frame) => query === "" || frame.name.toLowerCase().includes(query),
        )
        .slice(0, 12)
        .map((frame) => ({
          label: frame.name,
          detail: `ETH.${alias} frame`,
          insert: frame.name,
          caret: frame.name.length,
          rank: 0,
        }));
      if (items.length === 0) return null;
      return { start: caret - partial.length, end: caret, items };
    }
    const ethBus = /(?:^|[^\w.])ETH\.([A-Za-z_]\w*)?$/.exec(lineBefore);
    if (ethBus) {
      const partial = ethBus[1] ?? "";
      const items = aliasItems(catalog, "eth", "Ethernet cluster").filter(
        (item) =>
          partial === "" ||
          item.label.toLowerCase().includes(partial.toLowerCase()),
      );
      if (items.length === 0) return null;
      return { start: caret - partial.length, end: caret, items };
    }
    const frSignal =
      /(?:^|[^\w.])FR\.([A-Za-z_]\w*)\.([A-Za-z_]\w*)?$/.exec(lineBefore);
    if (frSignal) {
      const [, alias, partial = ""] = frSignal;
      const busChannel = catalogAliasBus(catalog, "fr", alias);
      if (!busChannel) return null;
      const items = frSignalItems(busChannel, partial);
      if (items.length === 0) return null;
      return { start: caret - partial.length, end: caret, items };
    }
    const frBus = /(?:^|[^\w.])FR\.([A-Za-z_]\w*)?$/.exec(lineBefore);
    if (frBus) {
      const partial = frBus[1] ?? "";
      const items = aliasItems(catalog, "fr", "FlexRay cluster").filter(
        (item) =>
          partial === "" ||
          item.label.toLowerCase().includes(partial.toLowerCase()),
      );
      if (items.length === 0) return null;
      return { start: caret - partial.length, end: caret, items };
    }
  }

  // `ETH0.Frame.` is two names deep: list that frame's signals. Checked
  // before the one-dot forms so `ETH0.Frame` is not read as a CAN message.
  if (!inString) {
    const ethSignal =
      /([A-Za-z_]\w*)\.([A-Za-z_]\w*)\.([A-Za-z_]\w*)?$/.exec(lineBefore);
    if (ethSignal) {
      const [, busName, frameName, partial = ""] = ethSignal;
      const bus = busIndex(busName, "ETH");
      if (bus !== null) {
        const frame = catalogBus(catalog, "eth", bus)?.messages.find(
          (message) => message.name === frameName || message.frame === frameName,
        );
        if (!frame) return null;
        const query = partial.toLowerCase();
        const items = frame.signals
          .filter(
            (signal) =>
              query === "" || signal.name.toLowerCase().includes(query),
          )
          .slice(0, 12)
          .map((signal) =>
            signalItem(
              signal.name,
              `ETH${bus} · ${frame.name}${signal.unit ? ` · ${signal.unit}` : ""}`,
            ),
          );
        if (items.length === 0) return null;
        return {
          start: caret - partial.length,
          end: caret,
          items,
        };
      }
    }
    // `ETH0.` lists the bus's ARXML frames.
    const ethFrame = /([A-Za-z_]\w*)\.([A-Za-z_]\w*)?$/.exec(lineBefore);
    if (ethFrame) {
      const [, busName, partial = ""] = ethFrame;
      const bus = busIndex(busName, "ETH");
      if (bus !== null) {
        const query = partial.toLowerCase();
        const items = (catalogBus(catalog, "eth", bus)?.messages ?? [])
          .filter(
            (frame) =>
              query === "" || frame.name.toLowerCase().includes(query),
          )
          .slice(0, 12)
          .map((frame) => ({
            label: frame.name,
            detail: `ETH${bus} frame`,
            insert: frame.name,
            caret: frame.name.length,
            rank: 0,
          }));
        if (items.length === 0) return null;
        return { start: caret - partial.length, end: caret, items };
      }
    }
    // `FR0.` lists every signal the cluster declares. A name shared by
    // frames of the same slot is one entry: it reads the same wire slot.
    const frSignal = /([A-Za-z_]\w*)\.([A-Za-z_]\w*)?$/.exec(lineBefore);
    if (frSignal) {
      const [, busName, partial = ""] = frSignal;
      const bus = busIndex(busName, "FR");
      if (bus !== null) {
        const busChannel = catalogBus(catalog, "fr", bus);
        if (!busChannel) return null;
        const seen = new Set<string>();
        const query = partial.toLowerCase();
        const items: CompletionItem[] = [];
        for (const frame of busChannel.messages) {
          for (const signal of frame.signals) {
            if (seen.has(signal.name)) continue;
            seen.add(signal.name);
            if (query !== "" && !signal.name.toLowerCase().includes(query)) {
              continue;
            }
            items.push(
              signalItem(
                signal.name,
                `FR${bus} · ${frame.name}${signal.unit ? ` · ${signal.unit}` : ""}`,
              ),
            );
          }
        }
        if (items.length === 0) return null;
        return {
          start: caret - partial.length,
          end: caret,
          items: items.slice(0, 12),
        };
      }
    }
  }

  // `Message.` is a DBC context: only that message's signals make sense.
  if (!inString) {
    const dotted = /([A-Za-z_]\w*)\.([A-Za-z_]\w*)?$/.exec(lineBefore);
    if (dotted) {
      const [, messageName, partial = ""] = dotted;
      const message = catalogMessage(catalog, channel, messageName);
      if (!message) return null;
      const query = partial.toLowerCase();
      const items = message.signals
        .filter(
          (signal) =>
            query === "" || signal.name.toLowerCase().includes(query),
        )
        .slice(0, 12)
        .map((signal) => ({
          label: signal.name,
          detail: signal.unit
            ? `${messageName} · ${signal.unit}`
            : messageName,
          insert: signal.name,
          caret: signal.name.length,
          rank: 0,
        }));
      if (items.length === 0) return null;
      return { start: caret - partial.length, end: caret, items };
    }
  }

  let start = caret;
  let end = caret;
  let prefix = "";
  let stringFn: string | null = null;
  let stringMessage: string | null = null;
  let stringFrame: string | null = null;
  if (inString) {
    const beforeQuote = lineBefore.slice(0, quoteAt).trimEnd();
    const bracket = /([A-Za-z_]\w*)\s*\[\s*$/.exec(beforeQuote);
    // Named-cluster bracket forms, deepest first.
    const ethNamedSignal =
      /(?:^|[^\w.])ETH\s*\[\s*"([^"]*)"\s*\]\s*\[\s*"([^"]*)"\s*\]\s*\[\s*$/.exec(
        beforeQuote,
      );
    const ethNamedFrame =
      /(?:^|[^\w.])ETH\s*\[\s*"([^"]*)"\s*\]\s*\[\s*$/.exec(beforeQuote);
    const frNamedSignal =
      /(?:^|[^\w.])FR\s*\[\s*"([^"]*)"\s*\]\s*\[\s*$/.exec(beforeQuote);
    // `ETH0["Frame"]["`: the second bracket completes that frame's signals.
    const ethSignal =
      /([A-Za-z_]\w*)\s*\[\s*"([^"]*)"\s*\]\s*\[\s*$/.exec(beforeQuote);
    if (/^\s*on\s+message\s*$/.test(beforeQuote)) {
      stringFn = "message";
    } else if (ethNamedSignal) {
      stringFn = "eth-named-signal";
      stringMessage = ethNamedSignal[1];
      stringFrame = ethNamedSignal[2];
    } else if (ethNamedFrame) {
      stringFn = "eth-named-frame";
      stringMessage = ethNamedFrame[1];
    } else if (frNamedSignal) {
      stringFn = "fr-named-signal";
      stringMessage = frNamedSignal[1];
    } else if (ethSignal && busIndex(ethSignal[1], "ETH") !== null) {
      stringFn = "eth-signal";
      stringMessage = String(busIndex(ethSignal[1], "ETH"));
      stringFrame = ethSignal[2];
    } else if (bracket && bracket[1] === "FR") {
      stringFn = "fr-bus";
    } else if (bracket && bracket[1] === "ETH") {
      stringFn = "eth-bus";
    } else if (bracket && busIndex(bracket[1], "FR") !== null) {
      stringFn = "fr-signal";
      stringMessage = String(busIndex(bracket[1], "FR"));
    } else if (bracket && busIndex(bracket[1], "ETH") !== null) {
      stringFn = "eth-frame";
      stringMessage = String(busIndex(bracket[1], "ETH"));
    } else if (bracket) {
      stringFn = "signal";
      stringMessage = bracket[1];
    } else {
      const fn =
        /([A-Za-z_]\w*)\s*\([^()]*$/.exec(beforeQuote)?.[1] ?? null;
      if (fn !== "derived" && fn !== "sys_get" && fn !== "sys_set") return null;
      stringFn = fn;
    }
    start = lineStart + quoteAt + 1;
    end = caret;
    prefix = value.slice(start, caret);
    if (
      prefix.length === 0 &&
      !allowEmpty &&
      stringFn !== "signal" &&
      stringFn !== "message" &&
      stringFn !== "fr-signal" &&
      stringFn !== "eth-frame" &&
      stringFn !== "eth-signal" &&
      stringFn !== "fr-bus" &&
      stringFn !== "eth-bus" &&
      stringFn !== "fr-named-signal" &&
      stringFn !== "eth-named-frame" &&
      stringFn !== "eth-named-signal"
    ) {
      return null;
    }
  } else {
    const word = /[A-Za-z_]\w*$/.exec(lineBefore)?.[0] ?? "";
    if (word.length < 2 && !allowEmpty) return null;
    start = caret - word.length;
    end = caret;
    prefix = word;
  }

  const items: CompletionItem[] = [];
  if (stringFn) {
    if (stringFn === "message") {
      for (const message of catalogMessages(catalog, channel)) {
        items.push({
          label: message.name,
          detail: messageDetail(message),
          insert: message.name,
          caret: message.name.length,
          rank: 0,
        });
      }
    } else if (stringFn === "signal") {
      const message = catalogMessage(
        catalog,
        channel,
        stringMessage ?? "",
      );
      if (!message) return null;
      for (const signal of message.signals) {
        items.push({
          label: signal.name,
          detail: signal.unit
            ? `${message.name} · ${signal.unit}`
            : message.name,
          insert: signal.name,
          caret: signal.name.length,
          rank: 0,
        });
      }
    } else if (stringFn === "fr-bus" || stringFn === "eth-bus") {
      const kind = stringFn === "fr-bus" ? "fr" : "eth";
      const detail = kind === "fr" ? "FlexRay cluster" : "Ethernet cluster";
      for (const item of aliasItems(catalog, kind, detail)) {
        const insert = quoted(item.label);
        items.push({
          label: item.label,
          detail: item.detail,
          insert,
          caret: insert.length,
          rank: 0,
        });
      }
    } else if (stringFn === "fr-named-signal") {
      const busChannel = catalogAliasBus(catalog, "fr", stringMessage ?? "");
      if (!busChannel) return null;
      for (const item of frSignalItems(busChannel, "")) {
        const insert = quoted(item.label);
        items.push({
          label: item.label,
          detail: item.detail,
          insert,
          caret: insert.length,
          rank: 0,
        });
      }
    } else if (stringFn === "eth-named-frame") {
      const busChannel = catalogAliasBus(catalog, "eth", stringMessage ?? "");
      if (!busChannel) return null;
      for (const frame of busChannel.messages) {
        const insert = quoted(frame.name);
        items.push({
          label: frame.name,
          detail: `${stringMessage} frame`,
          insert,
          caret: insert.length,
          rank: 0,
        });
      }
    } else if (stringFn === "eth-named-signal") {
      const frame = catalogAliasBus(catalog, "eth", stringMessage ?? "")
        ?.messages.find((message) => message.name === stringFrame);
      if (!frame) return null;
      for (const signal of frame.signals) {
        const insert = quoted(signal.name);
        items.push({
          label: signal.name,
          detail: signal.unit
            ? `${frame.name} · ${signal.unit}`
            : frame.name,
          insert,
          caret: insert.length,
          rank: 0,
        });
      }
    } else if (stringFn === "fr-signal") {
      const bus = Number(stringMessage);
      const busChannel = catalogBus(catalog, "fr", bus);
      if (!busChannel) return null;
      const seen = new Set<string>();
      for (const frame of busChannel.messages) {
        for (const signal of frame.signals) {
          if (seen.has(signal.name)) continue;
          seen.add(signal.name);
          items.push({
            label: signal.name,
            detail: `FR${bus} · ${frame.name}`,
            insert: quoted(signal.name),
            caret: quoted(signal.name).length,
            rank: 0,
          });
        }
      }
    } else if (stringFn === "eth-frame") {
      const bus = Number(stringMessage);
      const busChannel = catalogBus(catalog, "eth", bus);
      if (!busChannel) return null;
      for (const frame of busChannel.messages) {
        items.push({
          label: frame.name,
          detail: `ETH${bus} frame`,
          insert: quoted(frame.name),
          caret: quoted(frame.name).length,
          rank: 0,
        });
      }
    } else if (stringFn === "eth-signal") {
      const bus = Number(stringMessage);
      const frame = catalogBus(catalog, "eth", bus)?.messages.find(
        (message) => message.name === stringFrame,
      );
      if (!frame) return null;
      for (const signal of frame.signals) {
        const insert = quoted(signal.name);
        items.push({
          label: signal.name,
          detail: signal.unit
            ? `${frame.name} · ${signal.unit}`
            : frame.name,
          insert,
          caret: insert.length,
          rank: 0,
        });
      }
    } else if (stringFn === "derived") {
      for (const definition of derived) {
        items.push({
          label: definition.name,
          detail: "derived channel",
          insert: definition.name,
          caret: definition.name.length,
          rank: 0,
        });
      }
    } else {
      for (const variable of sysvars) {
        const key = `${variable.namespace}::${variable.name}`;
        items.push({
          label: key,
          detail: "system variable",
          insert: key,
          caret: key.length,
          rank: 0,
        });
      }
    }
  } else {
    const locals = localSymbols(value, caret);
    const localNames = new Set(locals.map((local) => local.name));
    for (const local of locals) {
      const isFn = local.kind === "fn";
      items.push({
        label: local.name,
        detail: isFn
          ? "script function"
          : local.kind === "param"
            ? "local · parameter"
            : "local · let",
        insert: isFn ? `${local.name}()` : local.name,
        caret: isFn ? local.name.length + 1 : local.name.length,
        rank: 0,
      });
    }
    for (const fn of functions) {
      if (localNames.has(fn.name)) continue;
      const insert = `${fn.name}()`;
      items.push({
        label: fn.name,
        detail: fn.usage,
        insert,
        caret: fn.name.length + 1,
        rank: 0,
      });
    }
    for (const keyword of KEYWORDS) {
      if (localNames.has(keyword)) continue;
      items.push({
        label: keyword,
        detail: "keyword",
        insert: keyword,
        caret: keyword.length,
        rank: 1,
      });
    }
    const claimed = new Set(items.map((item) => item.label));
    for (const message of catalogMessages(catalog, channel)) {
      if (
        !/^[A-Za-z_]\w*$/.test(message.name) ||
        claimed.has(message.name)
      ) {
        continue;
      }
      claimed.add(message.name);
      items.push({
        label: message.name,
        detail: `CAN message · ${messageDetail(message)}`,
        insert: message.name,
        caret: message.name.length,
        rank: 2,
      });
    }
    for (const definition of derived) {
      if (localNames.has(definition.name) || claimed.has(definition.name)) {
        continue;
      }
      claimed.add(definition.name);
      const insert = `derived("${quoted(definition.name)}")`;
      items.push({
        label: definition.name,
        detail: "derived channel",
        insert,
        caret: insert.length,
        rank: 2,
      });
    }
    for (const variable of sysvars) {
      const key = `${variable.namespace}::${variable.name}`;
      const insert = `sys_get("${quoted(key)}")`;
      items.push({
        label: key,
        detail: "system variable",
        insert,
        caret: insert.length,
        rank: 3,
      });
    }
  }

  const query = prefix.toLowerCase();
  const matches = items
    .filter(
      (item) => query === "" || item.label.toLowerCase().includes(query),
    )
    .sort((left, right) => {
      const a = left.label.toLowerCase();
      const b = right.label.toLowerCase();
      const aq = a === query ? 0 : a.startsWith(query) ? 1 : 2;
      const bq = b === query ? 0 : b.startsWith(query) ? 1 : 2;
      // Stable sort: equal candidates keep the language table's order, which
      // puts the everyday functions (print, send, sig, …) first.
      return aq - bq || left.rank - right.rank;
    })
    .slice(0, 12);
  if (matches.length === 0) return null;
  return { start, end, items: matches };
}

const TEMPLATE = `on start {
    print("node online");
}

on timer 1000 {
    // set_sig("EngineSpeed", 1000);
}
`;

/// Contextual event words after `on`. They are not reserved by the parser, so
/// they stay out of KEYWORDS (and its highlighter) and only appear here.
const EVENTS: [string, string][] = [
  ["start", "runs once when the measurement starts"],
  ["stop", "runs once when the measurement stops"],
  ["panel_action", "Panel button action by name"],
  ["message", "frame by id, name or *"],
  ["extended", "29-bit frame by id"],
  ["timer", "periodic tick or named one-shot"],
  ["errorFrame", "error frames"],
  ["fr", "FlexRay slot arrival"],
];

const KEYWORDS = new Set([
  "on",
  "start",
  "stop",
  "timer",
  "if",
  "else",
  "for",
  "while",
  "return",
  "let",
  "var",
  "const",
  "true",
  "false",
  "break",
  "continue",
  "this",
  "node",
]);

function escapeHtml(text: string): string {
  return text
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;");
}

export interface HighlightLine {
  html: string;
  /// The line ends inside a block comment.
  block: boolean;
}

const HL_TOKEN =
  /\/\/[^\n]*|\/\*[\s\S]*?\*\/|"(?:\\.|[^"\\])*"|'(?:\\.|[^'\\])*'|\b\d+(?:\.\d+)?\b|[A-Za-z_]\w*/g;

/// A deliberately small highlighter: comments, strings, numbers, keywords
/// and call names. It paints the <pre> behind the textarea; the textarea
/// itself keeps the caret and input handling. Lines are highlighted one at
/// a time so an edit only re-tokenizes the lines whose text or comment
/// state actually changed.
export function highlightLine(
  line: string,
  startBlock: boolean,
  nextParen: boolean,
): HighlightLine {
  let out = "";
  let at = 0;
  if (startBlock) {
    const end = line.indexOf("*/");
    if (end < 0) {
      return {
        html: `<span class="tok-comment">${escapeHtml(line)}</span>`,
        block: true,
      };
    }
    const stop = end + 2;
    out += `<span class="tok-comment">${escapeHtml(line.slice(0, stop))}</span>`;
    at = stop;
  }
  const rest = line.slice(at);
  HL_TOKEN.lastIndex = 0;
  let last = 0;
  for (const match of rest.matchAll(HL_TOKEN)) {
    const index = match.index ?? 0;
    const gap = rest.slice(last, index);
    const open = gap.indexOf("/*");
    if (open >= 0) {
      out += escapeHtml(gap.slice(0, open));
      out += `<span class="tok-comment">${escapeHtml(rest.slice(last + open))}</span>`;
      return { html: out, block: true };
    }
    out += escapeHtml(gap);
    const text = match[0];
    last = index + text.length;
    if (text.startsWith("//") || text.startsWith("/*")) {
      out += `<span class="tok-comment">${escapeHtml(text)}</span>`;
    } else if (text.startsWith('"') || text.startsWith("'")) {
      out += `<span class="tok-string">${escapeHtml(text)}</span>`;
    } else if (/^\d/.test(text)) {
      out += `<span class="tok-number">${text}</span>`;
    } else if (KEYWORDS.has(text)) {
      out += `<span class="tok-keyword">${text}</span>`;
    } else {
      const after = rest.slice(last);
      const call = /^\s*\(/.test(after) || (after.trim() === "" && nextParen);
      out += call
        ? `<span class="tok-fn">${text}</span>`
        : escapeHtml(text);
    }
  }
  const tail = rest.slice(last);
  const open = tail.indexOf("/*");
  if (open >= 0) {
    out += escapeHtml(tail.slice(0, open));
    out += `<span class="tok-comment">${escapeHtml(tail.slice(open))}</span>`;
    return { html: out, block: true };
  }
  out += escapeHtml(tail);
  return { html: out, block: false };
}

interface HighlightState {
  script: number | null;
  entries: HighlightLine[];
  nodes: HTMLDivElement[];
  cache: Map<string, HighlightLine>;
}

function busName(buses: Bus[], channel: number): string {
  return buses[channel]?.name ?? `CAN${channel + 1}`;
}

export function ScriptsModal({
  buses,
  catalog,
  derived,
  sysvars,
  onCommand,
  onClose,
}: Props) {
  const [scripts, setScripts] = useState<Script[]>([]);
  const [drafts, setDrafts] = useState<Record<number, string>>({});
  const [dirty, setDirty] = useState<Set<number>>(new Set());
  const [selected, setSelected] = useState<number | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState("");
  const [composing, setComposing] = useState(false);
  const highlightRef = useRef<HTMLPreElement>(null);
  const hlStateRef = useRef<HighlightState | null>(null);
  const textareaRef = useRef<HTMLTextAreaElement>(null);
  const [functions, setFunctions] = useState<ScriptFunctionHelp[]>([]);
  const [completion, setCompletion] = useState<Completion | null>(null);
  const [signature, setSignature] = useState<SignatureHint | null>(null);
  const [findOpen, setFindOpen] = useState(false);
  const [findText, setFindText] = useState("");
  const [replaceText, setReplaceText] = useState("");
  const findInputRef = useRef<HTMLInputElement>(null);
  const replaceInputRef = useRef<HTMLInputElement>(null);
  const issueLineRef = useRef<HTMLDivElement>(null);
  const [reload, setReload] = useState(0);
  const editing = useRef<Set<number>>(new Set());
  const selectNewest = useRef(false);
  const [name, setName] = useState("");
  const [savePath, setSavePath] = useState("");
  const [newOpen, setNewOpen] = useState(false);
  const [newName, setNewName] = useState("");
  const [newChannel, setNewChannel] = useState(0);
  const [newAttach, setNewAttach] = useState("");
  const [debugArmed, setDebugArmed] = useState(false);
  const [debugView, setDebugView] = useState<DebugView | null>(null);
  const [breakpointMap, setBreakpointMap] = useState<Record<number, number[]>>(
    {},
  );
  const [debugError, setDebugError] = useState("");
  const gutterRef = useRef<HTMLDivElement>(null);
  const gutterNumsRef = useRef<HTMLPreElement>(null);
  const debugLineRef = useRef<HTMLDivElement>(null);
  const debugArmedRef = useRef(false);

  useEffect(() => {
    let alive = true;
    void fetchScriptHelp().then((items) => {
      if (alive) setFunctions(items);
    });
    return () => {
      alive = false;
    };
  }, []);

  useEffect(() => {
    // A different script has a different caret context.
    setCompletion(null);
    setSignature(null);
    setFindOpen(false);
    setFindText("");
    setReplaceText("");
    if (debugArmedRef.current) {
      void sendDebug({ action: "stop" }).catch(() => {});
    }
    setDebugArmed(false);
    setDebugView(null);
    setDebugError("");
  }, [selected]);

  useEffect(() => {
    debugArmedRef.current = debugArmed;
  }, [debugArmed]);

  useEffect(
    () => () => {
      if (debugArmedRef.current) {
        void sendDebug({ action: "stop" }).catch(() => {});
      }
    },
    [],
  );

  useEffect(() => {
    if (!debugArmed) return;
    let alive = true;
    const poll = async () => {
      try {
        const view = await fetchDebug();
        if (!alive) return;
        setDebugView(view);
        if (view.note) setDebugError(view.note);
        if (!view.enabled) setDebugArmed(false);
      } catch (e) {
        if (alive) setDebugError(String(e));
      }
    };
    void poll();
    const timer = setInterval(poll, 400);
    return () => {
      alive = false;
      clearInterval(timer);
    };
  }, [debugArmed]);

  useEffect(() => {
    if (findOpen) {
      findInputRef.current?.focus();
      findInputRef.current?.select();
    }
  }, [findOpen]);

  useEffect(() => {
    const close = (event: KeyboardEvent) => {
      if (event.key === "Escape") onClose();
    };
    window.addEventListener("keydown", close);
    return () => window.removeEventListener("keydown", close);
  }, [onClose]);

  useEffect(() => {
    let alive = true;
    const load = async () => {
      try {
        const next = await fetchScripts();
        if (!alive) return;
        setScripts(next);
        setDrafts((current) => {
          const merged = { ...current };
          for (const script of next) {
            if (!editing.current.has(script.id)) merged[script.id] = script.source;
          }
          return merged;
        });
        setSelected((current) => {
          if (selectNewest.current) {
            selectNewest.current = false;
            const newest = next.reduce<Script | null>(
              (best, script) => (best == null || script.id > best.id ? script : best),
              null,
            );
            return newest?.id ?? null;
          }
          return current != null && next.some((script) => script.id === current)
            ? current
            : (next[0]?.id ?? null);
        });
        setError("");
      } catch (e) {
        if (alive) setError(String(e));
      } finally {
        if (alive) setLoading(false);
      }
    };
    void load();
    const timer = setInterval(load, 1200);
    return () => {
      alive = false;
      clearInterval(timer);
    };
  }, [reload]);

  const current = scripts.find((script) => script.id === selected) ?? null;
  const draft = selected != null ? (drafts[selected] ?? current?.source ?? "") : "";
  const breakpoints = selected != null ? (breakpointMap[selected] ?? []) : [];
  const debugPaused =
    debugArmed && debugView?.paused === true && debugView.node === current?.id;
  const debugLine = debugPaused ? (debugView?.line ?? null) : null;

  useLayoutEffect(() => {
    const pre = highlightRef.current;
    if (!pre) return;
    const lines = draft.split("\n");
    const gutter = gutterNumsRef.current;
    if (gutter && gutter.dataset.lines !== String(lines.length)) {
      const numbers: string[] = [];
      for (let i = 1; i <= lines.length; i += 1) numbers.push(String(i));
      gutter.textContent = numbers.join("\n");
      gutter.dataset.lines = String(lines.length);
    }
    const previous =
      hlStateRef.current?.script === selected ? hlStateRef.current : null;
    const cache = new Map<string, HighlightLine>();
    const entries: HighlightLine[] = [];
    let block = false;
    for (let i = 0; i < lines.length; i += 1) {
      const nextParen = /^\s*\(/.test(lines[i + 1] ?? "");
      const key: string = `${block ? 1 : 0}${nextParen ? 1 : 0}${lines[i]}`;
      let entry: HighlightLine | undefined =
        previous?.cache.get(key) ?? cache.get(key);
      if (!entry) entry = highlightLine(lines[i], block, nextParen);
      cache.set(key, entry);
      entries.push(entry);
      block = entry.block;
    }

    const rebuild = () => {
      pre.innerHTML = entries
        .map((entry) => `<div class="hl-line">${entry.html}</div>`)
        .join("");
      const nodes = Array.from(pre.children) as HTMLDivElement[];
      hlStateRef.current = { script: selected, entries, nodes, cache };
    };

    if (!previous) {
      rebuild();
      return;
    }

    const prefixLimit = Math.min(previous.entries.length, entries.length);
    let prefix = 0;
    while (
      prefix < prefixLimit &&
      previous.entries[prefix].html === entries[prefix].html
    ) {
      prefix += 1;
    }
    let suffix = 0;
    while (
      suffix < previous.entries.length - prefix &&
      suffix < entries.length - prefix &&
      previous.entries[previous.entries.length - 1 - suffix].html ===
        entries[entries.length - 1 - suffix].html
    ) {
      suffix += 1;
    }
    const oldMiddle = previous.entries.length - prefix - suffix;
    const newMiddle = entries.length - prefix - suffix;
    if (oldMiddle + newMiddle > 600) {
      rebuild();
      return;
    }
    if (oldMiddle === newMiddle) {
      const nodes = previous.nodes;
      for (let i = prefix; i < prefix + newMiddle; i += 1) {
        if (previous.entries[i].html !== entries[i].html) {
          nodes[i].innerHTML = entries[i].html;
        }
      }
      hlStateRef.current = { script: selected, entries, nodes, cache };
      return;
    }

    const fragment = document.createDocumentFragment();
    const added: HTMLDivElement[] = [];
    for (let i = prefix; i < entries.length - suffix; i += 1) {
      const node = document.createElement("div");
      node.className = "hl-line";
      node.innerHTML = entries[i].html;
      fragment.appendChild(node);
      added.push(node);
    }
    const before = previous.nodes[prefix] ?? null;
    pre.insertBefore(fragment, before);
    for (let i = prefix; i < previous.nodes.length - suffix; i += 1) {
      previous.nodes[i].remove();
    }
    const nodes = [
      ...previous.nodes.slice(0, prefix),
      ...added,
      ...previous.nodes.slice(previous.nodes.length - suffix),
    ];
    hlStateRef.current = { script: selected, entries, nodes, cache };
  }, [draft, selected]);

  /// Both the compile-error stripe and the paused line share the textarea's
  /// geometry, so they are placed together on every edit, scroll and pause.
  const syncEditorOverlays = (textarea: HTMLTextAreaElement | null) => {
    if (!textarea) return;
    const style = getComputedStyle(textarea);
    const lineHeight =
      Number.parseFloat(style.lineHeight) ||
      (Number.parseFloat(style.fontSize) || 11.5) * 1.45;
    const padTop = Number.parseFloat(style.paddingTop) || 0;
    const place = (marker: HTMLDivElement | null, line: number | null) => {
      if (!marker) return;
      if (line == null) {
        marker.style.display = "none";
        return;
      }
      const top = padTop + (line - 1) * lineHeight - textarea.scrollTop;
      marker.style.top = `${top}px`;
      marker.style.height = `${lineHeight}px`;
      marker.style.display =
        top + lineHeight < 0 || top > textarea.clientHeight ? "none" : "block";
    };
    const issueLine =
      current?.issue && selected != null && !dirty.has(selected)
        ? current.issue.line
        : null;
    place(issueLineRef.current, issueLine);
    place(debugLineRef.current, debugLine);
  };

  useEffect(() => {
    syncEditorOverlays(textareaRef.current);
  }, [current?.issue, selected, dirty, debugLine]);

  /// A new paused line scrolls into view without stealing the caret from
  /// an edit the user may be making while the node is parked.
  useEffect(() => {
    const textarea = textareaRef.current;
    if (!textarea || debugLine == null) return;
    revealPosition(textarea, lineOffset(textarea.value, debugLine));
    const pre = highlightRef.current;
    if (pre) {
      pre.scrollTop = textarea.scrollTop;
      pre.scrollLeft = textarea.scrollLeft;
    }
    if (gutterRef.current) gutterRef.current.scrollTop = textarea.scrollTop;
    syncEditorOverlays(textarea);
  }, [debugLine]);

  const lineAtCaret = (textarea: HTMLTextAreaElement | null) =>
    textarea
      ? textarea.value.slice(0, textarea.selectionStart).split("\n").length
      : 1;

  const revealLine = (line: number) => {
    const textarea = textareaRef.current;
    if (!textarea) return;
    const at = lineOffset(textarea.value, line);
    textarea.focus();
    textarea.setSelectionRange(at, at);
    revealPosition(textarea, at);
    syncEditorOverlays(textarea);
  };

  const toggleBreakpoint = (line: number) => {
    if (selected == null) return;
    const key = selected;
    const next = breakpoints.includes(line)
      ? breakpoints.filter((item) => item !== line)
      : [...breakpoints, line].sort((a, b) => a - b);
    setBreakpointMap((map) => ({ ...map, [key]: next }));
    if (debugArmedRef.current) {
      void sendDebug({ action: "set_breakpoints", lines: next }).catch((e) =>
        setDebugError(String(e)),
      );
    }
  };

  const toggleBreakpointAtCaret = () =>
    toggleBreakpoint(lineAtCaret(textareaRef.current));

  const armDebugger = async () => {
    if (!current) return;
    setDebugError("");
    try {
      const view = await sendDebug({
        action: "arm",
        node_id: current.id,
        lines: breakpoints,
      });
      setDebugView(view);
      setDebugArmed(true);
    } catch (e) {
      setDebugError(String(e));
    }
  };

  const stopDebugger = async () => {
    setDebugArmed(false);
    setDebugError("");
    try {
      setDebugView(await sendDebug({ action: "stop" }));
    } catch (e) {
      setDebugError(String(e));
    }
  };

  const runDebug = async (action: "continue" | "step" | "pause") => {
    try {
      setDebugView(await sendDebug({ action }));
    } catch (e) {
      setDebugError(String(e));
    }
  };

  useEffect(() => {
    setName(current?.name ?? "");
    setSavePath(current?.filePath || (current ? `${current.name}.rxcan` : ""));
    // Only reset when the selection changes; polling must not overwrite
    // the name or path boxes while the user types.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [selected]);

  const edit = (value: string) => {
    if (selected == null) return;
    editing.current.add(selected);
    setCompletion(null);
    setDrafts((currentDrafts) => ({ ...currentDrafts, [selected]: value }));
    setDirty((currentDirty) => new Set(currentDirty).add(selected));
  };

  const refreshCompletion = (
    textarea: HTMLTextAreaElement,
    allowEmpty: boolean,
    manual = false,
  ) => {
    if (composing) {
      setCompletion(null);
      return;
    }
    const caret = textarea.selectionStart ?? 0;
    const candidates = buildCandidates(
      textarea.value,
      caret,
      allowEmpty,
      functions,
      derived,
      sysvars,
      catalog,
      current?.channel ?? 0,
    );
    if (!candidates) {
      setCompletion(null);
      return;
    }
    const placement = completionPlacement(
      textarea,
      caret,
      candidates.items.length,
    );
    setCompletion({ ...candidates, active: 0, manual, ...placement });
  };

  const refreshSignature = (textarea: HTMLTextAreaElement) => {
    const caret = textarea.selectionStart ?? 0;
    const call = callAtCaret(textarea.value, caret);
    if (!call) {
      setSignature(null);
      return;
    }
    const builtin = functions.find((item) => item.name === call.name);
    const local = builtin
      ? null
      : localSymbols(textarea.value, caret).find(
          (symbol) => symbol.kind === "fn" && symbol.name === call.name,
        );
    if (!builtin && !local) {
      setSignature(null);
      return;
    }
    setSignature({
      name: call.name,
      args: builtin ? signatureArgs(builtin.usage) : (local?.params ?? []),
      active: call.active,
      ...caretPlacement(textarea, caret, 340, 28),
    });
  };

  const applyCompletion = (item: CompletionItem) => {
    const textarea = textareaRef.current;
    if (!textarea || !completion) return;
    replaceRange(
      textarea,
      completion.start,
      completion.end,
      item.insert,
      completion.start + item.caret,
    );
    setCompletion(null);
  };

  const handleEnter = (event: KeyboardEvent) => {
    const textarea = textareaRef.current;
    if (!textarea) return;
    event.preventDefault();
    const value = textarea.value;
    const start = textarea.selectionStart;
    const end = textarea.selectionEnd;
    const from = lineStartOf(value, start);
    const before = value.slice(from, start);
    const after = value.slice(start, lineEndOf(value, start));
    let indent = leadingSpace(value.slice(from));
    if (before.trimEnd().endsWith("{")) {
      indent += INDENT;
    } else if (/^[ \t]*\}/.test(after)) {
      indent = indent.slice(0, Math.max(0, indent.length - INDENT.length));
    }
    const insert = `\n${indent}`;
    replaceRange(textarea, start, end, insert, start + insert.length);
  };

  const handleTab = (event: KeyboardEvent) => {
    const textarea = textareaRef.current;
    if (!textarea) return;
    event.preventDefault();
    const value = textarea.value;
    const start = textarea.selectionStart;
    const end = textarea.selectionEnd;

    if (start !== end) {
      const from = lineStartOf(value, start);
      const to = lineEndOf(value, end);
      const lines = value.slice(from, to).split("\n");
      const shifted = lines.map((line) =>
        event.shiftKey ? line.replace(/^\t|^ {1,4}/, "") : `${INDENT}${line}`,
      );
      const next = shifted.join("\n");
      if (next !== value.slice(from, to)) {
        replaceRange(textarea, from, to, next, from, from + next.length);
      }
      return;
    }

    if (event.shiftKey) {
      const from = lineStartOf(value, start);
      const to = lineEndOf(value, start);
      const line = value.slice(from, to);
      const next = line.replace(/^\t|^ {1,4}/, "");
      if (next !== line) {
        replaceRange(
          textarea,
          from,
          to,
          next,
          Math.max(from, start - (line.length - next.length)),
        );
      }
      return;
    }

    replaceRange(textarea, start, end, INDENT, start + INDENT.length);
  };

  /// Typing `}` on an auto-indented line moves it out one level instead of
  /// leaving the closing brace at the block's inner indentation.
  const handleClosingBrace = (event: KeyboardEvent) => {
    const textarea = textareaRef.current;
    if (!textarea) return;
    const start = textarea.selectionStart;
    if (start !== textarea.selectionEnd) return;
    const from = lineStartOf(textarea.value, start);
    const prefix = textarea.value.slice(from, start);
    if (prefix.length < INDENT.length || !/^ +$/.test(prefix)) return;
    event.preventDefault();
    const shortened = prefix.slice(0, -INDENT.length);
    replaceRange(
      textarea,
      from,
      start,
      `${shortened}}`,
      from + shortened.length + 1,
    );
    refreshSignature(textarea);
  };

  /// Auto-closes `(`, `[` and `{`; wraps a selection; and skips over a closer
  /// that is already under the caret. The `}` skip still runs the block
  /// outdent, since an auto-closed brace sits on an indented line.
  const handleBracket = (event: KeyboardEvent) => {
    const textarea = textareaRef.current;
    if (!textarea) return;
    if (event.ctrlKey || event.metaKey || event.altKey) return;
    const start = textarea.selectionStart;
    const end = textarea.selectionEnd;

    const close = BRACKET_PAIRS[event.key];
    if (close) {
      event.preventDefault();
      if (start !== end) {
        const selected = textarea.value.slice(start, end);
        replaceRange(
          textarea,
          start,
          end,
          `${event.key}${selected}${close}`,
          start + 1,
          end + 1,
        );
        setCompletion(null);
        setSignature(null);
      } else {
        if (
          textarea.value[start] !== close &&
          !/\w/.test(textarea.value[start] ?? "")
        ) {
          replaceRange(
            textarea,
            start,
            end,
            `${event.key}${close}`,
            start + 1,
          );
        } else {
          replaceRange(textarea, start, end, event.key, start + 1);
        }
        setCompletion(null);
        // replaceRange fires input before moving the caret inside, so the
        // call context must be read once more from the opening bracket.
        refreshSignature(textarea);
      }
      return;
    }

    if (!BRACKET_CLOSERS.has(event.key) || start !== end) return;
    if (textarea.value[start] !== event.key) {
      if (event.key === "}") handleClosingBrace(event);
      return;
    }
    if (event.key === "}") {
      const from = lineStartOf(textarea.value, start);
      const prefix = textarea.value.slice(from, start);
      if (prefix.length >= INDENT.length && /^ +$/.test(prefix)) {
        event.preventDefault();
        const shortened = prefix.slice(0, -INDENT.length);
        replaceRange(
          textarea,
          from,
          start + 1,
          `${shortened}}`,
          from + shortened.length + 1,
        );
        setCompletion(null);
        refreshSignature(textarea);
        return;
      }
    }
    event.preventDefault();
    textarea.setSelectionRange(start + 1, start + 1);
    setCompletion(null);
    refreshSignature(textarea);
  };

  /// Outside a string a quote opens an empty pair; inside one the next quote
  /// closes it, and a quote already under the caret is stepped over.
  const handleQuote = (event: KeyboardEvent) => {
    const textarea = textareaRef.current;
    if (!textarea) return;
    if (event.ctrlKey || event.metaKey || event.altKey) return;
    const start = textarea.selectionStart;
    const end = textarea.selectionEnd;
    if (start !== end) {
      event.preventDefault();
      const selected = textarea.value.slice(start, end);
      replaceRange(textarea, start, end, `"${selected}"`, start + 1, end + 1);
      setCompletion(null);
      setSignature(null);
      return;
    }
    if (textarea.value[start] === '"') {
      event.preventDefault();
      textarea.setSelectionRange(start + 1, start + 1);
      setCompletion(null);
      refreshSignature(textarea);
      return;
    }
    if (inStringAt(textarea.value, start)) return;
    event.preventDefault();
    replaceRange(textarea, start, end, '""', start + 1);
    refreshCompletion(textarea, true);
    refreshSignature(textarea);
  };

  /// In leading whitespace, Backspace removes up to the previous indent stop
  /// instead of one space, matching the four-space INDENT above.
  const handleBackspace = (event: KeyboardEvent) => {
    const textarea = textareaRef.current;
    if (!textarea) return;
    const start = textarea.selectionStart;
    if (start !== textarea.selectionEnd) return;
    const from = lineStartOf(textarea.value, start);
    const prefix = textarea.value.slice(from, start);
    if (prefix.length === 0 || !/^ +$/.test(prefix)) return;
    event.preventDefault();
    const remove =
      prefix.length -
      Math.floor((prefix.length - 1) / INDENT.length) * INDENT.length;
    replaceRange(textarea, start - remove, start, "", start - remove);
    setCompletion(null);
  };

  const handleComment = (event: KeyboardEvent) => {
    const textarea = textareaRef.current;
    if (!textarea) return;
    const value = textarea.value;
    const start = textarea.selectionStart;
    const end = textarea.selectionEnd;
    const from = lineStartOf(value, start);
    const to = lineEndOf(value, end);
    const lines = value.slice(from, to).split("\n");
    const nonEmpty = lines.filter((line) => line.trim() !== "");
    const commented =
      nonEmpty.length > 0 && nonEmpty.every((line) => /^\s*\/\//.test(line));
    const nextLines = lines.map((line) => {
      if (!commented) {
        const indent = /^\s*/.exec(line)?.[0] ?? "";
        return `${indent}// ${line.slice(indent.length)}`;
      }
      const match = /^(\s*)\/\/ ?/.exec(line);
      return match ? `${match[1]}${line.slice(match[0].length)}` : line;
    });
    event.preventDefault();
    const next = nextLines.join("\n");
    if (next === value.slice(from, to)) return;

    let caret = start;
    if (start === end && lines.length === 1) {
      const indent = /^\s*/.exec(lines[0])?.[0].length ?? 0;
      const at = from + indent;
      if (commented) {
        const removed = lines[0].length - nextLines[0].length;
        caret = start <= at ? start : Math.max(at, start - removed);
      } else {
        caret =
          start >= at ? start + nextLines[0].length - lines[0].length : start;
      }
    }
    replaceRange(
      textarea,
      from,
      to,
      next,
      start === end ? caret : from,
      start === end ? caret : from + next.length,
    );
    setCompletion(null);
    setSignature(null);
  };

  const handleMoveLine = (event: KeyboardEvent, direction: -1 | 1) => {
    const textarea = textareaRef.current;
    if (!textarea) return;
    const value = textarea.value;
    const start = textarea.selectionStart;
    const end = textarea.selectionEnd;
    const from = lineStartOf(value, start);
    const effectiveEnd =
      end > start && end === lineStartOf(value, end) ? end - 1 : end;
    const to = lineEndOf(value, effectiveEnd);
    const block = value.slice(from, to);
    if (direction < 0) {
      if (from === 0) return;
      const prevStart = lineStartOf(value, from - 1);
      const prev = value.slice(prevStart, from - 1);
      const shift = from - prevStart;
      event.preventDefault();
      replaceRange(
        textarea,
        prevStart,
        to,
        `${block}\n${prev}`,
        start - shift,
        end - shift,
      );
    } else {
      if (to >= value.length) return;
      const nextStart = to + 1;
      const nextEnd = lineEndOf(value, nextStart);
      const nextLine = value.slice(nextStart, nextEnd);
      const shift = nextLine.length + 1;
      event.preventDefault();
      replaceRange(
        textarea,
        from,
        nextEnd,
        `${nextLine}\n${block}`,
        start + shift,
        end + shift,
      );
    }
    setCompletion(null);
    setSignature(null);
  };

  const handleDuplicateLine = (event: KeyboardEvent) => {
    const textarea = textareaRef.current;
    if (!textarea) return;
    const value = textarea.value;
    const start = textarea.selectionStart;
    const end = textarea.selectionEnd;
    const from = lineStartOf(value, start);
    const effectiveEnd =
      end > start && end === lineStartOf(value, end) ? end - 1 : end;
    const block = value.slice(from, lineEndOf(value, effectiveEnd));
    event.preventDefault();
    const added = block.length + 1;
    replaceRange(
      textarea,
      lineEndOf(value, effectiveEnd),
      lineEndOf(value, effectiveEnd),
      `\n${block}`,
      start + added,
      end + added,
    );
    setCompletion(null);
    setSignature(null);
  };

  const handleSmartHome = (event: KeyboardEvent) => {
    const textarea = textareaRef.current;
    if (!textarea) return;
    if (textarea.selectionStart !== textarea.selectionEnd) return;
    const start = textarea.selectionStart;
    const from = lineStartOf(textarea.value, start);
    const indent = leadingSpace(textarea.value.slice(from)).length;
    const first = from + indent;
    const target = start === first ? from : first;
    event.preventDefault();
    textarea.setSelectionRange(target, target);
    setCompletion(null);
    setSignature(null);
  };

  const jumpToIssue = () => {
    const textarea = textareaRef.current;
    const issue = current?.issue;
    if (!textarea || !issue) return;
    const base = lineOffset(textarea.value, issue.line);
    const end = lineEndOf(textarea.value, base);
    const position = Math.min(
      base + Math.max(0, (issue.col ?? 1) - 1),
      end,
    );
    textarea.focus();
    textarea.setSelectionRange(position, position);
    revealPosition(textarea, position);
  };

  const findNext = (direction: -1 | 1) => {
    const textarea = textareaRef.current;
    if (!textarea || !findText) return;
    const hits = findMatches(textarea.value, findText);
    if (hits.length === 0) return;
    const start = textarea.selectionStart;
    const end = textarea.selectionEnd;
    let index = -1;
    if (direction > 0) {
      const from = start === end ? start : end;
      index = hits.findIndex((hit) => hit >= from);
      if (index < 0) index = 0;
    } else {
      for (let i = hits.length - 1; i >= 0; i -= 1) {
        if (hits[i] < start) {
          index = i;
          break;
        }
      }
      if (index < 0) index = hits.length - 1;
    }
    textarea.setSelectionRange(hits[index], hits[index] + findText.length);
    revealPosition(textarea, hits[index]);
  };

  const replaceCurrent = () => {
    const textarea = textareaRef.current;
    if (!textarea || !findText) return;
    const start = textarea.selectionStart;
    const end = textarea.selectionEnd;
    if (
      textarea.value.slice(start, end).toLowerCase() !== findText.toLowerCase()
    ) {
      findNext(1);
      return;
    }
    replaceRange(
      textarea,
      start,
      end,
      replaceText,
      start + replaceText.length,
    );
    findNext(1);
    replaceInputRef.current?.focus();
  };

  const replaceAll = () => {
    const textarea = textareaRef.current;
    if (!textarea || !findText) return;
    const pattern = new RegExp(escapeRegExp(findText), "gi");
    const next = textarea.value.replace(pattern, () => replaceText);
    if (next === textarea.value) return;
    const caret = Math.min(textarea.selectionStart, next.length);
    replaceRange(textarea, 0, textarea.value.length, next, caret);
    replaceInputRef.current?.focus();
  };

  const closeFind = () => {
    setFindOpen(false);
    textareaRef.current?.focus();
  };

  const handleEditorKeyDown = (event: KeyboardEvent) => {
    const textarea = event.currentTarget as HTMLTextAreaElement | null;
    if (!textarea) return;
    if (composing || event.isComposing || event.keyCode === 229) return;

    if (event.key === "F9") {
      event.preventDefault();
      toggleBreakpointAtCaret();
      return;
    }

    if (
      event.key === "Backspace" &&
      !event.ctrlKey &&
      !event.metaKey &&
      !event.altKey &&
      !event.shiftKey
    ) {
      handleBackspace(event);
    }

    if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === "f") {
      event.preventDefault();
      setFindOpen(true);
      findInputRef.current?.focus();
      findInputRef.current?.select();
      setCompletion(null);
      setSignature(null);
      return;
    }
    if ((event.ctrlKey || event.metaKey) && event.key === "/") {
      handleComment(event);
      return;
    }
    if (event.altKey && !event.ctrlKey && !event.metaKey) {
      if (event.key === "ArrowUp" && !event.shiftKey) {
        handleMoveLine(event, -1);
        return;
      }
      if (event.key === "ArrowDown" && event.shiftKey) {
        handleDuplicateLine(event);
        return;
      }
      if (event.key === "ArrowDown") {
        handleMoveLine(event, 1);
        return;
      }
    }
    if (
      event.key === "Home" &&
      !event.shiftKey &&
      !event.ctrlKey &&
      !event.metaKey &&
      !event.altKey
    ) {
      handleSmartHome(event);
      return;
    }

    // Alt+/ rather than Ctrl+Space: the latter is the Chinese IME's
    // on/off toggle on Windows and never reaches the page.
    if (event.altKey && (event.key === "/" || event.code === "Slash")) {
      event.preventDefault();
      refreshCompletion(textarea, true, true);
      return;
    }

    if (completion) {
      if (event.key === "ArrowDown" || event.key === "ArrowUp") {
        event.preventDefault();
        const delta = event.key === "ArrowDown" ? 1 : -1;
        setCompletion((current) =>
          current
            ? {
                ...current,
                active:
                  (current.active + delta + current.items.length) %
                  current.items.length,
                manual: true,
              }
            : current,
        );
        return;
      }
      if (event.key === "Enter" || event.key === "Tab") {
        // Tab always accepts, but Enter must not turn a newline into a
        // completion just because typing opened the list.
        if (event.key === "Tab" || completion.manual) {
          event.preventDefault();
          applyCompletion(completion.items[completion.active]);
          return;
        }
        setCompletion(null);
      }
      if (event.key === "Escape") {
        event.preventDefault();
        event.stopPropagation();
        setCompletion(null);
        return;
      }
      if (
        event.key === "ArrowLeft" ||
        event.key === "ArrowRight" ||
        event.key === "Home" ||
        event.key === "End"
      ) {
        setCompletion(null);
      }
    }
    if (event.key === "Escape" && findOpen) {
      event.preventDefault();
      event.stopPropagation();
      closeFind();
      return;
    }

    if (
      event.key === "Enter" &&
      !event.shiftKey &&
      !event.ctrlKey &&
      !event.metaKey &&
      !event.altKey
    ) {
      handleEnter(event);
      return;
    }
    if (event.key === "Tab") {
      handleTab(event);
      return;
    }
    if (
      (event.key === "(" ||
        event.key === ")" ||
        event.key === "[" ||
        event.key === "]" ||
        event.key === "{" ||
        event.key === "}") &&
      !event.ctrlKey &&
      !event.metaKey &&
      !event.altKey
    ) {
      handleBracket(event);
    }
    if (
      event.key === '"' &&
      !event.ctrlKey &&
      !event.metaKey &&
      !event.altKey
    ) {
      handleQuote(event);
    }
  };

  const apply = () => {
    if (!current) return;
    onCommand({ type: "set_script_source", id: current.id, source: draft });
    editing.current.delete(current.id);
    setDirty((currentDirty) => {
      const next = new Set(currentDirty);
      next.delete(current.id);
      return next;
    });
    setScripts((list) =>
      list.map((script) =>
        script.id === current.id
          ? { ...script, source: draft, issue: null }
          : script,
      ),
    );
  };

  const revert = () => {
    if (!current) return;
    editing.current.delete(current.id);
    setDrafts((currentDrafts) => ({ ...currentDrafts, [current.id]: current.source }));
    setDirty((currentDirty) => {
      const next = new Set(currentDirty);
      next.delete(current.id);
      return next;
    });
  };

  const add = () => {
    const channel = newChannel;
    const [busText, node] = newAttach.split(":");
    const attached_bus = busText === "" ? null : Number(busText);
    onCommand({
      type: "add_script",
      name: newName.trim() || `Node ${scripts.length + 1}`,
      channel,
      attached_bus,
      attached_node: attached_bus == null ? null : node,
    });
    setNewOpen(false);
    setNewName("");
    setNewAttach("");
    selectNewest.current = true;
    setTimeout(() => setReload((tick) => tick + 1), 700);
  };

  const issue = current?.issue ?? null;
  const showIssue = !!issue && selected != null && !dirty.has(selected);
  const findCount = findOpen && findText ? findMatches(draft, findText).length : 0;
  const debugStatus = debugError
    ? debugError
    : debugPaused
      ? `Paused at line ${debugLine}${
          debugView?.function ? ` in ${debugView.function}` : ""
        }. The node is blocked until Continue, Step or Stop.`
      : debugArmed
        ? "Armed — waiting for a breakpoint. Start or restart the measurement."
        : current && !current.enabled
          ? "Enable the script to debug it."
          : current && dirty.has(current.id)
            ? "Apply changes before debugging."
            : "F9 toggles a breakpoint on the caret line.";
  const scriptTests = current?.tests ?? [];
  const failedTests = scriptTests.filter((test) => test.failures > 0).length;

  return (
    <div class="modal-backdrop" onMouseDown={onClose}>
      <div
        class="scripts-modal"
        role="dialog"
        aria-modal="true"
        aria-label="Node scripts"
        onMouseDown={(event) => event.stopPropagation()}
      >
        <header class="modal-head">
          <h2>Node Scripts</h2>
          <div class="picker-head-right">
            <span>{scripts.length} scripts</span>
            <button type="button" onClick={() => setNewOpen((open) => !open)}>
              + New script
            </button>
            <button type="button" onClick={onClose}>
              Close
            </button>
          </div>
        </header>

        {newOpen && (
          <form
            class="script-new"
            onSubmit={(event) => {
              event.preventDefault();
              add();
            }}
          >
            <input
              value={newName}
              placeholder={`Node ${scripts.length + 1}`}
              onInput={(event) =>
                setNewName((event.target as HTMLInputElement).value)
              }
            />
            <label>
              Bus
              <select
                value={newChannel}
                onChange={(event) => {
                  setNewChannel(Number((event.target as HTMLSelectElement).value));
                  setNewAttach("");
                }}
              >
                {buses.map((bus) => (
                  <option key={bus.index} value={bus.index}>
                    {bus.name}
                  </option>
                ))}
              </select>
            </label>
            <label>
              Bind to
              <select
                value={newAttach}
                onChange={(event) =>
                  setNewAttach((event.target as HTMLSelectElement).value)
                }
              >
                <option value="">unbound script</option>
                {(buses[newChannel]?.nodes ?? []).map((node) => (
                  <option key={node.name} value={`${newChannel}:${node.name}`}>
                    {node.name}
                  </option>
                ))}
              </select>
            </label>
            <button type="submit">Add</button>
          </form>
        )}

        <div class="scripts-body">
          <aside class="scripts-list">
            {scripts.map((script) => (
              <button
                type="button"
                key={script.id}
                class={`script-item ${script.id === selected ? "on" : ""} ${
                  script.errored ? "bad" : ""
                }`}
                onClick={() => setSelected(script.id)}
              >
                <strong>
                  {script.name}
                  {dirty.has(script.id) && <i>•</i>}
                </strong>
                <span>
                  {busName(buses, script.channel)}
                  {script.attachedNode ? ` · ${script.attachedNode}` : ""}
                </span>
                <span class="script-chips">
                  {!script.enabled && <em>disabled</em>}
                  {script.running && <em class="ok">running</em>}
                  {script.errored && <em class="bad">error</em>}
                  {script.tests.length > 0 && (
                    <em
                      class={
                        script.tests.some((test) => test.failures > 0)
                          ? "bad"
                          : "ok"
                      }
                    >
                      {script.tests.filter((test) => test.failures === 0).length}
                      /{script.tests.length} tests
                    </em>
                  )}
                </span>
              </button>
            ))}
            {!loading && scripts.length === 0 && (
              <div class="script-empty">
                No scripts yet. Add one and bind it to a DBC node.
              </div>
            )}
            {loading && <div class="script-empty">Loading scripts…</div>}
            {error && <div class="script-empty bad">{error}</div>}
          </aside>

          <section class="scripts-editor">
            {current ? (
              <>
                <div class="script-meta">
                  <label>
                    Name
                    <input
                      value={name}
                      onInput={(event) =>
                        setName((event.target as HTMLInputElement).value)
                      }
                      onBlur={() => {
                        const value = name.trim();
                        if (value && value !== current.name) {
                          onCommand({
                            type: "set_script_name",
                            id: current.id,
                            name: value,
                          });
                        }
                      }}
                    />
                  </label>
                  <span>
                    {busName(buses, current.channel)}
                    {current.attachedNode
                      ? ` · bound to ${current.attachedNode}`
                      : " · unbound"}
                  </span>
                  <label class="script-enable">
                    <input
                      type="checkbox"
                      checked={current.enabled}
                      onChange={(event) =>
                        onCommand({
                          type: "set_script_enabled",
                          id: current.id,
                          on: (event.target as HTMLInputElement).checked,
                        })
                      }
                    />
                    Enabled
                  </label>
                </div>
                {findOpen && (
                  <div class="script-find">
                    <input
                      ref={findInputRef}
                      value={findText}
                      placeholder="Find…"
                      spellcheck={false}
                      onInput={(event) =>
                        setFindText((event.target as HTMLInputElement).value)
                      }
                      onKeyDown={(event) => {
                        if (event.key === "Enter") {
                          event.preventDefault();
                          findNext(event.shiftKey ? -1 : 1);
                        } else if (event.key === "Escape") {
                          event.preventDefault();
                          event.stopPropagation();
                          closeFind();
                        }
                      }}
                    />
                    <input
                      ref={replaceInputRef}
                      value={replaceText}
                      placeholder="Replace…"
                      spellcheck={false}
                      onInput={(event) =>
                        setReplaceText((event.target as HTMLInputElement).value)
                      }
                      onKeyDown={(event) => {
                        if (event.key === "Enter") {
                          event.preventDefault();
                          replaceCurrent();
                        } else if (event.key === "Escape") {
                          event.preventDefault();
                          event.stopPropagation();
                          closeFind();
                        }
                      }}
                    />
                    <span class="script-find-count">
                      {findCount} {findCount === 1 ? "match" : "matches"}
                    </span>
                    <button type="button" onClick={() => findNext(-1)}>
                      Prev
                    </button>
                    <button type="button" onClick={() => findNext(1)}>
                      Next
                    </button>
                    <button type="button" onClick={replaceCurrent}>
                      Replace
                    </button>
                    <button type="button" onClick={replaceAll}>
                      Replace all
                    </button>
                    <button type="button" onClick={closeFind}>
                      Close
                    </button>
                  </div>
                )}
                <div class={`script-debug-bar ${debugArmed ? "on" : ""}`}>
                  {debugArmed ? (
                    <>
                      <button
                        type="button"
                        class="primary"
                        disabled={!debugPaused}
                        onClick={() => void runDebug("continue")}
                      >
                        Continue
                      </button>
                      <button
                        type="button"
                        disabled={!debugPaused}
                        onClick={() => void runDebug("step")}
                      >
                        Step
                      </button>
                      <button
                        type="button"
                        disabled={debugPaused}
                        onClick={() => void runDebug("pause")}
                      >
                        Pause
                      </button>
                      <button
                        type="button"
                        disabled={debugPaused}
                        title={
                          debugPaused
                            ? "Continue or stop before restarting"
                            : "Apply the editor and restart this node"
                        }
                        onClick={apply}
                      >
                        Restart
                      </button>
                      <button type="button" onClick={() => void stopDebugger()}>
                        Stop
                      </button>
                    </>
                  ) : (
                    <button
                      type="button"
                      class="primary"
                      disabled={!current.enabled || dirty.has(current.id)}
                      title={
                        !current.enabled
                          ? "Enable the script first"
                          : dirty.has(current.id)
                            ? "Apply changes before debugging"
                            : "Pause on breakpoints; F9 toggles one"
                      }
                      onClick={() => void armDebugger()}
                    >
                      Debug
                    </button>
                  )}
                  <span class={`script-debug-status ${debugError ? "bad" : ""}`}>
                    {debugStatus}
                  </span>
                </div>
                <div class="script-code">
                  <div
                    ref={gutterRef}
                    class="script-gutter"
                    title="Click a line to toggle a breakpoint"
                    onClick={(event) => {
                      const area = textareaRef.current;
                      if (!area) return;
                      const gutter = event.currentTarget as HTMLDivElement;
                      const style = getComputedStyle(area);
                      const lineHeight =
                        Number.parseFloat(style.lineHeight) ||
                        (Number.parseFloat(style.fontSize) || 11.5) * 1.45;
                      const padTop =
                        Number.parseFloat(style.paddingTop) || 8;
                      const border =
                        Number.parseFloat(
                          getComputedStyle(gutter).borderTopWidth,
                        ) || 0;
                      const rect = gutter.getBoundingClientRect();
                      const y =
                        event.clientY -
                        rect.top -
                        border +
                        gutter.scrollTop -
                        padTop;
                      toggleBreakpoint(
                        Math.max(1, Math.floor(y / lineHeight) + 1),
                      );
                    }}
                  >
                    <pre ref={gutterNumsRef} class="script-gutter-nums" />
                    {breakpoints.map((line) => (
                      <span
                        key={`bp:${line}`}
                        class="bp"
                        style={{
                          top: `calc(8px + ${((line - 0.5) * 1.45).toFixed(2)}em)`,
                        }}
                      />
                    ))}
                    {debugLine != null && (
                      <span
                        class="cur"
                        style={{
                          top: `calc(8px + ${((debugLine - 1) * 1.45).toFixed(2)}em)`,
                        }}
                      />
                    )}
                  </div>
                  <div class="script-editor">
                  <pre
                    ref={highlightRef}
                    class="script-highlight"
                    aria-hidden="true"
                  />
                  {showIssue && issue && (
                    <div ref={issueLineRef} class="script-issue-line" />
                  )}
                  {debugLine != null && (
                    <div ref={debugLineRef} class="script-debug-line" />
                  )}
                  <textarea
                    ref={textareaRef}
                    class={`script-textarea ${composing ? "composing" : ""}`}
                    spellcheck={false}
                    value={draft}
                    placeholder="on start { … }"
                    onKeyDown={handleEditorKeyDown}
                    onCompositionStart={() => {
                      setComposing(true);
                      setCompletion(null);
                    }}
                    onCompositionEnd={() => setComposing(false)}
                    onInput={(event) => {
                      const area = event.currentTarget as HTMLTextAreaElement;
                      edit(area.value);
                      if (!(event as InputEvent).isComposing) {
                        refreshSignature(area);
                        refreshCompletion(area, false);
                      }
                    }}
                    onScroll={(event) => {
                      const area = event.target as HTMLTextAreaElement;
                      const pre = highlightRef.current;
                      if (pre) {
                        pre.scrollTop = area.scrollTop;
                        pre.scrollLeft = area.scrollLeft;
                      }
                      if (gutterRef.current) {
                        gutterRef.current.scrollTop = area.scrollTop;
                      }
                      setCompletion(null);
                      setSignature(null);
                      syncEditorOverlays(area);
                    }}
                    onClick={(event) => {
                      setCompletion(null);
                      refreshSignature(event.currentTarget);
                    }}
                    onKeyUp={(event) => {
                      if (
                        event.key === "ArrowLeft" ||
                        event.key === "ArrowRight" ||
                        event.key === "Home" ||
                        event.key === "End"
                      ) {
                        refreshSignature(
                          event.currentTarget as HTMLTextAreaElement,
                        );
                      }
                    }}
                    onBlur={() => {
                      setCompletion(null);
                      setSignature(null);
                    }}
                  />
                  {completion && (
                    <div
                      class="script-complete"
                      role="listbox"
                      style={{
                        left: `${completion.left}px`,
                        top: `${completion.top}px`,
                      }}
                    >
                      {completion.items.map((item, index) => (
                        <div
                          key={`${item.label}:${index}`}
                          role="option"
                          aria-selected={index === completion.active}
                          class={`script-complete-item ${
                            index === completion.active ? "on" : ""
                          }`}
                          onMouseDown={(event) => {
                            event.preventDefault();
                            applyCompletion(item);
                          }}
                          onMouseEnter={() =>
                            setCompletion((current) =>
                              current ? { ...current, active: index } : current,
                            )
                          }
                        >
                          <strong>{item.label}</strong>
                          <small>{item.detail}</small>
                        </div>
                      ))}
                    </div>
                  )}
                  {signature && !completion && (
                    <div
                      class="script-signature"
                      aria-hidden="true"
                      style={{
                        left: `${signature.left}px`,
                        top: `${signature.top}px`,
                      }}
                    >
                      {signature.name}(
                      {signature.args.map((arg, index) => (
                        <span key={index}>
                          {index > 0 ? ", " : ""}
                          <span
                            class={
                              index ===
                              Math.min(
                                signature.active,
                                signature.args.length - 1,
                              )
                                ? "on"
                                : ""
                            }
                          >
                            {arg}
                          </span>
                        </span>
                      ))}
                      )
                    </div>
                  )}
                  </div>
                </div>
                {debugPaused && debugView && (
                  <div class="script-debug-panel">
                    <section>
                      <h4>Call stack</h4>
                      <div class="script-debug-rows">
                        {[...debugView.stack].reverse().map((frame, index) => (
                          <button
                            type="button"
                            key={`${frame.function}:${index}`}
                            class={index === 0 ? "on" : ""}
                            onClick={() => revealLine(frame.line)}
                          >
                            <strong>{frame.function}</strong>
                            <span>line {frame.line}</span>
                          </button>
                        ))}
                      </div>
                    </section>
                    <section>
                      <h4>Locals</h4>
                      <div class="script-debug-rows">
                        {debugView.locals.length === 0 ? (
                          <span class="script-debug-empty">None</span>
                        ) : (
                          debugView.locals.map((value, index) => (
                            <div
                              key={`${value.name}:${index}`}
                              class="script-debug-row"
                            >
                              <strong>{value.name}</strong>
                              <span>{value.value}</span>
                            </div>
                          ))
                        )}
                      </div>
                    </section>
                    <section>
                      <h4>Globals</h4>
                      <div class="script-debug-rows">
                        {debugView.globals.length === 0 ? (
                          <span class="script-debug-empty">None</span>
                        ) : (
                          debugView.globals.map((value, index) => (
                            <div
                              key={`${value.name}:${index}`}
                              class="script-debug-row"
                            >
                              <strong>{value.name}</strong>
                              <span>{value.value}</span>
                            </div>
                          ))
                        )}
                      </div>
                    </section>
                  </div>
                )}
                {showIssue && issue && (
                  <button type="button" class="script-issue" onClick={jumpToIssue}>
                    <strong>
                      line {issue.line}
                      {issue.col ? `:${issue.col}` : ""}
                    </strong>
                    <span>{issue.message}</span>
                  </button>
                )}
                <div class="script-actions">
                  <button
                    type="button"
                    class="primary"
                    disabled={!dirty.has(current.id) || debugPaused}
                    title={
                      debugPaused
                        ? "Continue or stop the debugger first"
                        : undefined
                    }
                    onClick={apply}
                  >
                    Apply
                  </button>
                  <button
                    type="button"
                    disabled={!dirty.has(current.id)}
                    onClick={revert}
                  >
                    Revert
                  </button>
                  <button
                    type="button"
                    onClick={() => edit(TEMPLATE)}
                  >
                    Template
                  </button>
                  <FilePick
                    label="Load file…"
                    accept=".rxcan,.txt"
                    onText={(text, name) => {
                      edit(text);
                      setSavePath(name);
                    }}
                    onError={setError}
                  />
                  <button
                    type="button"
                    class="danger"
                    disabled={debugPaused}
                    title={
                      debugPaused
                        ? "Continue or stop the debugger first"
                        : undefined
                    }
                    onClick={() => {
                      if (window.confirm(`Remove ${current.name}?`)) {
                        onCommand({ type: "remove_script", id: current.id });
                        setSelected(null);
                        setTimeout(() => setReload((tick) => tick + 1), 500);
                      }
                    }}
                  >
                    Remove
                  </button>
                  <span class="spacer" />
                  <input
                    class="script-path"
                    value={savePath}
                    placeholder="save path (.rxcan)"
                    onInput={(event) =>
                      setSavePath((event.target as HTMLInputElement).value)
                    }
                  />
                  <button
                    type="button"
                    disabled={!savePath.trim()}
                    onClick={() =>
                      onCommand({
                        type: "save_script_file",
                        path: savePath.trim(),
                        source: draft,
                      })
                    }
                  >
                    Save file
                  </button>
                </div>
                {scriptTests.length > 0 && (
                  <div class="script-tests">
                    <header class={failedTests > 0 ? "bad" : "ok"}>
                      <strong>Tests</strong>
                      <span>
                        {scriptTests.length - failedTests}/{scriptTests.length}{" "}
                        passed
                        {failedTests > 0
                          ? ` · ${failedTests} failed`
                          : ""}
                      </span>
                    </header>
                    <div class="script-test-rows">
                      {scriptTests.map((test) => (
                        <button
                          type="button"
                          key={test.name}
                          class={`script-test ${
                            test.failures > 0 ? "bad" : "ok"
                          }`}
                          title={
                            test.line > 0
                              ? `Jump to line ${test.line}`
                              : undefined
                          }
                          onClick={() => {
                            if (test.line > 0) revealLine(test.line);
                          }}
                        >
                          <span class="script-test-state">
                            {test.failures > 0 ? "FAIL" : "PASS"}
                          </span>
                          <strong>{test.name}</strong>
                          <span class="script-test-meta">
                            {test.runs} run{test.runs === 1 ? "" : "s"}
                            {test.line > 0 ? ` · line ${test.line}` : ""}
                            {` · ${test.timeS.toFixed(3)} s`}
                          </span>
                        </button>
                      ))}
                    </div>
                  </div>
                )}
                <div class={`script-log ${current.errored ? "bad" : ""}`}>
                  {current.log.length === 0 ? (
                    <span class="script-empty-line">
                      No output yet; print() and runtime errors appear here.
                    </span>
                  ) : (
                    current.log.slice(-8).map((line, index) => (
                      <div key={index}>{line}</div>
                    ))
                  )}
                </div>
                <p class="hint">
                  Scripts are C-like and recompile on every start; Apply while
                  measuring restarts the node. Tab indents, Shift+Tab outdents.
                  Suggestions follow the caret and Tab accepts them; Alt+/ opens
                  the list, Enter accepts after arrowing. Known calls show their
                  parameters. Brackets and double quotes pair automatically, and
                  Backspace removes a whole indent. Ctrl+/ comments lines,
                  Alt+↑/↓ moves them, Shift+Alt+↓ duplicates, Ctrl+F finds.
                  F9 toggles a breakpoint; Debug parks the node so you can
                  inspect locals and the call stack.
                </p>
              </>
            ) : (
              <div class="script-empty">
                Select a script on the left, or add one above.
              </div>
            )}
          </section>
        </div>
      </div>
    </div>
  );
}
