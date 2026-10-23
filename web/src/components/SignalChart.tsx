import { useEffect, useRef, useState } from "preact/hooks";
import type { SignalSeries } from "../types";
import { palette, type Theme } from "../theme";

const GUTTER = 108;
const AXIS_H = 18;
/// A thin status strip above the lanes keeps LIVE/HOLD off the curves.
const TOP = 14;

interface View {
  windowS: number;
  /// null = follow the newest sample; a number freezes a historical window.
  endS: number | null;
}

function clamp(value: number, min: number, max: number): number {
  return Math.min(max, Math.max(min, value));
}

function bounds(series: SignalSeries[]): { first: number; last: number } {
  let first = Number.POSITIVE_INFINITY;
  let last = 0;
  for (const signal of series) {
    if (signal.points.length === 0) continue;
    first = Math.min(first, signal.points[0][0]);
    last = Math.max(last, signal.points[signal.points.length - 1][0]);
  }
  return { first: Number.isFinite(first) ? first : 0, last };
}

/// Index of the first point whose time is >= target.
function lowerBound(points: [number, number][], target: number): number {
  let lo = 0;
  let hi = points.length;
  while (lo < hi) {
    const mid = (lo + hi) >> 1;
    if (points[mid][0] < target) lo = mid + 1;
    else hi = mid;
  }
  return lo;
}

function valueText(value: number): string {
  if (!Number.isFinite(value)) return "—";
  if (Math.abs(value) >= 1000) return value.toFixed(0);
  return value.toFixed(Math.abs(value) >= 100 ? 0 : 1);
}

/// Stacked signal lanes on one canvas. Curves are min/max decimated to the
/// pixel columns, so a 12k-point series costs one vertical span per column
/// instead of one line segment per sample.
export function SignalChart({
  series,
  markers = [],
  theme,
  onReorder,
}: {
  series: SignalSeries[];
  markers?: number[];
  theme: Theme;
  onReorder: (key: string, to: number) => void;
}) {
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const [view, setView] = useState<View>({ windowS: 30, endS: null });
  const dragRef = useRef<{ x: number; endS: number } | null>(null);
  const reorderRef = useRef<{ key: string; from: number; to: number } | null>(
    null,
  );
  const [reorder, setReorder] = useState<{ from: number; to: number } | null>(
    null,
  );
  const timeBounds = bounds(series);
  const liveEnd = view.endS ?? timeBounds.last;

  /// Which lane is under the pointer. Reorder drags clamp above/below the
  /// stack to the first/last lane; a plain click outside the lanes does not
  /// start a drag.
  const laneAt = (clientY: number, clampOutside = false): number | null => {
    const canvas = canvasRef.current;
    if (!canvas || series.length === 0) return null;
    const rect = canvas.getBoundingClientRect();
    const laneH = Math.max(1, (rect.height - TOP - AXIS_H) / series.length);
    const y = clientY - rect.top;
    if (y < TOP || y >= TOP + laneH * series.length) {
      if (!clampOutside) return null;
      return y < TOP ? 0 : series.length - 1;
    }
    return clamp(Math.floor((y - TOP) / laneH), 0, series.length - 1);
  };

  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;
    const colors = palette(theme);

    const draw = () => {
      const width = canvas.clientWidth;
      const height = canvas.clientHeight;
      if (width === 0 || height === 0) return;
      const dpr = window.devicePixelRatio || 1;
      canvas.width = Math.round(width * dpr);
      canvas.height = Math.round(height * dpr);
      const ctx = canvas.getContext("2d");
      if (!ctx) return;
      ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
      ctx.clearRect(0, 0, width, height);

      if (series.length === 0) {
        ctx.fillStyle = colors.signalEmpty;
        ctx.font = "11px 'Segoe UI', sans-serif";
        ctx.textAlign = "center";
        ctx.fillText("No signals subscribed", width / 2, height / 2 - 2);
        ctx.fillStyle = colors.signalHint;
        ctx.font = "9px 'Segoe UI', sans-serif";
        ctx.fillText(
          "The demo session wires eight DBC signals",
          width / 2,
          height / 2 + 14,
        );
        return;
      }

      const live = view.endS === null;
      const range = bounds(series);
      const end = live
        ? range.last
        : clamp(
            view.endS ?? range.last,
            range.first + view.windowS,
            range.last,
          );
      const start = end - view.windowS;
      const lanes = Math.max(1, series.length);
      const plotW = Math.max(1, width - GUTTER - 8);
      const laneH = Math.max(1, (height - TOP - AXIS_H) / lanes);

      // Trigger markers: amber dashed lines behind the lanes.
      ctx.save();
      ctx.strokeStyle = colors.signalMarker;
      ctx.globalAlpha = 0.5;
      ctx.lineWidth = 1;
      ctx.setLineDash([3, 4]);
      ctx.beginPath();
      for (const marker of markers) {
        if (marker < start || marker > end) continue;
        const x = GUTTER + ((marker - start) / view.windowS) * plotW + 0.5;
        ctx.moveTo(x, TOP);
        ctx.lineTo(x, height - AXIS_H);
      }
      ctx.stroke();
      ctx.restore();

      ctx.font = "10px 'Segoe UI', sans-serif";
      ctx.textBaseline = "alphabetic";
      ctx.textAlign = "left";

      for (let i = 0; i < series.length; i += 1) {
        const signal = series[i];
        const top = TOP + i * laneH;
        const color = colors.signals[signal.color % colors.signals.length];
        if (i % 2 === 1) {
          ctx.fillStyle = colors.signalStripe;
          ctx.fillRect(0, top, width, laneH);
        }
        ctx.strokeStyle = colors.signalLine;
        ctx.beginPath();
        ctx.moveTo(0, top + laneH - 0.5);
        ctx.lineTo(width, top + laneH - 0.5);
        ctx.stroke();

        const points = signal.points;
        const from = lowerBound(points, start);
        const to = lowerBound(points, end);
        const visible = to - from;
        const current = points.length ? points[points.length - 1][1] : null;

        // Gutter: colour tab, name, then bus/current value.
        ctx.fillStyle = color;
        ctx.fillRect(4, top + 5, 3, Math.max(10, laneH - 12));
        ctx.fillStyle = colors.signalName;
        ctx.font = "10px 'Segoe UI', sans-serif";
        ctx.fillText(signal.name, 13, top + 13);
        ctx.fillStyle = colors.signalMeta;
        ctx.font = "9px 'Cascadia Mono', Consolas, monospace";
        const unit = signal.unit ? ` ${signal.unit}` : "";
        ctx.fillText(
          `${signal.bus} · ${current === null ? "—" : valueText(current)}${unit}`,
          13,
          top + 25,
        );

        if (visible <= 0) {
          ctx.fillStyle = colors.signalNoSamples;
          ctx.font = "9px 'Segoe UI', sans-serif";
          ctx.fillText("no samples in view", GUTTER + 8, top + laneH / 2 + 3);
          continue;
        }

        let min = Number.POSITIVE_INFINITY;
        let max = Number.NEGATIVE_INFINITY;
        for (let k = from; k < to; k += 1) {
          const value = points[k][1];
          if (value < min) min = value;
          if (value > max) max = value;
        }
        if (min === max) {
          min -= 1;
          max += 1;
        }
        const span = max - min;
        min -= span * 0.1;
        max += span * 0.1;
        const plotTop = top + 4;
        const plotH = laneH - 8;
        const y = (value: number) =>
          plotTop + plotH - ((value - min) / (max - min)) * plotH;

        ctx.strokeStyle = colors.signalMidLine;
        ctx.beginPath();
        ctx.moveTo(GUTTER, plotTop + plotH / 2 + 0.5);
        ctx.lineTo(width - 8, plotTop + plotH / 2 + 0.5);
        ctx.stroke();

        const buckets = Math.max(1, Math.floor(plotW));
        const lows = new Float64Array(buckets).fill(Number.POSITIVE_INFINITY);
        const highs = new Float64Array(buckets).fill(Number.NEGATIVE_INFINITY);
        const last = new Float64Array(buckets).fill(Number.NaN);
        for (let k = from; k < to; k += 1) {
          const [t, value] = points[k];
          const bx = Math.min(
            buckets - 1,
            Math.max(0, Math.floor(((t - start) / view.windowS) * buckets)),
          );
          if (value < lows[bx]) lows[bx] = value;
          if (value > highs[bx]) highs[bx] = value;
          last[bx] = value;
        }

        ctx.strokeStyle = color;
        ctx.globalAlpha = 0.55;
        ctx.lineWidth = 1;
        ctx.beginPath();
        for (let bx = 0; bx < buckets; bx += 1) {
          if (Number.isNaN(last[bx])) continue;
          const x = GUTTER + bx + 0.5;
          ctx.moveTo(x, y(lows[bx]));
          ctx.lineTo(x, y(highs[bx]));
        }
        ctx.stroke();

        ctx.globalAlpha = 1;
        ctx.lineWidth = 1.2;
        ctx.beginPath();
        let started = false;
        for (let bx = 0; bx < buckets; bx += 1) {
          if (Number.isNaN(last[bx])) continue;
          const x = GUTTER + bx + 0.5;
          if (!started) {
            ctx.moveTo(x, y(last[bx]));
            started = true;
          } else {
            ctx.lineTo(x, y(last[bx]));
          }
        }
        ctx.stroke();
      }

      // Shared time axis.
      ctx.font = "9px 'Cascadia Mono', Consolas, monospace";
      ctx.textAlign = "center";
      for (let i = 0; i <= 4; i += 1) {
        const frac = i / 4;
        const x = GUTTER + plotW * frac;
        ctx.strokeStyle = colors.signalTimeGrid;
        ctx.beginPath();
        ctx.moveTo(x + 0.5, TOP);
        ctx.lineTo(x + 0.5, height - AXIS_H);
        ctx.stroke();
        const seconds = -view.windowS * (1 - frac);
        ctx.fillStyle = colors.signalAxisLabel;
        const label =
          Math.abs(seconds) >= 10
            ? `${seconds.toFixed(0)}s`
            : `${seconds.toFixed(1)}s`;
        ctx.fillText(label, x, height - 5);
      }

      if (reorder) {
        const top = TOP + reorder.from * laneH;
        ctx.fillStyle = colors.signalDrag;
        ctx.fillRect(0, top, width, laneH);
        ctx.strokeStyle = colors.signalAccent;
        ctx.lineWidth = 1;
        ctx.strokeRect(0.5, top + 0.5, width - 1, laneH - 1);
        if (reorder.to !== reorder.from) {
          const marker =
            TOP +
            (reorder.to > reorder.from ? reorder.to + 1 : reorder.to) * laneH;
          ctx.fillStyle = colors.signalAccent;
          ctx.fillRect(0, marker - 1, width, 2);
        }
      }

      ctx.textAlign = "right";
      ctx.font = "9px 'Segoe UI', sans-serif";
      const tag = live ? "LIVE" : "HOLD · double-click to follow";
      const tagWidth = ctx.measureText(tag).width;
      ctx.fillStyle = colors.signalTagBg;
      ctx.fillRect(width - tagWidth - 16, 1, tagWidth + 14, 12);
      ctx.fillStyle = live ? colors.signalLive : colors.signalHold;
      ctx.fillText(tag, width - 8, 10);
    };

    draw();
    const observer = new ResizeObserver(draw);
    observer.observe(canvas);
    return () => observer.disconnect();
  }, [series, view, markers, theme, reorder]);

  const plotWidth = () => {
    const canvas = canvasRef.current;
    return Math.max(1, (canvas?.clientWidth ?? 0) - GUTTER - 8);
  };

  return (
    <canvas
      class={`signal-chart ${reorder ? "reordering" : ""}`}
      ref={canvasRef}
      aria-label="Live signal curves"
      title="Drag a signal name to reorder · wheel zooms the time window"
      onWheel={(event) => {
        event.preventDefault();
        const canvas = canvasRef.current;
        if (!canvas) return;
        const rect = canvas.getBoundingClientRect();
        const plotW = plotWidth();
        const frac = clamp(
          (event.clientX - rect.left - GUTTER) / plotW,
          0,
          1,
        );
        const nextWindow = clamp(
          view.windowS * Math.exp(event.deltaY * 0.0015),
          1,
          120,
        );
        const end = liveEnd;
        const at = end - view.windowS + frac * view.windowS;
        const nextEnd = clamp(
          at + (1 - frac) * nextWindow,
          timeBounds.first + nextWindow,
          timeBounds.last,
        );
        // Zooming must not take the chart out of LIVE; only a pan changes
        // whether the window follows the newest sample.
        setView({
          windowS: nextWindow,
          endS: view.endS === null ? null : nextEnd,
        });
      }}
      onPointerDown={(event) => {
        if (event.button !== 0) return;
        const rect = event.currentTarget.getBoundingClientRect();
        if (event.clientX - rect.left < GUTTER) {
          const lane = laneAt(event.clientY);
          if (lane === null) return;
          event.currentTarget.setPointerCapture(event.pointerId);
          reorderRef.current = { key: series[lane].key, from: lane, to: lane };
          setReorder({ from: lane, to: lane });
          return;
        }
        event.currentTarget.setPointerCapture(event.pointerId);
        dragRef.current = {
          x: event.clientX,
          endS: liveEnd,
        };
        setView((old) => ({ ...old, endS: liveEnd }));
      }}
      onPointerMove={(event) => {
        const reordering = reorderRef.current;
        if (reordering) {
          const to = laneAt(event.clientY, true) ?? reordering.to;
          if (to !== reordering.to) {
            reorderRef.current = { ...reordering, to };
            setReorder({ from: reordering.from, to });
          }
          return;
        }
        const drag = dragRef.current;
        if (!drag) return;
        const delta =
          ((drag.x - event.clientX) / plotWidth()) * view.windowS;
        const nextEnd = clamp(
          drag.endS + delta,
          timeBounds.first + view.windowS,
          timeBounds.last,
        );
        setView((old) => ({
          ...old,
          endS: nextEnd >= timeBounds.last ? null : nextEnd,
        }));
      }}
      onPointerUp={(event) => {
        const reordering = reorderRef.current;
        reorderRef.current = null;
        setReorder(null);
        dragRef.current = null;
        if (event.currentTarget.hasPointerCapture(event.pointerId)) {
          event.currentTarget.releasePointerCapture(event.pointerId);
        }
        if (reordering && reordering.to !== reordering.from) {
          onReorder(reordering.key, reordering.to);
        }
      }}
      onPointerCancel={() => {
        reorderRef.current = null;
        setReorder(null);
        dragRef.current = null;
      }}
      onDblClick={(event) => {
        const rect = event.currentTarget.getBoundingClientRect();
        if (event.clientX - rect.left >= GUTTER) {
          setView({ windowS: 30, endS: null });
        }
      }}
    />
  );
}
