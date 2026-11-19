/// Shared filter grammar and evaluator. Panels supply a field list for the
/// editor and a row->string adapter for evaluation; pruning, counting and
/// the AND/OR/NOT semantics stay in one place.

export type FilterOp = "contains" | "notContains" | "equals" | "notEquals";

export interface FilterRule {
  kind: "rule";
  field: string;
  op: FilterOp;
  value: string;
}

export interface FilterGroup {
  kind: "group";
  op: "and" | "or";
  negate?: boolean;
  children: FilterNode[];
}

export type FilterNode = FilterRule | FilterGroup;

export interface FilterFieldSpec {
  key: string;
  label: string;
}

export const FILTER_OP_LABELS: Record<FilterOp, string> = {
  contains: "contains",
  notContains: "does not contain",
  equals: "equals",
  notEquals: "is not",
};

export function emptyFilterGroup(): FilterGroup {
  return { kind: "group", op: "and", children: [] };
}

export function newFilterRule(field: string): FilterRule {
  return { kind: "rule", field, op: "contains", value: "" };
}

export function newFilterGroup(): FilterGroup {
  return { kind: "group", op: "and", children: [] };
}

function matchesRule(rule: FilterRule, read: (field: string) => string): boolean {
  const needle = rule.value.trim().toLowerCase();
  if (!needle) return true;
  const value = read(rule.field).toLowerCase();
  const hit =
    rule.op === "contains" || rule.op === "notContains"
      ? value.includes(needle)
      : value === needle;
  return rule.op === "notContains" || rule.op === "notEquals" ? !hit : hit;
}

function matchesNode(
  node: FilterNode,
  read: (field: string) => string,
): boolean {
  if (node.kind === "rule") return matchesRule(node, read);
  const hit =
    node.op === "and"
      ? node.children.every((child) => matchesNode(child, read))
      : node.children.some((child) => matchesNode(child, read));
  return node.negate ? !hit : hit;
}

/// Blank rules and empty groups are ignored, so a half-written condition
/// never hides every row. Null means there is no active advanced filter.
export function pruneFilter(node: FilterNode): FilterNode | null {
  if (node.kind === "rule") {
    return node.value.trim() ? node : null;
  }
  const children = node.children
    .map(pruneFilter)
    .filter((child): child is FilterNode => child !== null);
  if (children.length === 0) return null;
  return { ...node, children };
}

export function filterActive(node: FilterNode): boolean {
  return pruneFilter(node) !== null;
}

export function filterRuleCount(node: FilterNode): number {
  const active = pruneFilter(node);
  if (!active) return 0;
  const count = (entry: FilterNode): number =>
    entry.kind === "rule"
      ? 1
      : entry.children.reduce((sum, child) => sum + count(child), 0);
  return count(active);
}

export function matchesFilter(
  filter: FilterNode | null,
  read: (field: string) => string,
): boolean {
  const active = filter ? pruneFilter(filter) : null;
  return active ? matchesNode(active, read) : true;
}
