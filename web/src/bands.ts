export interface Band {
  upper: number | null;
  inclusive: boolean;
  text?: string;
  color?: number;
}

/// First band holding `value`; null when no band matches.
export function bandIndex(bands: Band[], value: number | null): number | null {
  if (value === null || !Number.isFinite(value)) return null;
  for (let index = 0; index < bands.length; index += 1) {
    const band = bands[index];
    if (band.upper === null) return index;
    if (band.inclusive ? value <= band.upper : value < band.upper) return index;
  }
  return null;
}

export function bandAt(bands: Band[], value: number | null): Band | null {
  const index = bandIndex(bands, value);
  return index === null ? null : bands[index];
}

export function finalBand(color?: number): Band {
  return { upper: null, inclusive: false, color };
}

export function normaliseBands(bands: Band[]): Band[] {
  return [...bands].sort((a, b) => {
    if (a.upper === null) return 1;
    if (b.upper === null) return -1;
    return a.upper - b.upper || Number(a.inclusive) - Number(b.inclusive);
  });
}

export function validateBands(bands: Band[]): string {
  if (bands.length === 0) return "Add at least one band.";
  if (bands.length > 64) return "At most 64 bands are allowed.";
  for (const [index, band] of bands.entries()) {
    if (band.upper !== null && !Number.isFinite(band.upper)) {
      return `Band ${index + 1}: bound must be a number.`;
    }
    if ((band.text ?? "").length > 80) {
      return `Band ${index + 1}: text is limited to 80 characters.`;
    }
    if (band.color != null && (band.color < 0 || band.color > 63)) {
      return `Band ${index + 1}: colour must be 0–63.`;
    }
  }
  const finals = bands.filter((band) => band.upper === null).length;
  if (finals !== 1) return "Exactly one open-ended final band is required.";
  if (bands[bands.length - 1].upper !== null) {
    return "The open-ended band must be last.";
  }
  for (let index = 1; index < bands.length; index += 1) {
    const prev = bands[index - 1];
    const next = bands[index];
    if (prev.upper !== null && next.upper !== null) {
      if (next.upper < prev.upper) return "Bands must be ordered by bound.";
      if (next.upper === prev.upper && next.inclusive === prev.inclusive) {
        return "Bands need unique bound/mode pairs.";
      }
      if (next.upper === prev.upper && !prev.inclusive && next.inclusive) {
        return "At one bound, < must come before ≤.";
      }
    }
  }
  return "";
}
