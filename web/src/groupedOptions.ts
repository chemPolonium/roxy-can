export interface GroupedOption {
  key: string;
  group: string;
  groupMeta?: string;
  label: string;
  meta?: string;
  /// Extra search text that is not shown in the row (comments, aliases).
  search?: string;
}

export interface GroupedOptionGroup {
  key: string;
  label: string;
  meta?: string;
  options: GroupedOption[];
}

/// Filters across every visible column plus the group header, then keeps the
/// catalog order. A group whose header matches keeps all of its options.
export function filterGroupedOptions(
  options: GroupedOption[],
  query: string,
): GroupedOptionGroup[] {
  const needle = query.trim().toLowerCase();
  const groups = new Map<string, GroupedOptionGroup>();
  for (const option of options) {
    if (needle) {
      const text = [
        option.label,
        option.meta,
        option.group,
        option.groupMeta,
        option.search,
      ]
        .filter(Boolean)
        .join("\n")
        .toLowerCase();
      if (!text.includes(needle)) continue;
    }
    const groupKey = `${option.group}\u0000${option.groupMeta ?? ""}`;
    let group = groups.get(groupKey);
    if (!group) {
      group = {
        key: groupKey,
        label: option.group,
        meta: option.groupMeta,
        options: [],
      };
      groups.set(groupKey, group);
    }
    group.options.push(option);
  }
  return [...groups.values()];
}
