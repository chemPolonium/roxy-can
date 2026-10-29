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
import {
  describeError,
  fetchCatalog,
  fetchHardware,
  fetchState,
  sendCommand,
} from "./api";
import {
  clearDownload,
  isDownloadName,
  triggerDownload,
  waitForDownload,
} from "./download";
import { Activity } from "./components/Activity";
import { BusesModal } from "./components/BusesModal";
import { BusStatsPanel } from "./components/BusStatsPanel";
import { CursorAnalysis } from "./components/CursorAnalysis";
import { ColumnsMenu } from "./components/ColumnsMenu";
import { CommandPalette } from "./components/CommandPalette";
import type { PaletteCommand } from "./components/CommandPalette";
import { DataPanel } from "./components/DataPanel";
import { DbcExplorer } from "./components/DbcExplorer";
import { EventJumpModal } from "./components/EventJumpModal";
import {
  buildAlignedCsv,
  resolveDerivedSources,
  type DerivedDef,
  type DerivedReference,
} from "./derived";
import { ExportModal } from "./components/ExportModal";
import { FilePick } from "./components/FilePick";
import { GeneratorModal } from "./components/GeneratorModal";
import { HelpModal } from "./components/HelpModal";
import { TriggersModal } from "./components/TriggersModal";
import { MessageTable } from "./components/MessageTable";
import { MonitorPanel } from "./components/MonitorPanel";
import type { MonitorPatch } from "./components/MonitorPanel";
import { PagePanel } from "./components/PagePanel";
import { RateChart } from "./components/RateChart";
import {
  ReferenceModal,
  type ReferenceColumns,
  type ReferenceData,
} from "./components/ReferenceModal";
import {
  SignalChart,
  cursorLabels,
  type ChartCursor,
  type ChartSignalAction,
  type SignalView,
} from "./components/SignalChart";
import { SignalPicker } from "./components/SignalPicker";
import { SpecPanel } from "./components/SpecPanel";
import type { ChartYMode } from "./chartScale";
import { ScriptsModal } from "./components/ScriptsModal";
import { StatePanel } from "./components/StatePanel";
import { StateRuleModal } from "./components/StateRuleModal";
import { StatsTable } from "./components/StatsTable";
import { VariablesModal } from "./components/VariablesModal";
import type { SysVarEdit } from "./components/VariablesModal";
import { TimeJump } from "./components/TimeJump";
import { TraceTable } from "./components/TraceTable";
import type { TraceJump } from "./components/TraceTable";
import { traceQueryPredicate } from "./traceFilter";
import {
  EMPTY_TRACE_FILTER,
  TraceFilterModal,
  parseIds,
  traceFilterActive,
} from "./components/TraceFilterModal";
import type { TraceFilter } from "./components/TraceFilterModal";
import type { StateRow } from "./types";
import {
  catalogSignalCommand,
  catalogSignalSpec,
  findCatalogSignal,
  parseReadSignals,
  readSpecCommand,
  type ReadSignalSpec,
} from "./readSignals";
import {
  applyTheme,
  loadTheme,
  palette,
  saveTheme,
  type Theme,
} from "./theme";
import {
  DEFAULT_TRACE_COLUMNS,
  loadTraceColumns,
  parseTraceColumns,
  saveTraceColumns,
} from "./traceLayout";
import {
  defaultPanelBook,
  panelNumericTarget,
  parsePanelBook,
  parsePanelHex,
  type PanelBook,
  type PanelButton,
  type PanelFrameSend,
  type PanelNumeric,
  type PanelPage,
  type PanelToggle,
} from "./panelPage";
import {
  clampPanelNumber,
  findPanelSysVar,
  panelSourceOptions,
  runningPanelActions,
  type PanelActionContext,
  type PanelFrameDef,
  type PanelView,
  type PanelWriteTarget,
} from "./panelBinding";

type RightTab = "statistics" | "bus" | "spec" | "activity";
type MainView = "trace" | "messages" | "panel";
type ChartTab = "rate" | "signals" | "monitor" | "data" | "state";
type Drag = "main" | "right" | null;

interface Layout {
  main: number;
  right: number;
}

const LAYOUT_KEY = "roxy-can-web.layout";
const VIEW_KEY = "roxy-can-web.view";
const TRACE_CAP = 50_000;
const SIGNAL_CAP = 12_000;
const DEFAULT_LAYOUT: Layout = { main: 0.63, right: 0.56 };

interface SavedView {
  tab: RightTab;
  mainView: MainView;
  chartTab: ChartTab;
  chartTab2: ChartTab;
  chartSplit: boolean;
  splitRatio: number;
  overlay: boolean;
  signalView: SignalView;
  yModes?: Record<string, ChartYMode>;
  cursors: ChartCursor[];
  panelPage?: PanelPage;
  panelBook?: PanelBook;
}

interface SavedWorkspace extends Partial<SavedView> {
  traceColumns?: unknown;
  layout?: unknown;
  readSignals?: unknown;
}


function normalizeView(saved: Partial<SavedView>): Partial<SavedView> {
  const tabs: RightTab[] = ["statistics", "bus", "spec", "activity"];
  const chartTabs: ChartTab[] = [
    "rate",
    "signals",
    "monitor",
    "data",
    "state",
  ];
  const windowS = clamp(Number(saved.signalView?.windowS) || 30, 1, 120);
  const rawEnd = saved.signalView?.endS;
  const endS =
    typeof rawEnd === "number" && Number.isFinite(rawEnd) ? rawEnd : null;
  const cursors: ChartCursor[] = [];
  if (Array.isArray(saved.cursors)) {
    for (const cursor of saved.cursors.slice(0, 16)) {
      if (!Number.isFinite(cursor?.id) || !Number.isFinite(cursor?.t)) continue;
      const label =
        cursor.label === "A" || cursor.label === "B" ? cursor.label : undefined;
      cursors.push({
        id: cursor.id,
        t: cursor.t,
        ...(label ? { label } : {}),
      });
    }
  }
  const yModes: Record<string, ChartYMode> = {};
  if (saved.yModes && typeof saved.yModes === "object") {
    for (const [key, mode] of Object.entries(saved.yModes).slice(0, 256)) {
      if (mode === "auto" || mode === "memory" || mode === "dbc") {
        yModes[key] = mode;
      }
    }
  }
  return {
    tab: saved.tab && tabs.includes(saved.tab) ? saved.tab : undefined,
    mainView:
      saved.mainView === "trace" ||
      saved.mainView === "messages" ||
      saved.mainView === "panel"
        ? saved.mainView
        : undefined,
    chartTab:
      saved.chartTab && chartTabs.includes(saved.chartTab)
        ? saved.chartTab
        : undefined,
    chartTab2:
      saved.chartTab2 && chartTabs.includes(saved.chartTab2)
        ? saved.chartTab2
        : undefined,
    chartSplit: typeof saved.chartSplit === "boolean" ? saved.chartSplit : false,
    splitRatio: clamp(Number(saved.splitRatio) || 0.6, 0.25, 0.75),
    overlay: typeof saved.overlay === "boolean" ? saved.overlay : false,
    yModes,
    signalView: { windowS, endS },
    cursors,
    panelBook:
      parsePanelBook(saved.panelBook, saved.panelPage) ?? defaultPanelBook(),
  };
}

function loadView(): Partial<SavedView> {
  try {
    const raw = localStorage.getItem(VIEW_KEY);
    if (!raw) return {};
    return normalizeView(JSON.parse(raw));
  } catch {
    return {};
  }
}

interface ReferenceRun extends ReferenceData {
  series: Record<string, [number, number][]>;
}

/// Aligned-CSV headers are `bus:signal`; match them to the live lanes, with
/// a unique-name fallback when the bus labels differ.
function mapReferenceColumns(
  columns: ReferenceColumns,
  series: SignalSeries[],
): Record<string, [number, number][]> {
  const byKey: Record<string, [number, number][]> = {};
  for (const signal of series) {
    let points = columns[`${signal.bus}:${signal.name}`];
    if (!points) {
      const matches = Object.keys(columns).filter(
        (label) => label.slice(label.lastIndexOf(":") + 1) === signal.name,
      );
      if (matches.length === 1) points = columns[matches[0]];
    }
    if (points && points.length > 0) byKey[signal.key] = points;
  }
  return byKey;
}

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
  if (row.kind === "derived") return `derived:${row.frame}`;
  return `${row.ch}:${row.id}:${row.name}`;
}

function clamp(value: number, min: number, max: number): number {
  return Math.min(max, Math.max(min, value));
}

function parseLayout(value: unknown): Layout | null {
  if (!value || typeof value !== "object") return null;
  const saved = value as Partial<Layout>;
  return {
    main: clamp(Number(saved.main) || DEFAULT_LAYOUT.main, 0.4, 0.78),
    right: clamp(Number(saved.right) || DEFAULT_LAYOUT.right, 0.34, 0.8),
  };
}

function loadLayout(): Layout {
  try {
    const raw = localStorage.getItem(LAYOUT_KEY);
    if (!raw) return DEFAULT_LAYOUT;
    return parseLayout(JSON.parse(raw)) ?? DEFAULT_LAYOUT;
  } catch {
    return DEFAULT_LAYOUT;
  }
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
  const [readSignals, setReadSignals] = useState<ReadSignalSpec[]>([]);
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
  const [messageQuery, setMessageQuery] = useState("");
  const [writeKind, setWriteKind] = useState("all");
  const [writeQuery, setWriteQuery] = useState("");
  const [savedView] = useState(loadView);
  const [tab, setTab] = useState<RightTab>(savedView.tab ?? "statistics");
  const [mainView, setMainView] = useState<MainView>(
    savedView.mainView ?? "messages",
  );
  const [panelBook, setPanelBook] = useState<PanelBook>(
    savedView.panelBook ?? defaultPanelBook(),
  );
  const standardView = useRef<"trace" | "messages">(
    savedView.mainView === "trace" ? "trace" : "messages",
  );
  const [chartTab, setChartTab] = useState<ChartTab>(
    savedView.chartTab ?? "rate",
  );
  const [chartTab2, setChartTab2] = useState<ChartTab>(
    savedView.chartTab2 ?? "monitor",
  );
  const [chartSplit, setChartSplit] = useState(savedView.chartSplit ?? false);
  const [splitRatio, setSplitRatio] = useState(savedView.splitRatio ?? 0.6);
  const [signalOverlay, setSignalOverlay] = useState(
    savedView.overlay ?? false,
  );
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
  const [signalView, setSignalView] = useState<SignalView>(
    savedView.signalView ?? { windowS: 30, endS: null },
  );
  const [chartCursors, setChartCursors] = useState<ChartCursor[]>(
    savedView.cursors ?? [],
  );
  const [chartYModes, setChartYModes] = useState<Record<string, ChartYMode>>(
    savedView.yModes ?? {},
  );
  const [chartFullscreen, setChartFullscreen] = useState(false);
  const [toast, setToast] = useState("");
  const [paletteOpen, setPaletteOpen] = useState(false);
  const [timeJumpOpen, setTimeJumpOpen] = useState(false);
  const [traceJump, setTraceJump] = useState<TraceJump | null>(null);
  const [traceColumns, setTraceColumns] = useState(loadTraceColumns);
  const [explorerOpen, setExplorerOpen] = useState(false);
  const [reference, setReference] = useState<ReferenceRun | null>(null);
  const [referenceOpen, setReferenceOpen] = useState(false);
  const [derivedDefs, setDerivedDefs] = useState<DerivedDef[]>([]);
  const [variablesTab, setVariablesTab] = useState<"values" | "computed">(
    "values",
  );
  const [derivedRule, setDerivedRule] = useState<DerivedDef | null>(null);
  const [eventOpen, setEventOpen] = useState(false);
  const [workspaceHydrated, setWorkspaceHydrated] = useState(false);
  const appliedWorkspaceRev = useRef(-1);
  const appliedReferenceRev = useRef(-1);
  const uiFps = useUiFps();
  const workspaceRef = useRef<HTMLDivElement>(null);
  const rightRef = useRef<HTMLDivElement>(null);
  const chartSplitRef = useRef<HTMLDivElement>(null);
  const preSplitRight = useRef<number | null>(null);
  const chartCursorSeq = useRef(
    Math.max(0, ...(savedView.cursors ?? []).map((cursor) => cursor.id)) + 1,
  );
  const traceJumpSeq = useRef(0);
  const toastTimer = useRef<number | null>(null);
  const lastStatus = useRef("");
  const statusReady = useRef(false);
  const lastPhase = useRef("");
  const lastOnline = useRef<boolean | null>(null);
  const connectedOnce = useRef(false);
  const chartPalette = palette(theme);

  const showToast = (message: string) => {
    if (!message) return;
    if (toastTimer.current !== null) window.clearTimeout(toastTimer.current);
    setToast(message);
    toastTimer.current = window.setTimeout(() => {
      setToast("");
      toastTimer.current = null;
    }, 2600);
  };

  const applyWorkspace = (raw: unknown) => {
    if (!raw || typeof raw !== "object") {
      setTab("statistics");
      setMainView("messages");
      setChartTab("rate");
      setChartTab2("monitor");
      setChartSplit(false);
      setSplitRatio(0.6);
      setSignalOverlay(false);
      setSignalView({ windowS: 30, endS: null });
      setChartCursors([]);
      setReadSignals([]);
      setTraceColumns(DEFAULT_TRACE_COLUMNS);
      setLayout(DEFAULT_LAYOUT);
      setPanelBook(defaultPanelBook());
      chartCursorSeq.current = 1;
      return;
    }
    const saved = raw as SavedWorkspace;
    const view = normalizeView(saved);
    if (view.tab) setTab(view.tab);
    if (view.mainView) setMainView(view.mainView);
    if (view.chartTab) setChartTab(view.chartTab);
    if (view.chartTab2) setChartTab2(view.chartTab2);
    if (typeof view.chartSplit === "boolean") setChartSplit(view.chartSplit);
    if (typeof view.splitRatio === "number") setSplitRatio(view.splitRatio);
    if (typeof view.overlay === "boolean") setSignalOverlay(view.overlay);
    if (view.signalView) setSignalView(view.signalView);
    if (view.cursors) {
      setChartCursors(view.cursors);
      chartCursorSeq.current =
        Math.max(0, ...view.cursors.map((cursor) => cursor.id)) + 1;
    }
    const columns = parseTraceColumns(saved.traceColumns);
    if (columns) setTraceColumns(columns);
    const nextLayout = parseLayout(saved.layout);
    if (nextLayout) setLayout(nextLayout);
    const savedReads = parseReadSignals(saved.readSignals);
    setReadSignals(savedReads);
    setPanelBook(view.panelBook ?? defaultPanelBook());
    for (const spec of savedReads) void run(readSpecCommand(spec));
  };

  // A project load bumps the revision once; adopt its workspace then, but
  // leave the local view alone on the first (untitled) snapshot.
  useEffect(() => {
    if (!state) return;
    const rev = state.workspaceRev ?? 0;
    if (rev === appliedWorkspaceRev.current) {
      setWorkspaceHydrated(true);
      return;
    }
    appliedWorkspaceRev.current = rev;
    if (rev > 0) applyWorkspace(state.workspace);
    setWorkspaceHydrated(true);
  }, [state?.workspaceRev]);

  // The core owns the derived list; mirror it for the editor.
  const derivedSignature = useMemo(
    () => JSON.stringify(state?.derived ?? []),
    [state?.derived],
  );
  useEffect(() => {
    if (!state) return;
    setDerivedDefs(state.derived ?? []);
  }, [derivedSignature, state?.workspaceRev]);

  // Everything a computed expression may reference: the whole catalog plus
  // the computed channels already defined. This is independent of what the
  // Signals tab happens to plot.
  const derivedReferences = useMemo<DerivedReference[]>(() => {
    const refs: DerivedReference[] = [];
    for (const channel of catalog?.channels ?? []) {
      for (const message of channel.messages) {
        for (const signal of message.signals) {
          refs.push({
            key: signal.key,
            name: signal.name,
            bus: channel.name,
            frame:
              channel.kind === "can"
                ? message.name
                : channel.kind === "fr"
                  ? `slot${message.id}`
                  : (message.frame ?? message.name),
            frameId:
              channel.kind === "can"
                ? `0x${message.id
                    .toString(16)
                    .toUpperCase()
                    .padStart(message.extended ? 8 : 3, "0")}`
                : undefined,
          });
        }
      }
    }
    for (const definition of derivedDefs) {
      refs.push({
        key: `derived:${definition.id}`,
        name: definition.name,
        bus: "derived",
      });
    }
    return refs;
  }, [catalog, derivedDefs]);

  // Keep the backend copy warm so a save carries the current tabs, cursors,
  // splitters and Trace columns.
  useEffect(() => {
    if (!workspaceHydrated || !state) return;
    const timer = window.setTimeout(() => {
      void sendCommand({
        type: "set_web_workspace",
        rev: state.workspaceRev ?? 0,
        workspace: {
          tab,
          mainView,
          chartTab,
          chartTab2,
          chartSplit,
          splitRatio,
          overlay: signalOverlay,
          signalView,
          cursors: chartCursors,
          traceColumns,
          layout,
          readSignals,
          panelBook,
        },
      }).catch(() => {});
    }, 350);
    return () => window.clearTimeout(timer);
  }, [
    workspaceHydrated,
    tab,
    mainView,
    chartTab,
    chartTab2,
    chartSplit,
    splitRatio,
    signalOverlay,
    signalView,
    chartCursors,
    traceColumns,
    layout,
    readSignals,
    panelBook,
    state?.workspaceRev,
  ]);

  // A reference decoded by the core replaces the CSV reference; clearing it
  // in the core (or loading a new log) bumps the revision.
  useEffect(() => {
    if (!state) return;
    const rev = state.referenceRev ?? 0;
    if (rev === appliedReferenceRev.current) return;
    appliedReferenceRev.current = rev;
    if (rev <= 0) return;
    if (!state.reference) {
      setReference(null);
      return;
    }
    const columns: ReferenceColumns = {};
    for (const signal of state.reference.signals) {
      columns[`${signal.bus}:${signal.name}`] = signal.points;
    }
    setReference({
      name: state.reference.name,
      offsetS: 0,
      columns,
      series: mapReferenceColumns(columns, signals),
    });
    showToast(`Reference ${state.reference.name} loaded`);
  }, [state?.referenceRev]);

  useEffect(() => {
    applyTheme(theme);
    saveTheme(theme);
  }, [theme]);

  // Transport changes deserve a message even though the status line keeps
  // showing the bus state rather than the command result.
  useEffect(() => {
    if (!state) return;
    const phase = !state.measuring
      ? "Stopped"
      : state.tracePaused
        ? "Paused"
        : "Running";
    const previous = lastPhase.current;
    lastPhase.current = phase;
    if (!previous || previous === phase) return;
    if (phase !== "Running") showToast(phase);
    else showToast(previous === "Paused" ? "Resumed" : "Started");
  }, [state?.measuring, state?.tracePaused]);

  useEffect(() => {
    const status = state?.status?.trim() ?? "";
    if (!status || status === lastStatus.current) return;
    lastStatus.current = status;
    // The first snapshot only restores state; it is not a new command result.
    if (!statusReady.current) {
      statusReady.current = true;
      return;
    }
    // The transport effect above already announces these words.
    if (/^(stopped|paused|running)$/i.test(status)) return;
    showToast(status);
  }, [state?.status]);

  useEffect(() => {
    const previous = lastOnline.current;
    lastOnline.current = online;
    if (previous === null) return;
    // The boot poll turns the link on for the first time; that is not a
    // recovery, so only later transitions are worth announcing.
    if (!connectedOnce.current) {
      connectedOnce.current = online;
      return;
    }
    if (previous === online) return;
    showToast(
      online ? "Connection restored" : "Connection to the bus core lost — retrying…",
    );
  }, [online]);

  useEffect(
    () => () => {
      if (toastTimer.current !== null) window.clearTimeout(toastTimer.current);
    },
    [],
  );

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
    if (!chartFullscreen) return;
    const escape = (event: KeyboardEvent) => {
      if (event.key !== "Escape") return;
      // A menu opened inside fullscreen owns the first Escape.
      if (document.querySelector(".signal-menu, .trace-menu, .modal-backdrop")) {
        return;
      }
      setChartFullscreen(false);
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
    saveTraceColumns(traceColumns);
  }, [traceColumns]);

  useEffect(() => {
    try {
      localStorage.setItem(
        VIEW_KEY,
        JSON.stringify({
          tab,
          mainView,
          chartTab,
          chartTab2,
          chartSplit,
          splitRatio,
          overlay: signalOverlay,
          signalView,
          yModes: chartYModes,
          cursors: chartCursors,
          panelBook,
        }),
      );
    } catch {
      // Same fallback as the splitter layout.
    }
  }, [
    tab,
    mainView,
    chartTab,
    chartTab2,
    chartSplit,
    splitRatio,
    signalOverlay,
    signalView,
    chartYModes,
    chartCursors,
    panelBook,
  ]);

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
        setError(describeError(e));
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
      const message = describeError(e);
      setError(message);
      showToast(message);
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

  const saveDerived = (defs: DerivedDef[]) => {
    // Pin the editor's resolution; the core never guesses between same-named
    // signals on different buses.
    const pinned = defs.map((definition) => ({
      ...definition,
      sources: resolveDerivedSources(definition.expression, derivedReferences),
    }));
    void run({ type: "set_derived", defs: pinned });
  };

  const removeDerived = (key?: string) => {
    if (!key) return;
    const id = key.startsWith("derived:") ? key.slice("derived:".length) : key;
    saveDerived(derivedDefs.filter((definition) => definition.id !== id));
  };

  const stateRowRule = (row: StateRow) => {
    if (row.kind === "derived") {
      const def = derivedDefs.find((definition) => definition.id === row.frame);
      if (def) setDerivedRule(def);
      return;
    }
    setStateRule(row);
  };

  const stateRowRemove = (row: StateRow) => {
    if (row.kind === "derived") {
      removeDerived(row.frame);
      return;
    }
    void run({
      type: "remove_state",
      kind: row.kind,
      ch: row.ch,
      id: row.id,
      ext: row.ext,
      frame: row.frame,
      name: row.name,
    });
  };

  const setSignalYMode = (key: string, mode: ChartYMode) => {
    setChartYModes((current) => ({ ...current, [key]: mode }));
  };

  const chartSignalAction = (key: string, action: ChartSignalAction) => {
    if (key.startsWith("derived:")) {
      if (action === "remove") {
        removeDerived(key);
      } else {
        void run({ type: "chart_signal", key, action });
        setChartTab(action);
      }
      return;
    }
    void run({ type: "chart_signal", key, action });
    if (action !== "remove") {
      setChartTab(action);
    }
  };

  const copyChartSelection = async (
    target: "monitor" | "data" | "state",
  ) => {
    setPickerOpen(false);
    const keys = chartSeries.map((signal) => signal.key);
    if (keys.length === 0) {
      showToast("No plotted signals to copy");
      return;
    }
    setBusy(true);
    try {
      await Promise.all(
        keys.map((key) =>
          sendCommand({ type: "chart_signal", key, action: target }),
        ),
      );
      setError("");
      setChartTab(target);
      showToast(`Copied ${keys.length} signal(s) to ${target}`);
    } catch (e) {
      const message = describeError(e);
      setError(message);
      showToast(message);
    } finally {
      setBusy(false);
    }
  };

  const catalogMessageFor = (
    frame: Frame,
    source: Catalog | null = catalog,
  ): CatalogMessage | null => {
    const channel = source?.channels.find(
      (entry) => entry.kind === frame.kind && entry.index === frame.bus,
    );
    return (
      channel?.messages.find(
        (message) =>
          message.id === frame.id && message.extended === frame.extended,
      ) ?? null
    );
  };

  const signalCommand = (
    frame: Frame,
    message: CatalogMessage,
    name: string,
    target: "chart" | "monitor",
  ): Command => {
    if (target === "monitor") {
      return {
        type: "add_monitor",
        kind: frame.kind,
        ch: frame.bus,
        id: frame.id,
        ext: frame.extended,
        frame: message.frame,
        name,
      };
    }
    if (frame.kind === "fr") {
      return {
        type: "select_fr_signal",
        bus: frame.bus,
        slot: frame.id,
        name,
      };
    }
    if (frame.kind === "eth") {
      return {
        type: "select_eth_signal",
        bus: frame.bus,
        frame: message.frame ?? frame.name,
        name,
      };
    }
    return {
      type: "select_signal",
      ch: frame.bus,
      id: frame.id,
      ext: frame.extended,
      name,
    };
  };

  /// The Trace right-click path: decode the whole message instead of making
  /// the user reopen the catalog and hunt for each signal.
  const addFrameSignals = async (
    frame: Frame,
    target: "chart" | "monitor",
  ) => {
    setBusy(true);
    try {
      // The Trace menu can be the first thing a session touches; the picker
      // is not the only entry point for the catalog any more.
      let source = catalog;
      if (!source) {
        source = await fetchCatalog();
        setCatalog(source);
        setCatalogError("");
      }
      const message = catalogMessageFor(frame, source);
      if (!message || message.signals.length === 0) {
        showToast("No decoded signals in this frame");
        return;
      }
      const existing = new Set(
        target === "chart"
          ? chartSeries.map((signal) => signal.key)
          : (state?.monitor ?? []).map(rowKey),
      );
      const pending = message.signals.filter(
        (signal) => !existing.has(signal.key),
      );
      if (pending.length === 0) {
        showToast(
          target === "chart"
            ? "This message is already on the chart"
            : "This message is already watched",
        );
        return;
      }
      await Promise.all(
        pending.map((signal) =>
          sendCommand(signalCommand(frame, message, signal.name, target)),
        ),
      );
      setError("");
      setChartTab(target === "chart" ? "signals" : "monitor");
      showToast(
        `${pending.length} signal(s) added to ${target === "chart" ? "the chart" : "Monitor"}`,
      );
    } catch (e) {
      const text = describeError(e);
      setError(text);
      showToast(text);
    } finally {
      setBusy(false);
    }
  };

  const viewTimeUs = (tUs: number): number => {
    const latest = traceRows[0]?.tUs ?? 0;
    const target = latest > 0 ? Math.min(tUs, latest) : tUs;
    traceJumpSeq.current += 1;
    setTraceJump({ tUs: target, nonce: traceJumpSeq.current });
    setSignalView((old) => ({ ...old, endS: target / 1e6 + old.windowS / 2 }));
    setChartTab(cursorSeries.length > 0 ? "signals" : "rate");
    return target;
  };

  const goToTimeUs = (tUs: number, explainBehind = true) => {
    setTimeJumpOpen(false);
    const seconds = tUs / 1e6;
    const replaying =
      state?.runMode === "Replay" &&
      Boolean(state.log) &&
      state.measuring &&
      state.replayPosUs != null;
    // Replay is forward-only. A target ahead of the playhead becomes a play
    // target instead of a view jump, so the chart keeps following real data.
    if (replaying && state?.replayPosUs != null && tUs > state.replayPosUs) {
      void run({ type: "replay_play_to", target_us: tUs });
      showToast(`Replaying to ${seconds.toFixed(3)} s`);
      return;
    }
    const latest = traceRows[0]?.tUs ?? 0;
    if (latest > 0 && tUs > latest) {
      showToast(`Newest frame is at ${(latest / 1e6).toFixed(3)} s`);
    }
    const target = viewTimeUs(tUs);
    if (replaying && explainBehind) {
      showToast(`Replay only moves forward · view at ${(target / 1e6).toFixed(3)} s`);
    }
  };

  // Trigger jumps walk the *view*, not the playhead: after jumping to a
  // marker the next pair is relative to where the user is looking.
  const markerReferenceUs =
    signalView.endS !== null
      ? Math.round((signalView.endS - signalView.windowS / 2) * 1e6)
      : (state?.replayPosUs ?? traceRows[0]?.tUs ?? 0);

  const prevTriggerUs = useMemo(() => {
    let best: number | null = null;
    for (const marker of state?.markers ?? []) {
      const t = Math.round(marker * 1e6);
      if (t < markerReferenceUs - 1) best = t;
    }
    return best;
  }, [markerReferenceUs, state?.markers]);

  const nextTriggerUs = useMemo(() => {
    for (const marker of state?.markers ?? []) {
      const t = Math.round(marker * 1e6);
      if (t > markerReferenceUs + 1) return t;
    }
    return null;
  }, [markerReferenceUs, state?.markers]);

  const resetLayout = () => {
    setLayout(DEFAULT_LAYOUT);
    setTraceColumns(DEFAULT_TRACE_COLUMNS);
    setSignalView({ windowS: 30, endS: null });
    if (chartFullscreen) toggleChartFullscreen();
    showToast("Layout reset");
  };

  const applyLayoutPreset = (preset: "trace" | "analysis" | "replay") => {
    if (preset === "trace") {
      setLayout({ main: 0.78, right: 0.5 });
      setChartTab("rate");
      setMainView("trace");
      setTab("statistics");
      showToast("Layout: Trace focus");
    } else if (preset === "analysis") {
      setLayout({ main: 0.52, right: 0.62 });
      setChartTab("signals");
      setMainView("trace");
      setTab("statistics");
      showToast("Layout: Analysis");
    } else {
      setLayout({ main: 0.64, right: 0.55 });
      setChartTab("rate");
      setMainView("trace");
      setTab("statistics");
      showToast("Layout: Replay");
    }
  };

  const setChartCursorAt = (slot: "A" | "B", tUs: number) => {
    const t = tUs / 1e6;
    const id = chartCursorSeq.current++;
    setChartCursors((old) => {
      const existing = old.find((cursor) => cursor.label === slot);
      if (existing) {
        return old.map((cursor) =>
          cursor.id === existing.id ? { ...cursor, t } : cursor,
        );
      }
      const labels = cursorLabels(old);
      const index = labels.indexOf(slot);
      if (index >= 0 && !old[index].label) {
        return old.map((cursor, i) =>
          i === index ? { ...cursor, t, label: slot } : cursor,
        );
      }
      return [...old, { id, t, label: slot }];
    });
    setSignalView((old) => ({ ...old, endS: t + old.windowS / 2 }));
    setChartTab("signals");
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
    setChartFullscreen((old) => !old);
  };

  const toggleChartSplit = () => {
    if (chartSplit) {
      setChartSplit(false);
      if (preSplitRight.current !== null) {
        const right = preSplitRight.current;
        preSplitRight.current = null;
        setLayout((old) => ({ ...old, right }));
      }
      return;
    }
    preSplitRight.current = layout.right;
    // Two panes need vertical room; give the chart column more of the right
    // side while split, keeping the lower panel usable.
    setLayout((old) => ({ ...old, right: Math.max(old.right, 0.72) }));
    if (chartTab2 === chartTab) {
      setChartTab2(chartTab === "signals" ? "monitor" : "signals");
    }
    setChartSplit(true);
  };

  const startSplitDrag = (event: PointerEvent) => {
    event.preventDefault();
    const area = chartSplitRef.current;
    if (!area) return;
    const rect = area.getBoundingClientRect();
    const move = (moveEvent: PointerEvent) => {
      setSplitRatio(
        clamp((moveEvent.clientY - rect.top) / rect.height, 0.25, 0.75),
      );
    };
    const up = () => {
      document.body.classList.remove("row-resizing");
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", up);
    };
    document.body.classList.add("row-resizing");
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", up);
  };

  const resetChartTools = () => {
    setChartCursors([]);
    setSignalView({ windowS: 30, endS: null });
    setReference(null);
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

  const pushWorkspace = () =>
    sendCommand({
      type: "set_web_workspace",
      rev: state?.workspaceRev ?? 0,
      workspace: {
        tab,
        mainView,
        chartTab,
        chartTab2,
        chartSplit,
        splitRatio,
        overlay: signalOverlay,
        signalView,
        cursors: chartCursors,
        traceColumns,
        layout,
        readSignals,
      },
    });

  const saveProject = async (path: string | null) => {
    // Flush the current view before the file is written; the debounced sync
    // may still be waiting and would otherwise save the previous workspace.
    await pushWorkspace().catch(() => {});
    await run({ type: "save_project", path });
  };

  const saveAndDownload = async () => {
    const name = savePath.trim();
    if (!isDownloadName(name)) return;
    setSaveDownloading(true);
    setSaveDownloadError("");
    await clearDownload(name);
    await saveProject(name);
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
      setCatalogError(describeError(e));
    } finally {
      setCatalogLoading(false);
    }
  };

  // The script editor's `Message.Signal` completion needs the DBC tree even
  // when the signal picker was never opened.
  useEffect(() => {
    if (scriptsOpen && !catalog && !catalogLoading) void loadCatalog();
  }, [scriptsOpen, catalog, catalogLoading]);

  // Computed expressions reference the whole catalog, not just plotted lanes.
  useEffect(() => {
    if (sysvarsOpen && !catalog && !catalogLoading) {
      void loadCatalog();
    }
  }, [sysvarsOpen, catalog, catalogLoading]);

  // Panel setpoints pick from the catalog's writable signals.
  useEffect(() => {
    if (mainView === "panel" && !catalog && !catalogLoading) void loadCatalog();
  }, [mainView, catalog, catalogLoading]);

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

  const openVariables = (tab: "values" | "computed" = "values") => {
    setSessionOpen(false);
    setScriptsOpen(false);
    setBusesOpen(false);
    setPickerOpen(false);
    setMonitorPickerOpen(false);
    setDataPickerOpen(false);
    setStatePickerOpen(false);
    setGeneratorOpen(false);
    setTriggersOpen(false);
    setVariablesTab(tab);
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
      if (event.altKey && !mod && /^[1-5]$/.test(event.key)) {
        if (mainView === "panel") return;
        event.preventDefault();
        const tabs: ChartTab[] = [
          "rate",
          "signals",
          "monitor",
          "data",
          "state",
        ];
        setChartTab(tabs[Number(event.key) - 1]);
        return;
      }
      if (event.key === "F9") {
        event.preventDefault();
        void run({ type: state?.measuring ? "stop" : "start" });
      } else if (mod && event.shiftKey && key === "s") {
        event.preventDefault();
        setSessionOpen(true);
      } else if (mod && key === "s") {
        event.preventDefault();
        if (state?.projectPath) void saveProject(null);
        else setSessionOpen(true);
      } else if (mod && key === "o") {
        event.preventDefault();
        setSessionOpen(true);
      } else if (mod && key === "e") {
        event.preventDefault();
        if (mainView !== "panel") {
          openExport(mainView === "messages" ? "messages" : "trace");
        }
      } else if (mod && key === "k") {
        event.preventDefault();
        setPaletteOpen(true);
      } else if (mod && key === "g") {
        event.preventDefault();
        setTimeJumpOpen(true);
      } else if (event.key === "?") {
        setHelpOpen(true);
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [state?.measuring, state?.projectPath, mainView]);

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

  const closeDialogs = () => {
    setSessionOpen(false);
    setScriptsOpen(false);
    setBusesOpen(false);
    setPickerOpen(false);
    setMonitorPickerOpen(false);
    setDataPickerOpen(false);
    setStatePickerOpen(false);
    setGeneratorOpen(false);
    setTriggersOpen(false);
    setSysvarsOpen(false);
    setExportOpen(false);
    setHelpOpen(false);
    setTraceFilterOpen(false);
    setStateRule(null);
  };

  const openBuses = () => {
    closeDialogs();
    setBusesOpen(true);
  };

  const openSession = () => {
    closeDialogs();
    setSessionOpen(true);
  };

  const openHelp = () => {
    closeDialogs();
    setHelpOpen(true);
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
    if (row.kind === "derived") {
      showToast("Derived rows are managed in the Derived dialog");
      return;
    }
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
    if (!selected) {
      void run(catalogSignalCommand(channel, message, signal, null));
      return;
    }
    if (readKeys.has(signal.key)) {
      setReadSignals((old) => old.filter((spec) => spec.key !== signal.key));
    }
    void run(catalogSignalCommand(channel, message, signal, false));
  };

  const toggleReadSignal = (
    channel: CatalogChannel,
    message: CatalogMessage,
    signal: CatalogSignal,
    on: boolean,
  ) => {
    if (on) {
      const spec = catalogSignalSpec(channel, message, signal);
      setReadSignals((old) => [
        ...old.filter((candidate) => candidate.key !== spec.key),
        spec,
      ]);
      void run(catalogSignalCommand(channel, message, signal, true));
      return;
    }
    setReadSignals((old) => old.filter((spec) => spec.key !== signal.key));
    void run(catalogSignalCommand(channel, message, signal, null));
  };

  const moveSignal = (key: string, to: number) => {
    const from = chartSeries.findIndex((signal) => signal.key === key);
    const target = clamp(to, 0, chartSeries.length - 1);
    if (from < 0 || from === target) return;
    const nextChart = [...chartSeries];
    const [moved] = nextChart.splice(from, 1);
    nextChart.splice(target, 0, moved);
    const next = [...nextChart, ...readSeries];
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
  const setEntryBrs = (entry: TxEntry, brs: boolean) =>
    void run({ type: "set_entry_brs", ch: entry.channel, id: entry.id, brs });
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
      derived_id: edit.derivedId,
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

  const mainMessages = useMemo(() => {
    const rows = state?.messages ?? [];
    const needle = messageQuery.trim().toLowerCase();
    if (!needle) return rows;
    return rows.filter(
      (row) =>
        row.name.toLowerCase().includes(needle) ||
        row.addr.toLowerCase().includes(needle) ||
        row.busName.toLowerCase().includes(needle),
    );
  }, [state?.messages, messageQuery]);

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

  const tracePredicate = useMemo(() => traceQueryPredicate(query), [query]);
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
      return tracePredicate(frame);
    });
  }, [traceRows, tracePredicate, bus, traceFilter]);
  const traceFilterKey = useMemo(
    () => JSON.stringify([query, bus, traceFilter]),
    [query, bus, traceFilter],
  );

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
  const chartSeries = useMemo(
    () => signals.filter((signal) => !signal.read),
    [signals],
  );
  const signalRanges = useMemo(() => {
    const ranges: Record<string, { min: number; max: number }> = {};
    for (const channel of catalog?.channels ?? []) {
      for (const message of channel.messages) {
        for (const signal of message.signals) {
          if (signal.max > signal.min) {
            ranges[signal.key] = { min: signal.min, max: signal.max };
          }
        }
      }
    }
    return ranges;
  }, [catalog]);
  const readSeries = useMemo(
    () => signals.filter((signal) => signal.read),
    [signals],
  );
  const cursorSeries = useMemo(
    () => [...chartSeries, ...readSeries],
    [chartSeries, readSeries],
  );
  const readRange = useMemo(() => {
    let first = Number.POSITIVE_INFINITY;
    let last = 0;
    for (const signal of readSeries) {
      const start = signal.points[0]?.[0];
      const end = signal.points[signal.points.length - 1]?.[0];
      if (typeof start === "number") first = Math.min(first, start);
      if (typeof end === "number") last = Math.max(last, end);
    }
    return Number.isFinite(first) && last > first ? { first, last } : null;
  }, [readSeries]);
  const selectedKeys = new Set(chartSeries.map((signal) => signal.key));
  const readKeys = new Set(readSeries.map((signal) => signal.key));
  const monitorKeys = new Set((state?.monitor ?? []).map(rowKey));
  const exportRange = useMemo(() => {
    const labels = cursorLabels(chartCursors);
    const a = chartCursors[labels.indexOf("A")];
    const b = chartCursors[labels.indexOf("B")];
    if (!a || !b) return null;
    const fromUs = Math.round(Math.min(a.t, b.t) * 1e6);
    const toUs = Math.round(Math.max(a.t, b.t) * 1e6);
    return fromUs >= 0 && toUs > 0 && fromUs < toUs
      ? { fromUs, toUs }
      : null;
  }, [chartCursors]);
  const dataKeys = new Set((state?.data ?? []).map(rowKey));
  const stateKeys = new Set((state?.stateRows ?? []).map(rowKey));
  // Derived channels are ordinary core subscriptions now, so the chart
  // series and every observer row include them without synthesis here.
  const chartLatestS = useMemo(
    () =>
      signals.reduce((latest, signal) => {
        const point = signal.points[signal.points.length - 1];
        return point ? Math.max(latest, point[0]) : latest;
      }, 0),
    [signals],
  );
  const monitorRows = state?.monitor ?? [];
  const dataRows = state?.data ?? [];
  const stateRows = state?.stateRows ?? [];
  const chartCenterS =
    signalView.endS !== null
      ? signalView.endS - signalView.windowS / 2
      : chartLatestS;

  const downloadChartCsv = (fromS: number, toS: number) => {
    const csv = buildAlignedCsv(cursorSeries, fromS, toS);
    const url = URL.createObjectURL(new Blob([csv], { type: "text/csv" }));
    triggerDownload(
      url,
      `signals-${fromS.toFixed(3)}-${toS.toFixed(3)}.csv`,
    );
    window.setTimeout(() => URL.revokeObjectURL(url), 1000);
    showToast(`CSV downloaded · ${chartSeries.length} signals`);
  };

  const openExplorer = () => {
    setExplorerOpen((open) => !open);
    if (!catalog) void loadCatalog();
  };

  const addExplorerChart = (
    channel: CatalogChannel,
    message: CatalogMessage,
    signal: CatalogSignal,
  ) => {
    if (!selectedKeys.has(signal.key)) {
      toggleSignal(channel, message, signal, true);
    }
    setChartTab("signals");
  };

  const addExplorerMonitor = (
    channel: CatalogChannel,
    message: CatalogMessage,
    signal: CatalogSignal,
  ) => {
    if (!monitorKeys.has(signal.key)) {
      void run({
        type: "add_monitor",
        kind: channel.kind,
        ch: channel.index,
        id: message.id,
        ext: message.extended,
        frame: message.frame,
        name: signal.name,
      });
    }
    setChartTab("monitor");
  };

  const applyReference = (data: ReferenceData) => {
    const mapped = mapReferenceColumns(data.columns, chartSeries);
    const count = Object.keys(mapped).length;
    if (count === 0) {
      showToast("Reference has no columns matching the chart signals");
      return;
    }
    setReference({ ...data, series: mapped });
    setReferenceOpen(false);
    showToast(`Reference ${data.name} · ${count} signals`);
  };

  const clearReference = () => {
    setReference(null);
    setReferenceOpen(false);
    void sendCommand({ type: "clear_reference" }).catch(() => {});
    showToast("Reference removed");
  };

  const signalSignature = chartSeries.map((signal) => signal.key).join("|");

  useEffect(() => {
    if (!reference) return;
    setReference((old) =>
      old
        ? { ...old, series: mapReferenceColumns(old.columns, chartSeries) }
        : old,
    );
    // Only a lane set change needs remapping; sample updates do not.
  }, [signalSignature]);

  const addPaletteCursor = () => {
    if (cursorSeries.length === 0) {
      showToast("No signals to measure");
      return;
    }
    let last = 0;
    for (const signal of cursorSeries) {
      const point = signal.points[signal.points.length - 1];
      if (point) last = Math.max(last, point[0]);
    }
    const end = signalView.endS ?? last;
    addChartCursor(end - signalView.windowS / 2);
    setChartTab("signals");
  };

  const paletteCommands: PaletteCommand[] = [
    {
      id: "open-session",
      label: "Open Session…",
      group: "Open",
      hint: "Projects, replay logs and recording",
      run: openSession,
    },
    {
      id: "open-buses",
      label: "Open Buses…",
      group: "Open",
      hint: "DBC, hardware and bus modes",
      run: openBuses,
    },
    {
      id: "open-generator",
      label: "Open Generator…",
      group: "Open",
      hint: "Messages, cycles and signal sources",
      run: openGenerator,
    },
    {
      id: "open-triggers",
      label: "Open Triggers…",
      group: "Open",
      hint: "Conditions, actions and recording limits",
      run: openTriggers,
    },
    {
      id: "open-variables",
      label: "Open Variables…",
      group: "Open",
      hint: "System values and computed channels",
      run: () => openVariables(),
    },
    {
      id: "open-scripts",
      label: "Open Scripts…",
      group: "Open",
      hint: "Node scripts and their logs",
      run: () => {
        closeDialogs();
        setScriptsOpen(true);
      },
    },
    {
      id: "pick-signals",
      label: "Choose chart signals…",
      group: "Signals",
      hint: "Choose and order the Signals lanes",
      run: openPicker,
    },
    {
      id: "open-dbc",
      label: "Open DBC explorer",
      group: "Signals",
      hint: "Search the database and add lanes or watches",
      run: openExplorer,
    },
    {
      id: "reference-csv",
      label: "Overlay reference CSV…",
      group: "Signals",
      hint: "Compare a previous aligned export with an offset",
      run: () => setReferenceOpen(true),
    },
    {
      id: "derived-signal",
      label: "New derived signal…",
      group: "Signals",
      hint: "Math channel over the selected signals",
      run: () => openVariables("computed"),
    },
    {
      id: "signal-event",
      label: "Find signal event…",
      group: "Signals",
      hint: "Jump between rising edges of a condition",
      run: () => setEventOpen(true),
    },
    {
      id: "add-cursor",
      label: "Add signal cursor",
      group: "Signals",
      hint: "Place a time cursor in the middle of the view",
      run: addPaletteCursor,
    },
    {
      id: "clear-cursors",
      label: "Clear signal cursors",
      group: "Signals",
      run: clearChartCursors,
    },
    {
      id: "copy-monitor",
      label: "Copy chart signals to Monitor",
      group: "Signals",
      run: () => void copyChartSelection("monitor"),
    },
    {
      id: "copy-data",
      label: "Copy chart signals to Data",
      group: "Signals",
      run: () => void copyChartSelection("data"),
    },
    {
      id: "copy-state",
      label: "Copy chart signals to State",
      group: "Signals",
      run: () => void copyChartSelection("state"),
    },
    {
      id: "chart-rate",
      label: "Show Rate",
      group: "Chart",
      run: () => setChartTab("rate"),
    },
    {
      id: "chart-signals",
      label: "Show Signals",
      group: "Chart",
      run: () => setChartTab("signals"),
    },
    {
      id: "chart-monitor",
      label: "Show Monitor",
      group: "Chart",
      run: () => setChartTab("monitor"),
    },
    {
      id: "chart-data",
      label: "Show Data",
      group: "Chart",
      run: () => setChartTab("data"),
    },
    {
      id: "chart-state",
      label: "Show State",
      group: "Chart",
      run: () => setChartTab("state"),
    },
    {
      id: "go-to-time",
      label: "Go to time…",
      group: "Time",
      hint: "Ctrl+G · Trace, chart and replay",
      run: () => setTimeJumpOpen(true),
    },
    ...(prevTriggerUs !== null
      ? [
          {
            id: "prev-trigger",
            label: "Go to previous trigger",
            group: "Time",
            hint: `${(prevTriggerUs / 1e6).toFixed(3)} s`,
            run: () => viewTimeUs(prevTriggerUs),
          },
        ]
      : []),
    ...(nextTriggerUs !== null
      ? [
          {
            id: "next-trigger",
            label: "Go to next trigger",
            group: "Time",
            hint: `${(nextTriggerUs / 1e6).toFixed(3)} s`,
            run: () => viewTimeUs(nextTriggerUs),
          },
        ]
      : []),
    {
      id: "view-trace",
      label: "Show Trace",
      group: "Main view",
      run: () => setMainView("trace"),
    },
    {
      id: "view-messages",
      label: "Show Messages",
      group: "Main view",
      run: () => setMainView("messages"),
    },
    {
      id: "tab-statistics",
      label: "Show Statistics",
      group: "Lower panel",
      run: () => setTab("statistics"),
    },
    {
      id: "tab-bus",
      label: "Show Bus statistics",
      group: "Lower panel",
      run: () => setTab("bus"),
    },
    {
      id: "tab-spec",
      label: "Show Spec",
      group: "Lower panel",
      run: () => setTab("spec"),
    },
    {
      id: "tab-write",
      label: "Show Write log",
      group: "Lower panel",
      run: () => setTab("activity"),
    },
    {
      id: "fullscreen",
      label: chartFullscreen ? "Exit fullscreen chart" : "Fullscreen chart",
      group: "View",
      run: toggleChartFullscreen,
    },
    {
      id: "theme",
      label:
        theme === "dark" ? "Switch to light theme" : "Switch to dark theme",
      group: "View",
      run: () => setTheme(theme === "dark" ? "light" : "dark"),
    },
    {
      id: "layout-trace",
      label: "Layout: Trace focus",
      group: "View",
      hint: "Wide Trace, compact observers",
      run: () => applyLayoutPreset("trace"),
    },
    {
      id: "layout-analysis",
      label: "Layout: Analysis",
      group: "View",
      hint: "Signals and Statistics side by side",
      run: () => applyLayoutPreset("analysis"),
    },
    {
      id: "layout-replay",
      label: "Layout: Replay",
      group: "View",
      hint: "Trace with Rate and room for the replay bar",
      run: () => applyLayoutPreset("replay"),
    },
    {
      id: "reset-layout",
      label: "Reset layout",
      group: "View",
      hint: "Splitters, Trace columns and chart window",
      run: resetLayout,
    },
    ...(canControl && !measuring && !needsLog
      ? [
          {
            id: "start",
            label: "Start measurement",
            group: "Measurement",
            hint: "F9",
            run: () => void run({ type: "start" }),
          },
        ]
      : []),
    ...(canControl && measuring
      ? [
          {
            id: "stop",
            label: "Stop measurement",
            group: "Measurement",
            run: () => void run({ type: "stop" }),
          },
          {
            id: "pause",
            label: state?.tracePaused
              ? "Resume measurement"
              : "Pause measurement",
            group: "Measurement",
            run: () =>
              void run({ type: "pause", on: !state?.tracePaused }),
          },
        ]
      : []),
    {
      id: "help",
      label: "Shortcuts and about",
      group: "Help",
      run: openHelp,
    },
  ];

  const renderSecondary = (tab: ChartTab) => {
    if (tab === "rate") {
      return (
        <RateChart
          points={state?.series ?? []}
          channels={state?.channels ?? []}
          theme={theme}
        />
      );
    }
    if (tab === "signals") {
      return (
        <SignalChart
          series={chartSeries}
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
          measuring={measuring}
          paused={state?.tracePaused ?? false}
          reference={reference}
          onOpenReference={() => setReferenceOpen(true)}
          onOpenDerived={() => openVariables("computed")}
          overlay={signalOverlay}
          onToggleOverlay={() => setSignalOverlay((value) => !value)}
          yModes={chartYModes}
          onYMode={setSignalYMode}
          ranges={signalRanges}
        />
      );
    }
    if (tab === "monitor") {
      return (
        <MonitorPanel
          rows={monitorRows}
          theme={theme}
          onEdit={editMonitor}
          onRemove={(row) =>
            row.kind === "derived"
              ? removeDerived(row.frame)
              : void run({
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
      );
    }
    if (tab === "data") {
      return (
        <DataPanel
          rows={dataRows}
          onVisible={(row, visible) =>
            row.kind === "derived"
              ? showToast("Derived rows are always visible")
              : void run({
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
            row.kind === "derived"
              ? removeDerived(row.frame)
              : void run({
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
      );
    }
    return (
      <StatePanel
        rows={stateRows}
        windowS={state?.stateWindowS ?? 20}
        theme={theme}
        onWindow={(seconds) =>
          void run({ type: "set_state_window", seconds })
        }
        onRule={stateRowRule}
        onRemove={stateRowRemove}
      />
    );
  };

  const switchMainView = (view: MainView) => {
    if (view !== "panel") standardView.current = view;
    if (view === "panel") {
      setChartFullscreen(false);
      setExplorerOpen(false);
    }
    setMainView(view);
  };

  const showStandard = () => switchMainView(standardView.current);

  // The physical mode ships chosen signal values; the core encodes them and
  // keeps the database's start value for everything else.
  const panelPhysical = (frame: PanelFrameSend) => {
    if (frame.mode !== "physical") return {};
    const signals: Record<string, number> = {};
    for (const entry of frame.values ?? []) signals[entry.name] = entry.value;
    return { physical: true, signals };
  };

  const runPanelAction = (widget: PanelButton) => {
    if (widget.action === "send") {
      const frame = widget.frame;
      const data = frame ? parsePanelHex(frame.data) : null;
      if (!frame || data === null) {
        showToast("This button has no valid message");
        return;
      }
      if (frame.protocol === "can") {
        void run({
          type: "panel_send_frame",
          channel: frame.channel,
          id: frame.id,
          extended: frame.extended,
          data,
          ...panelPhysical(frame),
        });
      } else if (frame.protocol === "fr") {
        void run({
          type: "panel_send_fr_frame",
          bus: frame.bus,
          slot: frame.slot,
          data,
          ...panelPhysical(frame),
        });
      } else {
        void run({
          type: "panel_send_eth_frame",
          bus: frame.bus,
          frame: frame.frame,
          data,
          ...panelPhysical(frame),
        });
      }
      return;
    }
    if (widget.action === "script") {
      const name = widget.script?.trim() ?? "";
      if (!name) {
        showToast("This button has no script action");
        return;
      }
      void run({ type: "run_panel_action", name, value: widget.argument });
      return;
    }
    const writes = (widget.writes ?? [])
      .map((write) => ({ source: write.source.trim(), value: write.value }))
      .filter((write) => write.source.length > 0);
    if (writes.length > 0) {
      void run({ type: "panel_write_signals", writes });
    }
  };

  const pageWriteTargets = useMemo<PanelWriteTarget[]>(() => {
    const targets: PanelWriteTarget[] = [];
    for (const channel of catalog?.channels ?? []) {
      // Every protocol the catalog names can take a setpoint; CAN writes a
      // frame, FlexRay and Ethernet inject one session frame.
      for (const message of channel.messages) {
        for (const signal of message.signals) {
          targets.push({
            key: signal.key,
            min: signal.max > signal.min ? signal.min : null,
            max: signal.max > signal.min ? signal.max : null,
            unit: signal.unit,
          });
        }
      }
    }
    return targets;
  }, [catalog]);

  const pageView = useMemo<PanelView>(
    () => ({
      signals,
      sysvars: state?.sysvars ?? [],
      writeTargets: pageWriteTargets,
      frameBuses: (catalog?.channels ?? [])
        .filter((channel) => channel.kind === "can")
        .map((channel) => ({ index: channel.index, name: channel.name })),
      frBuses: (state?.frBuses ?? []).map((bus) => ({
        index: bus.bus,
        name: bus.name,
      })),
      ethBuses: (state?.ethBuses ?? []).map((bus) => ({
        index: bus.bus,
        name: bus.name,
      })),
      ethFrames: (catalog?.channels ?? [])
        .filter((channel) => channel.kind === "eth")
        .flatMap((channel) =>
          channel.messages.map((message) => ({
            bus: channel.index,
            name: message.frame ?? message.name,
            length: message.length ?? null,
          })),
        ),
      frameDefs: (catalog?.channels ?? []).flatMap(
        (channel): PanelFrameDef[] => {
        const signals = (message: (typeof channel.messages)[number]) =>
          message.signals.map((signal) => ({
            name: signal.name,
            unit: signal.unit,
            min: signal.min,
            max: signal.max,
            initial: signal.initial,
          }));
        if (channel.kind === "can") {
          return channel.messages.map((message) => ({
            protocol: "can" as const,
            bus: channel.index,
            id: message.id,
            extended: message.extended,
            name: message.name,
            signals: signals(message),
          }));
        }
        if (channel.kind === "fr") {
          return channel.messages.map((message) => ({
            protocol: "fr" as const,
            bus: channel.index,
            slot: message.id,
            name: message.name,
            signals: signals(message),
          }));
        }
        return channel.messages.map((message) => ({
          protocol: "eth" as const,
          bus: channel.index,
          frame: message.frame ?? message.name,
          name: message.frame ?? message.name,
          signals: signals(message),
        }));
      }),
    }),
    [signals, state?.sysvars, state?.frBuses, state?.ethBuses, pageWriteTargets, catalog],
  );

  // Setpoint readback: subscribe each written signal as a Read signal so its
  // live value reaches the tile. Keys added here are removed again when the
  // last setpoint stops using them, unless the user also reads/plots them.
  const panelReadKeys = useRef(
    new Map<string, { select: Command; unselect: Command }>(),
  );
  useEffect(() => {
    // A project/log switch resets the core's subscriptions; forget what this
    // session owned so the next run re-subscribes what the pages reference.
    panelReadKeys.current.clear();
  }, [state?.workspaceRev]);
  const panelSetpointSources = useMemo(() => {
    const keys = new Set<string>();
    for (const page of panelBook.pages) {
      for (const widget of page.widgets) {
        if (
          widget.kind === "numeric" &&
          panelNumericTarget(widget) === "signal" &&
          widget.source.trim()
        ) {
          keys.add(widget.source);
        }
      }
    }
    return [...keys];
  }, [panelBook]);
  useEffect(() => {
    const wanted = new Set(panelSetpointSources);
    for (const key of wanted) {
      if (panelReadKeys.current.has(key)) continue;
      if (signals.some((series) => series.key === key)) continue;
      const found = findCatalogSignal(catalog, key);
      if (!found) continue;
      const select = catalogSignalCommand(
        found.channel,
        found.message,
        found.signal,
        true,
      );
      const unselect = catalogSignalCommand(
        found.channel,
        found.message,
        found.signal,
        null,
      );
      panelReadKeys.current.set(key, { select, unselect });
      void run(select);
    }
    for (const [key, commands] of [...panelReadKeys.current]) {
      if (wanted.has(key)) continue;
      panelReadKeys.current.delete(key);
      const series = signals.find((entry) => entry.key === key);
      const userRead = readSignals.some((spec) => spec.key === key);
      if (series?.read && !userRead) void run(commands.unselect);
    }
  }, [panelSetpointSources, catalog, signals, readSignals, run]);

  const pageActionContext = useMemo<PanelActionContext>(
    () => ({
      online,
      measuring: state?.measuring ?? false,
      paused: state?.tracePaused ?? false,
      scriptActions: runningPanelActions(state?.scripts ?? []),
    }),
    [online, state?.measuring, state?.tracePaused, state?.scripts],
  );

  const pageSourceOptions = useMemo(
    () => panelSourceOptions(pageView, derivedDefs.map((def) => def.name)),
    [pageView, derivedDefs],
  );

  const pageSysVarOptions = useMemo(
    () =>
      pageView.sysvars.map(
        (variable) => `${variable.namespace}::${variable.name}`,
      ),
    [pageView],
  );

  const pageActionNames = useMemo(() => {
    const names = new Set<string>();
    for (const script of state?.scripts ?? []) {
      for (const name of script.panelActions) names.add(name);
    }
    return [...names].sort((a, b) => a.localeCompare(b));
  }, [state?.scripts]);

  const togglePageSysVar = (widget: PanelToggle, on: boolean) => {
    const known = findPanelSysVar(widget.sysvar, pageView);
    if (!known) {
      showToast(`Undefined sysvar ${widget.sysvar}`);
      return;
    }
    void run({
      type: "set_sys_var",
      namespace: known.namespace,
      name: known.name,
      value: on ? 1 : 0,
    });
  };

  const setPageNumber = (widget: PanelNumeric, value: number) => {
    const known = findPanelSysVar(widget.sysvar, pageView);
    if (!known) {
      showToast(`Undefined sysvar ${widget.sysvar}`);
      return;
    }
    const next = clampPanelNumber(value, known.min, known.max);
    if (next !== known.value) {
      void run({
        type: "set_sys_var",
        namespace: known.namespace,
        name: known.name,
        value: next,
      });
    }
    const action = widget.panelAction?.trim();
    if (action) {
      void run({ type: "run_panel_action", name: action, value: next });
    }
  };

  const writePageSignal = (widget: PanelNumeric, value: number) => {
    if (!widget.source) {
      showToast("This setpoint has no signal");
      return;
    }
    void run({ type: "panel_write_signal", source: widget.source, value });
    const action = widget.panelAction?.trim();
    if (action) {
      void run({ type: "run_panel_action", name: action, value });
    }
  };

  const mainTabs = (
    <div class="tabs main-tabs">
      <button
        type="button"
        class={mainView === "trace" ? "on" : ""}
        onClick={() => switchMainView("trace")}
      >
        Trace
      </button>
      <button
        type="button"
        class={mainView === "messages" ? "on" : ""}
        onClick={() => switchMainView("messages")}
      >
        Messages
      </button>
    </div>
  );

  return (
    <div class={`app ${state && !online ? "offline" : ""}`}>
      <header class="topbar">
        <div class="brand">
          <div class="brand-menu">
            <button
              type="button"
              class="logo"
              aria-label="Roxy CAN menu"
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
              Roxy CAN
              <span class={`presence ${online ? "on" : "off"}`}>
                <span class="dot" />
                {online ? "live" : "reconnecting"}
              </span>
              {state?.recording && <span class="rec-badge">REC</span>}
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
          <div class="view-switch" role="group" aria-label="Main view">
            <button
              type="button"
              class={mainView !== "panel" ? "on" : ""}
              onClick={showStandard}
            >
              Standard
            </button>
            <button
              type="button"
              class={mainView === "panel" ? "on" : ""}
              onClick={() => switchMainView("panel")}
            >
              Panel
            </button>
          </div>
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
          <button type="button" disabled={!online} onClick={openBuses}>
            Buses…
          </button>
          <button
            type="button"
            disabled={!online}
            onClick={() => openVariables()}
          >
            Variables…
          </button>
          <button type="button" disabled={!online} onClick={openSession}>
            Session…
          </button>
          <button
            type="button"
            class="primary"
            disabled={!canControl || measuring || needsLog}
            title={
              !online
                ? "Waiting for the bus core"
                : needsLog
                  ? "Load a replay log first"
                  : measuring
                    ? "Measurement is already running"
                    : "Start measurement"
            }
            onClick={() => void run({ type: "start" })}
          >
            Start
          </button>
          <button
            type="button"
            disabled={!canControl || !measuring}
            title={measuring ? "Stop measurement" : "Measurement is not running"}
            onClick={() => void run({ type: "stop" })}
          >
            Stop
          </button>
          <button
            type="button"
            disabled={!canControl || !measuring}
            title={
              measuring
                ? state?.tracePaused
                  ? "Resume measurement"
                  : "Pause measurement"
                : "Measurement is not running"
            }
            onClick={() => void run({ type: "pause", on: !state?.tracePaused })}
          >
            {state?.tracePaused ? "Resume" : "Pause"}
          </button>
          <button
            type="button"
            disabled={!canControl}
            title="Clear the visible trace; measurement keeps running"
            onClick={() => void run({ type: "clear_trace" })}
          >
            Clear trace
          </button>
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
            {state.markers.length > 0 && (
              <>
                <button
                  type="button"
                  disabled={prevTriggerUs === null}
                  title="Show the previous trigger in Trace and the chart; playback keeps running"
                  onClick={() => prevTriggerUs !== null && viewTimeUs(prevTriggerUs)}
                >
                  Prev trigger
                </button>
                <button
                  type="button"
                  disabled={nextTriggerUs === null}
                  title="Show the next trigger in Trace and the chart; playback keeps running"
                  onClick={() => nextTriggerUs !== null && viewTimeUs(nextTriggerUs)}
                >
                  Next trigger
                </button>
              </>
            )}
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
          {state.replayPosUs != null &&
            state.replayTotalUs != null &&
            state.replayTotalUs > 0 && (
              <div class="replay-track-row">
                <div
                  class="replay-track"
                  title="Click ahead to play there; clicking behind only moves the view"
                  onClick={(event) => {
                  const rect = event.currentTarget.getBoundingClientRect();
                  const frac = clamp(
                    (event.clientX - rect.left) / rect.width,
                    0,
                    1,
                  );
                  goToTimeUs(Math.round(frac * state.replayTotalUs!));
                }}
              >
                <span
                  class="replay-fill"
                  style={{
                    width: `${(state.replayPosUs / state.replayTotalUs) * 100}%`,
                  }}
                />
                {state.markers.map((marker) => (
                  <span
                    key={marker}
                    class="replay-mark"
                    style={{
                      left: `${clamp((marker * 1e6) / state.replayTotalUs! * 100, 0, 100)}%`,
                    }}
                  />
                ))}
                <span
                  class="replay-cursor"
                  style={{
                    left: `${(state.replayPosUs / state.replayTotalUs) * 100}%`,
                  }}
                />
                </div>
                <span class="replay-hint">
                  forward only · pauses at the target
                </span>
              </div>
            )}
        </section>
      )}

      <main
        class="workspace"
        ref={workspaceRef}
        style={{
          gridTemplateColumns:
            mainView === "panel"
              ? "minmax(0, 1fr)"
              : `calc((100% - 3px) * ${layout.main}) 3px minmax(0, 1fr)`,
        }}
      >
        {mainView === "panel" ? (
          <PagePanel
            book={panelBook}
            revision={state?.workspaceRev ?? 0}
            toast={toast}
            sourceOptions={pageSourceOptions}
            sysvarOptions={pageSysVarOptions}
            actionNames={pageActionNames}
            view={pageView}
            actionContext={pageActionContext}
            onBook={setPanelBook}
            onRun={runPanelAction}
            onToggle={togglePageSysVar}
            onSetNumber={setPageNumber}
            onWrite={writePageSignal}
            onNotice={showToast}
          />
        ) : (
          <>
        <section class={`panel trace-panel ${mainView}`}>
          {mainView === "trace" ? (
          <TraceTable
            key={`${bus}:${traceEpoch}`}
            rows={visibleRows}
            filterKey={traceFilterKey}
            columns={traceColumns}
            onColumnsChange={setTraceColumns}
            jump={traceJump}
            onJumpDone={() => setTraceJump(null)}
            leading={mainTabs}
            title={
              <span class="meta">
                {visibleRows.length.toLocaleString()} shown /{" "}
                {traceRows.length.toLocaleString()} rows
              </span>
            }
            tools={
              <>
                <button
                  type="button"
                  class={`mini ${explorerOpen ? "on" : ""}`}
                  title="Browse DBC signals and add them to the chart or Monitor"
                  onClick={openExplorer}
                >
                  DBC
                </button>
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
                <ColumnsMenu
                  columns={traceColumns}
                  onChange={setTraceColumns}
                />
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
                  title={
                    "Space-separated, all must match. Fields: id:0x100, bus:CAN1, dir:tx, name:RPM, data:A00F, addr, kind. Prefix - excludes (-tx, -id:0x100). Quotes keep spaces."
                  }
                  onInput={(e) =>
                    setQuery((e.target as HTMLInputElement).value)
                  }
                />
              </>
            }
            onQuery={setQuery}
            onChartCursor={placeChartCursor}
            onSetChartCursor={setChartCursorAt}
            onAddToChart={(frame) => void addFrameSignals(frame, "chart")}
            onWatch={(frame) => void addFrameSignals(frame, "monitor")}
            onShowTime={(tUs) => goToTimeUs(tUs, false)}
          />
          ) : (
            <>
              <div class="panel-head">
                {mainTabs}
                <span class="meta">{mainMessages.length} ids</span>
                <div class="panel-tools">
                  <input
                    class="panel-filter"
                    type="search"
                    value={messageQuery}
                    placeholder="Filter bus, name or id…"
                    onInput={(event) =>
                      setMessageQuery(
                        (event.target as HTMLInputElement).value,
                      )
                    }
                  />
                  <button
                    type="button"
                    class="mini"
                    onClick={() => openExport("messages")}
                    title="Export messages to a host CSV file"
                  >
                    Export…
                  </button>
                </div>
              </div>
              <MessageTable
                key={state?.log ?? state?.projectPath ?? "untitled"}
                rows={mainMessages}
                filtered={messageQuery.trim() !== ""}
                onExpand={(key, on) =>
                  void run({ type: "set_message_expanded", key, on })
                }
              />
            </>
          )}
          {mainView === "trace" && explorerOpen && (
            <DbcExplorer
              catalog={catalog}
              loading={catalogLoading}
              error={catalogError}
              charted={selectedKeys}
              watched={monitorKeys}
              onAddChart={addExplorerChart}
              onAddMonitor={addExplorerMonitor}
              onClose={() => setExplorerOpen(false)}
            />
          )}
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
            gridTemplateRows: `minmax(0, ${layout.right}fr) 3px minmax(0, ${1 - layout.right}fr)`,
          }}
        >
          <section
            class={`panel chart-panel ${chartFullscreen ? "chart-fullscreen" : ""}`}
          >
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
                  title="Choose signals to plot or read at cursors"
                  onClick={() => void openPicker()}
                >
                  + Signals
                </button>
              )}
              {chartTab === "monitor" && (
                <>
                  <button
                    type="button"
                    class="mini"
                    title="Choose the signals to watch"
                    onClick={() => void openMonitorPicker()}
                  >
                    + Signals
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
                    title="Choose the signals to track"
                    onClick={() => void openDataPicker()}
                  >
                    + Signals
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
                    title="Choose the signals to show as bands"
                    onClick={() => void openStatePicker()}
                  >
                    + Signals
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
                    : `${chartSeries.length} channels${
                        readSeries.length ? ` · ${readSeries.length} read` : ""
                      }${signalOverlay ? " · overlay" : " · wheel zoom"}`}
                </span>
              )}
              {chartTab === "rate" && (
                <strong class="readout">
                  {state ? state.fps.toFixed(0) : "—"} f/s
                </strong>
              )}
              <div class="chart-head-tools">
                <button
                  type="button"
                  class={`chart-focus ${chartSplit ? "on" : ""}`}
                  aria-label="Split the chart panel"
                  title={
                    chartSplit
                      ? "Remove the second chart pane"
                      : "Show a second chart pane"
                  }
                  onClick={toggleChartSplit}
                >
                  <svg viewBox="0 0 16 16" aria-hidden="true">
                    <rect x="2" y="2" width="12" height="5" rx="1" />
                    <rect x="2" y="9" width="12" height="5" rx="1" />
                  </svg>
                </button>
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
            <div class="chart-split-area" ref={chartSplitRef}>
              <div
                class="chart-pane"
                style={chartSplit ? { flex: `${splitRatio} 1 0` } : undefined}
              >
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
              <div class="signal-stack">
                <SignalChart
                  series={chartSeries}
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
                  measuring={measuring}
                  paused={state?.tracePaused ?? false}
                  reference={reference}
                  onOpenReference={() => setReferenceOpen(true)}
                  onOpenDerived={() => openVariables("computed")}
                  overlay={signalOverlay}
                  onToggleOverlay={() => setSignalOverlay((value) => !value)}
                  yModes={chartYModes}
                  onYMode={setSignalYMode}
                  ranges={signalRanges}
                  hasReadout={readSeries.length > 0}
                  readRange={readRange}
                  readCount={readSeries.length}
                />
                {!chartSplit && (chartFullscreen || chartCursors.length > 0) && (
                  <CursorAnalysis
                    series={cursorSeries}
                    cursors={chartCursors}
                    onDownloadRange={downloadChartCsv}
                    onExportRange={
                      exportRange
                        ? () => {
                            setExportKind("signals");
                            setExportOpen(true);
                          }
                        : undefined
                    }
                  />
                )}
              </div>
            ) : chartTab === "monitor" ? (
              <MonitorPanel
                rows={monitorRows}
                theme={theme}
                onEdit={editMonitor}
                onRemove={(row) =>
                  row.kind === "derived"
                    ? removeDerived(row.frame)
                    : void run({
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
                rows={dataRows}
                onVisible={(row, visible) =>
                  row.kind === "derived"
                    ? showToast("Derived rows are always visible")
                    : void run({
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
                  row.kind === "derived"
                    ? removeDerived(row.frame)
                    : void run({
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
                rows={stateRows}
                windowS={state?.stateWindowS ?? 20}
                theme={theme}
                onWindow={(seconds) =>
                  void run({ type: "set_state_window", seconds })
                }
                onRule={stateRowRule}
                onRemove={stateRowRemove}
              />
            )}
              </div>
              {chartSplit && (
                <>
                  <div class="chart-splitter" onPointerDown={startSplitDrag} />
                  <div
                    class="chart-pane"
                    style={{ flex: `${1 - splitRatio} 1 0` }}
                  >
                    <div class="chart-subhead">
                      <select
                        value={chartTab2}
                        aria-label="Second chart pane"
                        onChange={(event) =>
                          setChartTab2(
                            (event.target as HTMLSelectElement)
                              .value as ChartTab,
                          )
                        }
                      >
                        <option value="rate">Rate</option>
                        <option value="signals">Signals</option>
                        <option value="monitor">Monitor</option>
                        <option value="data">Data</option>
                        <option value="state">State</option>
                      </select>
                    </div>
                    {renderSecondary(chartTab2)}
                  </div>
                </>
              )}
            </div>
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
                {tab === "statistics"
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
                    openExport(tab === "statistics" ? "stats" : "write")
                  }
                  title="Export this panel to a host file"
                >
                  Export…
                </button>
              )}
            </div>
            {tab === "statistics" ? (
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
                <StatsTable
                  rows={lowerStats}
                  filtered={panelQuery.trim() !== ""}
                />
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
          </>
        )}
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
        {toast && (
          <span class="toast" role="status" title={toast}>
            {toast}
          </span>
        )}
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
          derived={derivedDefs}
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
          onBrs={setEntryBrs}
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
          onBlockEdit={(id, channel, name, path, node, ids) =>
            void run({
              type: "set_replay_block",
              id,
              channel,
              name,
              path,
              node,
              ids,
            })
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
          catalog={catalog}
          derived={derivedDefs}
          sysvars={state?.sysvars ?? []}
          onCommand={(command) => void run(command)}
          onClose={() => setScriptsOpen(false)}
        />
      )}

      {sysvarsOpen && (
        <VariablesModal
          vars={state?.sysvars ?? []}
          onSetValue={(namespace, name, value) =>
            void run({ type: "set_sys_var", namespace, name, value })
          }
          onDefine={defineSysVar}
          onDelete={(namespace, name) =>
            void run({ type: "delete_sys_var", namespace, name })
          }
          derived={derivedDefs}
          derivedRefs={derivedReferences}
          onAddDerived={(definition) => {
            saveDerived([...derivedDefs, definition]);
            showToast(`Computed ${definition.name} added`);
          }}
          onRemoveDerived={(id) =>
            saveDerived(derivedDefs.filter((definition) => definition.id !== id))
          }
          onRuleDerived={(definition) => {
            setSysvarsOpen(false);
            setDerivedRule(definition);
          }}
          initialTab={variablesTab}
          onClose={() => setSysvarsOpen(false)}
        />
      )}

      {pickerOpen && (
        <SignalPicker
          catalog={catalog}
          loading={catalogLoading}
          error={catalogError}
          selected={selectedKeys}
          watched={monitorKeys}
          read={readKeys}
          kinds={["can", "fr", "eth"]}
          onToggle={toggleSignal}
          onReadToggle={toggleReadSignal}
          onCopyTo={copyChartSelection}
          derived={derivedDefs}
          onManageDerived={() => {
            setPickerOpen(false);
            openVariables("computed");
          }}
          onClose={() => setPickerOpen(false)}
        />
      )}

      {monitorPickerOpen && (
        <SignalPicker
          catalog={catalog}
          loading={catalogLoading}
          error={catalogError}
          selected={monitorKeys}
          watched={monitorKeys}
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
          watched={monitorKeys}
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
          watched={monitorKeys}
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
          range={exportRange}
          onExport={(kind, path, range) => {
            void run({
              type: "export",
              kind,
              path,
              ...(range
                ? { from_us: range.fromUs, to_us: range.toUs }
                : {}),
            });
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

      {derivedRule && (
        <StateRuleModal
          signal={`${derivedRule.name} · Derived`}
          rule={
            derivedRule.cuts.length > 0
              ? { cuts: derivedRule.cuts, names: derivedRule.names }
              : null
          }
          onApply={(cuts, names) => {
            saveDerived(
              derivedDefs.map((definition) =>
                definition.id === derivedRule.id
                  ? { ...definition, cuts, names }
                  : definition,
              ),
            );
            setDerivedRule(null);
          }}
          onClear={() => {
            saveDerived(
              derivedDefs.map((definition) =>
                definition.id === derivedRule.id
                  ? { ...definition, cuts: [], names: [] }
                  : definition,
              ),
            );
            setDerivedRule(null);
          }}
          onClose={() => setDerivedRule(null)}
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
              <div class="session-tasks">
                <section class="session-task">
                  <h3>Connect real bus</h3>
                  <p>
                    Attach a Vector or Kvaser channel, load its DBC and map
                    node roles.
                  </p>
                  <div class="task-actions">
                    <button
                      type="button"
                      class="primary"
                      onClick={() => {
                        setSessionOpen(false);
                        openBuses();
                      }}
                    >
                      Open Buses…
                    </button>
                  </div>
                  <span class="task-state">
                    {(state?.buses.filter((bus) => bus.attached).length ?? 0) ===
                    0
                      ? "No adapter attached"
                      : `${state?.buses.filter((bus) => bus.attached).length} adapter(s) attached · ${
                          state?.realBus ? "real bus on" : "simulation only"
                        }`}
                  </span>
                </section>

                <section class="session-task">
                  <h3>Simulate</h3>
                  <p>
                    Start the loaded DBC with the generator. Attached hardware
                    is used when Real bus is on.
                  </p>
                  <div class="task-actions">
                    <button
                      type="button"
                      class="primary"
                      disabled={!canControl || measuring || needsLog}
                      title={
                        !online
                          ? "Waiting for the bus core"
                          : needsLog
                            ? "Load a replay log first"
                            : measuring
                              ? "Measurement is already running"
                              : "Start measurement"
                      }
                      onClick={() => {
                        void run({ type: "start" });
                        setSessionOpen(false);
                      }}
                    >
                      Start
                    </button>
                    <button
                      type="button"
                      onClick={() => {
                        setSessionOpen(false);
                        openGenerator();
                      }}
                    >
                      Generator…
                    </button>
                  </div>
                  <span class="task-state">
                    {measuring
                      ? `${state?.runMode ?? ""} · ${state?.fps.toFixed(0)} f/s`
                      : needsLog
                        ? "Load a replay log first"
                        : "Ready with the loaded DBC"}
                  </span>
                </section>

                <section class="session-task">
                  <h3>Replay</h3>
                  <p>
                    Open an ASC or BLF and play it forward with the same
                    analysis tools.
                  </p>
                  <form
                    class="task-form"
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
                    <div class="task-actions">
                      <button
                        type="submit"
                        class="primary"
                        disabled={!logPath.trim()}
                      >
                        Open &amp; play
                      </button>
                      <FilePick
                        label="Choose…"
                        accept=".asc,.blf"
                        onError={setError}
                        onPick={(path) => {
                          setLogPath(path);
                          loadLog(path);
                        }}
                      />
                    </div>
                  </form>
                  {recent(state?.recentLogs ?? [], loadLog)}
                </section>
              </div>

              <section class="session-section">
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
                    onClick={() => void saveProject(null)}
                  >
                    Save
                  </button>
                  <input
                    value={savePath}
                    placeholder={
                      state?.projectPath || "Save as file name or host path…"
                    }
                    onInput={(event) =>
                      setSavePath((event.target as HTMLInputElement).value)
                    }
                  />
                  <button
                    type="button"
                    disabled={!savePath.trim()}
                    onClick={() => void saveProject(savePath.trim())}
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

              <details class="session-more">
                <summary>Recording</summary>
                <div class="session-more-body">
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
                        setRecordName(
                          (event.target as HTMLInputElement).value,
                        )
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
                    Relative names land in Record/ beside the project; an
                    absolute path is used as typed. Recording while stopped arms
                    the file for the next Start.
                  </p>
                  <label class="record-filter">
                    Only ids / FR slots
                    <input
                      value={recordFilter}
                      placeholder="100, 3F4x, FR0:13"
                      spellcheck={false}
                      onInput={(event) =>
                        setRecordFilter(
                          (event.target as HTMLInputElement).value,
                        )
                      }
                      onBlur={() =>
                        void run({
                          type: "set_record_filter",
                          text: recordFilter,
                        })
                      }
                      onKeyDown={(event) => {
                        if (event.key === "Enter") {
                          (event.target as HTMLInputElement).blur();
                        }
                      }}
                    />
                  </label>
                </div>
              </details>

              <p class="hint">
                Paths are resolved on the host. Opening a project or log stops
                the current measurement and clears the trace.
              </p>
            </div>
          </div>
        </div>
      )}
      {paletteOpen && (
        <CommandPalette
          commands={paletteCommands}
          onClose={() => setPaletteOpen(false)}
        />
      )}
      {eventOpen && (
        <EventJumpModal
          series={chartSeries}
          nowS={chartCenterS}
          onJump={(time) => viewTimeUs(Math.round(time * 1e6))}
          onClose={() => setEventOpen(false)}
        />
      )}
      {referenceOpen && (
        <ReferenceModal
          current={
            reference
              ? {
                  name: reference.name,
                  offsetS: reference.offsetS,
                  columns: reference.columns,
                }
              : null
          }
          onApply={applyReference}
          onClear={clearReference}
          onLoadLog={(path) => {
            setReferenceOpen(false);
            showToast("Loading reference…");
            void run({ type: "load_reference", path });
          }}
          onClose={() => setReferenceOpen(false)}
        />
      )}
      {timeJumpOpen && (
        <TimeJump
          maxS={traceRows.length > 0 ? traceRows[0].tUs / 1e6 : null}
          onGo={(seconds) => goToTimeUs(Math.round(seconds * 1e6))}
          onClose={() => setTimeJumpOpen(false)}
        />
      )}
    </div>
  );
}
