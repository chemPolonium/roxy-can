import { useEffect, useRef } from "preact/hooks";

const GRID = "#1c2530";
const AXIS = "#697687";
const LINE = "#4c9dff";
const FILL_TOP = "rgba(76, 157, 255, 0.28)";
const FILL_BOTTOM = "rgba(76, 157, 255, 0.01)";

export function RateChart({ values }: { values: number[] }) {
  const canvasRef = useRef<HTMLCanvasElement>(null);

  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;

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

      const pad = { left: 10, right: 42, top: 12, bottom: 18 };
      const plotW = Math.max(1, width - pad.left - pad.right);
      const plotH = Math.max(1, height - pad.top - pad.bottom);
      const peak = Math.max(10, ...values);
      const max = peak * 1.15;

      ctx.strokeStyle = GRID;
      ctx.lineWidth = 1;
      ctx.font = "10px 'Cascadia Mono', Consolas, monospace";
      ctx.fillStyle = AXIS;
      ctx.textAlign = "right";
      ctx.textBaseline = "middle";
      for (let i = 0; i <= 4; i += 1) {
        const y = pad.top + (plotH * i) / 4;
        ctx.beginPath();
        ctx.moveTo(pad.left, y + 0.5);
        ctx.lineTo(pad.left + plotW, y + 0.5);
        ctx.stroke();
        const value = max * (1 - i / 4);
        ctx.fillText(value >= 100 ? value.toFixed(0) : value.toFixed(1), width - 4, y);
      }

      if (values.length > 0) {
        const n = values.length;
        const x = (i: number) =>
          pad.left + (n === 1 ? 0 : (plotW * i) / (n - 1));
        const y = (value: number) => pad.top + plotH - (value / max) * plotH;

        const fill = ctx.createLinearGradient(0, pad.top, 0, pad.top + plotH);
        fill.addColorStop(0, FILL_TOP);
        fill.addColorStop(1, FILL_BOTTOM);
        ctx.beginPath();
        ctx.moveTo(x(0), pad.top + plotH);
        values.forEach((value, i) => ctx.lineTo(x(i), y(value)));
        ctx.lineTo(x(n - 1), pad.top + plotH);
        ctx.closePath();
        ctx.fillStyle = fill;
        ctx.fill();

        ctx.beginPath();
        values.forEach((value, i) => {
          if (i === 0) ctx.moveTo(x(i), y(value));
          else ctx.lineTo(x(i), y(value));
        });
        ctx.strokeStyle = LINE;
        ctx.lineWidth = 1.6;
        ctx.lineJoin = "round";
        ctx.stroke();

        ctx.beginPath();
        ctx.arc(x(n - 1), y(values[n - 1]), 2.4, 0, Math.PI * 2);
        ctx.fillStyle = LINE;
        ctx.fill();
      }

      ctx.fillStyle = AXIS;
      ctx.font = "10px 'Segoe UI', sans-serif";
      ctx.textAlign = "left";
      ctx.textBaseline = "alphabetic";
      ctx.fillText("last 6 s", pad.left, height - 4);
      ctx.textAlign = "right";
      ctx.fillText("now", width - 4, height - 4);
    };

    draw();
    const observer = new ResizeObserver(draw);
    observer.observe(canvas);
    return () => observer.disconnect();
  }, [values]);

  return <canvas class="rate-chart" ref={canvasRef} />;
}
