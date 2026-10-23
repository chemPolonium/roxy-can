import { useEffect, useMemo, useRef, useState } from "preact/hooks";
import type {
  Catalog,
  CatalogChannel,
  CatalogMessage,
  CatalogSignal,
  Command,
  Frame,
  Hardware,
  SignalSeries,
  State,
  TriggerEdit,
  TriggerLimits,
  TxEntry,
} from "./types";
import { fetchCatalog, fetchHardware, fetchState, sendCommand } from "./api";
import {
  clearDownload,
  isDownloadName,
  triggerDownload,
  waitForDownload,
} from "./download";
import { Activity } from "./components/Activity";
import { BusesModal } from "./components/BusesModal";
import { BusStatsPanel } from "./components/BusStatsPanel";
import { DataPanel } from "./components/DataPanel";
import { ExportModal } from "./components/ExportModal";
import { FilePick } from "./components/FilePick";
import { GeneratorModal } from "./components/GeneratorModal";
import { HelpModal } from "./components/HelpModal";
import { TriggersModal } from "./components/TriggersModal";
import { MessageTable } from "./components/MessageTable";
import { MonitorPanel } from "./components/MonitorPanel";
import type { MonitorPatch } from "./components/MonitorPanel";
import { RateChart } from "./components/RateChart";
import {
  SignalChart,
  type ChartCursor,
  type ChartSignalAction,
  type SignalView,
} from "./components/SignalChart";
import { SignalPicker } from "./components/SignalPicker";
import { SpecPanel } from "./components/SpecPanel";
import { ScriptsModal } from "./components/ScriptsModal";
import { StatePanel } from "./components/StatePanel";
import { StateRuleModal } from "./components/StateRuleModal";
import { StatsTable } from "./components/StatsTable";
import { SysVarsModal } from "./components/SysVarsModal";
import type { SysVarEdit } from "./components/SysVarsModal";
import { TraceTable } from "./components/TraceTable";
import {
  EMPTY_TRACE_FILTER,
  TraceFilterModal,
  parseIds,
  traceFilterActive,
} from "./components/TraceFilterModal";
import type { TraceFilter } from "./components/TraceFilterModal";
import type { StateRow } from "./types";
import {
  applyTheme,
  loadTheme,
  palette,
  saveTheme,
  type Theme,
} from "./theme";

type RightTab = "messages" | "statistics" | "bus" | "spec" | "activity";
type ChartTab = "rate" | "signals" | "monitor" | "data" | "state";
type Drag = "main" | "right" | null;

interface Layout {
  main: number;
  right: number;
}

const LAYOUT_KEY = "roxy-can-web.layout";
const TRACE_CAP = 50_000;
const SIGNAL_CAP = 12_000;
const DEFAULT_LAYOUT: Layout = { main: 0.63, right: 0.56 };

/// The backend's series/catalog key for an observer row. CAN ids start at 0,
/// so FlexRay keys need the prefix to stay distinct.
function rowKey(row: {
  kind: string;
  ch: number;
  id: number;
  frame?: string;
  name: string;
}): string {
  if (row.kind === "fr") return `fr:${row.ch}:${row.id}:${row.name}`;
  if (row.kind === "eth") return `eth:${row.ch}:${row.frame}:${row.name}`;
  return `${row.ch}:${row.id}:${row.name}`;
}

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
    frame.addr.toLowerCase().includes(needle) ||
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

function baseName(path: string): string {
  const cut = Math.max(path.lastIndexOf("\\"), path.lastIndexOf("/"));
  return cut >= 0 ? path.slice(cut + 1) : path;
}

function fileNameOf(path: string): string {
  const cut = Math.max(path.lastIndexOf("\\"), path.lastIndexOf("/"));
  return cut >= 0 ? path.slice(cut + 1) : path;
}

function clock(wallUs: number): string {  const total = wallUs / 1e6;
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
  const [signals, setSignals] = useState<SignalSeries[]>([]);
  const signalRef = useRef<{
    epoch: number | null;
    since: number;
    series: SignalSeries[];
  }>({ epoch: null, since: 0, series: [] });
  const [online, setOnline] = useState(false);
  const [error, setError] = useState("");
  const [busy, setBusy] = useState(false);
  const [query, setQuery] = useState("");
  const [bus, setBus] = useState<string>("all");
  const [traceFilter, setTraceFilter] = useState<TraceFilter>(EMPTY_TRACE_FILTER);
  const [traceFilterOpen, setTraceFilterOpen] = useState(false);
  const [panelQuery, setPanelQuery] = useState("");
  const [writeKind, setWriteKind] = useState("all");
  const [writeQuery, setWriteQuery] = useState("");
  const [tab, setTab] = useState<RightTab>("messages");
  const [chartTab, setChartTab] = useState<ChartTab>("rate");
  const [sessionOpen, setSessionOpen] = useState(false);
  const [projectPath, setProjectPath] = useState("");
  const [savePath, setSavePath] = useState("");
  const [saveDownloading, setSaveDownloading] = useState(false);
  const [saveDownloadError, setSaveDownloadError] = useState("");
  const [replayStepMs, setReplayStepMs] = useState(100);
  const [replayForS, setReplayForS] = useState(5);
  const [replayTo, setReplayTo] = useState("");
  const [recordName, setRecordName] = useState("record");
  const [recordFormat, setRecordFormat] = useState(0);
  const [recordFilter, setRecordFilter] = useState("");
  const [helpOpen, setHelpOpen] = useState(false);
  const [logPath, setLogPath] = useState("");
  const [pickerOpen, setPickerOpen] = useState(false);
  const [monitorPickerOpen, setMonitorPickerOpen] = useState(false);
  const [dataPickerOpen, setDataPickerOpen] = useState(false);
  const [statePickerOpen, setStatePickerOpen] = useState(false);
  const [scriptsOpen, setScriptsOpen] = useState(false);
  const [exportOpen, setExportOpen] = useState(false);
  const [exportKind, setExportKind] = useState("trace");
  const [stateRule, setStateRule] = useState<StateRow | null>(null);
  const [catalog, setCatalog] = useState<Catalog | null>(null);
  const [catalogLoading, setCatalogLoading] = useState(false);
  const [catalogError, setCatalogError] = useState("");
  const [busesOpen, setBusesOpen] = useState(false);
  const [generatorOpen, setGeneratorOpen] = useState(false);
  const [triggersOpen, setTriggersOpen] = useState(false);
  const [sysvarsOpen, setSysvarsOpen] = useState(false);
  const [hardware, setHardware] = useState<Hardware | null>(null);
  const [hardwareLoading, setHardwareLoading] = useState(false);
  const [hardwareError, setHardwareError] = useState("");
  const [layout, setLayout] = useState<Layout>(loadLayout);
  const [drag, setDrag] = useState<Drag>(null);
  const [theme, setTheme] = useState<Theme>(loadTheme);
  const [menuOpen, setMenuOpen] = useState(false);
  const [signalView, setSignalView] = useState<SignalView>({
    windowS: 30,
    endS: null,
  });
  const [chartCursors, setChartCursors] = useState<ChartCursor[]>([]);
  const [chartFullscreen, setChartFullscreen] = useState(false);
  const uiFps = useUiFps();
  const workspaceRef = useRef<HTMLDivElement>(null);
  const rightRef = useRef<HTMLDivElement>(null);
  const chartPanelRef = useRef<HTMLElement>(null);
  const chartCursorSeq = useRef(1);
  const chartPalette = palette(theme);

  useEffect(() => {
    applyTheme(theme);
    saveTheme(theme);
  }, [theme]);

  useEffect(() => {
    if (!menuOpen) return;
    const close = (event: PointerEvent) => {
      if (
        !(event.target instanceof Element) ||
        !event.target.closest(".brand-menu")
      ) {
        setMenuOpen(false);
      }
    };
    const escape = (event: KeyboardEvent) => {
      if (event.key === "Escape") setMenuOpen(false);
    };
    window.addEventListener("pointerdown", close);
    window.addEventListener("keydown", escape);
    return () => {
      window.removeEventListener("pointerdown", close);
      window.removeEventListener("keydown", escape);
    };
  }, [menuOpen]);

  useEffect(() => {
    const panel = chartPanelRef.current;
    if (!panel) return;
    const sync = () => {
      if (document.fullscreenElement === panel) {
        setChartFullscreen(true);
        return;
      }
      panel.classList.remove("chart-fullscreen");
      setChartFullscreen(false);
    };
    document.addEventListener("fullscreenchange", sync);
    return () => {
      document.removeEventListener("fullscreenchange", sync);
      panel.classList.remove("chart-fullscreen");
      if (document.fullscreenElement === panel) {
        void document.exitFullscreen();
      }
    };
  }, []);

  useEffect(() => {
    if (!chartFullscreen) return;
    const escape = (event: KeyboardEvent) => {
      if (event.key !== "Escape") return;
      // A menu opened inside fullscreen owns the first Escape.
      if (document.querySelector(".signal-menu, .trace-menu, .modal-backdrop")) {
        return;
      }
      const panel = chartPanelRef.current;
      if (!panel) return;
      panel.classList.remove("chart-fullscreen");
      setChartFullscreen(false);
      if (document.fullscreenElement === panel) {
        void document.exitFullscreen();
      }
    };
    window.addEventListener("keydown", escape);
    return () => window.removeEventListener("keydown", escape);
  }, [chartFullscreen]);

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
        const next = await fetchState(
          epoch,
          seq,
          signalRef.current.epoch,
          signalRef.current.since,
        );
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

        let series = signalRef.current.series;
        if (next.signalsSeed) {
          series = next.signalsSeed;
          setSignals(series);
        } else if (next.signalsDelta?.length) {
          const byKey = new Map(series.map((entry) => [entry.key, entry]));
          for (const delta of next.signalsDelta) {
            const previous = byKey.get(delta.key);
            const points = previous
              ? [...previous.points, ...delta.points].slice(-SIGNAL_CAP)
              : delta.points;
            byKey.set(delta.key, { ...delta, points });
          }
          series = [...byKey.values()];
          setSignals(series);
        }
        signalRef.current = {
          epoch: next.signalsEpoch,
          since: next.signalsMaxT,
          series,
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
    if (!sessionOpen) return;
    const close = (event: KeyboardEvent) => {
      if (event.key === "Escape") setSessionOpen(false);
    };
    window.addEventListener("keydown", close);
    return () => window.removeEventListener("keydown", close);
  }, [sessionOpen]);

  useEffect(() => {
    if (!busesOpen) return;
    let alive = true;
    setHardwareLoading(true);
    fetchHardware()
      .then((data) => {
        if (!alive) return;
        setHardware(data);
        setHardwareError("");
      })
      .catch((error) => {
        if (alive) setHardwareError(String(error));
      })
      .finally(() => {
        if (alive) setHardwareLoading(false);
      });
    return () => {
      alive = false;
    };
  }, [busesOpen, state?.hwRefresh]);

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

  const addChartCursor = (t: number) => {
    const id = chartCursorSeq.current++;
    setChartCursors((old) => [...old, { id, t }]);
  };

  const moveChartCursor = (id: number, t: number) => {
    setChartCursors((old) =>
      old.map((cursor) => (cursor.id === id ? { ...cursor, t } : cursor)),
    );
  };

  const clearChartCursors = () => setChartCursors([]);

  const chartSignalAction = (key: string, action: ChartSignalAction) => {
    void run({ type: "chart_signal", key, action });
    if (action !== "remove") {
      setChartTab(action);
    }
  };

  const placeChartCursor = (tUs: number) => {
    const t = tUs / 1e6;
    if (!chartCursors.some((cursor) => Math.abs(cursor.t - t) < 1e-6)) {
      addChartCursor(t);
    }
    setSignalView((old) => ({ ...old, endS: t + old.windowS / 2 }));
    setChartTab("signals");
  };

  const toggleChartFullscreen = () => {
    const panel = chartPanelRef.current;
    if (!panel) return;
    if (chartFullscreen) {
      panel.classList.remove("chart-fullscreen");
      setChartFullscreen(false);
      if (document.fullscreenElement === panel) {
        void document.exitFullscreen();
      }
      return;
    }
    panel.classList.add("chart-fullscreen");
    setChartFullscreen(true);
    void panel.requestFullscreen().catch(() => {});
  };

  const resetChartTools = () => {
    setChartCursors([]);
    setSignalView({ windowS: 30, endS: null });
  };

  const loadProject = (path: string) => {
    const value = path.trim();
    if (!value) return;
    void run({ type: "open_project", path: value });
    resetChartTools();
    setSessionOpen(false);
    setScriptsOpen(false);
  };

  const loadLog = (path: string) => {
    const value = path.trim();
    if (!value) return;
    void run({ type: "open_log", path: value });
    resetChartTools();
    setSessionOpen(false);
    setScriptsOpen(false);
  };

  const saveProject = (path: string | null) => {
    void run({ type: "save_project", path });
  };

  const saveAndDownload = async () => {
    const name = savePath.trim();
    if (!isDownloadName(name)) return;
    setSaveDownloading(true);
    setSaveDownloadError("");
    await clearDownload(name);
    saveProject(name);
    const url = await waitForDownload(name);
    setSaveDownloading(false);
    if (!url) {
      setSaveDownloadError("The project was not written in time.");
      return;
    }
    triggerDownload(url, name);
    setSessionOpen(false);
  };

  const loadCatalog = async () => {
    setCatalogLoading(true);
    try {
      setCatalog(await fetchCatalog());
      setCatalogError("");
    } catch (e) {
      setCatalogError(String(e));
    } finally {
      setCatalogLoading(false);
    }
  };

  const openPicker = () => {
    setSessionOpen(false);
    setScriptsOpen(false);
    setBusesOpen(false);
    setGeneratorOpen(false);
    setTriggersOpen(false);
    setSysvarsOpen(false);
    setMonitorPickerOpen(false);
    setDataPickerOpen(false);
    setStatePickerOpen(false);
    setPickerOpen(true);
    void loadCatalog();
  };

  const openGenerator = () => {
    setSessionOpen(false);
    setScriptsOpen(false);
    setBusesOpen(false);
    setPickerOpen(false);
    setMonitorPickerOpen(false);
    setDataPickerOpen(false);
    setStatePickerOpen(false);
    setTriggersOpen(false);
    setSysvarsOpen(false);
    setGeneratorOpen(true);
    void loadCatalog();
  };

  const openTriggers = () => {
    setSessionOpen(false);
    setScriptsOpen(false);
    setBusesOpen(false);
    setPickerOpen(false);
    setMonitorPickerOpen(false);
    setDataPickerOpen(false);
    setStatePickerOpen(false);
    setGeneratorOpen(false);
    setSysvarsOpen(false);
    setTriggersOpen(true);
    void loadCatalog();
  };

  const openSysvars = () => {
    setSessionOpen(false);
    setScriptsOpen(false);
    setBusesOpen(false);
    setPickerOpen(false);
    setMonitorPickerOpen(false);
    setDataPickerOpen(false);
    setStatePickerOpen(false);
    setGeneratorOpen(false);
    setTriggersOpen(false);
    setSysvarsOpen(true);
  };

  const openMonitorPicker = () => {
    setSessionOpen(false);
    setScriptsOpen(false);
    setBusesOpen(false);
    setPickerOpen(false);
    setDataPickerOpen(false);
    setStatePickerOpen(false);
    setGeneratorOpen(false);
    setTriggersOpen(false);
    setSysvarsOpen(false);
    setMonitorPickerOpen(true);
    void loadCatalog();
  };

  const openExport = (kind: string) => {
    setExportKind(kind);
    setExportOpen(true);
  };

  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      const target = event.target as HTMLElement | null;
      if (
        target &&
        (target.tagName === "INPUT" ||
          target.tagName === "TEXTAREA" ||
          target.tagName === "SELECT" ||
          target.isContentEditable)
      ) {
        return;
      }
      const mod = event.ctrlKey || event.metaKey;
      const key = event.key.toLowerCase();
      if (event.key === "F9") {
        event.preventDefault();
        void run({ type: state?.measuring ? "stop" : "start" });
      } else if (mod && event.shiftKey && key === "s") {
        event.preventDefault();
        setSessionOpen(true);
      } else if (mod && key === "s") {
        event.preventDefault();
        if (state?.projectPath) void run({ type: "save_project", path: null });
        else setSessionOpen(true);
      } else if (mod && key === "o") {
        event.preventDefault();
        setSessionOpen(true);
      } else if (mod && key === "e") {
        event.preventDefault();
        openExport("trace");
      } else if (event.key === "?") {
        setHelpOpen(true);
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [state?.measuring, state?.projectPath]);

  const openDataPicker = () => {    setSessionOpen(false);
    setScriptsOpen(false);
    setBusesOpen(false);
    setPickerOpen(false);
    setMonitorPickerOpen(false);
    setStatePickerOpen(false);
    setGeneratorOpen(false);
    setTriggersOpen(false);
    setSysvarsOpen(false);
    setDataPickerOpen(true);
    void loadCatalog();
  };

  const openStatePicker = () => {
    setSessionOpen(false);
    setScriptsOpen(false);
    setBusesOpen(false);
    setPickerOpen(false);
    setMonitorPickerOpen(false);
    setDataPickerOpen(false);
    setGeneratorOpen(false);
    setTriggersOpen(false);
    setSysvarsOpen(false);
    setStatePickerOpen(true);
    void loadCatalog();
  };

  const editMonitor = (
    row: {
      kind: string;
      ch: number;
      id: number;
      ext: boolean;
      frame?: string;
      name: string;
    },
    patch: MonitorPatch,
  ) => {
    void run({
      type: "edit_monitor",
      kind: row.kind,
      ch: row.ch,
      id: row.id,
      ext: row.ext,
      frame: row.frame,
      name: row.name,
      digits: patch.digits,
      rule_on: patch.ruleOn,
      rising: patch.rising,
      threshold: patch.threshold,
      color: patch.color,
    });
  };

  const defineSysVar = (edit: SysVarEdit) => {
    void run({
      type: "define_sys_var",
      namespace: edit.namespace,
      name: edit.name,
      init: edit.init,
      min: edit.min,
      max: edit.max,
      unit: edit.unit,
      comment: edit.comment,
      old_namespace: edit.oldNamespace,
      old_name: edit.oldName,
    });
  };

  const toggleSignal = (
    channel: CatalogChannel,
    message: CatalogMessage,
    signal: CatalogSignal,
    selected: boolean,
  ) => {
    if (channel.kind === "fr") {
      void run({
        type: selected ? "select_fr_signal" : "unselect_fr_signal",
        bus: channel.index,
        slot: message.id,
        name: signal.name,
      });
      return;
    }
    if (channel.kind === "eth") {
      void run({
        type: selected ? "select_eth_signal" : "unselect_eth_signal",
        bus: channel.index,
        frame: message.frame ?? message.name,
        name: signal.name,
      });
      return;
    }
    void run(
      selected
        ? {
            type: "select_signal",
            ch: channel.index,
            id: message.id,
            ext: message.extended,
            name: signal.name,
          }
        : {
            type: "unselect_signal",
            ch: channel.index,
            id: message.id,
            ext: message.extended,
            name: signal.name,
          },
    );
  };

  const moveSignal = (key: string, to: number) => {
    const from = signals.findIndex((signal) => signal.key === key);
    const target = clamp(to, 0, signals.length - 1);
    if (from < 0 || from === target) return;
    const next = [...signals];
    const [moved] = next.splice(from, 1);
    next.splice(target, 0, moved);
    signalRef.current = { ...signalRef.current, series: next };
    setSignals(next);
    void run({ type: "move_signal", key, to: target });
  };

  const refreshHardware = () => void run({ type: "refresh_hardware" });
  const attachHardware = (bus: number, driver: string, index: number) =>
    void run({ type: "attach_hardware", bus, driver, index });
  const detachHardware = (bus: number) =>
    void run({ type: "detach_hardware", bus });
  const attachFrHardware = (bus: number, index: number) =>
    void run({ type: "attach_fr_hardware", bus, index });
  const detachFrHardware = (bus: number) =>
    void run({ type: "detach_fr_hardware", bus });
  const setRealBus = (real: boolean) =>
    void run({ type: "set_bus_mode", real });
  const addBus = () => void run({ type: "add_bus" });
  const removeBus = (bus: number) => void run({ type: "remove_bus", ch: bus });
  const renameBus = (bus: number, name: string) =>
    void run({ type: "rename_bus", ch: bus, name });
  const setBusRates = (bus: number, arb: number, fd: number) =>
    void run({
      type: "set_bus_rates",
      ch: bus,
      bitrate_kbps: arb,
      fd_data_kbps: fd,
    });
  const loadDbc = (bus: number, path: string) =>
    void run({ type: "load_dbc", ch: bus, path });
  const setEntryActive = (entry: TxEntry, on: boolean) =>
    void run({ type: "set_entry_active", ch: entry.channel, id: entry.id, on });
  const setEntryCycle = (entry: TxEntry, cycleUs: number) =>
    void run({
      type: "set_entry_cycle",
      ch: entry.channel,
      id: entry.id,
      cycle_us: cycleUs,
    });
  const setEntryFd = (entry: TxEntry, fd: boolean) =>
    void run({ type: "set_entry_fd", ch: entry.channel, id: entry.id, fd });
  const setEntryHex = (entry: TxEntry, text: string) =>
    void run({ type: "set_entry_hex", ch: entry.channel, id: entry.id, text });
  const addEntry = (ch: number, id: number) =>
    void run({ type: "add_entry", ch, id });
  const removeEntry = (entry: TxEntry) =>
    void run({ type: "remove_entry", ch: entry.channel, id: entry.id });
  const simulateBus = (ch: number) =>
    void run({ type: "simulate_bus_nodes", ch });
  const addTrigger = (kind: string) =>
    void run({ type: "add_trigger", kind });
  const setTriggerEnabled = (index: number, on: boolean) =>
    void run({ type: "set_trigger_enabled", index, on });
  const removeTrigger = (index: number) =>
    void run({ type: "remove_trigger", index });
  const rearmTriggers = () => void run({ type: "rearm_triggers" });
  const editTrigger = (edit: TriggerEdit) =>
    void run({
      type: "edit_trigger",
      index: edit.index,
      kind: edit.kind,
      ch: edit.ch,
      id: edit.id,
      ext: edit.ext,
      signal: edit.signal,
      threshold: edit.threshold,
      rising: edit.rising,
      bus: edit.bus,
      slot: edit.slot,
      key: edit.key,
      action: edit.action,
      send_ch: edit.sendCh,
      send_id: edit.sendId,
    });
  const setTriggerLimits = (limits: TriggerLimits) =>
    void run({
      type: "set_trigger_limits",
      pre_frames: limits.preFrames,
      post_frames: limits.postFrames,
      marker_cap: limits.markerCap,
    });

  const recent = (paths: string[], open: (path: string) => void) =>
    paths.length > 0 && (
      <div class="recent">
        <span>Recent</span>
        {paths.slice(0, 5).map((path) => (
          <button
            type="button"
            class="recent-item"
            title={path}
            key={path}
            onClick={() => open(path)}
          >
            {baseName(path)}
          </button>
        ))}
      </div>
    );

  const lowerMessages = useMemo(() => {
    const rows = state?.messages ?? [];
    const needle = panelQuery.trim().toLowerCase();
    if (!needle) return rows;
    return rows.filter(
      (row) =>
        row.name.toLowerCase().includes(needle) ||
        row.addr.toLowerCase().includes(needle) ||
        row.busName.toLowerCase().includes(needle),
    );
  }, [state?.messages, panelQuery]);

  const writeRows = useMemo(() => {
    const rows = state?.write ?? [];
    const needle = writeQuery.trim().toLowerCase();
    return rows.filter((row) => {
      if (writeKind !== "all" && row.kind !== writeKind) return false;
      return !needle || row.text.toLowerCase().includes(needle);
    });
  }, [state?.write, writeKind, writeQuery]);

  const lowerStats = useMemo(() => {
    const rows = state?.stats ?? [];
    const needle = panelQuery.trim().toLowerCase();
    if (!needle) return rows;
    return rows.filter(
      (row) =>
        row.label.toLowerCase().includes(needle) ||
        row.bus.toLowerCase().includes(needle),
    );
  }, [state?.stats, panelQuery]);

  const visibleRows = useMemo(() => {
    const payload = traceFilter.payload.trim().toUpperCase().replace(/\s+/g, "");
    const from = traceFilter.from.trim() === "" ? null : Number(traceFilter.from) * 1e6;
    const to = traceFilter.to.trim() === "" ? null : Number(traceFilter.to) * 1e6;
    const include = parseIds(traceFilter.idInclude);
    const exclude = parseIds(traceFilter.idExclude);
    return traceRows.filter((frame) => {
      if (bus !== "all" && frame.busName !== bus) return false;
      if (traceFilter.dir !== "all" && frame.dir !== traceFilter.dir) return false;
      if (traceFilter.dbcOnly && !frame.name) return false;
      if (include && !include.has(frame.id)) return false;
      if (exclude?.has(frame.id)) return false;
      if (payload && !frame.data.toUpperCase().replace(/\s+/g, "").includes(payload)) {
        return false;
      }
      if (from != null && frame.tUs < from) return false;
      if (to != null && frame.tUs > to) return false;
      return matches(frame, query);
    });
  }, [traceRows, query, bus, traceFilter]);

  const canControl = Boolean(state) && online && !busy;
  const measuring = state?.measuring ?? false;
  const needsLog = state?.runMode === "Replay" && !state?.log;
  const latest = state?.series.length
    ? state.series[state.series.length - 1]
    : undefined;
  const session = state
    ? [state.project || "Untitled", state.log].filter(Boolean).join(" · ")
    : "";
  const replayText =
    state?.replayPosUs != null && state?.replayTotalUs != null
      ? `${(state.replayPosUs / 1e6).toFixed(2)} / ${(state.replayTotalUs / 1e6).toFixed(2)} s`
      : "";
  const selectedKeys = new Set(signals.map((signal) => signal.key));
  const monitorKeys = new Set((state?.monitor ?? []).map(rowKey));
  const dataKeys = new Set((state?.data ?? []).map(rowKey));
  const stateKeys = new Set((state?.stateRows ?? []).map(rowKey));

  return (
    <div class="app">
      <header class="topbar">
        <div class="brand">
          <div class="brand-menu">
            <button
              type="button"
              class="logo"
              aria-label="roxy-can menu"
              aria-haspopup="menu"
              aria-expanded={menuOpen}
              aria-controls="brand-menu-panel"
              title="Menu"
              onClick={() => setMenuOpen((open) => !open)}
            >
              R
            </button>
            {menuOpen && (
              <div class="brand-menu-panel" id="brand-menu-panel" role="menu">
                <button
                  type="button"
                  role="menuitem"
                  onClick={() => {
                    setHelpOpen(true);
                    setMenuOpen(false);
                  }}
                >
                  <svg viewBox="0 0 16 16" aria-hidden="true">
                    <rect x="1.5" y="4" width="13" height="8" rx="1.5" />
                    <path d="M4 6.5h.01M6.5 6.5h.01M9 6.5h.01M11.5 6.5h.01M4 9.5h8" />
                  </svg>
                  <span>Shortcuts</span>
                </button>
                <button
                  type="button"
                  role="menuitemcheckbox"
                  aria-checked={theme === "dark"}
                  onClick={() => {
                    setTheme(theme === "dark" ? "light" : "dark");
                    setMenuOpen(false);
                  }}
                >
                  <svg viewBox="0 0 16 16" aria-hidden="true">
                    <path d="M13 10.2A5.6 5.6 0 0 1 5.8 3a5.5 5.5 0 1 0 7.2 7.2z" />
                  </svg>
                  <span>Dark mode</span>
                  <span class="brand-menu-state">
                    {theme === "dark" ? "On" : "Off"}
                  </span>
                </button>
              </div>
            )}
          </div>
          <div class="brand-text">
            <h1>
              roxy-can <span class="tag">web spike</span>
            </h1>
            <p>
              {state
                ? session ||
                  `${state.channels.length} buses · ${state.channels.map((c) => c.name).join(", ")}`
                : "connecting to the bus core"}
            </p>
          </div>
        </div>
        <div class="actions">
          <button
            type="button"
            disabled={!online}
            onClick={openGenerator}
          >
            Generator…
          </button>
          <button
            type="button"
            disabled={!online}
            onClick={openTriggers}
          >
            Triggers…
          </button>
          <button
            type="button"
            disabled={!online}
            onClick={() => {
              setSessionOpen(false);
    setScriptsOpen(false);
              setPickerOpen(false);
              setMonitorPickerOpen(false);
              setDataPickerOpen(false);
              setStatePickerOpen(false);
              setGeneratorOpen(false);
              setTriggersOpen(false);
              setSysvarsOpen(false);
              setScriptsOpen(false);
              setBusesOpen(true);
            }}
          >
            Buses…
          </button>
          <button
            type="button"
            disabled={!online}
            onClick={openSysvars}
          >
            SysVars…
          </button>
          <button
            type="button"
            disabled={!online}
            onClick={() => {
              setPickerOpen(false);
              setMonitorPickerOpen(false);
              setDataPickerOpen(false);
              setStatePickerOpen(false);
              setScriptsOpen(false);
              setBusesOpen(false);
              setGeneratorOpen(false);
              setTriggersOpen(false);
              setSysvarsOpen(false);
              setScriptsOpen(false);
              setSessionOpen(true);
            }}
          >
            Session…
          </button>
          <button
            type="button"
            class="primary"
            disabled={!canControl || measuring || needsLog}
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
          {state?.recording && <span class="rec-badge">REC</span>}
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
          <strong>{state?.runMode ?? state?.mode ?? "—"}</strong>
        </div>
        {error && <div class="error-pill">{error}</div>}
      </section>

      {state?.runMode === "Replay" && state.log && (
        <section class="replaybar">
          <span class="replay-mode">Replay</span>
          <span class="replay-pos">{replayText || "no position"}</span>
          <div class="replay-nav">
            <label>
              Speed
              <select
                value={state.replaySpeed >= 1e8 ? "fast" : String(state.replaySpeed)}
                onChange={(event) => {
                  const value = (event.target as HTMLSelectElement).value;
                  void run({
                    type: "set_replay_speed",
                    speed: value === "fast" ? 1e9 : Number(value),
                  });
                }}
              >
                {[0.5, 1, 2, 4].map((speed) => (
                  <option key={speed} value={speed}>
                    {speed}x
                  </option>
                ))}
                <option value="fast">Fast</option>
              </select>
            </label>
            <label>
              Step
              <select
                value={replayStepMs}
                onChange={(event) =>
                  setReplayStepMs(Number((event.target as HTMLSelectElement).value))
                }
              >
                {[10, 50, 100, 500, 1000, 5000].map((ms) => (
                  <option key={ms} value={ms}>
                    {ms >= 1000 ? `${ms / 1000} s` : `${ms} ms`}
                  </option>
                ))}
              </select>
            </label>
            <button
              type="button"
              disabled={!state.measuring}
              title="Advance the replay by the step size (forward only)"
              onClick={() =>
                void run({ type: "replay_step", delta_us: replayStepMs * 1000 })
              }
            >
              Step
            </button>
            <label>
              Play for
              <select
                value={replayForS}
                onChange={(event) =>
                  setReplayForS(Number((event.target as HTMLSelectElement).value))
                }
              >
                {[1, 5, 10, 30, 60].map((s) => (
                  <option key={s} value={s}>
                    {s} s
                  </option>
                ))}
              </select>
            </label>
            <button
              type="button"
              disabled={!state.measuring}
              title="Resume and pause after this much log time"
              onClick={() =>
                void run({
                  type: "replay_play_for",
                  duration_us: replayForS * 1_000_000,
                })
              }
            >
              Play
            </button>
            <form
              class="replay-to"
              onSubmit={(event) => {
                event.preventDefault();
                const secs = Number(replayTo);
                if (Number.isFinite(secs) && secs >= 0) {
                  void run({
                    type: "replay_play_to",
                    target_us: Math.round(secs * 1_000_000),
                  });
                }
              }}
            >
              <label>
                to
                <input
                  value={replayTo}
                  placeholder="s"
                  inputMode="decimal"
                  onInput={(event) =>
                    setReplayTo((event.target as HTMLInputElement).value)
                  }
                />
              </label>
              <button
                type="submit"
                disabled={
                  !state.measuring ||
                  !replayTo.trim() ||
                  !Number.isFinite(Number(replayTo))
                }
                title="Play forward to this absolute log time, then pause"
              >
                Go
              </button>
            </form>
          </div>
          <span class="spacer" />
          <span class="replay-hint">forward only · pauses at the target</span>
        </section>
      )}

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
              <button
                type="button"
                class="mini"
                onClick={() => openExport("trace")}
                title="Export the trace to an ASC or PCAP host file"
              >
                Export…
              </button>
              <button
                type="button"
                class={`mini ${traceFilterActive(traceFilter) ? "on" : ""}`}
                onClick={() => setTraceFilterOpen(true)}
                title="Direction, DBC-only, payload and time filters"
              >
                Filters
              </button>
              <select
                value={bus}
                onChange={(e) => {
                  setBus((e.target as HTMLSelectElement).value);
                }}
              >
                <option value="all">All buses</option>
                {state?.channels.map((channel) => (
                  <option key={channel.name} value={channel.name}>
                    {channel.name}
                  </option>
                ))}
                {state?.frBuses.map((fr) => (
                  <option key={fr.name} value={fr.name}>
                    {fr.name}
                  </option>
                ))}
                {state?.ethBuses.map((eth) => (
                  <option key={eth.name} value={eth.name}>
                    {eth.name}
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
            onQuery={setQuery}
            onChartCursor={placeChartCursor}
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
          <section class="panel chart-panel" ref={chartPanelRef}>
            <div class="panel-head">
              <div class="tabs chart-tabs">
                <button
                  type="button"
                  class={chartTab === "rate" ? "on" : ""}
                  onClick={() => setChartTab("rate")}
                >
                  Rate
                </button>
                <button
                  type="button"
                  class={chartTab === "signals" ? "on" : ""}
                  onClick={() => setChartTab("signals")}
                >
                  Signals
                </button>
                <button
                  type="button"
                  class={chartTab === "monitor" ? "on" : ""}
                  onClick={() => setChartTab("monitor")}
                >
                  Monitor
                </button>
                <button
                  type="button"
                  class={chartTab === "data" ? "on" : ""}
                  onClick={() => setChartTab("data")}
                >
                  Data
                </button>
                <button
                  type="button"
                  class={chartTab === "state" ? "on" : ""}
                  onClick={() => setChartTab("state")}
                >
                  State
                </button>
              </div>
              {chartTab === "signals" && (
                <button
                  type="button"
                  class="mini"
                  onClick={() => void openPicker()}
                >
                  Pick signals…
                </button>
              )}
              {chartTab === "monitor" && (
                <>
                  <button
                    type="button"
                    class="mini"
                    onClick={() => void openMonitorPicker()}
                  >
                    + Signal
                  </button>
                  <button
                    type="button"
                    class="mini"
                    disabled={!(state?.monitor.length ?? 0)}
                    onClick={() => void run({ type: "clear_monitor" })}
                  >
                    Clear
                  </button>
                </>
              )}
              {chartTab === "data" && (
                <>
                  <button
                    type="button"
                    class="mini"
                    onClick={() => void openDataPicker()}
                  >
                    + Signal
                  </button>
                  <button
                    type="button"
                    class="mini"
                    disabled={!(state?.data.length ?? 0)}
                    onClick={() => void run({ type: "clear_data" })}
                  >
                    Clear
                  </button>
                </>
              )}
              {chartTab === "state" && (
                <>
                  <button
                    type="button"
                    class="mini"
                    onClick={() => void openStatePicker()}
                  >
                    + Signal
                  </button>
                  <button
                    type="button"
                    class="mini"
                    disabled={!(state?.stateRows.length ?? 0)}
                    onClick={() => void run({ type: "clear_state" })}
                  >
                    Clear
                  </button>
                </>
              )}
              {(chartTab === "rate" || chartTab === "signals") && (
                <span class="meta">
                  {chartTab === "rate"
                    ? "solid f/s · dashed load"
                    : `${signals.length} channels · wheel zoom`}
                </span>
              )}
              {chartTab === "rate" && (
                <strong class="readout">
                  {state ? state.fps.toFixed(0) : "—"} f/s
                </strong>
              )}
              <div class="chart-head-tools">
                {chartFullscreen && (
                  <button
                    type="button"
                    class="chart-theme"
                    aria-label={
                      theme === "dark"
                        ? "Switch to light theme"
                        : "Switch to dark theme"
                    }
                    title={
                      theme === "dark"
                        ? "Switch to light theme"
                        : "Switch to dark theme"
                    }
                    onClick={() =>
                      setTheme(theme === "dark" ? "light" : "dark")
                    }
                  >
                    <svg viewBox="0 0 16 16" aria-hidden="true">
                      {theme === "dark" ? (
                        <path d="M8 1.5v2M8 12.5v2M1.5 8h2M12.5 8h2M3.4 3.4l1.4 1.4M11.2 11.2l1.4 1.4M12.6 3.4l-1.4 1.4M4.8 11.2l-1.4 1.4M8 5a3 3 0 1 0 3 3" />
                      ) : (
                        <path d="M13 10.2A5.6 5.6 0 0 1 5.8 3a5.5 5.5 0 1 0 7.2 7.2z" />
                      )}
                    </svg>
                  </button>
                )}
                <button
                  type="button"
                  class="chart-focus"
                  aria-label={
                    chartFullscreen ? "Exit fullscreen" : "Fullscreen chart"
                  }
                  title={
                    chartFullscreen ? "Exit fullscreen" : "Fullscreen chart"
                  }
                  onClick={toggleChartFullscreen}
                >
                  <svg viewBox="0 0 16 16" aria-hidden="true">
                    {chartFullscreen ? (
                      <path d="M6 2v4H2M10 2v4h4M6 14v-4H2M10 14v-4h4" />
                    ) : (
                      <path d="M2 6V2h4M14 6V2h-4M2 10v4h4M14 10v4h-4" />
                    )}
                  </svg>
                </button>
              </div>
            </div>
            {chartTab === "rate" ? (
              <>
                <div class="chart-legend">
                  <span class="legend-key">
                    <i style={{ background: chartPalette.rateTotal }} />
                    Total <b>{state ? state.fps.toFixed(0) : "—"}</b> f/s
                  </span>
                  {state?.channels.map((channel, index) => (
                    <span class="legend-key" key={channel.name}>
                      <i
                        style={{
                          background:
                            chartPalette.buses[index % chartPalette.buses.length],
                        }}
                      />
                      {channel.name}{" "}
                      <b>{(latest?.busRates[index] ?? 0).toFixed(0)}</b> f/s
                      <em>{(latest?.busLoads[index] ?? 0).toFixed(1)}%</em>
                    </span>
                  ))}
                </div>
                <RateChart
                  points={state?.series ?? []}
                  channels={state?.channels ?? []}
                  theme={theme}
                />
              </>
            ) : chartTab === "signals" ? (
              <SignalChart
                series={signals}
                markers={state?.markers ?? []}
                theme={theme}
                onReorder={moveSignal}
                view={signalView}
                onView={(update) => setSignalView(update)}
                cursors={chartCursors}
                onAddCursor={addChartCursor}
                onMoveCursor={moveChartCursor}
                onClearCursors={clearChartCursors}
                onSignalAction={chartSignalAction}
                fullscreen={chartFullscreen}
              />
            ) : chartTab === "monitor" ? (
              <MonitorPanel
                rows={state?.monitor ?? []}
                theme={theme}
                onEdit={editMonitor}
                onRemove={(row) =>
                  void run({
                    type: "remove_monitor",
                    kind: row.kind,
                    ch: row.ch,
                    id: row.id,
                    ext: row.ext,
                    frame: row.frame,
                    name: row.name,
                  })
                }
              />
            ) : chartTab === "data" ? (
              <DataPanel
                rows={state?.data ?? []}
                onVisible={(row, visible) =>
                  void run({
                    type: "set_data_visible",
                    kind: row.kind,
                    ch: row.ch,
                    id: row.id,
                    ext: row.ext,
                    frame: row.frame,
                    name: row.name,
                    visible,
                  })
                }
                onRemove={(row) =>
                  void run({
                    type: "remove_data",
                    kind: row.kind,
                    ch: row.ch,
                    id: row.id,
                    ext: row.ext,
                    frame: row.frame,
                    name: row.name,
                  })
                }
              />
            ) : (
              <StatePanel
                rows={state?.stateRows ?? []}
                windowS={state?.stateWindowS ?? 20}
                theme={theme}
                onWindow={(seconds) =>
                  void run({ type: "set_state_window", seconds })
                }
                onRule={setStateRule}
                onRemove={(row) =>
                  void run({
                    type: "remove_state",
                    kind: row.kind,
                    ch: row.ch,
                    id: row.id,
                    ext: row.ext,
                    frame: row.frame,
                    name: row.name,
                  })
                }
              />
            )}
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
                  class={tab === "statistics" ? "on" : ""}
                  onClick={() => setTab("statistics")}
                >
                  Statistics
                </button>
                <button
                  type="button"
                  class={tab === "bus" ? "on" : ""}
                  onClick={() => setTab("bus")}
                >
                  Bus
                </button>
                <button
                  type="button"
                  class={tab === "spec" ? "on" : ""}
                  onClick={() => setTab("spec")}
                >
                  Spec
                </button>
                <button
                  type="button"
                  class={tab === "activity" ? "on" : ""}
                  onClick={() => setTab("activity")}
                >
                  Write
                </button>
              </div>
              <span class="meta">
                {tab === "messages"
                  ? `${lowerMessages.length} ids`
                  : tab === "statistics"
                    ? `${lowerStats.length} rows`
                    : tab === "bus"
                      ? `${state?.busStats.length ?? 0} buses`
                      : tab === "spec"
                        ? `${state?.specRows.length ?? 0} violations`
                        : `${writeRows.length} line${writeRows.length === 1 ? "" : "s"}`}
              </span>
              {tab !== "bus" && tab !== "spec" && (
                <button
                  type="button"
                  class="mini"
                  onClick={() =>
                    openExport(
                      tab === "messages"
                        ? "messages"
                        : tab === "statistics"
                          ? "stats"
                          : "write",
                    )
                  }
                  title="Export this panel to a host file"
                >
                  Export…
                </button>
              )}
            </div>
            {tab === "messages" || tab === "statistics" ? (
              <div class="panel-list">
                <div class="panel-filter-row">
                  <input
                    class="panel-filter"
                    type="search"
                    value={panelQuery}
                    placeholder="Filter bus, name or id…"
                    onInput={(event) =>
                      setPanelQuery((event.target as HTMLInputElement).value)
                    }
                  />
                  {panelQuery && (
                    <button
                      type="button"
                      class="mini"
                      onClick={() => setPanelQuery("")}
                    >
                      Clear
                    </button>
                  )}
                </div>
                {tab === "messages" ? (
                  <MessageTable
                    rows={lowerMessages}
                    filtered={panelQuery.trim() !== ""}
                  />
                ) : (
                  <StatsTable
                    rows={lowerStats}
                    filtered={panelQuery.trim() !== ""}
                  />
                )}
              </div>
            ) : tab === "bus" ? (
              <BusStatsPanel buses={state?.busStats ?? []} theme={theme} />
            ) : tab === "spec" ? (
              <SpecPanel
                rows={state?.specRows ?? []}
                tolerance={state?.specTolerance ?? 5}
                grace={state?.specGrace ?? 3}
                periodic={state?.specPeriodic ?? 0}
                onTolerance={(value) =>
                  void run({ type: "set_spec_tolerance", value })
                }
                onGrace={(value) =>
                  void run({ type: "set_spec_grace", value })
                }
                onClear={() => void run({ type: "clear_spec" })}
                onExport={() => openExport("spec")}
              />
            ) : (
              <div class="panel-list">
                <div class="panel-filter-row">
                  <select
                    value={writeKind}
                    onChange={(event) =>
                      setWriteKind((event.target as HTMLSelectElement).value)
                    }
                  >
                    <option value="all">All lines</option>
                    <option value="script">Script</option>
                    <option value="info">Info</option>
                    <option value="warn">Warn</option>
                    <option value="error">Error</option>
                  </select>
                  <input
                    class="panel-filter"
                    type="search"
                    value={writeQuery}
                    placeholder="Filter text…"
                    onInput={(event) =>
                      setWriteQuery((event.target as HTMLInputElement).value)
                    }
                  />
                  {writeQuery && (
                    <button
                      type="button"
                      class="mini"
                      onClick={() => setWriteQuery("")}
                    >
                      Clear
                    </button>
                  )}
                </div>
                <Activity
                  rows={writeRows}
                  clock={clock}
                  filtered={writeKind !== "all" || writeQuery.trim() !== ""}
                />
              </div>
            )}
          </section>
        </div>
      </main>

      <footer class="statusbar">
        <span class={`state ${measuring ? "running" : "stopped"}`}>
          {measuring ? (state?.tracePaused ? "PAUSED" : "MEASURING") : "STOPPED"}
        </span>
        <span>
          {replayText ||
            (measuring
              ? state?.log
                ? `${state.runMode} · ${state.log}`
                : "auto demo traffic · generated from the loaded DBCs"
              : state?.status || "waiting for core")}
        </span>
        <span class="spacer" />
        <span>poll 10 Hz</span>
        <span>{uiFps ? `UI ${uiFps} fps` : "UI —"}</span>
        <span>{window.location.host}</span>
      </footer>

      {triggersOpen && (
        <TriggersModal
          triggers={state?.triggers ?? []}
          limits={
            state?.triggerLimits ?? {
              preFrames: 0,
              postFrames: 0,
              markerCap: 8,
            }
          }
          buses={state?.buses ?? []}
          txEntries={state?.tx ?? []}
          catalog={catalog}
          catalogLoading={catalogLoading}
          onAdd={addTrigger}
          onToggle={setTriggerEnabled}
          onRemove={removeTrigger}
          onRearm={rearmTriggers}
          onEdit={editTrigger}
          onLimits={setTriggerLimits}
          onClose={() => setTriggersOpen(false)}
        />
      )}

      {generatorOpen && (
        <GeneratorModal
          entries={state?.tx ?? []}
          frTx={state?.frTx ?? []}
          blocks={state?.blocks ?? []}
          buses={state?.buses ?? []}
          frBuses={state?.frBuses ?? []}
          catalog={catalog}
          catalogLoading={catalogLoading}
          catalogError={catalogError}
          onToggle={setEntryActive}
          onCycle={setEntryCycle}
          onFd={setEntryFd}
          onHex={setEntryHex}
          onSource={(entry, src) =>
            void run({
              type: "set_entry_source",
              ch: entry.channel,
              id: entry.id,
              src,
            })
          }
          onClearSource={(entry, name) =>
            void run({
              type: "clear_entry_source",
              ch: entry.channel,
              id: entry.id,
              name,
            })
          }
          onAdd={addEntry}
          onRemove={removeEntry}
          onSimulate={simulateBus}
          onBlockAdd={(channel, name, path, node) =>
            void run({ type: "add_replay_block", channel, name, path, node })
          }
          onBlockToggle={(id, on) =>
            void run({ type: "set_replay_block_enabled", id, on })
          }
          onBlockRemove={(id) =>
            void run({ type: "remove_replay_block", id })
          }
          onFrToggle={(entry, on) =>
            void run({
              type: "set_fr_entry_active",
              bus: entry.bus,
              slot: entry.slot,
              on,
            })
          }
          onFrCycle={(entry, cycleUs) =>
            void run({
              type: "set_fr_entry_cycle",
              bus: entry.bus,
              slot: entry.slot,
              cycle_us: cycleUs,
            })
          }
          onFrHex={(entry, text) =>
            void run({
              type: "set_fr_entry_hex",
              bus: entry.bus,
              slot: entry.slot,
              text,
            })
          }
          onFrAdd={(bus, slot) =>
            void run({ type: "add_fr_entry", bus, slot })
          }
          onFrRemove={(entry) =>
            void run({
              type: "remove_fr_entry",
              bus: entry.bus,
              slot: entry.slot,
            })
          }
          onClose={() => setGeneratorOpen(false)}
        />
      )}

      {busesOpen && (
        <BusesModal
          buses={state?.buses ?? []}
          frBuses={state?.frBuses ?? []}
          ethBuses={state?.ethBuses ?? []}
          replayMap={
            state?.replayMap ?? { can: [], fr: [], eth: [] }
          }
          realBus={state?.realBus ?? false}
          hardware={hardware}
          loading={hardwareLoading}
          error={hardwareError}
          onError={setError}
          onRefresh={refreshHardware}
          onAddBus={addBus}
          onRemoveBus={removeBus}
          onRenameBus={renameBus}
          onSetRates={setBusRates}
          onLoadDbc={loadDbc}
          onImportArxml={(path) => void run({ type: "import_arxml", path })}
          onSetNodeRole={(bus, node, simulated) =>
            void run({ type: "set_node_role", bus, node, simulated })
          }
          onLoadFr={(path) =>
            void run({ type: "load_fr_description", path, bus: null })
          }
          onRemoveFr={(bus) => void run({ type: "remove_fr_bus", bus })}
          onLoadEth={(path) =>
            void run({ type: "load_eth_description", path, bus: null })
          }
          onRemoveEth={(bus) => void run({ type: "remove_eth_bus", bus })}
          onAttach={attachHardware}
          onDetach={detachHardware}
          onAttachFr={attachFrHardware}
          onDetachFr={detachFrHardware}
          onSetReplay={(kind, raw, target) =>
            void run({ type: "set_replay_channel", kind, raw, target })
          }
          onSetReal={setRealBus}
          onOpenScripts={() => {
            setBusesOpen(false);
            setScriptsOpen(true);
          }}
          onClose={() => setBusesOpen(false)}
        />
      )}

      {scriptsOpen && (
        <ScriptsModal
          buses={state?.buses ?? []}
          onCommand={(command) => void run(command)}
          onClose={() => setScriptsOpen(false)}
        />
      )}

      {sysvarsOpen && (
        <SysVarsModal
          vars={state?.sysvars ?? []}
          onSetValue={(namespace, name, value) =>
            void run({ type: "set_sys_var", namespace, name, value })
          }
          onDefine={defineSysVar}
          onDelete={(namespace, name) =>
            void run({ type: "delete_sys_var", namespace, name })
          }
          onClose={() => setSysvarsOpen(false)}
        />
      )}

      {pickerOpen && (
        <SignalPicker
          catalog={catalog}
          loading={catalogLoading}
          error={catalogError}
          selected={selectedKeys}
          kinds={["can", "fr", "eth"]}
          onToggle={toggleSignal}
          onClose={() => setPickerOpen(false)}
        />
      )}

      {monitorPickerOpen && (
        <SignalPicker
          catalog={catalog}
          loading={catalogLoading}
          error={catalogError}
          selected={monitorKeys}
          title="Monitor signals"
          kinds={["can", "fr", "eth"]}
          onToggle={(channel, message, signal, on) =>
            void run(
              on
                ? {
                    type: "add_monitor",
                    kind: channel.kind,
                    ch: channel.index,
                    id: message.id,
                    ext: message.extended,
                    frame: message.frame,
                    name: signal.name,
                  }
                : {
                    type: "remove_monitor",
                    kind: channel.kind,
                    ch: channel.index,
                    id: message.id,
                    ext: message.extended,
                    frame: message.frame,
                    name: signal.name,
                  },
            )
          }
          onClose={() => setMonitorPickerOpen(false)}
        />
      )}

      {dataPickerOpen && (
        <SignalPicker
          catalog={catalog}
          loading={catalogLoading}
          error={catalogError}
          selected={dataKeys}
          title="Data signals"
          kinds={["can", "fr", "eth"]}
          onToggle={(channel, message, signal, on) =>
            void run(
              on
                ? {
                    type: "add_data",
                    kind: channel.kind,
                    ch: channel.index,
                    id: message.id,
                    ext: message.extended,
                    frame: message.frame,
                    name: signal.name,
                  }
                : {
                    type: "remove_data",
                    kind: channel.kind,
                    ch: channel.index,
                    id: message.id,
                    ext: message.extended,
                    frame: message.frame,
                    name: signal.name,
                  },
            )
          }
          onClose={() => setDataPickerOpen(false)}
        />
      )}

      {statePickerOpen && (
        <SignalPicker
          catalog={catalog}
          loading={catalogLoading}
          error={catalogError}
          selected={stateKeys}
          title="State signals"
          kinds={["can", "fr", "eth"]}
          onToggle={(channel, message, signal, on) =>
            void run(
              on
                ? {
                    type: "add_state",
                    kind: channel.kind,
                    ch: channel.index,
                    id: message.id,
                    ext: message.extended,
                    frame: message.frame,
                    name: signal.name,
                  }
                : {
                    type: "remove_state",
                    kind: channel.kind,
                    ch: channel.index,
                    id: message.id,
                    ext: message.extended,
                    frame: message.frame,
                    name: signal.name,
                  },
            )
          }
          onClose={() => setStatePickerOpen(false)}
        />
      )}

      {exportOpen && (
        <ExportModal
          initialKind={exportKind}
          onExport={(kind, path) => {
            void run({ type: "export", kind, path });
            setExportOpen(false);
          }}
          onClose={() => setExportOpen(false)}
        />
      )}

      {traceFilterOpen && (
        <TraceFilterModal
          filter={traceFilter}
          onApply={setTraceFilter}
          onClose={() => setTraceFilterOpen(false)}
        />
      )}

      {stateRule && (
        <StateRuleModal
          signal={`${stateRule.name} · ${stateRule.bus}`}
          rule={stateRule.rule}
          onApply={(cuts, names) => {
            void run({
              type: "set_state_rule",
              kind: stateRule.kind,
              ch: stateRule.ch,
              id: stateRule.id,
              ext: stateRule.ext,
              frame: stateRule.frame,
              name: stateRule.name,
              cuts,
              names,
            });
            setStateRule(null);
          }}
          onClear={() => {
            void run({
              type: "clear_state_rule",
              kind: stateRule.kind,
              ch: stateRule.ch,
              id: stateRule.id,
              ext: stateRule.ext,
              frame: stateRule.frame,
              name: stateRule.name,
            });
            setStateRule(null);
          }}
          onClose={() => setStateRule(null)}
        />
      )}

      {helpOpen && <HelpModal onClose={() => setHelpOpen(false)} />}

      {sessionOpen && (
        <div class="modal-backdrop" onMouseDown={() => setSessionOpen(false)}>
          <div
            class="session-modal"
            role="dialog"
            aria-modal="true"
            aria-label="Session"
            onMouseDown={(event) => event.stopPropagation()}
          >
            <header class="modal-head">
              <h2>Session</h2>
              <button type="button" onClick={() => setSessionOpen(false)}>
                Close
              </button>
            </header>
            <div class="session-body">
              <section>
                <h3>Project</h3>
                <p class="session-current">
                  {state?.project
                    ? `Current: ${state.project}${state.dirty ? " · unsaved changes" : ""}`
                    : "Untitled workspace"}
                </p>
                <form
                  onSubmit={(event) => {
                    event.preventDefault();
                    loadProject(projectPath);
                  }}
                >
                  <input
                    value={projectPath}
                    placeholder="C:\projects\demo.rxproj"
                    onInput={(event) =>
                      setProjectPath((event.target as HTMLInputElement).value)
                    }
                  />
                  <button
                    type="submit"
                    class="primary"
                    disabled={!projectPath.trim()}
                  >
                    Open project
                  </button>
                  <FilePick
                    label="Choose file…"
                    accept=".rxproj,application/json"
                    onError={setError}
                    onPick={(path) => {
                      setProjectPath(path);
                      loadProject(path);
                    }}
                  />
                </form>
                <div class="save-row">
                  <button
                    type="button"
                    disabled={!state?.projectPath}
                    onClick={() => saveProject(null)}
                  >
                    Save
                  </button>
                  <input
                    value={savePath}
                    placeholder={state?.projectPath || "Save as file name or host path…"}
                    onInput={(event) =>
                      setSavePath((event.target as HTMLInputElement).value)
                    }
                  />
                  <button
                    type="button"
                    disabled={!savePath.trim()}
                    onClick={() => saveProject(savePath.trim())}
                  >
                    Save as
                  </button>
                  <button
                    type="button"
                    disabled={!isDownloadName(savePath) || saveDownloading}
                    title="A bare file name lands in the host's web download folder"
                    onClick={() => void saveAndDownload()}
                  >
                    {saveDownloading ? "Waiting…" : "Save & download"}
                  </button>
                </div>
                {saveDownloadError && (
                  <p class="modal-error">{saveDownloadError}</p>
                )}
                {recent(state?.recentProjects ?? [], loadProject)}
              </section>

              <section>
                <h3>Replay log</h3>
                <p class="session-current">
                  {state?.log
                    ? `Current: ${state.log}${state.logInfo ? ` · ${state.logInfo}` : ""}`
                    : "No log loaded"}
                </p>
                <form
                  onSubmit={(event) => {
                    event.preventDefault();
                    loadLog(logPath);
                  }}
                >
                  <input
                    value={logPath}
                    placeholder="C:\logs\drive.asc or .blf"
                    onInput={(event) =>
                      setLogPath((event.target as HTMLInputElement).value)
                    }
                  />
                  <button
                    type="submit"
                    class="primary"
                    disabled={!logPath.trim()}
                  >
                    Open &amp; play
                  </button>
                  <FilePick
                    label="Choose file…"
                    accept=".asc,.blf"
                    onError={setError}
                    onPick={(path) => {
                      setLogPath(path);
                      loadLog(path);
                    }}
                  />
                </form>
                {recent(state?.recentLogs ?? [], loadLog)}
              </section>

              <section>
                <h3>Recording</h3>
                <p class="session-current">
                  {state?.recording
                    ? state.recordFile
                      ? `Recording: ${fileNameOf(state.recordFile)}`
                      : "Armed — starts with the measurement"
                    : state?.recordFile
                      ? `Last: ${fileNameOf(state.recordFile)}`
                      : "Not recording"}
                </p>
                <div class="record-row">
                  <input
                    value={recordName}
                    placeholder="record"
                    disabled={state?.recording}
                    title="File name; a date suffix is added under the project's Record/ folder"
                    onInput={(event) =>
                      setRecordName((event.target as HTMLInputElement).value)
                    }
                  />
                  <select
                    value={recordFormat}
                    disabled={state?.recording}
                    title="Recording format"
                    onChange={(event) =>
                      setRecordFormat(
                        Number((event.target as HTMLSelectElement).value),
                      )
                    }
                  >
                    <option value={0}>ASC</option>
                    <option value={1}>BLF</option>
                  </select>
                  <button
                    type="button"
                    class={state?.recording ? "danger" : ""}
                    onClick={() =>
                      void run({
                        type: "record_toggle",
                        name: state?.recording
                          ? null
                          : recordName.trim() || "record",
                        format: state?.recording ? null : recordFormat,
                      })
                    }
                  >
                    {state?.recording ? "Stop recording" : "Record"}
                  </button>
                  <button
                    type="button"
                    disabled={state?.recording || !state?.recordFile}
                    title={
                      state?.recording
                        ? "Stop recording before downloading"
                        : "Save the last recording through the browser"
                    }
                    onClick={() => {
                      if (!state?.recordFile) return;
                      triggerDownload(
                        "/api/recording",
                        fileNameOf(state.recordFile),
                      );
                    }}
                  >
                    Download
                  </button>
                </div>
                <p class="hint">
                  Relative names land in Record/ beside the project; an absolute
                  path is used as typed. Recording while stopped arms the file
                  for the next Start.
                </p>
                <label class="record-filter">
                  Only ids / FR slots
                  <input
                    value={recordFilter}
                    placeholder="100, 3F4x, FR0:13"
                    spellcheck={false}
                    onInput={(event) =>
                      setRecordFilter((event.target as HTMLInputElement).value)
                    }
                    onBlur={() =>
                      void run({ type: "set_record_filter", text: recordFilter })
                    }
                    onKeyDown={(event) => {
                      if (event.key === "Enter") {
                        (event.target as HTMLInputElement).blur();
                      }
                    }}
                  />
                </label>
              </section>

              <p class="hint">
                Paths are resolved on the host. Opening a project or log stops
                the current measurement and clears the trace.
              </p>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}
