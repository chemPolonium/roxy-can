import type { SignalSeries } from "../types";
import { cursorLabels, type ChartCursor } from "./SignalChart";

function valueAt(points: [number, number][], t: number): number | null {
  if (points.length === 0) return null;
  if (t <= points[0][0]) return points[0][1];
  const last = points[points.length - 1];
  if (t >= last[0]) return last[1];
  let lo = 0;
  let hi = points.length - 1;
  while (lo + 1 < hi) {
    const mid = (lo + hi) >> 1;
    if (points[mid][0] <= t) lo = mid;
    else hi = mid;
  }
  const [t0, v0] = points[lo];
  const [t1, v1] = points[hi];
  const span = t1 - t0;
  return span > 0 ? v0 + ((v1 - v0) * (t - t0)) / span : v0;
}

function formatValue(value: number | null): string {
  if (value === null || !Number.isFinite(value)) return "—";
  const magnitude = Math.abs(value);
  if (magnitude >= 1000) return value.toFixed(0);
  if (magnitude >= 100) return value.toFixed(1);
  if (magnitude >= 1) return value.toFixed(2);
  return value.toFixed(3);
}

function rangeStats(
  points: [number, number][],
  from: number,
  to: number,
): { min: number; max: number; avg: number; std: number; count: number } | null {
  let min = Number.POSITIVE_INFINITY;
  let max = Number.NEGATIVE_INFINITY;
  let sum = 0;
  let squares = 0;
  let count = 0;
  for (const [t, value] of points) {
    if (t < from || t > to) continue;
    min = Math.min(min, value);
    max = Math.max(max, value);
    sum += value;
    squares += value * value;
    count += 1;
  }
  if (count === 0) return null;
  const avg = sum / count;
  const variance = Math.max(0, squares / count - avg * avg);
  return { min, max, avg, std: Math.sqrt(variance), count };
}

export function CursorAnalysis({
  series,
  cursors,
  onExportRange,
}: {
  series: SignalSeries[];
  cursors: ChartCursor[];
  onExportRange?: () => void;
}) {
  const labels = cursorLabels(cursors);
  const aIndex = labels.indexOf("A");
  const bIndex = labels.indexOf("B");
  if (aIndex < 0 || bIndex < 0) return null;
  const a = cursors[aIndex];
  const b = cursors[bIndex];
  const from = Math.min(a.t, b.t);
  const to = Math.max(a.t, b.t);

  return (
    <section class="cursor-analysis" aria-label="A/B cursor analysis">
      <header class="cursor-analysis-head">
        <strong>A/B analysis</strong>
        <span class="hint">
          {from.toFixed(3)} – {to.toFixed(3)} s
        </span>
        {onExportRange && (
          <button
            type="button"
            class="mini"
            title="Export this interval as an aligned signal CSV"
            onClick={onExportRange}
          >
            Export range…
          </button>
        )}
      </header>
      <div class="cursor-analysis-labels" aria-hidden="true">
        <span>Signal</span>
        <span>A</span>
        <span>B</span>
        <span>Δ</span>
      </div>
      <div class="cursor-analysis-body">
        {series.map((signal) => {
          const valueA = valueAt(signal.points, a.t);
          const valueB = valueAt(signal.points, b.t);
          const stats = rangeStats(signal.points, from, to);
          return (
            <div class="cursor-analysis-row" key={signal.key}>
              <div class="cursor-analysis-main">
                <span class="cursor-analysis-name" title={signal.name}>
                  {signal.name}
                  {signal.unit && <em>{signal.unit}</em>}
                </span>
                <span>{formatValue(valueA)}</span>
                <span>{formatValue(valueB)}</span>
                <span class="cursor-analysis-delta">
                  {valueA !== null && valueB !== null
                    ? `${valueB - valueA >= 0 ? "+" : ""}${formatValue(valueB - valueA)}`
                    : "—"}
                </span>
              </div>
              {stats && (
                <div class="cursor-analysis-sub">
                  min {formatValue(stats.min)} · avg {formatValue(stats.avg)} ·
                  max {formatValue(stats.max)} · σ {formatValue(stats.std)} ·{" "}
                  {stats.count.toLocaleString()} pts
                </div>
              )}
            </div>
          );
        })}
      </div>
    </section>
  );
}
