/// Shared filter grammar and evaluator. Panels supply a field list for the
/// editor and a row->string adapter for evaluation; pruning, counting and
/// the AND/OR/NOT semantics stay in one place.

export type FilterOp =
  | "contains"
  | "notContains"
  | "equals"
  | "notEquals"
  | "gte"
  | "lte"
  | "gt"
  | "lt";

export interface FilterRule {
  kind: "rule";
  field: string;
  op: FilterOp;
  value: string;
  /// Numeric fields compare numbers for equals/range operators.
  numeric?: boolean;
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
  numeric?: boolean;
}

export const FILTER_OP_LABELS: Record<FilterOp, string> = {
  contains: "contains",
  notContains: "does not contain",
  equals: "equals",
  notEquals: "is not",
  gte: "≥",
  lte: "≤",
  gt: ">",
  lt: "<",
};

export const FILTER_TEXT_OPS: FilterOp[] = [
  "contains",
  "notContains",
  "equals",
  "notEquals",
];

export const FILTER_NUMBER_OPS: FilterOp[] = [
  "equals",
  "notEquals",
  "gte",
  "lte",
  "gt",
  "lt",
];

export function emptyFilterGroup(): FilterGroup {
  return { kind: "group", op: "and", children: [] };
}

export function newFilterRule(field: string, numeric = false): FilterRule {
  return {
    kind: "rule",
    field,
    op: numeric ? "equals" : "contains",
    value: "",
    numeric: numeric || undefined,
  };
}

export function newFilterGroup(): FilterGroup {
  return { kind: "group", op: "and", children: [] };
}

function matchesRule(rule: FilterRule, read: (field: string) => string): boolean {
  const needle = rule.value.trim().toLowerCase();
  if (!needle) return true;
  const raw = read(rule.field);
  if (rule.numeric && rule.op !== "contains" && rule.op !== "notContains") {
    const left = Number(raw);
    const right = Number(needle);
    if (!Number.isFinite(left) || !Number.isFinite(right)) return false;
    switch (rule.op) {
      case "equals":
        return left === right;
      case "notEquals":
        return left !== right;
      case "gte":
        return left >= right;
      case "lte":
        return left <= right;
      case "gt":
        return left > right;
      case "lt":
        return left < right;
    }
  }
  const value = raw.toLowerCase();
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

const TEXT_OPS: FilterOp[] = ["contains", "notContains", "equals", "notEquals"];
const NUMBER_OPS: FilterOp[] = ["equals", "notEquals", "gte", "lte", "gt", "lt"];
const MAX_FILTER_DEPTH = 3;
const MAX_FILTER_CHILDREN = 32;
const MAX_FILTER_VALUE = 200;

/// Rebuilds a persisted tree from untrusted project/localStorage JSON.
/// Unknown fields, operators or over-deep groups are dropped; the result is
/// always a group, so callers can pass it straight to the editor.
export function normalizeFilter(
  value: unknown,
  fields: FilterFieldSpec[],
): FilterGroup {
  const byKey = new Map(fields.map((field) => [field.key, field]));
  let nodes = 0;

  const node = (raw: unknown, depth: number): FilterNode | null => {
    if (!raw || typeof raw !== "object" || depth > MAX_FILTER_DEPTH) return null;
    const entry = raw as Record<string, unknown>;
    if (entry.kind === "rule") {
      if (nodes >= MAX_FILTER_CHILDREN * 2) return null;
      const field = byKey.get(String(entry.field));
      const op = entry.op as FilterOp;
      if (!field || (!TEXT_OPS.includes(op) && !NUMBER_OPS.includes(op))) {
        return null;
      }
      const numeric = !!field.numeric;
      if (!numeric && !TEXT_OPS.includes(op)) return null;
      if (numeric && !NUMBER_OPS.includes(op)) return null;
      const text = typeof entry.value === "string"
        ? entry.value.slice(0, MAX_FILTER_VALUE)
        : "";
      nodes += 1;
      return {
        kind: "rule",
        field: field.key,
        op,
        value: text,
        numeric: numeric || undefined,
      };
    }
    if (entry.kind === "group") {
      const op = entry.op === "or" ? "or" : "and";
      const children = Array.isArray(entry.children)
        ? entry.children
            .slice(0, MAX_FILTER_CHILDREN)
            .map((child) => node(child, depth + 1))
            .filter((child): child is FilterNode => child !== null)
        : [];
      return {
        kind: "group",
        op,
        negate: entry.negate === true || undefined,
        children,
      };
    }
    return null;
  };

  const root = node(value, 0);
  return root?.kind === "group" ? root : emptyFilterGroup();
}

