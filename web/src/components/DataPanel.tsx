import { useState } from "preact/hooks";
import type { DataRow } from "../types";

interface Props {
  rows: DataRow[];
  filtered?: boolean;
  /// Row drag: the `sig:` key lands before slot `to`.
  onMoveRow?: (key: string, to: number) => void;
  onVisible: (row: DataRow, visible: boolean) => void;
  onRemove: (row: DataRow) => void;
}

export function DataPanel({
  rows,
  filtered = false,
  onMoveRow,
  onVisible,
  onRemove,
}: Props) {
  // Reordering needs the unfiltered list; the hover index would otherwise
  // not match the backend's slots.
  const sortable = !!onMoveRow && !filtered;
  const [dragKey, setDragKey] = useState<string | null>(null);
  const [overKey, setOverKey] = useState<string | null>(null);

  const dropOn = (target: DataRow) => {
    if (!onMoveRow || !dragKey || dragKey === target.key) {
      setDragKey(null);
      setOverKey(null);
      return;
    }
    const from = rows.findIndex((row) => row.key === dragKey);
    const over = rows.findIndex((row) => row.key === target.key);
    if (from >= 0 && over >= 0) {
      // `to` is the post-removal index, so the row lands before the one
      // it was dropped on in either direction.
      onMoveRow(dragKey, from < over ? over - 1 : over);
    }
    setDragKey(null);
    setOverKey(null);
  };

  if (rows.length === 0) {
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
    <div class="data-list">
      {rows.map((row) => (
        <div
          class={`data-row ${row.visible ? "" : "hidden"} ${
            overKey === row.key && dragKey !== row.key ? "drag-over" : ""
          } ${dragKey === row.key ? "dragging" : ""}`}
          key={`${row.kind}:${row.ch}:${row.id}:${row.ext}:${row.name}`}
          draggable={sortable}
          title={
            sortable
              ? "Drag onto another row to reorder; the order is saved with the project"
              : undefined
          }
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
            setOverKey(row.key);
          }}
          onDragLeave={() => {
            if (overKey === row.key) setOverKey(null);
          }}
          onDrop={(event) => {
            event.preventDefault();
            dropOn(row);
          }}
          onDragEnd={() => {
            setDragKey(null);
            setOverKey(null);
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
            title={`${row.bus} · ${row.path || row.name}`}
          >
            <strong>{row.name}</strong>
            <small>
              {row.bus}
              {row.path ? ` · ${row.path}` : ""}
            </small>
          </div>
          <div class="data-value" title={row.value}>
            {row.value}
            {row.unit && <em>{row.unit}</em>}
          </div>
          <div class="data-raw" title={`raw ${row.raw}`}>
            raw {row.raw}
          </div>
          <button
            type="button"
            class="mini"
            title="Remove this signal"
            onClick={() => onRemove(row)}
          >
            ×
          </button>
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
  );
}
