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

/// One palette slot per band; missing slots reuse the last selected one.
export function fitBandColors(colors: number[], count: number): number[] {
  const next = colors.slice(0, count);
  while (next.length < count) next.push(next[next.length - 1] ?? 0);
  return next;
}

export function addCutBand(
  cuts: number[],
  colors: number[],
  nextColor: number,
): { cuts: number[]; colors: number[] } {
  const last = cuts[cuts.length - 1];
  const next = Number.isFinite(last) ? last + 1 : 0;
  return {
    cuts: [...cuts, next],
    colors: [...fitBandColors(colors, cuts.length + 1), nextColor],
  };
}

export function removeCutBand(
  cuts: number[],
  colors: number[],
  cutIndex: number,
): { cuts: number[]; colors: number[] } {
  return {
    cuts: cuts.filter((_, index) => index !== cutIndex),
    colors: fitBandColors(
      colors.filter((_, index) => index !== cutIndex + 1),
      cuts.length,
    ),
  };
}

/// The next default cut: just above the last valid one, or 0 for an empty
/// or broken draft.
export function nextCut(raw: string[]): string {
  const last = Number(raw[raw.length - 1]);
  return Number.isFinite(last) ? String(last + 1) : "0";
}