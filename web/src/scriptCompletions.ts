import type {
  Catalog,
  CatalogMessage,
  DerivedDef,
  ScriptFunctionHelp,
  SysVar,
} from "./types";
import { SCRIPT_KEYWORDS as KEYWORDS } from "./scriptKeywords";
import { frameOrigins } from "./scriptFrames";

export interface CompletionItem {
  label: string;
  detail: string;
  insert: string;
  caret: number;
  rank: number;
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

