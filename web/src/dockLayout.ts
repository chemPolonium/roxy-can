// Fixed dock layouts: a handful of preset slot arrangements plus the panel
// assigned to each slot. This is deliberately not a window manager -- slots
// are fixed by the layout, panels move by menu, and every panel exists once.

export type PanelId =
  | "trace"
  | "messages"
  | "rate"
  | "signals"
  | "monitor"
  | "data"
  | "state"
  | "statistics"
  | "bus"
  | "spec"
  | "someip"
  | "write";

export type LayoutId = "focus" | "split" | "main-side" | "quad";

export interface DockRatios {
  x: number;
  y: number;
}

export interface DockState {
  layout: LayoutId;
  panels: Record<LayoutId, PanelId[]>;
  ratios: Record<LayoutId, DockRatios>;
}

export interface PanelInfo {
  id: PanelId;
  label: string;
  group: "Stream" | "Live" | "Analysis";
  hint: string;
}

export interface LayoutInfo {
  id: LayoutId;
  label: string;
  hint: string;
  slotLabels: string[];
  slots: number;
}

export const PANELS: PanelInfo[] = [
  { id: "trace", label: "Trace", group: "Stream", hint: "Raw frames and decoded rows" },
  { id: "messages", label: "Messages", group: "Stream", hint: "Per-ID counts, cycle and payload" },
  { id: "rate", label: "Rate", group: "Live", hint: "Frame rate and bus load, last 12 s" },
  { id: "signals", label: "Signals", group: "Live", hint: "Signal curves and cursor readout" },
  { id: "monitor", label: "Monitor", group: "Live", hint: "Glanceable values with thresholds" },
  { id: "data", label: "Data", group: "Live", hint: "Raw, scaled, min/avg/max per signal" },
  { id: "state", label: "State", group: "Live", hint: "Signal history as state bands" },
  { id: "statistics", label: "Messages stats", group: "Analysis", hint: "Cycle, jitter and share per message" },
  { id: "bus", label: "Bus load", group: "Analysis", hint: "Cumulative bus counters and burst data" },
  { id: "spec", label: "Spec", group: "Analysis", hint: "Measured traffic vs database declarations" },
  { id: "someip", label: "SOME/IP", group: "Analysis", hint: "Service/method traffic and availability" },
  { id: "write", label: "Write", group: "Analysis", hint: "System log: node output, warnings and errors" },
];

export const LAYOUTS: LayoutInfo[] = [
  {
    id: "focus",
    label: "Focus",
    hint: "One full-width page",
    slotLabels: ["Page"],
    slots: 1,
  },
  {
    id: "split",
    label: "Split",
    hint: "Two pages side by side",
    slotLabels: ["Left", "Right"],
    slots: 2,
  },
  {
    id: "main-side",
    label: "Main + side",
    hint: "Main page with two stacked side pages",
    slotLabels: ["Main", "Side top", "Side bottom"],
    slots: 3,
  },
  {
    id: "quad",
    label: "Quad",
    hint: "Four equal pages",
    slotLabels: ["Top left", "Top right", "Bottom left", "Bottom right"],
    slots: 4,
  },
];

const PANEL_IDS = new Set<string>(PANELS.map((panel) => panel.id));
const LAYOUT_IDS = new Set<string>(LAYOUTS.map((layout) => layout.id));

const DEFAULT_PANELS: Record<LayoutId, PanelId[]> = {
  focus: ["messages"],
  split: ["trace", "rate"],
  "main-side": ["messages", "rate", "statistics"],
  quad: ["trace", "signals", "rate", "statistics"],
};

const DEFAULT_RATIOS: Record<LayoutId, DockRatios> = {
  focus: { x: 0.5, y: 0.5 },
  split: { x: 0.5, y: 0.5 },
  "main-side": { x: 0.63, y: 0.56 },
  quad: { x: 0.5, y: 0.5 },
};

export const DEFAULT_DOCK: DockState = {
  layout: "main-side",
  panels: {
    focus: [...DEFAULT_PANELS.focus],
    split: [...DEFAULT_PANELS.split],
    "main-side": [...DEFAULT_PANELS["main-side"]],
    quad: [...DEFAULT_PANELS.quad],
  },
  ratios: {
    focus: { ...DEFAULT_RATIOS.focus },
    split: { ...DEFAULT_RATIOS.split },
    "main-side": { ...DEFAULT_RATIOS["main-side"] },
    quad: { ...DEFAULT_RATIOS.quad },
  },
};

export function isPanelId(value: unknown): value is PanelId {
  return typeof value === "string" && PANEL_IDS.has(value);
}

export function isLayoutId(value: unknown): value is LayoutId {
  return typeof value === "string" && LAYOUT_IDS.has(value);
}

export function layoutInfo(id: LayoutId): LayoutInfo {
  return LAYOUTS.find((layout) => layout.id === id) ?? LAYOUTS[2];
}

export function panelInfo(id: PanelId): PanelInfo {
  return PANELS.find((panel) => panel.id === id) ?? PANELS[0];
}

export function panelLabel(id: PanelId): string {
  return panelInfo(id).label;
}

export function defaultPanelsFor(id: LayoutId): PanelId[] {
  return [...DEFAULT_PANELS[id]];
}

export function defaultDock(): DockState {
  return {
    layout: "main-side",
    panels: {
      focus: [...DEFAULT_DOCK.panels.focus],
      split: [...DEFAULT_DOCK.panels.split],
      "main-side": [...DEFAULT_DOCK.panels["main-side"]],
      quad: [...DEFAULT_DOCK.panels.quad],
    },
    ratios: {
      focus: { ...DEFAULT_DOCK.ratios.focus },
      split: { ...DEFAULT_DOCK.ratios.split },
      "main-side": { ...DEFAULT_DOCK.ratios["main-side"] },
      quad: { ...DEFAULT_DOCK.ratios.quad },
    },
  };
}

function clamp(value: number, min: number, max: number): number {
  return Math.min(max, Math.max(min, value));
}

function ratioBounds(id: LayoutId): { x: [number, number]; y: [number, number] } {
  if (id === "main-side") return { x: [0.3, 0.8], y: [0.3, 0.8] };
  if (id === "quad") return { x: [0.25, 0.75], y: [0.25, 0.75] };
  if (id === "split") return { x: [0.25, 0.75], y: [0.25, 0.75] };
  return { x: [0.25, 0.75], y: [0.25, 0.75] };
}

export function normalizeRatios(id: LayoutId, raw: unknown): DockRatios {
  const fallback = DEFAULT_RATIOS[id];
  const source =
    raw && typeof raw === "object" ? (raw as Partial<DockRatios>) : {};
  const bounds = ratioBounds(id);
  const x = Number(source.x);
  const y = Number(source.y);
  return {
    x: Number.isFinite(x) ? clamp(x, bounds.x[0], bounds.x[1]) : fallback.x,
    y: Number.isFinite(y) ? clamp(y, bounds.y[0], bounds.y[1]) : fallback.y,
  };
}

export function normalizePanels(id: LayoutId, raw: unknown): PanelId[] {
  const slots = layoutInfo(id).slots;
  const defaults = defaultPanelsFor(id);
  const out: PanelId[] = [];
  if (Array.isArray(raw)) {
    for (const value of raw) {
      if (!isPanelId(value) || out.includes(value)) continue;
      out.push(value);
      if (out.length === slots) break;
    }
  }
  for (const fallback of defaults) {
    if (out.length === slots) break;
    if (!out.includes(fallback)) out.push(fallback);
  }
  for (const panel of PANELS) {
    if (out.length === slots) break;
    if (!out.includes(panel.id)) out.push(panel.id);
  }
  return out;
}

export interface LegacyDockView {
  mainView?: unknown;
  tab?: unknown;
  chartTab?: unknown;
  layout?: unknown;
}

function legacyPanels(view: LegacyDockView | undefined): PanelId[] {
  if (!view) return defaultPanelsFor("main-side");
  const main: PanelId =
    view.mainView === "trace" ? "trace" : "messages";
  const upper = isPanelId(view.chartTab)
    ? view.chartTab
    : "rate";
  const lowerCandidate =
    view.tab === "activity"
      ? "write"
      : isPanelId(view.tab)
        ? view.tab
        : "statistics";
  const lower: PanelId = ["statistics", "bus", "spec", "someip", "write"].includes(
    lowerCandidate,
  )
    ? lowerCandidate
    : "statistics";
  return [main, upper, lower];
}

/// Accepts both the current dock shape and the pre-dock `mainView`/`tab`/
/// `chartTab`/`layout` fields, so opening an old project keeps the user's
/// three visible pages instead of jumping to a hard-coded default.
export function normalizeDock(
  raw: unknown,
  legacy?: LegacyDockView,
): DockState {
  const source =
    raw && typeof raw === "object" ? (raw as Record<string, unknown>) : {};
  const layout = isLayoutId(source.layout) ? source.layout : "main-side";
  const rawPanels =
    source.panels && typeof source.panels === "object"
      ? (source.panels as Record<string, unknown>)
      : {};
  const rawRatios =
    source.ratios && typeof source.ratios === "object"
      ? (source.ratios as Record<string, unknown>)
      : {};
  const panels = {} as Record<LayoutId, PanelId[]>;
  const ratios = {} as Record<LayoutId, DockRatios>;
  for (const info of LAYOUTS) {
    panels[info.id] = normalizePanels(info.id, rawPanels[info.id]);
    ratios[info.id] = normalizeRatios(info.id, rawRatios[info.id]);
  }
  // No dock data at all: carry over the three pages that were visible in the
  // fixed shell and its two splitter ratios.
  if (!raw || typeof raw !== "object") {
    panels["main-side"] = normalizePanels("main-side", legacyPanels(legacy));
    const oldLayout =
      legacy?.layout && typeof legacy.layout === "object"
        ? (legacy.layout as Record<string, unknown>)
        : {};
    ratios["main-side"] = normalizeRatios("main-side", {
      x: oldLayout.main,
      y: oldLayout.right,
    });
    return { layout: "main-side", panels, ratios };
  }
  return { layout, panels, ratios };
}

export function panelsFor(dock: DockState): PanelId[] {
  return dock.panels[dock.layout];
}

export function slotLabels(dock: DockState): string[] {
  return layoutInfo(dock.layout).slotLabels;
}

/// Assign a panel to a slot. Selecting a panel that another slot already
/// shows swaps the two slots, so a panel is never rendered twice and the
/// displaced panel stays reachable.
export function assignPanel(
  dock: DockState,
  slot: number,
  panel: PanelId,
): DockState {
  const panels = [...dock.panels[dock.layout]];
  if (slot < 0 || slot >= panels.length) return dock;
  if (panels[slot] === panel) return dock;
  const existing = panels.indexOf(panel);
  if (existing >= 0) {
    panels[existing] = panels[slot];
  }
  panels[slot] = panel;
  return {
    ...dock,
    panels: { ...dock.panels, [dock.layout]: panels },
  };
}

/// Move the current slot's panel to another slot and bring the target's
/// panel back. This is a swap, therefore no page is lost and the total count
/// stays unchanged.
export function moveSlot(
  dock: DockState,
  from: number,
  to: number,
): DockState {
  const panels = [...dock.panels[dock.layout]];
  if (
    from === to ||
    from < 0 ||
    to < 0 ||
    from >= panels.length ||
    to >= panels.length
  ) {
    return dock;
  }
  [panels[from], panels[to]] = [panels[to], panels[from]];
  return {
    ...dock,
    panels: { ...dock.panels, [dock.layout]: panels },
  };
}

/// Reveal a panel from the command palette. If it is already on screen the
/// slot is returned untouched; otherwise it replaces `preferredSlot`.
export function showPanel(
  dock: DockState,
  panel: PanelId,
  preferredSlot = 0,
): { dock: DockState; slot: number } {
  const panels = dock.panels[dock.layout];
  const existing = panels.indexOf(panel);
  if (existing >= 0) return { dock, slot: existing };
  return { dock: assignPanel(dock, preferredSlot, panel), slot: preferredSlot };
}

export function setLayout(dock: DockState, layout: LayoutId): DockState {
  return { ...dock, layout };
}

export function setRatio(
  dock: DockState,
  layout: LayoutId,
  axis: keyof DockRatios,
  value: number,
): DockState {
  const next = { ...dock.ratios[layout], [axis]: value };
  return {
    ...dock,
    ratios: { ...dock.ratios, [layout]: normalizeRatios(layout, next) },
  };
}

export function resetLayoutPanels(dock: DockState): DockState {
  return {
    ...dock,
    panels: {
      ...dock.panels,
      [dock.layout]: defaultPanelsFor(dock.layout),
    },
  };
}
