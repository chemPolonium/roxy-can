import type { DataRow } from "../types";

interface Props {
  rows: DataRow[];
  onVisible: (row: DataRow, visible: boolean) => void;
  onRemove: (row: DataRow) => void;
}

export function DataPanel({ rows, onVisible, onRemove }: Props) {
  if (rows.length === 0) {
    return (
      <div class="data-empty">
        no signals -- add one to watch value, raw wire value, min/avg/max and
        the database-scaled bar
      </div>
    );
  }
  return (
    <div class="data-list">
      {rows.map((row) => (
        <div
          class={`data-row ${row.visible ? "" : "hidden"}`}
          key={`${row.ch}:${row.id}:${row.ext}:${row.name}`}
        >
          <label class="data-check" title="Show this signal in the value table">
            <input
              type="checkbox"
              checked={row.visible}
              onChange={(event) =>
                onVisible(row, (event.target as HTMLInputElement).checked)
              }
            />
          </label>
          <div class="data-name" title={`${row.bus} · ${row.name}`}>
            <strong>{row.name}</strong>
            <small>{row.bus}</small>
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
