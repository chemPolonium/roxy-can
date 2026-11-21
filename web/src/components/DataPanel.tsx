import type { ComponentChildren } from "preact";
import { useEffect, useRef, useState } from "preact/hooks";
import type { DataRow } from "../types";
import { rowPathWithoutName } from "../signalRows";

interface Props {
  rows: DataRow[];
  filtered?: boolean;
  /// Replaces the signal empty state when no bus/project is open.
  empty?: ComponentChildren;
  /// Row drag: the `sig:` key lands before slot `to`.
  onMoveRow?: (key: string, to: number) => void;
  onVisible: (row: DataRow, visible: boolean) => void;
  onRemove: (row: DataRow) => void;
}

export function DataPanel({
  rows,
  filtered = false,
  empty,
  onMoveRow,
  onVisible,
  onRemove,
}: Props) {
  // Reordering needs the unfiltered list; the hover index would otherwise
  // not match the backend's slots.
  const sortable = !!onMoveRow && !filtered;
  const [dragKey, setDragKey] = useState<string | null>(null);
  const [drop, setDrop] = useState<{
    key: string;
    after: boolean;
  } | null>(null);
  const [menu, setMenu] = useState<{
    row: DataRow;
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

  const openMenu = (event: MouseEvent, row: DataRow) => {
    event.preventDefault();
    setMenu({
      row,
      x: Math.max(8, Math.min(event.clientX, window.innerWidth - 160 - 8)),
      y: Math.max(8, Math.min(event.clientY, window.innerHeight - 50 - 8)),
    });
  };

  const sideAt = (event: DragEvent, element: HTMLElement) => {
    const rect = element.getBoundingClientRect();
    return event.clientY >= rect.top + rect.height / 2;
  };

  const dropOn = (target: DataRow, after: boolean) => {
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
    if (empty) return <>{empty}</>;
    return (
      <div class="empty">
        <strong>{filtered ? "No matching signals" : "No signals yet"}</strong>
        <span>
          {filtered
            ? "Try a different signal, frame, PDU, bus or unit."
            : "Add a signal to watch live, raw and scaled values."}
        </span>
      </div>
    );
  }
  return (
    <>
    <div class="data-list">
      {rows.map((row) => (
        <div
          class={`data-row ${row.visible ? "" : "hidden"} ${
            drop?.key === row.key && dragKey !== row.key
              ? drop.after
                ? "drag-after"
                : "drag-before"
              : ""
          } ${dragKey === row.key ? "dragging" : ""}`}
          key={`${row.kind}:${row.ch}:${row.id}:${row.ext}:${row.name}`}
          draggable={sortable}
          title={
            sortable
              ? "Right-click to remove · drag to reorder"
              : "Right-click to remove"
          }
          onContextMenu={(event) => openMenu(event, row)}
          onDragStart={(event) => {
            if (!sortable) return;
            // Dragging from the checkbox or a field keeps its normal
            // behaviour; the row handle is everywhere else on the card.
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
          {row.kind === "derived" ? (
            <span class="data-check" />
          ) : (
            <label class="data-check" title="Show this signal in the value table">
              <input
                type="checkbox"
                checked={row.visible}
                onChange={(event) =>
                  onVisible(row, (event.target as HTMLInputElement).checked)
                }
              />
            </label>
          )}
          <div
            class="data-name"
            title={`${row.bus}${
              row.path ? ` · ${rowPathWithoutName(row.path, row.name)}` : ""
            }`}
          >
            <strong>{row.name}</strong>
            <small>
              {row.bus}
              {row.path ? ` · ${rowPathWithoutName(row.path, row.name)}` : ""}
            </small>
          </div>
          <div class="data-value" title={row.value}>
            {row.value}
            {row.unit && <em>{row.unit}</em>}
          </div>
          <div class="data-raw" title={`raw ${row.raw}`}>
            raw {row.raw}
          </div>
          <div class="data-stats">
            <span>min {row.min}</span>
            <span>avg {row.avg}</span>
            <span>max {row.max}</span>
          </div>
          <div class="data-bar" title="latest value in the declared range">
            <i style={{ width: `${Math.round(row.frac * 1000) / 10}%` }} />
          </div>
        </div>
      ))}
    </div>
    {menu && (
      <div
        ref={menuRef}
        class="row-menu"
        role="menu"
        style={{ top: `${menu.y}px`, left: `${menu.x}px` }}
        onPointerDown={(event) => event.stopPropagation()}
      >
        <button
          type="button"
          role="menuitem"
          class="danger"
          onClick={() => {
            onRemove(menu.row);
            setMenu(null);
          }}
        >
          Remove
        </button>
      </div>
    )}
    </>
  );
}
