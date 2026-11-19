import { useEffect, useRef, useState } from "preact/hooks";
import type { MonitorRow } from "../types";
import { palette, type Theme } from "../theme";
import { rowPathWithoutName } from "../signalRows";

interface Props {
  rows: MonitorRow[];
  filtered?: boolean;
  columns?: number;
  theme: Theme;
  /// Row drag: the `sig:` key lands before/after the target slot.
  onMoveRow?: (key: string, to: number) => void;
  onConfigure: (row: MonitorRow) => void;
  onRemove: (row: MonitorRow) => void;
}

/// The band holding `value`: the number of cuts at or below it.
export function monitorBand(value: number, cuts: number[]): number {
  let index = 0;
  while (index < cuts.length && value >= cuts[index]) index += 1;
  return index;
}

export function MonitorPanel({
  rows,
  filtered = false,
  columns = 1,
  theme,
  onMoveRow,
  onConfigure,
  onRemove,
}: Props) {
  const colors = palette(theme).monitors;
  // Reordering needs the unfiltered list; the hover index would otherwise
  // not match the backend's slots.
  const sortable = !!onMoveRow && !filtered;
  const [dragKey, setDragKey] = useState<string | null>(null);
  const [drop, setDrop] = useState<{
    key: string;
    after: boolean;
  } | null>(null);
  const [menu, setMenu] = useState<{
    row: MonitorRow;
    x: number;
    y: number;
  } | null>(null);
  const menuRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!menu) return;
    menuRef.current?.querySelector<HTMLButtonElement>("button")?.focus();
    const close = (event: MouseEvent) => {
      if (!menuRef.current?.contains(event.target as Node)) setMenu(null);
    };
    const key = (event: KeyboardEvent) => {
      if (event.key === "Escape") setMenu(null);
    };
    const scroll = () => setMenu(null);
    document.addEventListener("mousedown", close);
    document.addEventListener("keydown", key);
    window.addEventListener("scroll", scroll, true);
    return () => {
      document.removeEventListener("mousedown", close);
      document.removeEventListener("keydown", key);
      window.removeEventListener("scroll", scroll, true);
    };
  }, [menu]);

  const openMenu = (event: MouseEvent, row: MonitorRow) => {
    event.preventDefault();
    const width = 160;
    const height = 80;
    setMenu({
      row,
      x: Math.max(8, Math.min(event.clientX, window.innerWidth - width - 8)),
      y: Math.max(8, Math.min(event.clientY, window.innerHeight - height - 8)),
    });
  };

  const sideAt = (event: DragEvent, element: HTMLElement) => {
    const rect = element.getBoundingClientRect();
    return event.clientY >= rect.top + rect.height / 2;
  };

  const dropOn = (target: MonitorRow, after: boolean) => {
    if (!onMoveRow || !dragKey || dragKey === target.key) {
      setDragKey(null);
      setDrop(null);
      return;
    }
    const from = rows.findIndex((row) => row.key === dragKey);
    const over = rows.findIndex((row) => row.key === target.key);
    if (from >= 0 && over >= 0) {
      // `to` is the index in the post-removal list. Before the target is
      // `over - 1` when dragging down, after it is `over`; the other
      // direction uses the target's own slot.
      const to = after
        ? from < over
          ? over
          : over + 1
        : from < over
          ? over - 1
          : over;
      onMoveRow(dragKey, Math.max(0, Math.min(rows.length - 1, to)));
    }
    setDragKey(null);
    setDrop(null);
  };

  if (rows.length === 0) {
    return (
      <div class="empty">
        <strong>{filtered ? "No matching signals" : "No signals yet"}</strong>
        <span>
          {filtered
            ? "Try a different signal, frame, PDU, bus or unit."
            : "Add a signal to watch its live value."}
        </span>
      </div>
    );
  }
  const colorAt = (slot: number) => colors[Math.abs(slot) % colors.length];
  return (
    <div class="monitor-host">
      <div class={`monitor-list ${columns >= 2 ? "two-col" : ""}`}>
        {rows.map((row) => {
          const base = colorAt(row.colors[0] ?? 0);
          const band = row.cuts.length ? monitorBand(row.value, row.cuts) : 0;
          const active = colorAt(row.colors[band] ?? row.colors[0] ?? 0);
          return (
            <div
              class={`monitor-row ${
                drop?.key === row.key && dragKey !== row.key
                  ? drop.after
                    ? "drag-after"
                    : "drag-before"
                  : ""
              } ${dragKey === row.key ? "dragging" : ""}`}
              key={`${row.kind}:${row.ch}:${row.id}:${row.ext}:${row.name}`}
              style={{ borderLeftColor: base }}
              draggable={sortable}
              title={
                sortable
                  ? "Right-click for settings or remove · drag to reorder"
                  : "Right-click for settings or remove"
              }
              onContextMenu={(event) => openMenu(event, row)}
              onDragStart={(event) => {
                if (!sortable) return;
                // Dragging from inside a field stays a text selection;
                // the row handle is everywhere else on the card.
                const tag = (event.target as HTMLElement).tagName;
                if (tag === "INPUT" || tag === "TEXTAREA" || tag === "SELECT") {
                  event.preventDefault();
                  return;
                }
                setDragKey(row.key);
                event.dataTransfer?.setData("text/plain", row.key);
                if (event.dataTransfer) {
                  event.dataTransfer.effectAllowed = "move";
                }
              }}
              onDragOver={(event) => {
                if (!sortable || !dragKey) return;
                event.preventDefault();
                if (event.dataTransfer) {
                  event.dataTransfer.dropEffect = "move";
                }
                const after = sideAt(event, event.currentTarget as HTMLElement);
                setDrop((old) =>
                  old?.key === row.key && old.after === after
                    ? old
                    : { key: row.key, after },
                );
              }}
              onDragLeave={(event) => {
                const next = event.relatedTarget as Node | null;
                if (
                  drop?.key === row.key &&
                  (!next || !(event.currentTarget as HTMLElement).contains(next))
                ) {
                  setDrop(null);
                }
              }}
              onDrop={(event) => {
                event.preventDefault();
                dropOn(row, sideAt(event, event.currentTarget as HTMLElement));
              }}
              onDragEnd={() => {
                setDragKey(null);
                setDrop(null);
              }}
            >
              <div class="monitor-main">
                <span
                  class="monitor-label"
                  title={`${row.bus}${
                    row.path ? ` · ${rowPathWithoutName(row.path, row.name)}` : ""
                  }`}
                >
                  {row.label}
                  <small>
                    {row.bus}
                    {row.path
                      ? ` · ${rowPathWithoutName(row.path, row.name)}`
                      : ""}
                  </small>
                </span>
                <strong
                  class="monitor-value"
                  style={{
                    color:
                      row.hasValue && row.cuts.length ? active : undefined,
                  }}
                >
                  {row.hasValue ? row.value.toFixed(row.digits) : "—"}
                  {row.hasValue && row.unit && <em>{row.unit}</em>}
                </strong>
              </div>
            </div>
          );
        })}
      </div>
      {menu && (
        <div
          class="row-menu monitor-row-menu"
          role="menu"
          ref={menuRef}
          style={{ top: `${menu.y}px`, left: `${menu.x}px` }}
          onMouseDown={(event) => event.stopPropagation()}
        >
          <button
            type="button"
            role="menuitem"
            onClick={() => {
              setMenu(null);
              onConfigure(menu.row);
            }}
          >
            Settings…
          </button>
          <button
            type="button"
            role="menuitem"
            class="danger"
            onClick={() => {
              setMenu(null);
              onRemove(menu.row);
            }}
          >
            Remove
          </button>
        </div>
      )}
    </div>
  );
}
