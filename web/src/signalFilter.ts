/// One filter grammar for the signal-row panels (Monitor / Data today,
/// Trace and Messages can adopt the same node shape later). The AST is
/// deliberately small: AND/OR groups, NOT, and a text rule per row field.

export const SIGNAL_FILTER_FIELDS = [
  "signal",
  "label",
  "bus",
  "frame",
  "path",
  "unit",
] as const;

export type SignalFilterField = (typeof SIGNAL_FILTER_FIELDS)[number];
export type SignalFilterOp = "contains" | "notContains" | "equals" | "notEquals";

export interface SignalFilterRule {
  kind: "rule";
  field: SignalFilterField;
  op: SignalFilterOp;
  value: string;
}

export interface SignalFilterGroup {
  kind: "group";
  op: "and" | "or";
  negate?: boolean;
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

export const SIGNAL_FILTER_OP_LABELS: Record<SignalFilterOp, string> = {
  contains: "contains",
  notContains: "does not contain",
  equals: "equals",
  notEquals: "is not",
};

export function emptySignalFilter(): SignalFilterGroup {
  return { kind: "group", op: "and", children: [] };
}

export function newSignalRule(): SignalFilterRule {
  return { kind: "rule", field: "signal", op: "contains", value: "" };
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

function matchesRule(rule: SignalFilterRule, row: SignalFilterRow): boolean {
  const needle = rule.value.trim().toLowerCase();
  if (!needle) return true;
  const value = fieldValue(row, rule.field).toLowerCase();
  const hit = rule.op === "contains" || rule.op === "notContains"
    ? value.includes(needle)
    : value === needle;
  return rule.op === "notContains" || rule.op === "notEquals" ? !hit : hit;
}

function matchesNode(node: SignalFilterNode, row: SignalFilterRow): boolean {
  if (node.kind === "rule") return matchesRule(node, row);
  const hit =
    node.op === "and"
      ? node.children.every((child) => matchesNode(child, row))
      : node.children.some((child) => matchesNode(child, row));
  return node.negate ? !hit : hit;
}

/// Drops conditions that are still blank, so an unfinished group does not
/// hide every row. Null means "no advanced filter at all".
export function pruneSignalFilter(
  node: SignalFilterNode,
): SignalFilterNode | null {
  if (node.kind === "rule") {
    return node.value.trim() ? node : null;
  }
  const children = node.children
    .map(pruneSignalFilter)
    .filter((child): child is SignalFilterNode => child !== null);
  if (children.length === 0) return null;
  return { ...node, children };
}

export function signalFilterActive(node: SignalFilterNode): boolean {
  return pruneSignalFilter(node) !== null;
}

export function signalFilterRuleCount(node: SignalFilterNode): number {
  const active = pruneSignalFilter(node);
  if (!active) return 0;
  const count = (entry: SignalFilterNode): number =>
    entry.kind === "rule"
      ? 1
      : entry.children.reduce((sum, child) => sum + count(child), 0);
  return count(active);
}

export function matchesSignalFilter(
  filter: SignalFilterNode | null,
  row: SignalFilterRow,
): boolean {
  const active = filter ? pruneSignalFilter(filter) : null;
  return active ? matchesNode(active, row) : true;
}
