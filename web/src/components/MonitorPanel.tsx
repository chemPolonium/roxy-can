import { useEffect, useState } from "preact/hooks";
import type { MonitorRow } from "../types";

export interface MonitorPatch {
  digits?: number;
  ruleOn?: boolean;
  rising?: boolean;
  threshold?: number;
  color?: number;
}

interface Props {
  rows: MonitorRow[];
  onEdit: (row: MonitorRow, patch: MonitorPatch) => void;
  onRemove: (row: MonitorRow) => void;
}

/** Rule colors; the row's `color` index cycles through this palette. */
const COLORS = ["#4c9dff", "#59c48a", "#e6b455", "#e17171", "#b07cf0", "#57c8d8"];

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

export function MonitorPanel({ rows, onEdit, onRemove }: Props) {
  if (rows.length === 0) {
    return (
      <div class="monitor-empty">
        no rows -- add a signal to watch its live value from across the room
      </div>
    );
  }
  return (
    <div class="monitor-list">
      {rows.map((row) => {
        const color = COLORS[row.color % COLORS.length];
        const hot =
          row.hasValue &&
          row.ruleOn &&
          ((row.rising && row.value >= row.threshold) ||
            (!row.rising && row.value <= row.threshold));
        return (
          <div
            class={`monitor-row ${hot ? "hot" : ""}`}
            key={`${row.kind}:${row.ch}:${row.id}:${row.ext}:${row.name}`}
            style={{ borderLeftColor: color }}
          >
            <div class="monitor-main">
              <span class="monitor-label">
                {row.label}
                <small>
                  {row.bus} · {row.name}
                </small>
              </span>
              <strong class="monitor-value" style={{ color: hot ? color : undefined }}>
                {row.hasValue ? row.value.toFixed(row.digits) : "—"}
                {row.hasValue && row.unit && <em>{row.unit}</em>}
              </strong>
              <button
                type="button"
                class="mini"
                title="Remove this row"
                onClick={() => onRemove(row)}
              >
                ×
              </button>
            </div>
            <div class="monitor-rule">
              <button
                type="button"
                class={`mini ${row.ruleOn ? "on" : ""}`}
                title="Toggle the coloring rule"
                onClick={() => onEdit(row, { ruleOn: !row.ruleOn })}
              >
                rule {row.ruleOn ? "on" : "off"}
              </button>
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
                onClick={() => onEdit(row, { digits: (row.digits + 1) % 4 })}
              >
                {row.digits} dp
              </button>
              <button
                type="button"
                class="monitor-color"
                style={{ background: color }}
                title="Cycle the rule color"
                onClick={() => onEdit(row, { color: (row.color + 1) % COLORS.length })}
              />
            </div>
          </div>
        );
      })}
    </div>
  );
}
