import { matchesFilter, type FilterFieldSpec, type FilterNode } from "./filterTree";
import type { StatsRow } from "./types";

export const STATS_FILTER_SPECS: FilterFieldSpec[] = [
  { key: "label", label: "Label" },
  { key: "bus", label: "Bus" },
  { key: "count", label: "Count", numeric: true },
  { key: "avgMs", label: "Avg cycle (ms)", numeric: true },
  { key: "jitterMs", label: "Jitter (ms)", numeric: true },
  { key: "len", label: "Length", numeric: true },
];

function fieldValue(row: StatsRow, field: string): string {
  switch (field) {
    case "label":
      return row.label;
    case "bus":
      return row.bus;
    case "count":
      return String(row.count);
    case "avgMs":
      return row.avgMs;
    case "jitterMs":
      return row.jitterMs;
    case "len":
      return String(row.len);
    default:
      return "";
  }
}

export function matchesStatsFilter(
  filter: FilterNode | null,
  row: StatsRow,
): boolean {
  return matchesFilter(filter, (field) => fieldValue(row, field));
}
