import { useEffect, useRef, useState } from "preact/hooks";
import { lazy, Suspense } from "preact/compat";
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
import type { ScriptEditorApi } from "../scriptEditor";
import { SCRIPT_KEYWORDS as KEYWORDS } from "../scriptKeywords";
import { frameOrigins } from "../scriptFrames";

// CodeMirror is only needed once a script is edited; keep it out of the
// initial shell bundle.
const ScriptCodeEditor = lazy(() =>
  import("./ScriptCodeEditor").then((module) => ({
    default: module.ScriptCodeEditor,
  })),
);

interface Props {
  buses: Bus[];
  catalog: Catalog | null;
  derived: DerivedDef[];
  sysvars: SysVar[];
  onCommand: (command: Command) => void;
  onClose: () => void;
}


export interface CompletionItem {
  label: string;
  detail: string;
  insert: string;
  /// Caret offset from the start of the inserted text.
  caret: number;
  rank: number;
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


/// Replaces a range through the browser's own insertion command so typing
/// remains one undo step. Falls back to `setRangeText` if the command is
/// unavailable (e.g. an embedded webview).
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
function quoted(text: string): string {
  return text.replace(/\\/g, "\\\\").replace(/"/g, '\\"');
}

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

  // A frame instance's fields, including instances assigned after `let` and
  // function parameters that receive a frame at every call site. This runs
  // before the DBC dotted context, which would otherwise read the variable as
  // a message name.
  if (!inString) {
    const member = /([A-Za-z_]\w*)\.([A-Za-z_]\w*)?$/.exec(lineBefore);
    if (member) {
      const [, variable, partial = ""] = member;
      const path = frameOrigins(value).get(variable);
      if (path) {
        const [first, second] = path.split(".");
        let owner: Catalog["channels"][number] | undefined;
        let messageName = first;
        if (second) {
          messageName = second;
          owner =
            first.startsWith("FR") || first.startsWith("ETH")
              ? (catalogBus(
                  catalog,
                  first.startsWith("FR") ? "fr" : "eth",
                  Number(first.replace(/^(FR|ETH)/, "")),
                ) ?? undefined)
              : (catalog?.channels ?? []).find(
                  (entry) => entry.kind === "can" && entry.name === first,
                );
        } else {
          owner = (catalog?.channels ?? []).find(
            (entry) => entry.kind === "can" && entry.index === channel,
          );
        }
        const frame = owner?.messages.find(
          (message) =>
            message.name === messageName || message.frame === messageName,
        );
        if (frame) {
          const query = partial.toLowerCase();
          const items = frame.signals
            .filter(
              (signal) =>
                query === "" || signal.name.toLowerCase().includes(query),
            )
            .slice(0, 12)
            .map((signal) => ({
              label: signal.name,
              detail: signal.unit
                ? `${frame.name} · ${signal.unit}`
                : frame.name,
              insert: signal.name,
              caret: signal.name.length,
              rank: 0,
            }));
          if (items.length === 0) return null;
          return { start: caret - partial.length, end: caret, items };
        }
      }
    }
  }

  // `Message.` is a DBC context: only that message's signals make sense.
  if (!inString) {
    const dotted = /([A-Za-z_]\w*)\.([A-Za-z_]\w*)?$/.exec(lineBefore);
    if (dotted && !/\bframe\(\s*[A-Za-z0-9_.]*$/.test(lineBefore)) {
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

  if (!inString) {
    const frameCall = /(?:^|[^\w])frame\(\s*([A-Za-z0-9_.]*)$/.exec(lineBefore);
    if (frameCall) {
      const partial = frameCall[1] ?? "";
      const query = partial.toLowerCase();
      const items: CompletionItem[] = [];
      const add = (label: string, detail: string, rank: number) => {
        if (query !== "" && !label.toLowerCase().includes(query)) return;
        items.push({
          label,
          detail,
          insert: label,
          caret: label.length,
          rank,
        });
      };
      for (const entry of catalog?.channels ?? []) {
        const prefix =
          entry.kind === "fr"
            ? `FR${entry.index}`
            : entry.kind === "eth"
              ? `ETH${entry.index}`
              : entry.name;
        for (const message of entry.messages) {
          const name = message.frame ?? message.name;
          const bare = /^[A-Za-z_]\w*$/.test(name);
          const detail =
            entry.kind === "fr"
              ? `FR${entry.index} frame`
              : entry.kind === "eth"
                ? `ETH${entry.index} frame`
                : messageDetail(message);
          add(bare ? `${prefix}.${name}` : `${prefix}["${name}"]`, detail, 0);
          if (entry.kind === "can" && entry.index === channel) {
            add(bare ? name : `"${name}"`, messageDetail(message), 1);
          }
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

  // Inside a quoted frame name: `frame("…` lists this channel's frames,
  // `frame(CAN1["…` / `frame(FR0["…` lists that bus's frames.
  if (inString) {
    const qualified = /(?:^|[^\w])frame\(\s*([A-Za-z_]\w*)\s*\[\s*"([^"]*)$/.exec(
      lineBefore,
    );
    const plain = /(?:^|[^\w])frame\(\s*"([^"]*)$/.exec(lineBefore);
    if (qualified || plain) {
      const busToken = qualified?.[1];
      const partial = qualified ? qualified[2] : plain![1];
      let owner: Catalog["channels"][number] | undefined;
      if (!busToken) {
        owner = (catalog?.channels ?? []).find(
          (entry) => entry.kind === "can" && entry.index === channel,
        );
      } else if (busToken.startsWith("FR") || busToken.startsWith("ETH")) {
        owner =
          catalogBus(
            catalog,
            busToken.startsWith("FR") ? "fr" : "eth",
            Number(busToken.replace(/^(FR|ETH)/, "")),
          ) ?? undefined;
      } else {
        owner = (catalog?.channels ?? []).find(
          (entry) => entry.kind === "can" && entry.name === busToken,
        );
      }
      if (!owner) return null;
      const bus = owner;
      const query = partial.toLowerCase();
      const items = bus.messages
        .filter((message) => {
          const name = message.frame ?? message.name;
          return query === "" || name.toLowerCase().includes(query);
        })
        .slice(0, 12)
        .map((message) => {
          const name = message.frame ?? message.name;
          return {
            label: name,
            detail:
              bus.kind === "can" ? messageDetail(message) : `${bus.name} frame`,
            insert: name,
            caret: name.length,
            rank: 0,
          };
        });
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
    // let cmd = frame(EngineStatus);
    // cmd.EngineSpeed = 1000;
    // send_frame(cmd);
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
  const [functions, setFunctions] = useState<ScriptFunctionHelp[]>([]);
  const [signature, setSignature] = useState<SignatureHint | null>(null);
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
  const editorApi = useRef<ScriptEditorApi | null>(null);
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
    setSignature(null);
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
    const close = (event: KeyboardEvent) => {
      if (event.defaultPrevented) return;
      // CodeMirror owns Escape while its search panel or completion list is
      // open; the first Escape dismisses that, not the whole editor.
      if (
        document.querySelector(".cm-search") ||
        document.querySelector(".cm-tooltip-autocomplete")
      ) {
        return;
      }
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

  const issueLine =
    current?.issue && selected != null && !dirty.has(selected)
      ? current.issue.line
      : null;

  useEffect(() => {
    if (debugLine != null) editorApi.current?.revealLine(debugLine);
  }, [debugLine]);

  const revealLine = (line: number) => {
    editorApi.current?.revealLine(line);
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
    setDrafts((currentDrafts) => ({ ...currentDrafts, [selected]: value }));
    setDirty((currentDirty) => new Set(currentDirty).add(selected));
  };

  const complete = (text: string, caret: number, explicit: boolean) =>
    buildCandidates(
      text,
      caret,
      explicit,
      functions,
      derived,
      sysvars,
      catalog,
      current?.channel ?? 0,
    );

  const updateSignature = (
    text: string,
    caret: number,
    coords: { left: number; top: number } | null,
  ) => {
    if (!coords) {
      setSignature(null);
      return;
    }
    const call = callAtCaret(text, caret);
    if (!call) {
      setSignature(null);
      return;
    }
    const builtin = functions.find((item) => item.name === call.name);
    const local = builtin
      ? null
      : localSymbols(text, caret).find(
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
      left: coords.left,
      top: coords.top + 2,
    });
  };

  const jumpToIssue = () => {
    const issue = current?.issue;
    if (!issue) return;
    editorApi.current?.revealLine(issue.line, issue.col ?? 1);
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
                {(buses[newChannel]?.can?.nodes ?? []).map((node) => (
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
                <div class="script-cm-wrap">
                  <Suspense
                    fallback={
                      <div class="script-cm-loading">Loading editor…</div>
                    }
                  >
                    <ScriptCodeEditor
                      doc={draft}
                      scriptId={selected}
                      breakpoints={breakpoints}
                      debugLine={debugLine}
                      issueLine={issueLine}
                      completion={complete}
                      onChange={edit}
                      onToggleBreakpoint={toggleBreakpoint}
                      onCursor={updateSignature}
                      apiRef={editorApi}
                    />
                  </Suspense>
                  {signature && (
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
