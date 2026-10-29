import type { Frame } from "./types";

/// One parsed Trace filter token. `field` is null for a bare substring; a
/// leading `-` negates the whole token (`-tx`, `-id:0x100`).
export interface TraceQueryToken {
  field: string | null;
  value: string;
  negate: boolean;
}

/// Whitespace-separated tokens; double quotes keep spaces inside one value.
export function parseTraceQuery(query: string): TraceQueryToken[] {
  const raw: string[] = [];
  let current = "";
  let quoted = false;
  for (const char of query) {
    if (char === '"') {
      quoted = !quoted;
      continue;
    }
    if (/\s/.test(char) && !quoted) {
      if (current) raw.push(current);
      current = "";
    } else {
      current += char;
    }
  }
  if (current) raw.push(current);
  return raw.map((token) => {
    const negate = token.length > 1 && token.startsWith("-");
    const body = negate ? token.slice(1) : token;
    const at = body.indexOf(":");
    if (at <= 0) return { field: null, value: body, negate };
    return {
      field: body.slice(0, at).toLowerCase(),
      value: body.slice(at + 1),
      negate,
    };
  });
}

/// The original substring rule: name, bus, address, id, payload or direction.
/// A fielded token that names no known field falls back to this so a typo
/// still searches rather than hiding every row.
function bareMatch(frame: Frame, value: string): boolean {
  const needle = value.toLowerCase();
  if (!needle) return true;
  const id = `0x${frame.id.toString(16)}`;
  return (
    frame.name.toLowerCase().includes(needle) ||
    frame.busName.toLowerCase().includes(needle) ||
    frame.addr.toLowerCase().includes(needle) ||
    frame.dir.toLowerCase() === needle ||
    id.includes(needle) ||
    frame.data.toLowerCase().includes(needle)
  );
}

const HEX = /^(?:0x)?([0-9a-f]+)$/i;

function tokenMatches(frame: Frame, token: TraceQueryToken): boolean {
  const value = token.value.trim();
  let hit: boolean;
  switch (token.field) {
    case "id": {
      const digits = HEX.exec(value);
      hit = digits ? frame.id === Number.parseInt(digits[1], 16) : false;
      break;
    }
    case "bus": {
      hit = /^\d+$/.test(value)
        ? frame.bus === Number(value)
        : frame.busName.toLowerCase().includes(value.toLowerCase());
      break;
    }
    case "dir":
      hit = frame.dir.toLowerCase() === value.toLowerCase();
      break;
    case "name":
      hit = frame.name.toLowerCase().includes(value.toLowerCase());
      break;
    case "addr":
      hit = frame.addr.toLowerCase().includes(value.toLowerCase());
      break;
    case "data": {
      const needle = value.toUpperCase().replace(/\s+/g, "");
      hit =
        needle === "" ||
        frame.data.toUpperCase().replace(/\s+/g, "").includes(needle);
      break;
    }
    case "kind":
      hit = frame.kind.toLowerCase() === value.toLowerCase();
      break;
    default:
      hit = bareMatch(
        frame,
        token.field === null ? token.value : `${token.field}:${token.value}`,
      );
  }
  return token.negate ? !hit : hit;
}

/// Compiles a query once per keystroke; every token must match (AND), and a
/// negated token removes its matches.
export function traceQueryPredicate(query: string): (frame: Frame) => boolean {
  const tokens = parseTraceQuery(query);
  if (tokens.length === 0) return () => true;
  return (frame) => tokens.every((token) => tokenMatches(frame, token));
}
