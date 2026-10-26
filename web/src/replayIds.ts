/// Replay-block id filters are `(id, extended)` pairs. The editor shows them
/// as a compact hex list: `100` is standard, a trailing `x` forces extended,
/// and values above 0x7FF are extended even without the suffix.
export function formatReplayIds(ids: [number, boolean][]): string {
  return ids
    .map(
      ([id, extended]) =>
        `0x${id.toString(16).toUpperCase()}${extended ? "x" : ""}`,
    )
    .join(", ");
}

export function parseReplayIds(text: string): {
  ids: [number, boolean][];
  error: string;
} {
  const ids: [number, boolean][] = [];
  const seen = new Set<string>();
  for (const token of text.split(/[\s,]+/).filter(Boolean)) {
    const extendedSuffix = /x$/i.test(token);
    const raw = extendedSuffix ? token.slice(0, -1) : token;
    const prefixed = /^0x[0-9a-f]+$/i.test(raw);
    const bareHex = /^[0-9a-f]+$/i.test(raw);
    if (!prefixed && !bareHex) {
      return { ids: [], error: `invalid id "${token}" (use 100 or 3F4x)` };
    }
    const value = Number.parseInt(prefixed ? raw.slice(2) : raw, 16);
    if (!Number.isFinite(value) || value < 0 || value > 0x1fffffff) {
      return { ids: [], error: `id ${token} is outside 0..0x1FFFFFFF` };
    }
    const extended = extendedSuffix || value > 0x7ff;
    const key = `${value}:${extended}`;
    if (seen.has(key)) continue;
    seen.add(key);
    ids.push([value, extended]);
  }
  return { ids, error: "" };
}
