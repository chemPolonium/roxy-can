import type { ComponentChildren } from "preact";
import { useEffect, useRef, useState } from "preact/hooks";
import type { StateBand, StateRow } from "../types";
import { palette, type Theme } from "../theme";
import { ContextMenu } from "./ContextMenu";
import { EmptyState } from "./EmptyState";

interface Props {
  rows: StateRow[];
  filtered?: boolean;
  /// Replaces the signal empty state when no bus/project is open.
  empty?: ComponentChildren;
  windowS: number;
  theme: Theme;
  onWindow: (seconds: number) => void;
  onRemove: (row: StateRow) => void;
  onRule: (row: StateRow) => void;
}

/** Few-state fills are deliberately high-contrast: a 0/1 signal reads as a
    square wave even when each band is only a few pixels wide. */
const WINDOWS: [number, string][] = [
  [5, "5 s"],
  [20, "20 s"],
  [60, "1 min"],
  [300, "5 min"],
  [1800, "30 min"],
];

function StateStrip({ bands, theme }: { bands: StateBand[]; theme: Theme }) {
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
      ctx.fillStyle = colors.stateBg;
      ctx.fillRect(0, 0, w, h);
      const distinct = new Set(bands.map((band) => band.label)).size;
      if (distinct > colors.states.length) {
        // An analog sweep is not a set of readable states; one neutral band
        // reads better than hundreds of one-pixel color stripes.
        ctx.fillStyle = colors.stateNeutral;
        ctx.fillRect(0, 1, w, h - 2);
      } else {
        for (const band of bands) {
          const x = band.start * w;
          const bw = Math.max(1, (band.end - band.start) * w);
          ctx.globalAlpha = 0.82;
          ctx.fillStyle = colors.states[band.color % colors.states.length];
          ctx.fillRect(x, 1, Math.max(1, bw), h - 2);
          ctx.globalAlpha = 1;
          if (bw > 34) {
            ctx.fillStyle = colors.stateLabel;
            ctx.font = '9px "Cascadia Mono", Consolas, monospace';
            ctx.textBaseline = "middle";
            ctx.fillText(band.label, x + 4, h / 2);
          }
        }
      }
      ctx.fillStyle = colors.stateMarker;
      ctx.fillRect(w - 1, 0, 1, h);
    };
    draw();
    const observer = new ResizeObserver(draw);
    observer.observe(canvas);
    return () => observer.disconnect();
  }, [bands, theme]);
  return <canvas class="state-strip" ref={ref} />;
}

export function StatePanel({
  rows,
  filtered = false,
  empty,
  windowS,
  theme,
  onWindow,
  onRemove,
  onRule,
}: Props) {
  const options = WINDOWS.some(([seconds]) => seconds === windowS)
    ? WINDOWS
    : [...WINDOWS, [windowS, `${windowS} s`] as [number, string]].sort(
        (a, b) => a[0] - b[0],
      );
  const [menu, setMenu] = useState<{
    row: StateRow;
    x: number;
    y: number;
  } | null>(null);

  const openMenu = (event: MouseEvent, row: StateRow) => {
    event.preventDefault();
    setMenu({
      row,
      x: Math.max(8, Math.min(event.clientX, window.innerWidth - 160 - 8)),
      y: Math.max(8, Math.min(event.clientY, window.innerHeight - 50 - 8)),
    });
  };

  return (
    <>
    <div class="state-panel">
      <div class="state-tools">
        <label>
          Window
          <select
            value={String(windowS)}
            onChange={(event) =>
              onWindow(Number((event.target as HTMLSelectElement).value))
            }
          >
            {options.map(([seconds, label]) => (
              <option key={seconds} value={seconds}>
                {label}
              </option>
            ))}
          </select>
        </label>
        <span class="state-hint">
          6 significant digits · right edge follows the run
        </span>
      </div>
      {rows.length === 0 ? (
        empty ?? (
          <EmptyState
            title={filtered ? "No matching signals" : "No signals yet"}
            hint={
              filtered
                ? "Change the quick search or the funnel filter."
                : "Add a signal to watch its states as bands."
            }
          />
        )
      ) : (
        <div class="state-rows">
          {rows.map((row) => (
            <div
              class="state-row"
              key={`${row.kind}:${row.ch}:${row.id}:${row.ext}:${row.name}`}
              title="Right-click to remove this state row"
              onContextMenu={(event) => openMenu(event, row)}
            >
              <div class="state-name" title={`${row.bus} · ${row.name}`}>
                <strong>{row.name}</strong>
                <small>{row.bus}</small>
              </div>
              <StateStrip bands={row.bands} theme={theme} />
              <span class="state-current" title="Current state">
                {row.current}
              </span>
              <button
                type="button"
                class={`mini state-bands ${row.rule ? "on" : ""}`}
                title="Edit value-definition bands"
                onClick={() => onRule(row)}
              >
                bands
              </button>
            </div>
          ))}
        </div>
      )}
    </div>
    {menu && (
      <ContextMenu
        x={menu.x}
        y={menu.y}
        label="State row actions"
        items={[
          {
            label: "Remove",
            danger: true,
            onSelect: () => onRemove(menu.row),
          },
        ]}
        onClose={() => setMenu(null)}
      />
    )}
    </>
  );
}
