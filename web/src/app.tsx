import { useEffect, useMemo, useRef, useState } from "preact/hooks";
import type { Command, Frame, State } from "./types";
import { fetchState, sendCommand } from "./api";
import { Activity } from "./components/Activity";
import { MessageTable } from "./components/MessageTable";
import { BUS_COLORS, RateChart } from "./components/RateChart";
import { TraceTable } from "./components/TraceTable";

type RightTab = "messages" | "activity";
type Drag = "main" | "right" | null;

interface Layout {
  main: number;
  right: number;
}

const LAYOUT_KEY = "roxy-can-web.layout";
const TRACE_CAP = 50_000;
const DEFAULT_LAYOUT: Layout = { main: 0.63, right: 0.56 };

function clamp(value: number, min: number, max: number): number {
  return Math.min(max, Math.max(min, value));
}

function loadLayout(): Layout {
  try {
    const raw = localStorage.getItem(LAYOUT_KEY);
    if (!raw) return DEFAULT_LAYOUT;
    const saved = JSON.parse(raw) as Partial<Layout>;
    return {
      main: clamp(Number(saved.main) || DEFAULT_LAYOUT.main, 0.4, 0.78),
      right: clamp(Number(saved.right) || DEFAULT_LAYOUT.right, 0.34, 0.8),
    };
  } catch {
    return DEFAULT_LAYOUT;
  }
}

function matches(frame: Frame, query: string): boolean {
  if (!query) return true;
  const needle = query.toLowerCase();
  const id = `0x${frame.id.toString(16)}`;
  return (
    frame.name.toLowerCase().includes(needle) ||
    frame.busName.toLowerCase().includes(needle) ||
    id.includes(needle) ||
    frame.data.toLowerCase().includes(needle)
  );
}

function seconds(us: number, digits = 3): string {
  return (us / 1e6).toFixed(digits);
}

function useUiFps(): number {
  const [fps, setFps] = useState(0);
  useEffect(() => {
    let frames = 0;
    let last = performance.now();
    let raf = 0;
    const tick = (now: number) => {
      frames += 1;
      if (now - last >= 500) {
        setFps(Math.round((frames * 1000) / (now - last)));
        frames = 0;
        last = now;
      }
      raf = requestAnimationFrame(tick);
    };
    raf = requestAnimationFrame(tick);
    return () => window.cancelAnimationFrame(raf);
  }, []);
  return fps;
}

function clock(wallUs: number): string {
  const total = wallUs / 1e6;
  const h = Math.floor(total / 3600);
  const m = Math.floor((total % 3600) / 60);
  const s = Math.floor(total % 60);
  const ms = Math.floor((wallUs % 1e6) / 1000);
  return `${String(h).padStart(2, "0")}:${String(m).padStart(2, "0")}:${String(s).padStart(2, "0")}.${String(ms).padStart(3, "0")}`;
}

export function App() {
  const [state, setState] = useState<State | null>(null);
  const [traceRows, setTraceRows] = useState<Frame[]>([]);
  const [traceEpoch, setTraceEpoch] = useState(0);
  const traceRef = useRef<{
    epoch: number | null;
    seq: number;
    rows: Frame[];
  }>({ epoch: null, seq: 0, rows: [] });
  const [online, setOnline] = useState(false);
  const [error, setError] = useState("");
  const [busy, setBusy] = useState(false);
  const [query, setQuery] = useState("");
  const [bus, setBus] = useState<number | "all">("all");
  const [tab, setTab] = useState<RightTab>("messages");
  const [layout, setLayout] = useState<Layout>(loadLayout);
  const [drag, setDrag] = useState<Drag>(null);
  const uiFps = useUiFps();
  const workspaceRef = useRef<HTMLDivElement>(null);
  const rightRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    try {
      localStorage.setItem(LAYOUT_KEY, JSON.stringify(layout));
    } catch {
      // Storage can be unavailable in private mode; the session layout still works.
    }
  }, [layout]);

  useEffect(() => {
    let alive = true;
    let inFlight = false;
    const tick = async () => {
      if (inFlight) return;
      inFlight = true;
      try {
        const { epoch, seq } = traceRef.current;
        const next = await fetchState(epoch, seq);
        if (!alive) return;
        let rows = traceRef.current.rows;
        if (next.traceSeed) {
          rows = next.traceSeed;
          setTraceRows(rows);
          setTraceEpoch(next.traceEpoch);
        } else if (next.traceDelta?.length) {
          rows = [...next.traceDelta, ...rows].slice(0, TRACE_CAP);
          setTraceRows(rows);
        }
        traceRef.current = {
          epoch: next.traceEpoch,
          seq: next.traceSeq,
          rows,
        };
        setState(next);
        setOnline(true);
        setError("");
      } catch (e) {
        if (!alive) return;
        setOnline(false);
        setError(String(e));
      } finally {
        inFlight = false;
      }
    };
    void tick();
    const timer = window.setInterval(tick, 100);
    return () => {
      alive = false;
      window.clearInterval(timer);
    };
  }, []);

  useEffect(() => {
    if (!drag) return;
    const move = (event: PointerEvent) => {
      if (drag === "main" && workspaceRef.current) {
        const rect = workspaceRef.current.getBoundingClientRect();
        const next = (event.clientX - rect.left) / rect.width;
        setLayout((old) => ({ ...old, main: clamp(next, 0.4, 0.78) }));
      } else if (drag === "right" && rightRef.current) {
        const rect = rightRef.current.getBoundingClientRect();
        const next = (event.clientY - rect.top) / rect.height;
        setLayout((old) => ({ ...old, right: clamp(next, 0.34, 0.8) }));
      }
    };
    const up = () => setDrag(null);
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", up);
    return () => {
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", up);
    };
  }, [drag]);

  const run = async (command: Command) => {
    setBusy(true);
    try {
      await sendCommand(command);
      setError("");
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };

  const visibleRows = useMemo(
    () =>
      traceRows.filter(
        (frame) => (bus === "all" || frame.bus === bus) && matches(frame, query),
      ),
    [traceRows, query, bus],
  );

  const canControl = Boolean(state) && online && !busy;
  const measuring = state?.measuring ?? false;
  const latest = state?.series.length
    ? state.series[state.series.length - 1]
    : undefined;

  return (
    <div class="app">
      <header class="topbar">
        <div class="brand">
          <div class="logo">R</div>
          <div class="brand-text">
            <h1>
              roxy-can <span class="tag">web spike</span>
            </h1>
            <p>
              {state
                ? `${state.channels.length} buses · ${state.channels.map((c) => c.name).join(", ")}`
                : "connecting to the bus core"}
            </p>
          </div>
        </div>
        <div class="actions">
          <button
            type="button"
            class="primary"
            disabled={!canControl || measuring}
            onClick={() => void run({ type: "start" })}
          >
            Start
          </button>
          <button
            type="button"
            disabled={!canControl || !measuring}
            onClick={() => void run({ type: "stop" })}
          >
            Stop
          </button>
          <button
            type="button"
            disabled={!canControl || !measuring}
            onClick={() => void run({ type: "pause", on: !state?.tracePaused })}
          >
            {state?.tracePaused ? "Resume" : "Pause"}
          </button>
          <button
            type="button"
            disabled={!canControl}
            onClick={() => void run({ type: "clear_trace" })}
          >
            Clear trace
          </button>
        </div>
        <div class={`link ${online ? "on" : "off"}`}>
          <span class="dot" />
          {online ? "live" : "reconnecting"}
        </div>
      </header>

      <section class="stats">
        <div class="stat">
          <span>Frames</span>
          <strong>{state?.frames.toLocaleString() ?? "—"}</strong>
        </div>
        <div class="stat">
          <span>Frame rate</span>
          <strong>{state ? `${state.fps.toFixed(0)} f/s` : "—"}</strong>
        </div>
        <div class="stat">
          <span>Trace ring</span>
          <strong>{state?.traceLen.toLocaleString() ?? "—"}</strong>
        </div>
        <div class="stat">
          <span>Sim time</span>
          <strong>{state ? `${seconds(state.simTimeUs, 2)} s` : "—"}</strong>
        </div>
        <div class="stat">
          <span>Mode</span>
          <strong>{state?.mode ?? "—"}</strong>
        </div>
        {error && <div class="error-pill">{error}</div>}
      </section>

      <main
        class="workspace"
        ref={workspaceRef}
        style={{
          gridTemplateColumns: `calc((100% - 5px) * ${layout.main}) 5px minmax(0, 1fr)`,
        }}
      >
        <section class="panel trace-panel">
          <div class="panel-head">
            <div class="panel-title">
              <h2>Trace</h2>
              <span>
                {visibleRows.length.toLocaleString()} shown /{" "}
                {traceRows.length.toLocaleString()} rows
              </span>
            </div>
            <div class="panel-tools">
              <select
                value={bus === "all" ? "all" : String(bus)}
                onChange={(e) => {
                  const value = (e.target as HTMLSelectElement).value;
                  setBus(value === "all" ? "all" : Number(value));
                }}
              >
                <option value="all">All buses</option>
                {state?.channels.map((channel, index) => (
                  <option key={channel.name} value={index}>
                    {channel.name}
                  </option>
                ))}
              </select>
              <input
                type="search"
                value={query}
                placeholder="Filter id, name, data…"
                onInput={(e) => setQuery((e.target as HTMLInputElement).value)}
              />
            </div>
          </div>
          <TraceTable
            key={`${bus}:${query}:${traceEpoch}`}
            rows={visibleRows}
          />
        </section>

        <div
          class={`splitter vertical ${drag === "main" ? "active" : ""}`}
          onPointerDown={(e) => {
            e.preventDefault();
            setDrag("main");
          }}
        />

        <div
          class="right-column"
          ref={rightRef}
          style={{
            gridTemplateRows: `minmax(0, ${layout.right}fr) 5px minmax(0, ${1 - layout.right}fr)`,
          }}
        >
          <section class="panel chart-panel">
            <div class="panel-head">
              <div class="panel-title">
                <h2>Bus activity</h2>
                <span>solid f/s · dashed load</span>
              </div>
              <strong class="readout">{state ? state.fps.toFixed(0) : "—"} f/s</strong>
            </div>
            <div class="chart-legend">
              <span class="legend-key">
                <i style={{ background: "#4c9dff" }} />
                Total <b>{state ? state.fps.toFixed(0) : "—"}</b> f/s
              </span>
              {state?.channels.map((channel, index) => (
                <span class="legend-key" key={channel.name}>
                  <i style={{ background: BUS_COLORS[index % BUS_COLORS.length] }} />
                  {channel.name} <b>{(latest?.busRates[index] ?? 0).toFixed(0)}</b> f/s
                  <em>{(latest?.busLoads[index] ?? 0).toFixed(1)}%</em>
                </span>
              ))}
            </div>
            <RateChart
              points={state?.series ?? []}
              channels={state?.channels ?? []}
            />
          </section>

          <div
            class={`splitter horizontal ${drag === "right" ? "active" : ""}`}
            onPointerDown={(e) => {
              e.preventDefault();
              setDrag("right");
            }}
          />

          <section class="panel lower-panel">
            <div class="panel-head">
              <div class="tabs">
                <button
                  type="button"
                  class={tab === "messages" ? "on" : ""}
                  onClick={() => setTab("messages")}
                >
                  Messages
                </button>
                <button
                  type="button"
                  class={tab === "activity" ? "on" : ""}
                  onClick={() => setTab("activity")}
                >
                  Activity
                </button>
              </div>
              <span class="meta">
                {tab === "messages"
                  ? `${state?.messages.length ?? 0} ids`
                  : `${state?.write.length ?? 0} lines`}
              </span>
            </div>
            {tab === "messages" ? (
              <MessageTable rows={state?.messages ?? []} />
            ) : (
              <Activity rows={state?.write ?? []} clock={clock} />
            )}
          </section>
        </div>
      </main>

      <footer class="statusbar">
        <span class={`state ${measuring ? "running" : "stopped"}`}>
          {measuring ? (state?.tracePaused ? "PAUSED" : "MEASURING") : "STOPPED"}
        </span>
        <span>
          {measuring
            ? "auto demo traffic · generated from the loaded DBCs"
            : state?.status || "waiting for core"}
        </span>
        <span class="spacer" />
        <span>poll 10 Hz</span>
        <span>{uiFps ? `UI ${uiFps} fps` : "UI —"}</span>
        <span>{window.location.host}</span>
      </footer>
    </div>
  );
}
