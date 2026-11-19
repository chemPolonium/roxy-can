import {
  filterRuleCount,
  normalizeFilter,
  pruneFilter,
  type FilterFieldSpec,
  type FilterGroup,
} from "./filterTree";
import { SIGNAL_FILTER_SPECS } from "./signalFilter";
import { MESSAGE_FILTER_SPECS } from "./messageFilter";
import { STATS_FILTER_SPECS } from "./statsFilter";
import { TRACE_FILTER_SPECS } from "./traceFilter";

export type FilterScope = "signal" | "message" | "stats" | "trace";

export interface SavedFilter {
  id: string;
  name: string;
  scope: FilterScope;
  filter: FilterGroup;
}

export const FILTER_SCOPE_FIELDS: Record<FilterScope, FilterFieldSpec[]> = {
  signal: SIGNAL_FILTER_SPECS,
  message: MESSAGE_FILTER_SPECS,
  stats: STATS_FILTER_SPECS,
  trace: TRACE_FILTER_SPECS,
};

export const FILTER_SCOPE_LABELS: Record<FilterScope, string> = {
  signal: "Signal",
  message: "Message",
  stats: "Message stats",
  trace: "Trace",
};

let savedSeq = 0;

export function savedFilterId(): string {
  savedSeq += 1;
  return `flt-${Date.now().toString(36)}-${savedSeq}`;
}

export function cloneFilter(filter: FilterGroup): FilterGroup {
  return JSON.parse(JSON.stringify(filter)) as FilterGroup;
}

export function normalizeFilterLibrary(value: unknown): SavedFilter[] {
  if (!Array.isArray(value)) return [];
  const out: SavedFilter[] = [];
  const used = new Set<string>();
  for (const raw of value.slice(0, 32)) {
    if (!raw || typeof raw !== "object") continue;
    const entry = raw as Record<string, unknown>;
    const scope =
      entry.scope === "signal" ||
      entry.scope === "message" ||
      entry.scope === "stats" ||
      entry.scope === "trace"
        ? entry.scope
        : null;
    if (!scope) continue;
    const name =
      typeof entry.name === "string" ? entry.name.trim().slice(0, 40) : "";
    if (!name) continue;
    const id =
      typeof entry.id === "string" && entry.id && !used.has(entry.id)
        ? entry.id
        : savedFilterId();
    if (used.has(id)) continue;
    used.add(id);
    const filter = normalizeFilter(entry.filter, FILTER_SCOPE_FIELDS[scope]);
    const pruned = pruneFilter(filter);
    if (
      !pruned ||
      pruned.kind !== "group" ||
      filterRuleCount(pruned) === 0
    ) {
      continue;
    }
    out.push({ id, name, scope, filter: pruned });
  }
  return out;
}
