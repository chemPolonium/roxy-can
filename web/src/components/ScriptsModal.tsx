import { useEffect, useRef, useState } from "preact/hooks";
import { fetchScriptHelp, fetchScripts } from "../api";
import type {
  Bus,
  Catalog,
  CatalogMessage,
  Command,
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

interface CompletionItem {
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
}

function lineStartOf(text: string, position: number): number {
  return text.lastIndexOf("\n", Math.max(0, position - 1)) + 1;
}

function lineEndOf(text: string, position: number): number {
  const end = text.indexOf("\n", position);
  return end < 0 ? text.length : end;
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
function completionPlacement(
  textarea: HTMLTextAreaElement,
  position: number,
  itemCount: number,
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
  const popupWidth = 300;
  const popupHeight = Math.min(190, 8 + Math.min(itemCount, 8) * 22);
  const left = Math.max(4, Math.min(x, Math.max(4, width - popupWidth - 6)));
  const top =
    y + 2 + popupHeight <= height - 4
      ? y + 2
      : Math.max(4, y - popupHeight - 2);
  return { left, top };
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
function buildCandidates(
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

  // `on message ` is a DBC context: list the channel's message names.
  if (!inString) {
    const handler = /^\s*on\s+message\s+([A-Za-z_]\w*)?$/.exec(lineBefore);
    if (handler) {
      const partial = handler[1] ?? "";
      const query = partial.toLowerCase();
      const items = catalogMessages(catalog, channel)
        .filter(
          (message) =>
            query === "" || message.name.toLowerCase().includes(query),
        )
        .slice(0, 12)
        .map((message) => ({
          label: message.name,
          detail: messageDetail(message),
          insert: message.name,
          caret: message.name.length,
          rank: 0,
        }));
      if (items.length === 0) return null;
      return { start: caret - partial.length, end: caret, items };
    }
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
    for (const fn of functions) {
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
      items.push({
        label: keyword,
        detail: "keyword",
        insert: keyword,
        caret: keyword.length,
        rank: 1,
      });
    }
    for (const definition of derived) {
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

/// A deliberately small highlighter: comments, strings, numbers, keywords
/// and call names. It paints the <pre> behind the textarea; the textarea
/// itself keeps the caret and input handling.
function highlight(source: string): string {
  const token =
    /\/\/[^\n]*|\/\*[\s\S]*?\*\/|"(?:\\.|[^"\\])*"|'(?:\\.|[^'\\])*'|\b\d+(?:\.\d+)?\b|[A-Za-z_]\w*/g;
  let out = "";
  let last = 0;
  for (const match of source.matchAll(token)) {
    const at = match.index ?? 0;
    out += escapeHtml(source.slice(last, at));
    const text = match[0];
    last = at + text.length;
    if (text.startsWith("//") || text.startsWith("/*")) {
      out += `<span class="tok-comment">${escapeHtml(text)}</span>`;
    } else if (text.startsWith('"') || text.startsWith("'")) {
      out += `<span class="tok-string">${escapeHtml(text)}</span>`;
    } else if (/^\d/.test(text)) {
      out += `<span class="tok-number">${text}</span>`;
    } else if (KEYWORDS.has(text)) {
      out += `<span class="tok-keyword">${text}</span>`;
    } else if (/^\s*\(/.test(source.slice(last))) {
      out += `<span class="tok-fn">${text}</span>`;
    } else {
      out += escapeHtml(text);
    }
  }
  return out + escapeHtml(source.slice(last));
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
  const textareaRef = useRef<HTMLTextAreaElement>(null);
  const [functions, setFunctions] = useState<ScriptFunctionHelp[]>([]);
  const [completion, setCompletion] = useState<Completion | null>(null);
  const [reload, setReload] = useState(0);
  const editing = useRef<Set<number>>(new Set());
  const selectNewest = useRef(false);
  const [name, setName] = useState("");
  const [savePath, setSavePath] = useState("");
  const [newOpen, setNewOpen] = useState(false);
  const [newName, setNewName] = useState("");
  const [newChannel, setNewChannel] = useState(0);
  const [newAttach, setNewAttach] = useState("");

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
  }, [selected]);

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
    setCompletion({ ...candidates, active: 0, ...placement });
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
  };

  const handleEditorKeyDown = (event: KeyboardEvent) => {
    const textarea = event.currentTarget as HTMLTextAreaElement | null;
    if (!textarea) return;
    if (composing || event.isComposing || event.keyCode === 229) return;

    // Alt+/ rather than Ctrl+Space: the latter is the Chinese IME's
    // on/off toggle on Windows and never reaches the page.
    if (event.altKey && (event.key === "/" || event.code === "Slash")) {
      event.preventDefault();
      refreshCompletion(textarea, true);
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
              }
            : current,
        );
        return;
      }
      if (event.key === "Enter" || event.key === "Tab") {
        event.preventDefault();
        applyCompletion(completion.items[completion.active]);
        return;
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
      event.key === "}" &&
      !event.ctrlKey &&
      !event.metaKey &&
      !event.altKey &&
      !event.shiftKey
    ) {
      handleClosingBrace(event);
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
        script.id === current.id ? { ...script, source: draft } : script,
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
                <div class="script-editor">
                  <pre
                    ref={highlightRef}
                    class="script-highlight"
                    aria-hidden="true"
                    dangerouslySetInnerHTML={{ __html: highlight(`${draft}\n`) }}
                  />
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
                      setCompletion(null);
                    }}
                    onClick={() => setCompletion(null)}
                    onBlur={() => setCompletion(null)}
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
                </div>
                <div class="script-actions">
                  <button
                    type="button"
                    class="primary"
                    disabled={!dirty.has(current.id)}
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
                  measuring restarts the node. Tab indents, Shift+Tab outdents,
                  Alt+/ completes; comments, strings, numbers, keywords
                  and calls are colored as you type.
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
