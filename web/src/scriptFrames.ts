/// Static `frame(...)` origins for editor completion. The compiler accepts a
/// frame instance in a variable, a re-assignment, or a function argument; the
/// completion only needs to know which catalog frame a name currently holds.

/// Normalize one `frame(...)` argument to `Bus.Frame` (`Frame` when the bus is
/// implicit). Quoted and bracket spellings cover catalog names that are not
/// valid identifiers.
export function parseFramePath(text: string): string | null {
  const trimmed = text.trim();
  const bracketed = /^([A-Za-z_]\w*)\s*\[\s*"([^"]*)"\s*\]$/.exec(trimmed);
  if (bracketed) {
    return `${bracketed[1]}.${bracketed[2]}`;
  }
  const quoted = /^"([^"]*)"$/.exec(trimmed);
  if (quoted) {
    return quoted[1];
  }
  const dotted = /^([A-Za-z_]\w*)(?:\.([A-Za-z_]\w*))?$/.exec(trimmed);
  if (!dotted) return null;
  return dotted[2] ? `${dotted[1]}.${dotted[2]}` : dotted[1];
}

/// Arguments of every call to `name`, with the argument text split at
/// top-level commas. The definition itself (`fn name(...)`) is skipped.
function callArgs(source: string, name: string): string[][] {
  const out: string[][] = [];
  const call = new RegExp(`\\b${name}\\s*\\(`, "g");
  for (let match = call.exec(source); match; match = call.exec(source)) {
    if (/\bfn\s*$/.test(source.slice(0, match.index))) continue;
    const open = match.index + match[0].length - 1;
    let depth = 1;
    let inString = false;
    let i = open + 1;
    for (; i < source.length && depth > 0; i += 1) {
      const ch = source[i];
      if (inString) {
        if (ch === '"' && source[i - 1] !== "\\") inString = false;
        continue;
      }
      if (ch === '"') inString = true;
      else if (ch === "(" || ch === "[" || ch === "{") depth += 1;
      else if (ch === ")" || ch === "]" || ch === "}") depth -= 1;
    }
    if (depth !== 0) continue;
    out.push(splitArgs(source.slice(open + 1, i - 1)));
  }
  return out;
}

function splitArgs(text: string): string[] {
  const out: string[] = [];
  let start = 0;
  let depth = 0;
  let inString = false;
  for (let i = 0; i < text.length; i += 1) {
    const ch = text[i];
    if (inString) {
      if (ch === '"' && text[i - 1] !== "\\") inString = false;
      continue;
    }
    if (ch === '"') inString = true;
    else if (ch === "(" || ch === "[" || ch === "{") depth += 1;
    else if (ch === ")" || ch === "]" || ch === "}") depth -= 1;
    else if (ch === "," && depth === 0) {
      out.push(text.slice(start, i));
      start = i + 1;
    }
  }
  out.push(text.slice(start));
  return out.map((arg) => arg.trim()).filter((arg) => arg !== "");
}

/// `variable -> "Bus.Frame"` for every statically known instance. Direct
/// assignments win; a parameter is mapped only when every call site hands it
/// the same frame (or another mapped name), so an ambiguous helper stays
/// silent instead of offering wrong fields.
export function frameOrigins(source: string): Map<string, string> {
  const origins = new Map<string, string>();
  const direct = new Set<string>();
  const assign =
    /(?:\b(?:let|const)\s+)?\b([A-Za-z_]\w*)\s*=(?!=)\s*frame\s*\(\s*([^()]*?)\s*\)/g;
  for (let match = assign.exec(source); match; match = assign.exec(source)) {
    const path = parseFramePath(match[2]);
    if (!path) continue;
    direct.add(match[1]);
    origins.set(match[1], path);
  }

  const defs: { name: string; params: string[] }[] = [];
  const def = /\bfn\s+([A-Za-z_]\w*)\s*\(([^)]*)\)/g;
  for (let match = def.exec(source); match; match = def.exec(source)) {
    const params = match[2]
      .split(",")
      .map((param) => param.trim())
      .filter(Boolean);
    if (params.length > 0) defs.push({ name: match[1], params });
  }

  for (let round = 0; round < 5; round += 1) {
    const found = new Map<string, Set<string>>();
    for (const fn of defs) {
      const calls = callArgs(source, fn.name);
      for (const args of calls) {
        for (let i = 0; i < args.length && i < fn.params.length; i += 1) {
          const arg = args[i];
          const frameCall = /^frame\s*\(\s*([^()]*?)\s*\)$/.exec(arg);
          const path = frameCall
            ? parseFramePath(frameCall[1])
            : /^[A-Za-z_]\w*$/.test(arg)
              ? (origins.get(arg) ?? null)
              : null;
          if (!path) continue;
          const key = fn.params[i];
          const set = found.get(key) ?? new Set<string>();
          set.add(path);
          found.set(key, set);
        }
      }
    }
    let changed = false;
    for (const [name, paths] of found) {
      if (direct.has(name)) continue;
      if (paths.size === 1) {
        const path = [...paths][0];
        if (origins.get(name) !== path) {
          origins.set(name, path);
          changed = true;
        }
      } else if (origins.delete(name)) {
        changed = true;
      }
    }
    if (!changed) break;
  }
  return origins;
}
