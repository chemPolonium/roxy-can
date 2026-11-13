import { useCallback, useEffect, useRef, useState } from "preact/hooks";

/// One table column that can be dragged to a new width. The flexible column
/// absorbs whatever is left and has no drag handle (same model as Trace).
export interface ColumnWidthDef {
  id: string;
  width: number;
  min: number;
  max: number;
  flex?: boolean;
}

export function clampColumnWidth(
  width: number,
  min: number,
  max: number,
): number {
  return Math.min(max, Math.max(min, width));
}

export function defaultColumnWidths(
  defs: ColumnWidthDef[],
): Record<string, number> {
  return Object.fromEntries(
    defs.filter((def) => !def.flex).map((def) => [def.id, def.width]),
  );
}

/// Reads a stored width map, clamping each entry to its column bounds. A
/// missing or malformed entry keeps the default; unknown ids are ignored.
export function parseColumnWidths(
  raw: unknown,
  defs: ColumnWidthDef[],
): Record<string, number> | null {
  if (!raw || typeof raw !== "object") return null;
  const saved = raw as Record<string, unknown>;
  const widths = defaultColumnWidths(defs);
  for (const def of defs) {
    if (def.flex) continue;
    const value = saved[def.id];
    if (typeof value === "number" && Number.isFinite(value)) {
      widths[def.id] = clampColumnWidth(value, def.min, def.max);
    }
  }
  return widths;
}

export function loadColumnWidths(
  key: string,
  defs: ColumnWidthDef[],
): Record<string, number> {
  try {
    const raw = localStorage.getItem(key);
    if (!raw) return defaultColumnWidths(defs);
    return parseColumnWidths(JSON.parse(raw), defs) ?? defaultColumnWidths(defs);
  } catch {
    return defaultColumnWidths(defs);
  }
}

export function saveColumnWidths(
  key: string,
  widths: Record<string, number>,
): void {
  try {
    localStorage.setItem(key, JSON.stringify(widths));
  } catch {
    // Private mode can reject storage; the session still resizes.
  }
}

/// Drag a header edge and keep the new width. Widths persist per table, the
/// same way Trace keeps its column widths.
export function useColumnWidths(
  key: string,
  defs: ColumnWidthDef[],
  external?: {
    widths?: unknown;
    onChange?: (widths: Record<string, number>) => void;
  },
) {
  const [widths, setWidths] = useState(
    () =>
      parseColumnWidths(external?.widths, defs) ?? loadColumnWidths(key, defs),
  );
  const latest = useRef(widths);
  latest.current = widths;
  const lastExternal = useRef<unknown>(external?.widths);
  useEffect(() => {
    const incoming = external?.widths;
    if (lastExternal.current === incoming) return;
    lastExternal.current = incoming;
    const parsed = parseColumnWidths(incoming, defs);
    // A project that carries no widths starts from the defaults; an absent
    // external map on first mount kept the localStorage widths above.
    setWidths(parsed ?? defaultColumnWidths(defs));
  }, [external?.widths, defs]);
  const commit = useCallback(
    (next: Record<string, number>) => {
      setWidths(next);
      saveColumnWidths(key, next);
      lastExternal.current = next;
      external?.onChange?.(next);
    },
    [key, external?.onChange],
  );
  const lastDown = useRef<{ id: string; at: number } | null>(null);
  const startResize = useCallback(
    (event: PointerEvent, id: string) => {
      event.preventDefault();
      event.stopPropagation();
      const def = defs.find((column) => column.id === id);
      if (!def || def.flex) return;
      const now = performance.now();
      // A second press on the same boundary within the double-click window
      // restores that column's default width instead of another drag.
      if (lastDown.current?.id === id && now - lastDown.current.at < 500) {
        lastDown.current = null;
        commit({ ...latest.current, [id]: def.width });
        return;
      }
      lastDown.current = { id, at: now };
      const startX = event.clientX;
      const startWidth = latest.current[id] ?? def.width;
      let width = startWidth;
      const move = (moveEvent: PointerEvent) => {
        width = clampColumnWidth(
          startWidth + moveEvent.clientX - startX,
          def.min,
          def.max,
        );
        setWidths((old) => ({ ...old, [id]: width }));
      };
      const up = () => {
        commit({ ...latest.current, [id]: width });
        document.body.classList.remove("col-resizing");
        window.removeEventListener("pointermove", move);
        window.removeEventListener("pointerup", up);
      };
      document.body.classList.add("col-resizing");
      window.addEventListener("pointermove", move);
      window.addEventListener("pointerup", up);
    },
    [commit, defs],
  );
  return { widths, startResize };
}

/// A resize handle for a header cell. The cell must be positioned (the shared
/// `th` style is sticky, so it already is).
export function ColumnResizeHandle({
  onResize,
}: {
  onResize: (event: PointerEvent) => void;
}) {
  return (
    <span
      class="col-resize"
      role="separator"
      aria-orientation="vertical"
      title="Drag to resize · double-click to reset"
      onPointerDown={onResize}
    />
  );
}
