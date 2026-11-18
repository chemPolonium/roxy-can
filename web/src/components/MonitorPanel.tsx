import { useEffect, useState } from "preact/hooks";
import type { MonitorRow } from "../types";
import { palette, type Theme } from "../theme";

export interface MonitorPatch {
  digits?: number;
  ruleOn?: boolean;
  rising?: boolean;
  threshold?: number;
  color?: number;
}

interface Props {
  rows: MonitorRow[];
  filtered?: boolean;
  theme: Theme;
  /// Row drag: the `sig:` key lands before slot `to`. Absent disables
  /// dragging (the panel is also read-only in some callers).
  onMoveRow?: (key: string, to: number) => void;
  onEdit: (row: MonitorRow, patch: MonitorPatch) => void;
  onRemove: (row: MonitorRow) => void;
}

function ThresholdInput({
  value,
  onCommit,
}: {
  value: number;
  onCommit: (value: number) => void;
}) {
  const [text, setText] = useState(String(value));
  const [editing, setEditing] = useState(false);
  useEffect(() => {
    if (!editing) setText(String(value));
  }, [value, editing]);
  const commit = () => {
    setEditing(false);
    const parsed = Number(text);
    if (Number.isFinite(parsed)) {
      if (parsed !== value) onCommit(parsed);
    } else {
      setText(String(value));
    }
  };
  return (
    <input
      class="monitor-threshold"
      value={text}
      inputMode="decimal"
      title="Color threshold"
      onFocus={() => setEditing(true)}
      onInput={(event) => setText((event.target as HTMLInputElement).value)}
      onBlur={commit}
      onKeyDown={(event) => {
        if (event.key === "Enter") (event.target as HTMLInputElement).blur();
        if (event.key === "Escape") {
          setText(String(value));
          (event.target as HTMLInputElement).blur();
        }
      }}
    />
  );
}

export function MonitorPanel({
  rows,
  filtered = false,
  theme,
  onMoveRow,
  onEdit,
  onRemove,
}: Props) {
  const colors = palette(theme).monitors;
  // Dragging reorders the full list, so it waits until the filter is
  // clear; otherwise "before the hovered row" has no stable meaning.
  const sortable = !!onMoveRow && !filtered;
  const [dragKey, setDragKey] = useState<string | null>(null);
  const [drop, setDrop] = useState<{
    key: string;
    after: boolean;
  } | null>(null);

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
  return (
    <div class="monitor-host">
      <div class="monitor-list">
        {rows.map((row) => {
          const color = colors[row.color % colors.length];
          const hot =
            row.hasValue &&
            row.ruleOn &&
            ((row.rising && row.value >= row.threshold) ||
              (!row.rising && row.value <= row.threshold));
          return (
            <div
              class={`monitor-row ${hot ? "hot" : ""} ${
                drop?.key === row.key && dragKey !== row.key
                  ? drop.after
                    ? "drag-after"
                    : "drag-before"
                  : ""
              } ${dragKey === row.key ? "dragging" : ""}`}
              key={`${row.kind}:${row.ch}:${row.id}:${row.ext}:${row.name}`}
              style={{ borderLeftColor: color }}
              draggable={sortable}
              title={
                sortable
                  ? "Drag onto another row to reorder; the order is saved with the project"
                  : undefined
              }
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
                dropOn(
                  row,
                  sideAt(event, event.currentTarget as HTMLElement),
                );
              }}
              onDragEnd={() => {
                setDragKey(null);
                setDrop(null);
              }}
            >
              <div class="monitor-main">
                <span
                  class="monitor-label"
                  title={`${row.bus} · ${row.path || row.name}`}
                >
                  {row.label}
                  <small>
                    {row.bus}
                    {row.path ? ` · ${row.path}` : ` · ${row.name}`}
                  </small>
                </span>
                <strong
                  class="monitor-value"
                  style={{ color: hot ? color : undefined }}
                >
                  {row.hasValue ? row.value.toFixed(row.digits) : "—"}
                  {row.hasValue && row.unit && <em>{row.unit}</em>}
                </strong>
                {row.kind !== "derived" && (
                  <button
                    type="button"
                    class={`mini monitor-rule-toggle ${
                      row.ruleOn ? "on" : ""
                    }`}
                    aria-pressed={row.ruleOn}
                    title={
                      row.ruleOn
                        ? "Color threshold on; click to turn off"
                        : "Add a color threshold rule"
                    }
                    onClick={() => onEdit(row, { ruleOn: !row.ruleOn })}
                  >
                    rule
                  </button>
                )}
                <button
                  type="button"
                  class="mini"
                  title="Remove this row"
                  onClick={() => onRemove(row)}
                >
                  ×
                </button>
              </div>
              {row.kind !== "derived" && row.ruleOn && (
                <div class="monitor-rule">
                  <ThresholdInput
                    value={row.threshold}
                    onCommit={(threshold) => onEdit(row, { threshold })}
                  />
                  <button
                    type="button"
                    class="mini"
                    title="Flip the comparison direction"
                    onClick={() => onEdit(row, { rising: !row.rising })}
                  >
                    {row.rising ? ">=" : "<="}
                  </button>
                  <button
                    type="button"
                    class="mini"
                    title="Decimal places"
                    onClick={() =>
                      onEdit(row, { digits: (row.digits + 1) % 4 })
                    }
                  >
                    {row.digits} dp
                  </button>
                  <button
                    type="button"
                    class="monitor-color"
                    style={{ background: color }}
                    title="Cycle the rule color"
                    onClick={() =>
                      onEdit(row, { color: (row.color + 1) % colors.length })
                    }
                  />
                </div>
              )}
            </div>
          );
        })}
      </div>
    </div>
  );
}
