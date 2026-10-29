import { useEffect, useRef, useState } from "preact/hooks";
import type { SignalSeries } from "../types";
import { palette, type Theme } from "../theme";

const GUTTER = 108;
const AXIS_H = 18;
/// A thin status strip above the lanes keeps LIVE/HOLD off the curves.
const TOP = 14;

export interface SignalView {
  windowS: number;
  /// null = follow the newest sample; a number freezes a historical window.
  endS: number | null;
}

export interface ChartCursor {
  id: number;
  t: number;
  /// Trace's Set cursor A/B owns these labels; extra chart cursors take the
  /// next free letter.
  label?: "A" | "B";
}

export type ChartSignalAction = "monitor" | "data" | "state" | "remove";

export interface ReferenceOverlay {
  name: string;
  offsetS: number;
  series: Record<string, [number, number][]>;
}

export function cursorLabels(cursors: ChartCursor[]): string[] {
  const used = new Set<string>();
  for (const cursor of cursors) {
    if (cursor.label) used.add(cursor.label);
  }
  let next = 0;
  return cursors.map((cursor) => {
    if (cursor.label) return cursor.label;
    let label = "";
    do {
      label = String.fromCharCode(65 + (next++ % 26));
    } while (used.has(label));
    used.add(label);
    return label;
  });
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

/// Signal value at a cursor time: the latest sample at or before it, matching
/// what the bus actually carried. The line between samples is only a visual
/// join; it must not invent values the signal never had.
export function sampleAt(
  points: [number, number][],
  target: number,
): number | null {
  if (points.length === 0) return null;
  const index = lowerBound(points, target);
  if (index === 0) return points[0][1];
  if (index >= points.length) return points[points.length - 1][1];
  const [t1, v1] = points[index];
  if (t1 === target) return v1;
  return points[index - 1][1];
}

interface MinMaxBin {
  t0: number;
  min: number;
  max: number;
  last: number;
}

interface BinCache {
  step: number;
  firstT: number;
  lastT: number;
  bins: MinMaxBin[];
}

const BIN_STEPS = [
  0.001, 0.002, 0.004, 0.008, 0.016, 0.032, 0.064, 0.128, 0.25, 0.5, 1, 2, 4,
  8, 16, 32, 64,
];

/// A stable absolute-time bucket grid: the same sample belongs to the same
/// bucket no matter where the LIVE window currently starts.
function pickBinStep(windowS: number, plotW: number): number {
  const target = windowS / Math.max(1, plotW);
  let step = BIN_STEPS[0];
  for (const candidate of BIN_STEPS) {
    if (candidate > target) break;
    step = candidate;
  }
  return step;
}

function addBinPoint(
  bins: MinMaxBin[],
  step: number,
  t: number,
  value: number,
): void {
  const t0 = Math.floor(t / step) * step;
  const tail = bins[bins.length - 1];
  if (tail && tail.t0 === t0) {
    if (value < tail.min) tail.min = value;
    if (value > tail.max) tail.max = value;
    tail.last = value;
    return;
  }
  bins.push({ t0, min: value, max: value, last: value });
}

function rebuildBins(points: [number, number][], step: number): BinCache {
  const bins: MinMaxBin[] = [];
  for (const [t, value] of points) addBinPoint(bins, step, t, value);
  return {
    step,
    firstT: points[0][0],
    lastT: points[points.length - 1][0],
    bins,
  };
}

/// Appends only the samples that arrived since the last draw and drops bins
/// whose points have left the ring. Rebuilds from scratch on a reset or a
/// zoom step change.
function updateBins(
  cache: BinCache,
  points: [number, number][],
  step: number,
): BinCache {
  const firstT = points[0][0];
  const lastT = points[points.length - 1][0];
  if (
    cache.step !== step ||
    lastT < cache.lastT ||
    firstT < cache.firstT - 1e-9 ||
    cache.bins.length === 0
  ) {
    return rebuildBins(points, step);
  }

  while (cache.bins.length && cache.bins[0].t0 + step <= firstT) {
    cache.bins.shift();
  }
  // The head bin may still contain samples that were trimmed from the ring.
  if (cache.bins.length && cache.bins[0].t0 < firstT) {
    const t0 = cache.bins[0].t0;
    const from = lowerBound(points, t0);
    const to = lowerBound(points, t0 + step);
    if (from >= to) {
      cache.bins.shift();
    } else {
      let min = Number.POSITIVE_INFINITY;
      let max = Number.NEGATIVE_INFINITY;
      let last = points[from][1];
      for (let i = from; i < to; i++) {
        const value = points[i][1];
        if (value < min) min = value;
        if (value > max) max = value;
        last = value;
      }
      cache.bins[0].min = min;
      cache.bins[0].max = max;
      cache.bins[0].last = last;
    }
  }

  const start = lowerBound(points, cache.lastT + 1e-9);
  for (let i = start; i < points.length; i += 1) {
    addBinPoint(cache.bins, step, points[i][0], points[i][1]);
  }
  cache.firstT = firstT;
  cache.lastT = lastT;
  return cache;
}

function visibleBinRange(
  bins: MinMaxBin[],
  step: number,
  start: number,
  end: number,
): [number, number] {
  let lo = 0;
  let hi = bins.length;
  while (lo < hi) {
    const mid = (lo + hi) >> 1;
    if (bins[mid].t0 + step <= start) lo = mid + 1;
    else hi = mid;
  }
  const from = lo;
  hi = bins.length;
  while (lo < hi) {
    const mid = (lo + hi) >> 1;
    if (bins[mid].t0 > end) hi = mid;
    else lo = mid + 1;
  }
  return [from, lo];
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
  view,
  onView,
  cursors,
  onAddCursor,
  onMoveCursor,
  onClearCursors,
  onSignalAction,
  fullscreen,
  measuring,
  paused,
  reference = null,
  onOpenReference,
  onOpenDerived,
  overlay = false,
  onToggleOverlay,
  hasReadout = false,
  readRange = null,
  readCount = 0,
}: {
  series: SignalSeries[];
  markers?: number[];
  theme: Theme;
  onReorder: (key: string, to: number) => void;
  view: SignalView;
  onView: (update: (old: SignalView) => SignalView) => void;
  cursors: ChartCursor[];
  onAddCursor: (t: number) => void;
  onMoveCursor: (id: number, t: number) => void;
  onClearCursors: () => void;
  onSignalAction: (key: string, action: ChartSignalAction) => void;
  fullscreen: boolean;
  measuring: boolean;
  paused: boolean;
  reference?: ReferenceOverlay | null;
  onOpenReference?: () => void;
  onOpenDerived?: () => void;
  overlay?: boolean;
  onToggleOverlay?: () => void;
  /// Read-only signals exist: keep a cursor usable on an otherwise empty
  /// chart and follow their newest sample.
  hasReadout?: boolean;
  readRange?: { first: number; last: number } | null;
  readCount?: number;
}) {
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const dragRef = useRef<{ x: number; endS: number } | null>(null);
  const reorderRef = useRef<{ key: string; from: number; to: number } | null>(
    null,
  );
  const [reorder, setReorder] = useState<{ from: number; to: number } | null>(
    null,
  );
  const [hoverCursorId, setHoverCursorId] = useState<number | null>(null);
  const [overlayHidden, setOverlayHidden] = useState<Set<string>>(new Set());
  const [laneMenu, setLaneMenu] = useState<{
    x: number;
    y: number;
    index: number;
  } | null>(null);
  const cursorDragRef = useRef<number | null>(null);
  const binCacheRef = useRef(new Map<string, BinCache>());
  const timeBounds =
    series.length > 0 ? bounds(series) : (readRange ?? bounds(series));
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

  const viewWindow = () => {
    const end =
      view.endS === null
        ? timeBounds.last
        : clamp(
            view.endS,
            timeBounds.first + view.windowS,
            timeBounds.last,
          );
    return {
      first: timeBounds.first,
      last: timeBounds.last,
      start: end - view.windowS,
      end,
    };
  };

  const addCursor = () => {
    if (series.length === 0 && !hasReadout) return;
    const { start, end } = viewWindow();
    // Keep successive cursors apart; dropping every one at the exact centre
    // would stack their lines and labels.
    const slots = [0.5, 0.7, 0.3, 0.6, 0.4, 0.8, 0.2, 0.9];
    const fraction = slots[cursors.length % slots.length];
    onAddCursor(start + (end - start) * fraction);
  };

  const cursorAt = (clientX: number, clientY: number): number | null => {
    const canvas = canvasRef.current;
    if (
      !canvas ||
      (series.length === 0 && !hasReadout) ||
      cursors.length === 0
    ) {
      return null;
    }
    const rect = canvas.getBoundingClientRect();
    if (clientY < rect.top + TOP || clientY > rect.bottom - AXIS_H) {
      return null;
    }
    const plotW = Math.max(1, rect.width - GUTTER - 8);
    const { start, end } = viewWindow();
    let hit: { id: number; distance: number } | null = null;
    for (const cursor of cursors) {
      if (cursor.t < start || cursor.t > end) continue;
      const x = GUTTER + ((cursor.t - start) / view.windowS) * plotW;
      const distance = Math.abs(clientX - rect.left - x);
      if (distance <= 5 && (!hit || distance < hit.distance)) {
        hit = { id: cursor.id, distance };
      }
    }
    return hit?.id ?? null;
  };

  const timeAt = (clientX: number): number => {
    const canvas = canvasRef.current;
    if (!canvas) return 0;
    const rect = canvas.getBoundingClientRect();
    const plotW = Math.max(1, rect.width - GUTTER - 8);
    const { start, end } = viewWindow();
    const frac = clamp((clientX - rect.left - GUTTER) / plotW, 0, 1);
    return start + frac * (end - start);
  };

  useEffect(() => {
    if (!laneMenu) return;
    const close = (event: PointerEvent) => {
      if (
        !(event.target instanceof Element) ||
        !event.target.closest(".signal-menu")
      ) {
        setLaneMenu(null);
      }
    };
    const escape = (event: KeyboardEvent) => {
      if (event.key === "Escape") setLaneMenu(null);
    };
    window.addEventListener("pointerdown", close);
    window.addEventListener("keydown", escape);
    return () => {
      window.removeEventListener("pointerdown", close);
      window.removeEventListener("keydown", escape);
    };
  }, [laneMenu]);

  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;
    const colors = palette(theme);
    const labels = cursorLabels(cursors);
    const aIndex = labels.indexOf("A");
    const bIndex = labels.indexOf("B");
    const cursorA = aIndex >= 0 ? cursors[aIndex] : undefined;
    const cursorB = bIndex >= 0 ? cursors[bIndex] : undefined;

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

      if (series.length === 0 && !hasReadout) {
        binCacheRef.current.clear();
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

      const liveKeys = new Set(series.map((signal) => signal.key));
      for (const key of binCacheRef.current.keys()) {
        if (!liveKeys.has(key)) binCacheRef.current.delete(key);
      }

      const live = view.endS === null;
      const range = timeBounds;
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

      if (overlay) {
        const plotTop = TOP + 4;
        const plotBottom = height - AXIS_H - 4;
        const plotH = Math.max(1, plotBottom - plotTop);
        ctx.save();
        ctx.beginPath();
        ctx.rect(GUTTER, TOP, width - GUTTER, height - TOP - AXIS_H);
        ctx.clip();
        for (const signal of series) {
          if (overlayHidden.has(signal.key)) continue;
          const color = colors.signals[signal.color % colors.signals.length];
          let min = Number.POSITIVE_INFINITY;
          let max = Number.NEGATIVE_INFINITY;
          for (const [t, value] of signal.points) {
            if (t < start || t > end) continue;
            if (value < min) min = value;
            if (value > max) max = value;
          }
          if (!Number.isFinite(min)) continue;
          if (min === max) {
            min -= 0.5;
            max += 0.5;
          }
          const toY = (value: number) =>
            plotBottom - ((value - min) / (max - min)) * plotH;
          const stroke = (
            points: [number, number][],
            offset: number,
            dashed: boolean,
          ) => {
            ctx.strokeStyle = color;
            ctx.lineWidth = 1;
            ctx.globalAlpha = dashed ? 0.45 : 0.9;
            ctx.setLineDash(dashed ? [3, 3] : []);
            ctx.beginPath();
            let started = false;
            for (const [rawT, value] of points) {
              const t = rawT + offset;
              if (t < start || t > end) continue;
              const x = GUTTER + ((t - start) / view.windowS) * plotW;
              const y = toY(value);
              if (!started) {
                ctx.moveTo(x, y);
                started = true;
              } else {
                ctx.lineTo(x, y);
              }
            }
            if (started) ctx.stroke();
          };
          stroke(signal.points, 0, false);
          const ref = reference?.series[signal.key];
          if (reference && ref) stroke(ref, reference.offsetS, true);
        }
        ctx.restore();

        // Compact legend: one row per signal until the gutter runs out.
        const rows = Math.max(
          1,
          Math.floor((height - TOP - AXIS_H - 22) / 13),
        );
        const shown = Math.min(series.length, rows);
        ctx.save();
        ctx.beginPath();
        ctx.rect(0, TOP, GUTTER, height - TOP - AXIS_H);
        ctx.clip();
        ctx.fillStyle = colors.signalMeta;
        ctx.font = "8px 'Segoe UI', sans-serif";
        ctx.fillText("OVERLAY · normalized", 13, TOP + 9);
        for (let i = 0; i < shown; i += 1) {
          const signal = series[i];
          const color = colors.signals[signal.color % colors.signals.length];
          const y = TOP + 24 + i * 13;
          const visible = !overlayHidden.has(signal.key);
          ctx.globalAlpha = visible ? 1 : 0.4;
          ctx.fillStyle = color;
          ctx.fillRect(4, y - 7, 7, 7);
          ctx.fillStyle = colors.signalName;
          ctx.font = "9px 'Segoe UI', sans-serif";
          ctx.fillText(signal.name, 15, y);
          ctx.globalAlpha = 1;
        }
        if (series.length > shown) {
          ctx.fillStyle = colors.signalMeta;
          ctx.font = "8px 'Segoe UI', sans-serif";
          ctx.fillText(`+${series.length - shown} more`, 13, TOP + 30 + shown * 13);
        }
        ctx.restore();
      }

      for (let i = 0; i < series.length; i += 1) {
        if (overlay) break;
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

        // Gutter: colour tab, name, then bus/current value. A short lane
        // (split pane) keeps only the name so neighbouring labels do not
        // overlap.
        const compactLane = laneH < 30;
        ctx.fillStyle = color;
        ctx.fillRect(4, top + 4, 3, Math.max(8, laneH - 8));
        ctx.fillStyle = colors.signalName;
        ctx.font = `${compactLane ? 9 : 10}px 'Segoe UI', sans-serif`;
        ctx.fillText(
          signal.name,
          13,
          compactLane ? top + laneH / 2 + 3 : top + 13,
        );
        const unit = signal.unit ? ` ${signal.unit}` : "";
        if (!compactLane) {
          ctx.fillStyle = colors.signalMeta;
          ctx.font = "9px 'Cascadia Mono', Consolas, monospace";
          ctx.fillText(
            `${signal.bus} · ${current === null ? "—" : valueText(current)}${unit}`,
            13,
            top + 25,
          );
        }

        if (visible <= 0) {
          ctx.fillStyle = colors.signalNoSamples;
          ctx.font = "9px 'Segoe UI', sans-serif";
          ctx.fillText("no samples in view", GUTTER + 8, top + laneH / 2 + 3);
          continue;
        }

        if (fullscreen && laneH >= 52 && cursorA && cursorB) {
          const first = sampleAt(points, cursorA.t);
          const second = sampleAt(points, cursorB.t);
          if (first !== null && second !== null) {
            ctx.fillStyle = colors.signalMeta;
            ctx.font = "9px 'Cascadia Mono', Consolas, monospace";
            ctx.fillText(
              `Δ ${valueText(second - first)}${unit}`,
              13,
              top + 39,
            );
          }
        }

        let raw = visible <= Math.max(1, Math.floor(plotW));
        let step = 0;
        let bins: MinMaxBin[] | null = null;
        let binFrom = 0;
        let binTo = 0;
        let min = Number.POSITIVE_INFINITY;
        let max = Number.NEGATIVE_INFINITY;

        if (raw) {
          for (let k = from; k < to; k += 1) {
            const value = points[k][1];
            if (value < min) min = value;
            if (value > max) max = value;
          }
        } else {
          step = pickBinStep(view.windowS, plotW);
          const cached = binCacheRef.current.get(signal.key);
          const updated = cached
            ? updateBins(cached, points, step)
            : rebuildBins(points, step);
          binCacheRef.current.set(signal.key, updated);
          bins = updated.bins;
          [binFrom, binTo] = visibleBinRange(bins, step, start, end);
          for (let i = binFrom; i < binTo; i += 1) {
            const bin = bins[i];
            if (bin.min < min) min = bin.min;
            if (bin.max > max) max = bin.max;
          }
        }

        // A very short or edge-clipped range may produce no bins; fall back
        // to the raw samples for this lane.
        if (min === Number.POSITIVE_INFINITY) {
          raw = true;
          for (let k = from; k < to; k += 1) {
            const value = points[k][1];
            if (value < min) min = value;
            if (value > max) max = value;
          }
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

        if (raw) {
          ctx.strokeStyle = color;
          ctx.globalAlpha = 1;
          ctx.lineWidth = 1.2;
          ctx.beginPath();
          let started = false;
          for (let k = from; k < to; k += 1) {
            const [t, value] = points[k];
            const x = GUTTER + ((t - start) / view.windowS) * plotW;
            if (!started) {
              ctx.moveTo(x, y(value));
              started = true;
            } else {
              ctx.lineTo(x, y(value));
            }
          }
          ctx.stroke();
          // Sparse windows keep the sample points visible so a reader can
          // see exactly where the curve steps from one message to the next.
          const count = to - from;
          if (count > 0 && plotW / count >= 4) {
            ctx.fillStyle = color;
            for (let k = from; k < to; k += 1) {
              const [t, value] = points[k];
              const x = GUTTER + ((t - start) / view.windowS) * plotW;
              if (x < GUTTER || x > GUTTER + plotW) continue;
              ctx.fillRect(x - 1.5, y(value) - 1.5, 3, 3);
            }
          }
        } else if (bins) {
          ctx.strokeStyle = color;
          ctx.globalAlpha = 0.55;
          ctx.lineWidth = 1;
          ctx.beginPath();
          for (let i = binFrom; i < binTo; i += 1) {
            const bin = bins[i];
            const x =
              GUTTER +
              ((bin.t0 + step / 2 - start) / view.windowS) * plotW +
              0.5;
            ctx.moveTo(x, y(bin.max));
            ctx.lineTo(x, y(bin.min));
          }
          ctx.stroke();

          ctx.globalAlpha = 1;
          ctx.lineWidth = 1.2;
          ctx.beginPath();
          let started = false;
          for (let i = binFrom; i < binTo; i += 1) {
            const bin = bins[i];
            const x =
              GUTTER +
              ((bin.t0 + step / 2 - start) / view.windowS) * plotW +
              0.5;
            if (!started) {
              ctx.moveTo(x, y(bin.last));
              started = true;
            } else {
              ctx.lineTo(x, y(bin.last));
            }
          }
          ctx.stroke();
        }
        ctx.globalAlpha = 1;

        const refPoints = reference?.series[signal.key];
        if (reference && refPoints && refPoints.length > 1) {
          const stride = Math.max(
            1,
            Math.floor(refPoints.length / Math.max(1, plotW * 2)),
          );
          ctx.save();
          ctx.globalAlpha = 0.55;
          ctx.strokeStyle = color;
          ctx.lineWidth = 1;
          ctx.setLineDash([3, 3]);
          ctx.beginPath();
          let refStarted = false;
          for (let ri = 0; ri < refPoints.length; ri += stride) {
            const t = refPoints[ri][0] + reference.offsetS;
            if (t < start || t > end) continue;
            const x = GUTTER + ((t - start) / view.windowS) * plotW;
            const vy = y(refPoints[ri][1]);
            if (!refStarted) {
              ctx.moveTo(x, vy);
              refStarted = true;
            } else {
              ctx.lineTo(x, vy);
            }
          }
          if (refStarted) ctx.stroke();
          ctx.restore();
        }

        if (fullscreen || hoverCursorId !== null) {
          for (let ci = 0; ci < cursors.length; ci += 1) {
            const cursor = cursors[ci];
            if (!fullscreen && cursor.id !== hoverCursorId) continue;
            if (cursor.t < start || cursor.t > end) continue;
            const value = sampleAt(points, cursor.t);
            if (value === null) continue;
            const x = GUTTER + ((cursor.t - start) / view.windowS) * plotW;
            const vy = y(value);
            const labelIndex = labels[ci].charCodeAt(0) - 65;
            const cursorColor =
              colors.cursorColors[labelIndex % colors.cursorColors.length];
            ctx.fillStyle = cursorColor;
            ctx.beginPath();
            ctx.arc(x, vy, 2.2, 0, Math.PI * 2);
            ctx.fill();
            const label = `${labels[ci]} ${valueText(value)}${unit}`;
            ctx.font = "9px 'Cascadia Mono', Consolas, monospace";
            const labelW = ctx.measureText(label).width;
            const onRight = x + 6 + labelW + 8 <= width - 8;
            const labelX = onRight ? x + 6 : x - 6 - labelW - 8;
            const labelY = clamp(vy - 6, top + 2, top + laneH - 14);
            ctx.fillStyle = colors.signalTagBg;
            ctx.fillRect(labelX, labelY, labelW + 8, 12);
            ctx.fillStyle = cursorColor;
            ctx.textAlign = "left";
            ctx.fillText(label, labelX + 4, labelY + 9);
          }
        }
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

      ctx.textAlign = "left";
      ctx.font = "9px 'Cascadia Mono', Consolas, monospace";
      const timeLabels: {
        x: number;
        color: string;
        text: string;
        w: number;
        labelX: number;
      }[] = [];
      for (let i = 0; i < cursors.length; i += 1) {
        const cursor = cursors[i];
        if (cursor.t < start || cursor.t > end) continue;
        const x = GUTTER + ((cursor.t - start) / view.windowS) * plotW;
        const labelIndex = labels[i].charCodeAt(0) - 65;
        const color = colors.cursorColors[labelIndex % colors.cursorColors.length];
        ctx.strokeStyle = color;
        ctx.globalAlpha = 0.9;
        ctx.lineWidth = 1;
        ctx.beginPath();
        ctx.moveTo(x + 0.5, TOP);
        ctx.lineTo(x + 0.5, height - AXIS_H);
        ctx.stroke();
        ctx.globalAlpha = 1;
        const text = `${labels[i]} ${cursor.t.toFixed(3)}s`;
        timeLabels.push({
          x,
          color,
          text,
          w: ctx.measureText(text).width + 8,
          labelX: 0,
        });
      }
      // Nearby cursors would stack their time chips on top of each other:
      // lay the chips out in order, then slide the row back into view.
      timeLabels.sort((a, b) => a.x - b.x);
      let nextLabel = 4;
      for (const entry of timeLabels) {
        entry.labelX = Math.max(entry.x + 4, nextLabel);
        nextLabel = entry.labelX + entry.w + 4;
      }
      const shift = Math.max(0, nextLabel - 4 - (width - 4));
      for (const entry of timeLabels) {
        const labelX = Math.max(4, entry.labelX - shift);
        ctx.fillStyle = colors.signalTagBg;
        ctx.fillRect(labelX, TOP + 2, entry.w, 12);
        ctx.fillStyle = entry.color;
        ctx.fillText(entry.text, labelX + 4, TOP + 10);
      }

      if (series.length === 0) {
        ctx.fillStyle = colors.signalEmpty;
        ctx.font = "11px 'Segoe UI', sans-serif";
        ctx.textAlign = "center";
        ctx.fillText(
          readCount === 1
            ? "1 read-only signal"
            : `${readCount} read-only signals`,
          width / 2,
          TOP + laneH / 2 - 2,
        );
        ctx.fillStyle = colors.signalHint;
        ctx.font = "9px 'Segoe UI', sans-serif";
        ctx.fillText(
          "Values follow the cursor below",
          width / 2,
          TOP + laneH / 2 + 14,
        );
        ctx.textAlign = "left";
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
      const tag = !measuring
        ? "STOPPED"
        : paused
          ? "PAUSED"
          : live
            ? "LIVE"
            : "HOLD · double-click to follow";
      const tagWidth = ctx.measureText(tag).width;
      ctx.fillStyle = colors.signalTagBg;
      ctx.fillRect(width - tagWidth - 16, 1, tagWidth + 14, 12);
      ctx.fillStyle = live && measuring && !paused
        ? colors.signalLive
        : colors.signalHold;
      ctx.fillText(tag, width - 8, 10);
    };

    draw();
    const observer = new ResizeObserver(draw);
    observer.observe(canvas);
    return () => observer.disconnect();
  }, [
    series,
    view,
    markers,
    theme,
    reorder,
    cursors,
    fullscreen,
    hoverCursorId,
    measuring,
    paused,
    reference,
    overlay,
    overlayHidden,
    hasReadout,
    readRange,
    readCount,
  ]);

  const plotWidth = () => {
    const canvas = canvasRef.current;
    return Math.max(1, (canvas?.clientWidth ?? 0) - GUTTER - 8);
  };
  const menuSignal = laneMenu ? series[laneMenu.index] : undefined;
  const cursorLabelList = cursorLabels(cursors);
  const cursorA =
    cursorLabelList.indexOf("A") >= 0
      ? cursors[cursorLabelList.indexOf("A")]
      : undefined;
  const cursorB =
    cursorLabelList.indexOf("B") >= 0
      ? cursors[cursorLabelList.indexOf("B")]
      : undefined;

  return (
    <div class="signal-chart-wrap">
      <canvas
        class={`signal-chart ${reorder ? "reordering" : ""} ${hoverCursorId !== null ? "cursor-hover" : ""}`}
        ref={canvasRef}
        aria-label="Live signal curves"
        title={
          overlay
            ? "Click a legend row to show or hide a signal · drag a cursor to move it · wheel zooms"
            : "Drag a signal name to reorder · drag a cursor to move it · wheel zooms"
        }
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
          onView((old) => ({
            windowS: nextWindow,
            endS: old.endS === null ? null : nextEnd,
          }));
        }}
        onPointerDown={(event) => {
          if (event.button !== 0) return;
          const cursor = cursorAt(event.clientX, event.clientY);
          if (cursor !== null) {
            setHoverCursorId(cursor);
            event.currentTarget.setPointerCapture(event.pointerId);
            cursorDragRef.current = cursor;
            return;
          }
          setHoverCursorId(null);
          const rect = event.currentTarget.getBoundingClientRect();
          if (event.clientX - rect.left < GUTTER) {
            if (overlay) {
              const rows = Math.max(
                1,
                Math.floor((rect.height - TOP - AXIS_H - 22) / 13),
              );
              const index = Math.floor(
                (event.clientY - rect.top - (TOP + 24)) / 13,
              );
              if (index >= 0 && index < Math.min(series.length, rows)) {
                const key = series[index].key;
                setOverlayHidden((old) => {
                  const next = new Set(old);
                  if (next.has(key)) next.delete(key);
                  else next.add(key);
                  return next;
                });
              }
              return;
            }
            const lane = laneAt(event.clientY);
            if (lane === null || series[lane].key.startsWith("derived:")) return;
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
          onView((old) => ({ ...old, endS: liveEnd }));
        }}
        onPointerMove={(event) => {
          const cursor = cursorDragRef.current;
          if (cursor !== null) {
            setHoverCursorId(cursor);
            onMoveCursor(cursor, timeAt(event.clientX));
            return;
          }
          const reordering = reorderRef.current;
          if (reordering) {
            setHoverCursorId(null);
            const to = laneAt(event.clientY, true) ?? reordering.to;
            if (to !== reordering.to) {
              reorderRef.current = { ...reordering, to };
              setReorder({ from: reordering.from, to });
            }
            return;
          }
          const drag = dragRef.current;
          if (!drag) {
            const over = cursorAt(event.clientX, event.clientY);
            if (over !== hoverCursorId) setHoverCursorId(over);
            return;
          }
          setHoverCursorId(null);
          const delta =
            ((drag.x - event.clientX) / plotWidth()) * view.windowS;
          const nextEnd = clamp(
            drag.endS + delta,
            timeBounds.first + view.windowS,
            timeBounds.last,
          );
          onView((old) => ({
            ...old,
            endS: nextEnd >= timeBounds.last ? null : nextEnd,
          }));
        }}
        onPointerUp={(event) => {
          const cursor = cursorDragRef.current;
          cursorDragRef.current = null;
          const reordering = reorderRef.current;
          reorderRef.current = null;
          setReorder(null);
          dragRef.current = null;
          if (event.currentTarget.hasPointerCapture(event.pointerId)) {
            event.currentTarget.releasePointerCapture(event.pointerId);
          }
          if (cursor !== null) return;
          if (reordering && reordering.to !== reordering.from) {
            onReorder(reordering.key, reordering.to);
          }
        }}
        onPointerCancel={() => {
          cursorDragRef.current = null;
          reorderRef.current = null;
          setReorder(null);
          dragRef.current = null;
          setHoverCursorId(null);
        }}
        onPointerLeave={() => {
          if (
            cursorDragRef.current === null &&
            reorderRef.current === null &&
            dragRef.current === null
          ) {
            setHoverCursorId(null);
          }
        }}
        onDblClick={(event) => {
          const rect = event.currentTarget.getBoundingClientRect();
          if (event.clientX - rect.left >= GUTTER) {
            onView(() => ({ windowS: 30, endS: null }));
          }
        }}
        onContextMenu={(event) => {
          event.preventDefault();
          const rect = event.currentTarget.getBoundingClientRect();
          if (overlay || event.clientX - rect.left >= GUTTER) return;
          const lane = laneAt(event.clientY);
          if (lane === null) return;
          setLaneMenu({
            x: event.clientX,
            y: event.clientY,
            index: lane,
          });
        }}
      />
      <div class="chart-cursor-tools">
        <button
          type="button"
          disabled={series.length === 0 && !hasReadout}
          title="Add a time cursor at the middle of the view"
          onClick={addCursor}
        >
          + Cursor
        </button>
        {cursors.length > 0 && (
          <button
            type="button"
            title="Remove all time cursors"
            onClick={onClearCursors}
          >
            Clear
          </button>
        )}
        {onToggleOverlay && (
          <button
            type="button"
            class={overlay ? "on" : ""}
            title={
              overlay
                ? "Back to one lane per signal"
                : "Overlay all signals on one normalized plot"
            }
            onClick={onToggleOverlay}
          >
            Overlay
          </button>
        )}
        {cursorA && cursorB && (
          <span
            class="chart-cursor-delta"
            title="Time between cursors A and B"
          >
            Δt {Math.abs(cursorB.t - cursorA.t).toFixed(3)}s
          </span>
        )}
        {onOpenDerived && (
          <button
            type="button"
            title="Create a math channel from the selected signals"
            onClick={onOpenDerived}
          >
            Derived…
          </button>
        )}
        {onOpenReference && (
          <button
            type="button"
            title="Overlay an aligned CSV from a previous run"
            onClick={onOpenReference}
          >
            Ref…
          </button>
        )}
        {reference && (
          <span
            class="reference-chip"
            title={`Reference ${reference.name}; its time + ${reference.offsetS.toFixed(3)} s is matched to the live time`}
          >
            REF {reference.offsetS >= 0 ? "+" : ""}
            {reference.offsetS.toFixed(3)}s
          </span>
        )}
      </div>
      {laneMenu && menuSignal && (
        <div
          class="trace-menu signal-menu"
          style={{
            left: `${Math.min(laneMenu.x, window.innerWidth - 190)}px`,
            top: `${Math.min(laneMenu.y, window.innerHeight - 240)}px`,
          }}
          onPointerDown={(event) => event.stopPropagation()}
          onContextMenu={(event) => event.preventDefault()}
        >
          <button
            type="button"
            onClick={() => {
              onSignalAction(menuSignal.key, "monitor");
              setLaneMenu(null);
            }}
          >
            Watch in Monitor
          </button>
          <button
            type="button"
            onClick={() => {
              onSignalAction(menuSignal.key, "data");
              setLaneMenu(null);
            }}
          >
            Watch in Data
          </button>
          <button
            type="button"
            onClick={() => {
              onSignalAction(menuSignal.key, "state");
              setLaneMenu(null);
            }}
          >
            Watch in State
          </button>
          <div class="signal-menu-sep" />
          <button
            type="button"
            disabled={laneMenu.index === 0}
            onClick={() => {
              onReorder(menuSignal.key, laneMenu.index - 1);
              setLaneMenu(null);
            }}
          >
            Move up
          </button>
          <button
            type="button"
            disabled={laneMenu.index === series.length - 1}
            onClick={() => {
              onReorder(menuSignal.key, laneMenu.index + 1);
              setLaneMenu(null);
            }}
          >
            Move down
          </button>
          <button
            type="button"
            class="signal-menu-remove"
            onClick={() => {
              onSignalAction(menuSignal.key, "remove");
              setLaneMenu(null);
            }}
          >
            Remove from chart
          </button>
        </div>
      )}
    </div>
  );
}
