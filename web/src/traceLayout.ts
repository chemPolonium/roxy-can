export type TraceColumnId =
  | "time"
  | "bus"
  | "id"
  | "name"
  | "dir"
  | "len"
  | "data";

export interface TraceColumnPrefs {
  widths: Partial<Record<TraceColumnId, number>>;
  hidden: TraceColumnId[];
}

export interface TraceColumnDef {
  id: TraceColumnId;
  label: string;
  width: number;
  min: number;
  max: number;
  /// The last column absorbs the remaining width and has no drag handle.
  flex?: boolean;
  /// Time is the row anchor and cannot be removed.
  locked?: boolean;
}

export const TRACE_COLUMNS: TraceColumnDef[] = [
  { id: "time", label: "Time", width: 92, min: 72, max: 220, locked: true },
  { id: "bus", label: "Bus", width: 54, min: 40, max: 140 },
  { id: "id", label: "ID", width: 80, min: 56, max: 180 },
  { id: "name", label: "Name", width: 170, min: 80, max: 420 },
  { id: "dir", label: "Dir", width: 40, min: 34, max: 90 },
  { id: "len", label: "Len", width: 46, min: 38, max: 120 },
  { id: "data", label: "Data", width: 0, min: 0, max: 0, flex: true },
];

const KEY = "roxy-can-web.trace-columns";

export const DEFAULT_TRACE_COLUMNS: TraceColumnPrefs = {
  widths: Object.fromEntries(
    TRACE_COLUMNS.filter((column) => !column.flex).map((column) => [
      column.id,
      column.width,
    ]),
  ),
  hidden: [],
};

export function columnWidth(
  prefs: TraceColumnPrefs,
  id: TraceColumnId,
): number {
  const def = TRACE_COLUMNS.find((column) => column.id === id);
  const width = prefs.widths[id];
  return typeof width === "number" && Number.isFinite(width)
    ? Math.min(def?.max ?? width, Math.max(def?.min ?? 0, width))
    : (def?.width ?? 80);
}

export function loadTraceColumns(): TraceColumnPrefs {
  try {
    const raw = localStorage.getItem(KEY);
    if (!raw) return DEFAULT_TRACE_COLUMNS;
    const saved = JSON.parse(raw) as Partial<TraceColumnPrefs>;
    const widths: TraceColumnPrefs["widths"] = {};
    for (const column of TRACE_COLUMNS) {
      if (column.flex) continue;
      widths[column.id] = columnWidth(
        { widths: saved.widths ?? {}, hidden: [] },
        column.id,
      );
    }
    const hidden = (Array.isArray(saved.hidden) ? saved.hidden : []).filter(
      (id): id is TraceColumnId =>
        TRACE_COLUMNS.some((column) => column.id === id && !column.locked && !column.flex),
    );
    return { widths, hidden };
  } catch {
    return DEFAULT_TRACE_COLUMNS;
  }
}

export function saveTraceColumns(prefs: TraceColumnPrefs): void {
  try {
    localStorage.setItem(KEY, JSON.stringify(prefs));
  } catch {
    // Storage can be unavailable in private mode; the session still works.
  }
}
