/// Shared cut-point helpers for the State and Monitor band editors. The two
/// dialogs carry different per-band fields (names vs. colors), but the
/// numeric cut semantics must stay identical.

export function rangeText(cuts: number[], index: number): string {
  const lo = index === 0 ? null : cuts[index - 1];
  const hi = index === cuts.length ? null : cuts[index];
  if (lo == null && hi == null) return "all values";
  if (lo == null) return `< ${hi}`;
  if (hi == null) return `>= ${lo}`;
  return `${lo} .. ${hi}`;
}

export function cutError(cuts: number[]): string {
  if (cuts.some((cut) => !Number.isFinite(cut))) {
    return "cuts must be numbers";
  }
  if (cuts.some((cut, index) => index > 0 && cut <= cuts[index - 1])) {
    return "cuts must be strictly ascending";
  }
  return "";
}

export function cutIsInvalid(cuts: number[], index: number): boolean {
  return (
    !Number.isFinite(cuts[index]) ||
    (index > 0 && cuts[index] <= cuts[index - 1])
  );
}

/// The next default cut: just above the last valid one, or 0 for an empty
/// or broken draft.
export function nextCut(raw: string[]): string {
  const last = Number(raw[raw.length - 1]);
  return Number.isFinite(last) ? String(last + 1) : "0";
}