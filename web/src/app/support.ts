//! Pure helpers shared by the App shell: saved-view normalization, local
//! storage access, small formatters and the UI-FPS hook.

import { useEffect, useState } from "preact/hooks";
import type { SignalSeries } from "../types";
import { normalizeFilter } from "../filterTree";
import type { FilterGroup } from "../filterTree";
import {
  normalizeFilterLibrary,
  type SavedFilter,
} from "../filterLibrary";
import { SIGNAL_FILTER_SPECS } from "../signalFilter";
import { MESSAGE_FILTER_SPECS } from "../messageFilter";
import { STATS_FILTER_SPECS } from "../statsFilter";
import { TRACE_FILTER_SPECS } from "../traceFilter";
import { normalizeDock, type DockState } from "../dockLayout";
import type { ChartCursor, SignalView } from "../components/SignalChart";
import type { ChartYMode } from "../chartScale";
import type { ReferenceColumns, ReferenceData } from "../components/ReferenceModal";

export type ShellMode = "standard" | "panel";
export type DockDrag = "x" | "y" | null;

export const LAYOUT_KEY = "roxy-can-web.layout";
export const VIEW_KEY = "roxy-can-web.view";
export const TRACE_CAP = 50_000;
export const SIGNAL_CAP = 12_000;

interface LegacyView {
  mainView?: unknown;
  tab?: unknown;
  chartTab?: unknown;
  chartTab2?: unknown;
  chartSplit?: unknown;
  splitRatio?: unknown;
  layout?: unknown;
}

interface ViewFilters {
  monitor: FilterGroup;
  data: FilterGroup;
  message: FilterGroup;
  trace: FilterGroup;
  signals: FilterGroup;
  state: FilterGroup;
  stats: FilterGroup;
}

export interface SavedView {
  shellMode: ShellMode;
  dock: DockState;
  overlay: boolean;
  signalView: SignalView;
  yModes?: Record<string, ChartYMode>;
  derivedReads?: { id: string; name: string }[];
  cursors: ChartCursor[];
  filters?: ViewFilters;
  filterLibrary?: SavedFilter[];
}

export interface SavedWorkspace extends Partial<SavedView>, LegacyView {
  traceColumns?: unknown;
  messageColumns?: unknown;
  statsColumns?: unknown;
  readSignals?: unknown;
}

export function normalizeView(
  saved: SavedWorkspace,
  legacyLayout?: unknown,
): Partial<SavedView> {
  const raw: SavedWorkspace =
    saved && typeof saved === "object" ? saved : {};
  const docking = normalizeDock(raw.dock, {
    mainView: raw.mainView,
    tab: raw.tab,
    chartTab: raw.chartTab,
    layout: raw.layout ?? legacyLayout,
  });
  const windowS = clamp(Number(raw.signalView?.windowS) || 30, 1, 120);
  const rawEnd = raw.signalView?.endS;
  const endS =
    typeof rawEnd === "number" && Number.isFinite(rawEnd) ? rawEnd : null;
  const cursors: ChartCursor[] = [];
  if (Array.isArray(raw.cursors)) {
    for (const cursor of raw.cursors.slice(0, 16)) {
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
  if (raw.yModes && typeof raw.yModes === "object") {
    for (const [key, mode] of Object.entries(raw.yModes).slice(0, 256)) {
      if (mode === "auto" || mode === "memory" || mode === "dbc") {
        yModes[key] = mode;
      }
    }
  }
  const derivedReads: { id: string; name: string }[] = [];
  if (Array.isArray(raw.derivedReads)) {
    for (const entry of raw.derivedReads.slice(0, 64)) {
      if (
        entry &&
        typeof entry.id === "string" &&
        entry.id &&
        typeof entry.name === "string" &&
        entry.name
      ) {
        derivedReads.push({ id: entry.id, name: entry.name });
      }
    }
  }
  const shellMode: ShellMode =
    raw.shellMode === "panel" || raw.shellMode === "standard"
      ? raw.shellMode
      : raw.mainView === "panel"
        ? "panel"
        : "standard";
  const hasFilters = !!raw.filters && typeof raw.filters === "object";
  const rawFilters = hasFilters
    ? (raw.filters as unknown as Record<string, unknown>)
    : {};
  const filters: ViewFilters = {
    monitor: normalizeFilter(rawFilters.monitor, SIGNAL_FILTER_SPECS),
    data: normalizeFilter(rawFilters.data, SIGNAL_FILTER_SPECS),
    message: normalizeFilter(rawFilters.message, MESSAGE_FILTER_SPECS),
    trace: normalizeFilter(rawFilters.trace, TRACE_FILTER_SPECS),
    signals: normalizeFilter(rawFilters.signals, SIGNAL_FILTER_SPECS),
    state: normalizeFilter(rawFilters.state, SIGNAL_FILTER_SPECS),
    stats: normalizeFilter(rawFilters.stats, STATS_FILTER_SPECS),
  };
  const hasLibrary = Array.isArray(raw.filterLibrary);
  const filterLibrary = normalizeFilterLibrary(raw.filterLibrary);
  return {
    shellMode,
    dock: docking,
    overlay: typeof raw.overlay === "boolean" ? raw.overlay : false,
    yModes,
    derivedReads,
    signalView: { windowS, endS },
    cursors,
    ...(hasFilters ? { filters } : {}),
    ...(hasLibrary ? { filterLibrary } : {}),
  };
}

function loadLegacyLayout(): unknown {
  try {
    const raw = localStorage.getItem(LAYOUT_KEY);
    return raw ? JSON.parse(raw) : undefined;
  } catch {
    return undefined;
  }
}

export function loadView(): Partial<SavedView> {
  try {
    const raw = localStorage.getItem(VIEW_KEY);
    if (!raw) return {};
    const parsed = JSON.parse(raw) as SavedWorkspace;
    return normalizeView(parsed, parsed.layout ?? loadLegacyLayout());
  } catch {
    return {};
  }
}

export interface ReferenceRun extends ReferenceData {
  series: Record<string, [number, number][]>;
}

/// Aligned-CSV headers are `bus:signal`; match them to the live lanes, with
/// a unique-name fallback when the bus labels differ.
export function mapReferenceColumns(
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

export function clamp(value: number, min: number, max: number): number {
  return Math.min(max, Math.max(min, value));
}

export function seconds(us: number, digits = 3): string {
  return (us / 1e6).toFixed(digits);
}

export function useUiFps(): number {
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

export function baseName(path: string): string {
  const cut = Math.max(path.lastIndexOf("\\"), path.lastIndexOf("/"));
  return cut >= 0 ? path.slice(cut + 1) : path;
}

export function shortDateTime(ms: number): string {
  if (!ms) return "—";
  const d = new Date(ms);
  const p = (value: number) => String(value).padStart(2, "0");
  return `${d.getFullYear()}-${p(d.getMonth() + 1)}-${p(d.getDate())} ${p(d.getHours())}:${p(d.getMinutes())}`;
}

export function shortSize(bytes: number): string {
  if (bytes >= 1024 * 1024) return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
  if (bytes >= 1024) return `${(bytes / 1024).toFixed(0)} KB`;
  return `${bytes} B`;
}

const RECENT_PROJECTS_KEY = "roxy-can-web.recentProjects";

export function loadLocalRecentProjects(): string[] {
  try {
    const raw = localStorage.getItem(RECENT_PROJECTS_KEY);
    const parsed: unknown = raw ? JSON.parse(raw) : [];
    return Array.isArray(parsed)
      ? parsed
          .filter((path): path is string => typeof path === "string")
          .slice(0, 8)
      : [];
  } catch {
    return [];
  }
}

export function saveLocalRecentProjects(paths: string[]): void {
  try {
    localStorage.setItem(RECENT_PROJECTS_KEY, JSON.stringify(paths));
  } catch {
    // Private mode can reject storage; the session still works.
  }
}

export function fileNameOf(path: string): string {
  const cut = Math.max(path.lastIndexOf("\\"), path.lastIndexOf("/"));
  return cut >= 0 ? path.slice(cut + 1) : path;
}

export function handleMenuKey(
  event: KeyboardEvent,
  panel: HTMLElement | null,
  close: () => void,
  trigger: HTMLElement | null,
): void {
  if (!panel) return;
  const items = [
    ...panel.querySelectorAll<HTMLButtonElement>("button:not(:disabled)"),
  ];
  if (items.length === 0) return;
  const active = document.activeElement;
  const index = items.indexOf(active as HTMLButtonElement);
  switch (event.key) {
    case "ArrowDown":
      event.preventDefault();
      items[index < 0 ? 0 : (index + 1) % items.length].focus();
      break;
    case "ArrowUp":
      event.preventDefault();
      items[
        index < 0 ? items.length - 1 : (index - 1 + items.length) % items.length
      ].focus();
      break;
    case "Home":
      event.preventDefault();
      items[0].focus();
      break;
    case "End":
      event.preventDefault();
      items[items.length - 1].focus();
      break;
    case "Escape":
      event.preventDefault();
      close();
      trigger?.focus();
      break;
  }
}

export function clock(wallUs: number): string {
  const total = wallUs / 1e6;
  const h = Math.floor(total / 3600);
  const m = Math.floor((total % 3600) / 60);
  const s = Math.floor(total % 60);
  const ms = Math.floor((wallUs % 1e6) / 1000);
  return `${String(h).padStart(2, "0")}:${String(m).padStart(2, "0")}:${String(s).padStart(2, "0")}.${String(ms).padStart(3, "0")}`;
}
