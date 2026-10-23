import { useEffect, useRef } from "preact/hooks";
import type { BusStats } from "../types";
import { palette, type Theme } from "../theme";

function Spark({ points, theme }: { points: number[]; theme: Theme }) {
  const ref = useRef<HTMLCanvasElement>(null);
  useEffect(() => {
    const canvas = ref.current;
    if (!canvas) return;
    const colors = palette(theme);
    const draw = () => {
      const w = canvas.clientWidth;
      const h = canvas.clientHeight;
      if (!w || !h) return;
      const dpr = window.devicePixelRatio || 1;
      canvas.width = Math.round(w * dpr);
      canvas.height = Math.round(h * dpr);
      const ctx = canvas.getContext("2d");
      if (!ctx) return;
      ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
      ctx.clearRect(0, 0, w, h);
      ctx.fillStyle = colors.sparkBg;
      ctx.fillRect(0, 0, w, h);
      if (points.length < 2) return;
      const peak = Math.max(0.25, ...points);
      const step = w / (points.length - 1);
      ctx.beginPath();
      points.forEach((value, index) => {
        const x = index * step;
        const y = h - (value / peak) * h;
        if (index === 0) ctx.moveTo(x, y);
        else ctx.lineTo(x, y);
      });
      ctx.strokeStyle = colors.sparkLine;
      ctx.lineWidth = 1.5;
      ctx.stroke();
      if (peak >= 1) {
        const y = h - (1 / peak) * h;
        ctx.strokeStyle = colors.sparkThreshold;
        ctx.lineWidth = 1;
        ctx.beginPath();
        ctx.moveTo(0, y);
        ctx.lineTo(w, y);
        ctx.stroke();
      }
    };
    draw();
    const observer = new ResizeObserver(draw);
    observer.observe(canvas);
    return () => observer.disconnect();
  }, [points, theme]);
  return <canvas class="busstat-spark" ref={ref} />;
}

export function BusStatsPanel({
  buses,
  theme,
}: {
  buses: BusStats[];
  theme: Theme;
}) {
  if (buses.length === 0) {
    return (
      <div class="busstat-empty">
        no bus statistics yet -- start a measurement to collect load numbers
      </div>
    );
  }
  return (
    <div class="busstat-list">
      {buses.map((bus) => (
        <section class="busstat-bus" key={bus.name}>
          <header class="busstat-head">
            <strong>{bus.name}</strong>
            <span>{bus.kbps} kbit/s</span>
            <em>now {(bus.loadNow * 100).toFixed(2)}%</em>
            <Spark points={bus.loadHistory} theme={theme} />
          </header>
          <div class="busstat-rows">
            {bus.rows.map((row) => (
              <div class="busstat-row" key={row.label}>
                <span class="busstat-label">{row.label}</span>
                <strong class="busstat-current">{row.current}</strong>
                <span class="busstat-range">
                  min {row.min} · max {row.max} · avg {row.avg}
                </span>
              </div>
            ))}
          </div>
        </section>
      ))}
    </div>
  );
}
