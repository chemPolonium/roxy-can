import { useEffect, useRef } from "preact/hooks";
import type { Channel, SeriesPoint } from "../types";
import { palette, type Theme } from "../theme";

export function RateChart({
  points,
  channels,
  theme,
}: {
  points: SeriesPoint[];
  channels: Channel[];
  theme: Theme;
}) {
  const canvasRef = useRef<HTMLCanvasElement>(null);

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

      const pad = { left: 38, right: 46, top: 12, bottom: 18 };
      const plotW = Math.max(1, width - pad.left - pad.right);
      const plotH = Math.max(1, height - pad.top - pad.bottom);
      let peak = 10;
      let loadPeak = 0;
      for (const point of points) {
        peak = Math.max(peak, point.total);
        peak = Math.max(peak, ...point.busRates);
        loadPeak = Math.max(loadPeak, ...point.busLoads);
      }
      const max = peak * 1.15;
      // An idle two-bus demo sits around 1–2 % load; a fixed 0–100 % axis
      // would pin both dashed lines to the floor. Keep a 5 % minimum so the
      // shape stays readable without misstating the unit.
      const loadMax = Math.min(100, Math.max(5, loadPeak * 1.15));
      const n = points.length;

      ctx.strokeStyle = colors.rateGrid;
      ctx.lineWidth = 1;
      ctx.font = "10px 'Cascadia Mono', Consolas, monospace";
      ctx.textBaseline = "middle";
      for (let i = 0; i <= 4; i += 1) {
        const y = pad.top + (plotH * i) / 4;
        ctx.beginPath();
        ctx.moveTo(pad.left, y + 0.5);
        ctx.lineTo(pad.left + plotW, y + 0.5);
        ctx.stroke();
        const value = max * (1 - i / 4);
        ctx.fillStyle = colors.rateAxis;
        ctx.textAlign = "right";
        ctx.fillText(
          value >= 100 ? value.toFixed(0) : value.toFixed(1),
          pad.left - 4,
          y,
        );
      }

      // Right axis is load on its own scale, independent of frames/s.
      ctx.fillStyle = colors.rateEmpty;
      ctx.textAlign = "left";
      for (const pct of [loadMax, loadMax / 2, 0]) {
        const y = pad.top + plotH - (pct / loadMax) * plotH;
        const label = pct === 0 || pct >= 10 ? pct.toFixed(0) : pct.toFixed(1);
        ctx.fillText(`${label}%`, width - pad.right + 6, y);
      }

      if (n > 0) {
        const x = (i: number) =>
          pad.left + (n === 1 ? 0 : (plotW * i) / (n - 1));
        const yRate = (value: number) =>
          pad.top + plotH - (value / max) * plotH;
        const yLoad = (value: number) =>
          pad.top + plotH - (Math.min(loadMax, value) / loadMax) * plotH;

        const fill = ctx.createLinearGradient(0, pad.top, 0, pad.top + plotH);
        fill.addColorStop(0, colors.rateFillTop);
        fill.addColorStop(1, colors.rateFillBottom);
        ctx.beginPath();
        ctx.moveTo(x(0), pad.top + plotH);
        points.forEach((point, i) => ctx.lineTo(x(i), yRate(point.total)));
        ctx.lineTo(x(n - 1), pad.top + plotH);
        ctx.closePath();
        ctx.fillStyle = fill;
        ctx.fill();

        const line = (
          value: (point: SeriesPoint) => number | undefined,
          color: string,
          y: (value: number) => number,
          widthPx = 1.4,
        ) => {
          ctx.beginPath();
          let started = false;
          points.forEach((point, i) => {
            const v = value(point);
            if (v === undefined) return;
            if (!started) {
              ctx.moveTo(x(i), y(v));
              started = true;
            } else {
              ctx.lineTo(x(i), y(v));
            }
          });
          ctx.strokeStyle = color;
          ctx.lineWidth = widthPx;
          ctx.lineJoin = "round";
          ctx.stroke();
        };

        line((point) => point.total, colors.rateTotal, yRate, 1.8);
        channels.forEach((_, bus) => {
          const color = colors.buses[bus % colors.buses.length];
          line((point) => point.busRates[bus], color, yRate);
        });
        ctx.setLineDash([3, 3]);
        channels.forEach((_, bus) => {
          const color = colors.buses[bus % colors.buses.length];
          line((point) => point.busLoads[bus], color, yLoad, 1);
        });
        ctx.setLineDash([]);

        const last = points[n - 1];
        ctx.beginPath();
        ctx.arc(x(n - 1), yRate(last.total), 2.4, 0, Math.PI * 2);
        ctx.fillStyle = colors.rateTotal;
        ctx.fill();
      }

      ctx.fillStyle = colors.rateAxis;
      ctx.font = "10px 'Segoe UI', sans-serif";
      ctx.textAlign = "left";
      ctx.textBaseline = "alphabetic";
      ctx.fillText("last 12 s", pad.left, height - 4);
      ctx.textAlign = "right";
      ctx.fillText("now", width - 4, height - 4);
    };

    draw();
    const observer = new ResizeObserver(draw);
    observer.observe(canvas);
    return () => observer.disconnect();
  }, [points, channels, theme]);

  return <canvas class="rate-chart" ref={canvasRef} />;
}
