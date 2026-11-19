export interface SignalRowFilter {
  name: string;
  bus: string;
  unit?: string;
  frame?: string;
  path?: string;
  label?: string;
}

/// Monitor and Data share one filter: every visible identity field counts,
/// including the ARXML frame/PDU path.
export function filterSignalRows<T extends SignalRowFilter>(
  rows: T[],
  query: string,
): T[] {
  const needle = query.trim().toLowerCase();
  if (!needle) return rows;
  return rows.filter((row) =>
    [row.name, row.label, row.bus, row.unit, row.frame, row.path].some((text) =>
      text?.toLowerCase().includes(needle),
    ),
  );
}

/// The row title already prints the signal name; drop the final name
/// segment from `Frame / PDU / Signal` so the second line only carries
/// the frame/PDU context.
export function rowPathWithoutName(path: string, name: string): string {
  const suffix = ` / ${name}`;
  return path.endsWith(suffix) ? path.slice(0, -suffix.length) : path;
}
