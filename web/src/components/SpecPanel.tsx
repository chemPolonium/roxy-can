import { useEffect, useState } from "preact/hooks";
import type { SpecRow } from "../types";

interface Props {
  rows: SpecRow[];
  tolerance: number;
  grace: number;
  periodic: number;
  onTolerance: (value: number) => void;
  onGrace: (value: number) => void;
  onClear: () => void;
  onExport: () => void;
}

const RULES = ["Unknown id", "DLC", "Cycle", "Dropped"];

function NumCommit({
  value,
  min,
  max,
  suffix,
  onCommit,
  title,
}: {
  value: number;
  min: number;
  max: number;
  suffix: string;
  onCommit: (value: number) => void;
  title: string;
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
      const next = Math.min(max, Math.max(min, Math.round(parsed)));
      setText(String(next));
      if (next !== value) onCommit(next);
    } else {
      setText(String(value));
    }
  };
  return (
    <label title={title}>
      <input
        value={text}
        inputMode="numeric"
        onFocus={() => setEditing(true)}
        onInput={(event) => setText((event.target as HTMLInputElement).value)}
        onBlur={commit}
        onKeyDown={(event) => {
          if (event.key === "Enter") (event.target as HTMLInputElement).blur();
        }}
      />
      {suffix}
    </label>
  );
}

export function SpecPanel({
  rows,
  tolerance,
  grace,
  periodic,
  onTolerance,
  onGrace,
  onClear,
  onExport,
}: Props) {
  const [shown, setShown] = useState<Set<string>>(new Set(RULES));
  const visible = rows.filter((row) => shown.has(row.rule));
  const toggle = (rule: string) => {
    setShown((current) => {
      const next = new Set(current);
      if (next.has(rule)) next.delete(rule);
      else next.add(rule);
      return next;
    });
  };

  return (
    <div class="spec-panel">
      <div class="spec-tools">
        <span class="spec-label">tolerance</span>
        <NumCommit
          value={tolerance}
          min={0}
          max={100}
          suffix="%"
          title="Allowed cycle deviation before a verdict"
          onCommit={onTolerance}
        />
        <span class="spec-label">grace</span>
        <NumCommit
          value={grace}
          min={1}
          max={20}
          suffix="x"
          title="Cycles a message may stay absent before a Missing verdict"
          onCommit={onGrace}
        />
        <span class="spec-periodic">{periodic} periodic messages declared</span>
        <span class="spacer" />
        <button type="button" class="mini" onClick={onExport}>
          Export…
        </button>
        <button type="button" class="mini" onClick={onClear}>
          Clear
        </button>
      </div>
      <div class="spec-filters">
        {RULES.map((rule) => (
          <label key={rule} class={shown.has(rule) ? "on" : ""}>
            <input
              type="checkbox"
              checked={shown.has(rule)}
              onChange={() => toggle(rule)}
            />
            {rule}
          </label>
        ))}
        <span class="spacer" />
        <span class="spec-count">{visible.length} violations</span>
      </div>
      {visible.length === 0 ? (
        <div class="spec-empty">no violations</div>
      ) : (
        <div class="spec-list">
          {visible.map((row) => (
            <div class="spec-row" key={`${row.bus}:${row.addr}:${row.rule}`}>
              <div class="spec-main">
                <strong>
                  {row.name} <em>({row.addr})</em>
                </strong>
                <span class="spec-rule">{row.rule}</span>
                <span class="spec-bus">{row.bus}</span>
              </div>
              <div class="spec-detail">
                declared {row.declared} · measured {row.measured} · count{" "}
                {row.count} · {row.first} → {row.last}
              </div>
            </div>
          ))}
        </div>
      )}
      <p class="hint">
        Rules judge the database's promises: unknown ids, DLC mismatches,
        cycle deviation beyond tolerance, and messages absent past the grace
        window. Verdicts latch until Clear or the next run.
      </p>
    </div>
  );
}
