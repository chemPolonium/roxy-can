import type { SignalSeries } from "../types";
import { cursorLabels, sampleAt, type ChartCursor } from "./SignalChart";

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
  onDownloadRange,
}: {
  series: SignalSeries[];
  cursors: ChartCursor[];
  onExportRange?: () => void;
  onDownloadRange?: (fromS: number, toS: number) => void;
}) {
  const labels = cursorLabels(cursors);
  const aIndex = labels.indexOf("A");
  const bIndex = labels.indexOf("B");
  const live = cursors.length === 0;
  const paired = !live && aIndex >= 0 && bIndex >= 0;
  const primaryIndex = aIndex >= 0 ? aIndex : bIndex >= 0 ? bIndex : 0;
  const primary = live ? null : cursors[primaryIndex];
  const a = paired ? cursors[aIndex] : primary;
  const b = paired ? cursors[bIndex] : null;
  const from = a && b ? Math.min(a.t, b.t) : (primary?.t ?? 0);
  const to = a && b ? Math.max(a.t, b.t) : (primary?.t ?? 0);
  let latest = 0;
  for (const signal of series) {
    const last = signal.points[signal.points.length - 1];
    if (last) latest = Math.max(latest, last[0]);
  }

  return (
    <section
      class={`cursor-analysis ${paired ? "" : "single"}`}
      aria-label={
        paired
          ? "A/B cursor analysis"
          : live
            ? "Live signal values"
            : "Cursor values"
      }
    >
      <header class="cursor-analysis-head">
        <strong>
          {paired ? "A/B analysis" : live ? "Live values" : "Cursor values"}
        </strong>
        <span class="hint">
          {paired
            ? `${from.toFixed(3)} – ${to.toFixed(3)} s`
            : live
              ? latest > 0
                ? `latest ${latest.toFixed(3)} s`
                : "no samples"
              : `${primary!.t.toFixed(3)} s`}
        </span>
        {paired && onDownloadRange && (
          <button
            type="button"
            class="mini"
            title="Download this interval including derived channels"
            onClick={() => onDownloadRange(from, to)}
          >
            CSV ↓
          </button>
        )}
        {paired && onExportRange && (
          <button
            type="button"
            class="mini"
            title="Host export of the live signals in this interval"
            onClick={onExportRange}
          >
            Export range…
          </button>
        )}
      </header>
      <div class="cursor-analysis-labels" aria-hidden="true">
        <span>Signal</span>
        <span>{live ? "Now" : labels[primaryIndex]}</span>
        {paired && (
          <>
            <span>B</span>
            <span>Δ</span>
          </>
        )}
      </div>
      <div class="cursor-analysis-body">
        {series.length === 0 && (
          <div class="cursor-analysis-empty">No signals selected</div>
        )}
        {series.map((signal) => {
          const last = signal.points[signal.points.length - 1];
          const valueA = live
            ? (last?.[1] ?? null)
            : sampleAt(signal.points, a!.t);
          const valueB = b ? sampleAt(signal.points, b.t) : null;
          const stats = b ? rangeStats(signal.points, from, to) : null;
          return (
            <div class="cursor-analysis-row" key={signal.key}>
              <div class="cursor-analysis-main">
                <span class="cursor-analysis-name" title={signal.name}>
                  {signal.name}
                  {signal.unit && <em>{signal.unit}</em>}
                  {signal.read && <em class="cursor-analysis-read">read</em>}
                </span>
                <span>{formatValue(valueA)}</span>
                {paired && <span>{formatValue(valueB)}</span>}
                {paired && (
                  <span class="cursor-analysis-delta">
                    {valueA !== null && valueB !== null
                      ? `${valueB - valueA >= 0 ? "+" : ""}${formatValue(valueB - valueA)}`
                      : "—"}
                  </span>
                )}
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
