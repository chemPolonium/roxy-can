import {
  FILTER_OP_LABELS,
  filterActive,
  filterRuleCount,
  matchesFilter,
  newFilterRule,
  pruneFilter,
  type FilterFieldSpec,
  type FilterGroup,
  type FilterNode,
  type FilterOp,
  type FilterRule,
} from "./filterTree";

export const SIGNAL_FILTER_FIELDS = [
  "signal",
  "label",
  "bus",
  "frame",
  "path",
  "unit",
] as const;

export type SignalFilterField = (typeof SIGNAL_FILTER_FIELDS)[number];
export type SignalFilterOp = FilterOp;

export interface SignalFilterRule extends FilterRule {
  field: SignalFilterField;
}

export interface SignalFilterGroup extends FilterGroup {
  children: SignalFilterNode[];
}

export type SignalFilterNode = SignalFilterRule | SignalFilterGroup;

export interface SignalFilterRow {
  name: string;
  label?: string;
  bus: string;
  frame?: string;
  path?: string;
  unit?: string;
}

export const SIGNAL_FILTER_FIELD_LABELS: Record<SignalFilterField, string> = {
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

export { FILTER_OP_LABELS };

export function emptySignalFilter(): SignalFilterGroup {
  return { kind: "group", op: "and", children: [] };
}

export function newSignalRule(): SignalFilterRule {
  return { ...newFilterRule("signal"), field: "signal" };
}

export function newSignalGroup(): SignalFilterGroup {
  return { kind: "group", op: "and", children: [] };
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

export function pruneSignalFilter(
  node: FilterNode,
): FilterNode | null {
  return pruneFilter(node);
}

export function signalFilterActive(node: FilterNode): boolean {
  return filterActive(node);
}

export function signalFilterRuleCount(node: FilterNode): number {
  return filterRuleCount(node);
}

export function matchesSignalFilter(
  filter: FilterNode | null,
  row: SignalFilterRow,
): boolean {
  return matchesFilter(filter, (field) =>
    fieldValue(row, field as SignalFilterField),
  );
}
