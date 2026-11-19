import {
  matchesSignalFilter,
  type SignalFilterNode,
} from "./signalFilter";

export interface SignalRowFilter {
  name: string;
  bus: string;
  unit?: string;
  frame?: string;
  path?: string;
  label?: string;
}

/// Monitor and Data share one filter: every visible identity field counts,
/// including the ARXML frame/PDU path; the advanced tree is ANDed on top.
export function filterSignalRows<T extends SignalRowFilter>(
  rows: T[],
  query: string,
  filter?: SignalFilterNode,
): T[] {
  const needle = query.trim().toLowerCase();
  if (!needle && !filter) return rows;
  return rows.filter(
    (row) =>
      (!needle ||
        [row.name, row.label, row.bus, row.unit, row.frame, row.path].some(
          (text) => text?.toLowerCase().includes(needle),
        )) &&
      (!filter || matchesSignalFilter(filter, row)),
  );
}

/// The row title already prints the signal name; drop the final name
/// segment from `Frame / PDU / Signal` so the second line only carries
/// the frame/PDU context.
export function rowPathWithoutName(path: string, name: string): string {
  const suffix = ` / ${name}`;
  return path.endsWith(suffix) ? path.slice(0, -suffix.length) : path;
}
