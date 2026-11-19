import {
  emptyFilterGroup,
  matchesFilter,
  type FilterFieldSpec,
  type FilterGroup,
  type FilterNode,
} from "./filterTree";

const SIGNAL_FILTER_FIELDS = [
  "signal",
  "label",
  "bus",
  "frame",
  "path",
  "unit",
] as const;

type SignalFilterField = (typeof SIGNAL_FILTER_FIELDS)[number];

export interface SignalFilterRow {
  name: string;
  label?: string;
  bus: string;
  frame?: string;
  path?: string;
  unit?: string;
}

const SIGNAL_FILTER_FIELD_LABELS: Record<SignalFilterField, string> = {
  signal: "Signal",
  label: "Label",
  bus: "Bus",
  frame: "Frame",
  path: "Frame / PDU",
  unit: "Unit",
};

export const SIGNAL_FILTER_SPECS: FilterFieldSpec[] = SIGNAL_FILTER_FIELDS.map(
  (field) => ({ key: field, label: SIGNAL_FILTER_FIELD_LABELS[field] }),
);

export function emptySignalFilter(): FilterGroup {
  return emptyFilterGroup();
}

function fieldValue(row: SignalFilterRow, field: SignalFilterField): string {
  switch (field) {
    case "signal":
      return row.name;
    case "label":
      return row.label ?? "";
    case "bus":
      return row.bus;
    case "frame":
      return row.frame ?? "";
    case "path":
      return row.path ?? "";
    case "unit":
      return row.unit ?? "";
  }
}

export function matchesSignalFilter(
  filter: FilterNode | null,
  row: SignalFilterRow,
): boolean {
  return matchesFilter(filter, (field) =>
    fieldValue(row, field as SignalFilterField),
  );
}
