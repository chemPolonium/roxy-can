import type { Command } from "./types";

export const PAGE_GRID = 8;
export const PAGE_WIDTH = 160;
export const PAGE_HEIGHT = 90;
export const PAGE_MIN_W = 40;
export const PAGE_MIN_H = 20;
export const PAGE_MAX_W = 400;
export const PAGE_MAX_H = 300;

export type PanelAction =
  | "start"
  | "stop"
  | "pause"
  | "resume"
  | "clear_trace"
  | "rearm_triggers";

/// What a Panel button runs: an existing web command or a named script
/// handler (`on panel_action("…")`).
export type PanelActionKind = PanelAction | "script";

export const PANEL_ACTIONS: { value: PanelAction; label: string }[] = [
  { value: "start", label: "Start" },
  { value: "stop", label: "Stop" },
  { value: "pause", label: "Pause" },
  { value: "resume", label: "Resume" },
  { value: "clear_trace", label: "Clear trace" },
  { value: "rearm_triggers", label: "Rearm triggers" },
];

export type PanelWidgetKind =
  | "button"
  | "value"
  | "toggle"
  | "numeric"
  | "lamp"
  | "bar"
  | "gauge";

interface Geometry {
  x: number;
  y: number;
  w: number;
  h: number;
}

export interface PanelButton extends Geometry {
  id: string;
  kind: "button";
  label: string;
  action: PanelActionKind;
  /// Handler name for `action === "script"`.
  script?: string;
}

export type PanelBandColor = "neutral" | "ok" | "warn" | "crit";

/// Ascending "up to" thresholds; null colours everything above the last one.
export interface PanelBand {
  upTo: number | null;
  color: PanelBandColor;
}

/// Read-only value tile: signal key, `derived:Name` or `sysvar:ns::name`.
export interface PanelValue extends Geometry {
  id: string;
  kind: "value";
  label: string;
  source: string;
  digits: number;
  bands?: PanelBand[];
}

/// Boolean tile bound to a system variable (0/1).
export interface PanelToggle extends Geometry {
  id: string;
  kind: "toggle";
  label: string;
  sysvar: string;
}

/// Numeric setpoint bound to a system variable, clamped to its min/max.
export interface PanelNumeric extends Geometry {
  id: string;
  kind: "numeric";
  label: string;
  sysvar: string;
  digits: number;
  step: number;
}

/// Read-only indicator: lit when the source is at or above `threshold`.
export interface PanelLamp extends Geometry {
  id: string;
  kind: "lamp";
  label: string;
  source: string;
  threshold: number;
  color: PanelBandColor;
}

interface PanelMeter extends Geometry {
  id: string;
  label: string;
  source: string;
  min: number;
  max: number;
  digits: number;
  bands?: PanelBand[];
}

/// Horizontal fill over a fixed range.
export interface PanelBar extends PanelMeter {
  kind: "bar";
}

/// Semicircular arc over a fixed range.
export interface PanelGauge extends PanelMeter {
  kind: "gauge";
}

export type PanelWidget =
  | PanelButton
  | PanelValue
  | PanelToggle
  | PanelNumeric
  | PanelLamp
  | PanelBar
  | PanelGauge;

export interface PanelPage {
  version: 1;
  id: string;
  name: string;
  width: number;
  height: number;
  widgets: PanelWidget[];
}

export interface PanelBook {
  version: 1;
  activeId: string;
  pages: PanelPage[];
}

export interface PanelWidgetPatch {
  label?: string;
  action?: PanelActionKind;
  script?: string;
  source?: string;
  digits?: number;
  sysvar?: string;
  step?: number;
  bands?: PanelBand[];
  threshold?: number;
  color?: PanelBandColor;
  min?: number;
  max?: number;
}

export type PanelFieldKey = keyof PanelWidgetPatch;

export interface PanelFieldSpec {
  key: PanelFieldKey;
  label: string;
  kind: "text" | "select" | "list" | "digits" | "number" | "bands";
  options?: { value: string; label: string }[];
  placeholder?: string;
  maxLength?: number;
  listId?: string;
}

export interface PanelWidgetSpec {
  kind: PanelWidgetKind;
  label: string;
  minW: number;
  minH: number;
  defaultW: number;
  defaultH: number;
  fields: PanelFieldSpec[];
  create: (id: string, x: number, y: number, w: number, h: number) => PanelWidget;
  fieldValue: (widget: PanelWidget, key: PanelFieldKey) => string | number;
  applyPatch: (widget: PanelWidget, patch: PanelWidgetPatch) => PanelWidget;
}

export function panelActionCommand(action: PanelAction): Command | null {
  switch (action) {
    case "start":
      return { type: "start" };
    case "stop":
      return { type: "stop" };
    case "pause":
      return { type: "pause", on: true };
    case "resume":
      return { type: "pause", on: false };
    case "clear_trace":
      return { type: "clear_trace" };
    case "rearm_triggers":
      return { type: "rearm_triggers" };
    default:
      return null;
  }
}

/// "ns::name" -> parts, or null when the text is not a system variable.
export function parseSysVarRef(
  text: string,
): { namespace: string; name: string } | null {
  const trimmed = text.trim();
  const at = trimmed.indexOf("::");
  if (at <= 0 || at + 2 >= trimmed.length) return null;
  return {
    namespace: trimmed.slice(0, at),
    name: trimmed.slice(at + 2),
  };
}

export function formatPanelValue(value: number | null, digits: number): string {
  if (value === null || !Number.isFinite(value)) return "—";
  return value.toFixed(clamp(Math.round(digits), 0, 3));
}

export const PANEL_BAND_COLORS: { value: PanelBandColor; label: string }[] = [
  { value: "neutral", label: "Neutral" },
  { value: "ok", label: "OK" },
  { value: "warn", label: "Warning" },
  { value: "crit", label: "Critical" },
];

export const PANEL_MAX_BANDS = 8;

function isBandColor(value: unknown): value is PanelBandColor {
  return PANEL_BAND_COLORS.some((entry) => entry.value === value);
}

function isPanelKind(value: unknown): value is PanelWidgetKind {
  return (
    value === "button" ||
    value === "value" ||
    value === "toggle" ||
    value === "numeric" ||
    value === "lamp" ||
    value === "bar" ||
    value === "gauge"
  );
}

/// Keeps valid bands, sorts them by threshold and drops duplicate limits.
export function normalisePanelBands(raw: unknown): PanelBand[] | undefined {
  if (!Array.isArray(raw)) return undefined;
  const bands: PanelBand[] = [];
  for (const entry of raw.slice(0, PANEL_MAX_BANDS)) {
    if (!entry || typeof entry !== "object") continue;
    const value = entry as Record<string, unknown>;
    if (!isBandColor(value.color)) continue;
    const upTo = value.upTo === null ? null : Number(value.upTo);
    if (upTo !== null && !Number.isFinite(upTo)) continue;
    if (bands.some((band) => band.upTo === upTo)) continue;
    bands.push({ upTo, color: value.color });
  }
  bands.sort((a, b) => {
    if (a.upTo === null) return b.upTo === null ? 0 : 1;
    if (b.upTo === null) return -1;
    return a.upTo - b.upTo;
  });
  return bands.length > 0 ? bands : undefined;
}

function clamp(value: number, min: number, max: number): number {
  return Math.min(max, Math.max(min, value));
}

function isAction(value: unknown): value is PanelAction {
  return PANEL_ACTIONS.some((action) => action.value === value);
}

function isActionKind(value: unknown): value is PanelActionKind {
  return value === "script" || isAction(value);
}

function text(value: unknown, cap: number): string {
  return typeof value === "string" ? value.slice(0, cap) : "";
}

function overlaps(a: Geometry, b: Geometry): boolean {
  return a.x < b.x + b.w && b.x < a.x + a.w && a.y < b.y + b.h && b.y < a.y + a.h;
}

const BUTTON_FIELDS: PanelFieldSpec[] = [
  { key: "label", label: "Label", kind: "text", maxLength: 80 },
  {
    key: "action",
    label: "Action",
    kind: "select",
    options: [
      ...PANEL_ACTIONS.map((action) => ({
        value: action.value as string,
        label: action.label,
      })),
      { value: "script", label: "Script action" },
    ],
  },
  {
    key: "script",
    label: "Script action",
    kind: "text",
    maxLength: 80,
    placeholder: 'on panel_action("…")',
  },
];

const SOURCE_FIELD: PanelFieldSpec = {
  key: "source",
  label: "Source",
  kind: "list",
  maxLength: 120,
  listId: "page-sources",
  placeholder: "signal key / derived:Name / sysvar:ns::name",
};

const VALUE_FIELDS: PanelFieldSpec[] = [
  { key: "label", label: "Label", kind: "text", maxLength: 80 },
  SOURCE_FIELD,
  { key: "digits", label: "Decimals", kind: "digits" },
  { key: "bands", label: "Color bands", kind: "bands" },
];

const TOGGLE_FIELDS: PanelFieldSpec[] = [
  { key: "label", label: "Label", kind: "text", maxLength: 80 },
  {
    key: "sysvar",
    label: "System variable",
    kind: "list",
    maxLength: 120,
    listId: "page-sysvars",
    placeholder: "namespace::name",
  },
];

const NUMERIC_FIELDS: PanelFieldSpec[] = [
  { key: "label", label: "Label", kind: "text", maxLength: 80 },
  {
    key: "sysvar",
    label: "System variable",
    kind: "list",
    maxLength: 120,
    listId: "page-sysvars",
    placeholder: "namespace::name",
  },
  { key: "digits", label: "Decimals", kind: "digits" },
  {
    key: "step",
    label: "Step",
    kind: "number",
    placeholder: "1",
  },
];

const LAMP_FIELDS: PanelFieldSpec[] = [
  { key: "label", label: "Label", kind: "text", maxLength: 80 },
  SOURCE_FIELD,
  {
    key: "threshold",
    label: "Lit at or above",
    kind: "number",
    placeholder: "0.5",
  },
  {
    key: "color",
    label: "Lit color",
    kind: "select",
    options: PANEL_BAND_COLORS.map((entry) => ({
      value: entry.value as string,
      label: entry.label,
    })),
  },
];

const METER_FIELDS: PanelFieldSpec[] = [
  { key: "label", label: "Label", kind: "text", maxLength: 80 },
  SOURCE_FIELD,
  { key: "min", label: "Minimum", kind: "number", placeholder: "0" },
  { key: "max", label: "Maximum", kind: "number", placeholder: "100" },
  { key: "digits", label: "Decimals", kind: "digits" },
  { key: "bands", label: "Color bands", kind: "bands" },
];

const BUTTON_SPEC: PanelWidgetSpec = {
  kind: "button",
  label: "Button",
  minW: 12,
  minH: 4,
  defaultW: 20,
  defaultH: 6,
  fields: BUTTON_FIELDS,
  create: (id, x, y, w, h) => ({
    id,
    kind: "button",
    label: "Button",
    action: "start",
    x,
    y,
    w,
    h,
  }),
  fieldValue: (widget, key) => {
    if (widget.kind !== "button") return "";
    if (key === "action") return widget.action;
    if (key === "script") return widget.script ?? "";
    if (key === "label") return widget.label;
    return "";
  },
  applyPatch: (widget, patch) => {
    if (widget.kind !== "button") return widget;
    return {
      ...widget,
      label: patch.label === undefined ? widget.label : patch.label.slice(0, 80),
      action:
        patch.action !== undefined && isActionKind(patch.action)
          ? patch.action
          : widget.action,
      script:
        patch.script === undefined
          ? widget.script
          : patch.script.slice(0, 80),
    };
  },
};

const VALUE_SPEC: PanelWidgetSpec = {
  kind: "value",
  label: "Value",
  minW: 12,
  minH: 5,
  defaultW: 16,
  defaultH: 6,
  fields: VALUE_FIELDS,
  create: (id, x, y, w, h) => ({
    id,
    kind: "value",
    label: "Value",
    source: "",
    digits: 1,
    x,
    y,
    w,
    h,
  }),
  fieldValue: (widget, key) => {
    if (widget.kind !== "value") return "";
    if (key === "source") return widget.source;
    if (key === "digits") return widget.digits;
    if (key === "label") return widget.label;
    return "";
  },
  applyPatch: (widget, patch) => {
    if (widget.kind !== "value") return widget;
    return {
      ...widget,
      label: patch.label === undefined ? widget.label : patch.label.slice(0, 80),
      source:
        patch.source === undefined
          ? widget.source
          : patch.source.slice(0, 120),
      digits:
        patch.digits === undefined
          ? widget.digits
          : clamp(Math.round(patch.digits), 0, 3),
      bands:
        patch.bands === undefined
          ? widget.bands
          : normalisePanelBands(patch.bands),
    };
  },
};

const TOGGLE_SPEC: PanelWidgetSpec = {
  kind: "toggle",
  label: "Toggle",
  minW: 12,
  minH: 4,
  defaultW: 16,
  defaultH: 5,
  fields: TOGGLE_FIELDS,
  create: (id, x, y, w, h) => ({
    id,
    kind: "toggle",
    label: "Toggle",
    sysvar: "",
    x,
    y,
    w,
    h,
  }),
  fieldValue: (widget, key) => {
    if (widget.kind !== "toggle") return "";
    if (key === "sysvar") return widget.sysvar;
    if (key === "label") return widget.label;
    return "";
  },
  applyPatch: (widget, patch) => {
    if (widget.kind !== "toggle") return widget;
    return {
      ...widget,
      label: patch.label === undefined ? widget.label : patch.label.slice(0, 80),
      sysvar:
        patch.sysvar === undefined
          ? widget.sysvar
          : patch.sysvar.slice(0, 120),
    };
  },
};

const NUMERIC_SPEC: PanelWidgetSpec = {
  kind: "numeric",
  label: "Numeric",
  minW: 14,
  minH: 5,
  defaultW: 18,
  defaultH: 6,
  fields: NUMERIC_FIELDS,
  create: (id, x, y, w, h) => ({
    id,
    kind: "numeric",
    label: "Setpoint",
    sysvar: "",
    digits: 1,
    step: 1,
    x,
    y,
    w,
    h,
  }),
  fieldValue: (widget, key) => {
    if (widget.kind !== "numeric") return "";
    if (key === "sysvar") return widget.sysvar;
    if (key === "digits") return widget.digits;
    if (key === "step") return widget.step;
    if (key === "label") return widget.label;
    return "";
  },
  applyPatch: (widget, patch) => {
    if (widget.kind !== "numeric") return widget;
    const step = Number(patch.step);
    return {
      ...widget,
      label: patch.label === undefined ? widget.label : patch.label.slice(0, 80),
      sysvar:
        patch.sysvar === undefined
          ? widget.sysvar
          : patch.sysvar.slice(0, 120),
      digits:
        patch.digits === undefined
          ? widget.digits
          : clamp(Math.round(patch.digits), 0, 3),
      step:
        patch.step === undefined || !Number.isFinite(step) || step <= 0
          ? widget.step
          : Math.min(step, 1e9),
    };
  },
};

const LAMP_SPEC: PanelWidgetSpec = {
  kind: "lamp",
  label: "Lamp",
  minW: 10,
  minH: 3,
  defaultW: 14,
  defaultH: 4,
  fields: LAMP_FIELDS,
  create: (id, x, y, w, h) => ({
    id,
    kind: "lamp",
    label: "Lamp",
    source: "",
    threshold: 0.5,
    color: "ok",
    x,
    y,
    w,
    h,
  }),
  fieldValue: (widget, key) => {
    if (widget.kind !== "lamp") return "";
    if (key === "source") return widget.source;
    if (key === "threshold") return widget.threshold;
    if (key === "color") return widget.color;
    if (key === "label") return widget.label;
    return "";
  },
  applyPatch: (widget, patch) => {
    if (widget.kind !== "lamp") return widget;
    const threshold = Number(patch.threshold);
    return {
      ...widget,
      label: patch.label === undefined ? widget.label : patch.label.slice(0, 80),
      source:
        patch.source === undefined
          ? widget.source
          : patch.source.slice(0, 120),
      threshold:
        patch.threshold === undefined || !Number.isFinite(threshold)
          ? widget.threshold
          : threshold,
      color:
        patch.color !== undefined && isBandColor(patch.color)
          ? patch.color
          : widget.color,
    };
  },
};

function meterFieldValue(
  widget: PanelBar | PanelGauge,
  key: PanelFieldKey,
): string | number {
  if (key === "source") return widget.source;
  if (key === "min") return widget.min;
  if (key === "max") return widget.max;
  if (key === "digits") return widget.digits;
  if (key === "label") return widget.label;
  return "";
}

function meterPatch(widget: PanelBar | PanelGauge, patch: PanelWidgetPatch) {
  const min = Number(patch.min);
  const max = Number(patch.max);
  return {
    label: patch.label === undefined ? widget.label : patch.label.slice(0, 80),
    source:
      patch.source === undefined ? widget.source : patch.source.slice(0, 120),
    min: patch.min === undefined || !Number.isFinite(min) ? widget.min : min,
    max: patch.max === undefined || !Number.isFinite(max) ? widget.max : max,
    digits:
      patch.digits === undefined
        ? widget.digits
        : clamp(Math.round(patch.digits), 0, 3),
    bands:
      patch.bands === undefined
        ? widget.bands
        : normalisePanelBands(patch.bands),
  };
}

const BAR_SPEC: PanelWidgetSpec = {
  kind: "bar",
  label: "Bar",
  minW: 14,
  minH: 5,
  defaultW: 20,
  defaultH: 7,
  fields: METER_FIELDS,
  create: (id, x, y, w, h) => ({
    id,
    kind: "bar",
    label: "Bar",
    source: "",
    min: 0,
    max: 100,
    digits: 1,
    x,
    y,
    w,
    h,
  }),
  fieldValue: (widget, key) =>
    widget.kind === "bar" ? meterFieldValue(widget, key) : "",
  applyPatch: (widget, patch) =>
    widget.kind === "bar" ? { ...widget, ...meterPatch(widget, patch) } : widget,
};

const GAUGE_SPEC: PanelWidgetSpec = {
  kind: "gauge",
  label: "Gauge",
  minW: 12,
  minH: 8,
  defaultW: 18,
  defaultH: 11,
  fields: METER_FIELDS,
  create: (id, x, y, w, h) => ({
    id,
    kind: "gauge",
    label: "Gauge",
    source: "",
    min: 0,
    max: 100,
    digits: 1,
    x,
    y,
    w,
    h,
  }),
  fieldValue: (widget, key) =>
    widget.kind === "gauge" ? meterFieldValue(widget, key) : "",
  applyPatch: (widget, patch) =>
    widget.kind === "gauge"
      ? { ...widget, ...meterPatch(widget, patch) }
      : widget,
};

export const PANEL_WIDGET_SPECS: Record<PanelWidgetKind, PanelWidgetSpec> = {
  button: BUTTON_SPEC,
  value: VALUE_SPEC,
  toggle: TOGGLE_SPEC,
  numeric: NUMERIC_SPEC,
  lamp: LAMP_SPEC,
  bar: BAR_SPEC,
  gauge: GAUGE_SPEC,
};

export const PANEL_WIDGET_KINDS: { kind: PanelWidgetKind; label: string }[] = [
  { kind: "button", label: BUTTON_SPEC.label },
  { kind: "value", label: VALUE_SPEC.label },
  { kind: "bar", label: BAR_SPEC.label },
  { kind: "gauge", label: GAUGE_SPEC.label },
  { kind: "lamp", label: LAMP_SPEC.label },
  { kind: "toggle", label: TOGGLE_SPEC.label },
  { kind: "numeric", label: NUMERIC_SPEC.label },
];

export function panelWidgetSpec(kind: PanelWidgetKind): PanelWidgetSpec {
  return PANEL_WIDGET_SPECS[kind];
}

export function widgetMinSize(kind: PanelWidgetKind): { w: number; h: number } {
  const spec = panelWidgetSpec(kind);
  return { w: spec.minW, h: spec.minH };
}

export function defaultPanelPage(): PanelPage {
  const start = BUTTON_SPEC.create("start", 8, 8, 20, 6) as PanelButton;
  start.label = "Start";
  start.action = "start";
  const stop = BUTTON_SPEC.create("stop", 8, 16, 20, 6) as PanelButton;
  stop.label = "Stop";
  stop.action = "stop";
  const clear = BUTTON_SPEC.create("clear", 8, 24, 20, 6) as PanelButton;
  clear.label = "Clear trace";
  clear.action = "clear_trace";
  return {
    version: 1,
    id: "page1",
    name: "Overview",
    width: PAGE_WIDTH,
    height: PAGE_HEIGHT,
    widgets: [start, stop, clear],
  };
}

export function defaultPanelBook(): PanelBook {
  const page = defaultPanelPage();
  return { version: 1, activeId: page.id, pages: [page] };
}

export function activePanelPage(book: PanelBook): PanelPage {
  return book.pages.find((page) => page.id === book.activeId) ?? book.pages[0];
}

function pageId(book: PanelBook): string {
  let n = book.pages.length + 1;
  const ids = new Set(book.pages.map((page) => page.id));
  while (ids.has(`page${n}`)) n += 1;
  return `page${n}`;
}

function uniquePageName(book: PanelBook, wanted: string): string {
  const base = wanted.trim().slice(0, 40) || "Page";
  if (!book.pages.some((page) => page.name === base)) return base;
  let n = 2;
  while (book.pages.some((page) => page.name === `${base} ${n}`)) n += 1;
  return `${base} ${n}`;
}

export function updatePanelPage(book: PanelBook, page: PanelPage): PanelBook {
  return {
    ...book,
    pages: book.pages.map((entry) => (entry.id === page.id ? page : entry)),
  };
}

export function selectPanelPage(book: PanelBook, id: string): PanelBook {
  return book.pages.some((page) => page.id === id)
    ? { ...book, activeId: id }
    : book;
}

export function addPanelPage(book: PanelBook): { book: PanelBook; id: string } {
  const current = activePanelPage(book);
  const id = pageId(book);
  const page: PanelPage = {
    version: 1,
    id,
    name: uniquePageName(book, `Page ${book.pages.length + 1}`),
    width: current.width,
    height: current.height,
    widgets: [],
  };
  return {
    book: { ...book, activeId: id, pages: [...book.pages, page] },
    id,
  };
}

export function duplicatePanelPage(
  book: PanelBook,
  id: string,
): { book: PanelBook; id: string } | null {
  const index = book.pages.findIndex((page) => page.id === id);
  if (index < 0) return null;
  const source = book.pages[index];
  const copyId = pageId(book);
  const copy: PanelPage = {
    ...source,
    id: copyId,
    name: uniquePageName(book, `${source.name} copy`),
    widgets: source.widgets.map((widget) => ({ ...widget })),
  };
  const pages = [...book.pages];
  pages.splice(index + 1, 0, copy);
  return { book: { ...book, activeId: copyId, pages }, id: copyId };
}

export function renamePanelPage(
  book: PanelBook,
  id: string,
  name: string,
): PanelBook {
  const trimmed = name.trim().slice(0, 40);
  if (!trimmed) return book;
  return {
    ...book,
    pages: book.pages.map((page) =>
      page.id === id ? { ...page, name: trimmed } : page,
    ),
  };
}

export function removePanelPage(book: PanelBook, id: string): PanelBook {
  if (book.pages.length <= 1) return book;
  const index = book.pages.findIndex((page) => page.id === id);
  if (index < 0) return book;
  const pages = book.pages.filter((page) => page.id !== id);
  const activeId =
    book.activeId === id
      ? pages[Math.min(index, pages.length - 1)].id
      : book.activeId;
  return { ...book, activeId, pages };
}

function widgetPatch(value: Record<string, unknown>): PanelWidgetPatch {
  const kind = value.kind as PanelWidgetKind;
  return {
    label: text(value.label, 80),
    ...(kind === "button"
      ? {
          action: value.action as PanelActionKind,
          script: text(value.script, 80),
        }
      : {}),
    ...(kind === "value"
      ? {
          source: text(value.source, 120),
          digits: clamp(Math.round(Number(value.digits) || 0), 0, 3),
          bands: normalisePanelBands(value.bands),
        }
      : {}),
    ...(kind === "toggle" ? { sysvar: text(value.sysvar, 120) } : {}),
    ...(kind === "numeric"
      ? {
          sysvar: text(value.sysvar, 120),
          digits: clamp(Math.round(Number(value.digits) || 0), 0, 3),
          step: Number(value.step),
        }
      : {}),
    ...(kind === "lamp"
      ? {
          source: text(value.source, 120),
          threshold: Number(value.threshold),
          ...(isBandColor(value.color) ? { color: value.color } : {}),
        }
      : {}),
    ...(kind === "bar" || kind === "gauge"
      ? {
          source: text(value.source, 120),
          min: Number(value.min),
          max: Number(value.max),
          digits: clamp(Math.round(Number(value.digits) || 0), 0, 3),
          bands: normalisePanelBands(value.bands),
        }
      : {}),
  };
}

function buildWidget(
  kind: PanelWidgetKind,
  id: string,
  geometry: Geometry,
  value: Record<string, unknown>,
): PanelWidget {
  const spec = panelWidgetSpec(kind);
  const widget = spec.create(
    id,
    geometry.x,
    geometry.y,
    geometry.w,
    geometry.h,
  );
  return spec.applyPatch(widget, widgetPatch(value));
}

function sanitise(
  raw: unknown,
  width: number,
  height: number,
  used: Set<string>,
): PanelWidget | null {
  if (!raw || typeof raw !== "object") return null;
  const value = raw as Record<string, unknown>;
  const kind = value.kind;
  if (!isPanelKind(kind)) return null;
  if (typeof value.id !== "string" || value.id.length === 0) return null;
  if (used.has(value.id)) return null;
  const x = Math.round(Number(value.x));
  const y = Math.round(Number(value.y));
  const w = Math.round(Number(value.w));
  const h = Math.round(Number(value.h));
  if (![x, y, w, h].every(Number.isFinite)) return null;
  const spec = panelWidgetSpec(kind);
  if (w < spec.minW || h < spec.minH) return null;
  if (w > width || h > height) return null;
  const geometry = {
    x: clamp(x, 0, width - w),
    y: clamp(y, 0, height - h),
    w,
    h,
  };
  if (kind === "button" && !isActionKind(value.action)) return null;
  return buildWidget(kind, value.id, geometry, value);
}

export function parsePanelPage(raw: unknown): PanelPage | null {
  if (!raw || typeof raw !== "object") return null;
  const value = raw as Record<string, unknown>;
  const width = Math.round(Number(value.width));
  const height = Math.round(Number(value.height));
  const pageWidth =
    Number.isFinite(width) && width >= PAGE_MIN_W
      ? clamp(width, PAGE_MIN_W, PAGE_MAX_W)
      : PAGE_WIDTH;
  const pageHeight =
    Number.isFinite(height) && height >= PAGE_MIN_H
      ? clamp(height, PAGE_MIN_H, PAGE_MAX_H)
      : PAGE_HEIGHT;
  const widgets: PanelWidget[] = [];
  const used = new Set<string>();
  const source = Array.isArray(value.widgets) ? value.widgets : [];
  for (const entry of source.slice(0, 100)) {
    const widget = sanitise(entry, pageWidth, pageHeight, used);
    if (!widget) continue;
    if (widgets.some((other) => overlaps(widget, other))) continue;
    used.add(widget.id);
    widgets.push(widget);
  }
  return {
    version: 1,
    id: text(value.id, 40) || "page1",
    name: text(value.name, 40).trim() || "Overview",
    width: pageWidth,
    height: pageHeight,
    widgets,
  };
}

export function parsePanelBook(
  raw: unknown,
  legacyPage?: unknown,
): PanelBook | null {
  const fallback = parsePanelPage(legacyPage);
  if (!raw || typeof raw !== "object") {
    return fallback ? { version: 1, activeId: fallback.id, pages: [fallback] } : null;
  }
  const value = raw as Record<string, unknown>;
  const source = Array.isArray(value.pages) ? value.pages : [];
  const pages: PanelPage[] = [];
  const ids = new Set<string>();
  const names = new Set<string>();
  for (const entry of source.slice(0, 50)) {
    const page = parsePanelPage(entry);
    if (!page) continue;
    let id = page.id;
    let n = pages.length + 1;
    while (ids.has(id)) id = `page${n++}`;
    let name = page.name;
    let suffix = 2;
    while (names.has(name)) name = `${page.name} ${suffix++}`;
    ids.add(id);
    names.add(name);
    pages.push({ ...page, id, name });
  }
  if (pages.length === 0) {
    return fallback
      ? { version: 1, activeId: fallback.id, pages: [fallback] }
      : null;
  }
  const wanted = typeof value.activeId === "string" ? value.activeId : "";
  const activeId = pages.some((page) => page.id === wanted)
    ? wanted
    : pages[0].id;
  return { version: 1, activeId, pages };
}

export const PANEL_FILE_FORMAT = "roxy-can.panel-page";
export const PANEL_FILE_VERSION = 1;

export function panelFileName(name: string): string {
  const base = name
    .trim()
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, "-")
    .replace(/^-+|-+$/g, "");
  return `${base || "panel"}.rxpanel`;
}

/// Page as a portable, versioned file; widget ids stay page-local.
export function exportPanelFile(page: PanelPage): string {
  return JSON.stringify(
    {
      format: PANEL_FILE_FORMAT,
      version: PANEL_FILE_VERSION,
      page: {
        name: page.name,
        width: page.width,
        height: page.height,
        widgets: page.widgets.map((widget) => {
          const copy: Record<string, unknown> = { ...widget };
          delete copy.id;
          return copy;
        }),
      },
    },
    null,
    2,
  );
}

function strictWidget(
  raw: unknown,
  index: number,
  width: number,
  height: number,
  placed: Geometry[],
  id: string,
): PanelWidget | { error: string } {
  const where = `Control ${index + 1}`;
  if (!raw || typeof raw !== "object") {
    return { error: `${where}: not an object` };
  }
  const value = raw as Record<string, unknown>;
  if (!isPanelKind(value.kind)) {
    return { error: `${where}: unknown type "${String(value.kind ?? "")}"` };
  }
  const kind = value.kind;
  const x = Math.round(Number(value.x));
  const y = Math.round(Number(value.y));
  const w = Math.round(Number(value.w));
  const h = Math.round(Number(value.h));
  if (![x, y, w, h].every(Number.isFinite)) {
    return { error: `${where}: position and size must be numbers` };
  }
  const spec = panelWidgetSpec(kind);
  if (w < spec.minW || h < spec.minH) {
    return {
      error: `${where}: ${spec.label} needs at least ${spec.minW} × ${spec.minH} cells`,
    };
  }
  if (x < 0 || y < 0 || x + w > width || y + h > height) {
    return {
      error: `${where}: outside the ${width * PAGE_GRID} × ${height * PAGE_GRID} canvas`,
    };
  }
  if (placed.some((other) => overlaps({ x, y, w, h }, other))) {
    return { error: `${where}: overlaps another control` };
  }
  if (kind === "button" && !isActionKind(value.action)) {
    return { error: `${where}: unknown action` };
  }
  return buildWidget(kind, id, { x, y, w, h }, value);
}

/// Reads an exported page strictly: unknown types, bad geometry and overlaps
/// are errors instead of being dropped.
export function importPanelFile(
  raw: unknown,
): { page: PanelPage } | { error: string } {
  if (!raw || typeof raw !== "object" || Array.isArray(raw)) {
    return { error: "Not a Panel file" };
  }
  const file = raw as Record<string, unknown>;
  if (file.format !== PANEL_FILE_FORMAT) return { error: "Not a Panel file" };
  if (file.version !== PANEL_FILE_VERSION) {
    return {
      error: `Unsupported Panel file version ${String(file.version ?? "")}`,
    };
  }
  const rawPage = file.page;
  if (!rawPage || typeof rawPage !== "object") {
    return { error: "No page in this file" };
  }
  const value = rawPage as Record<string, unknown>;
  const width = Number(value.width);
  const height = Number(value.height);
  if (
    !Number.isInteger(width) ||
    width < PAGE_MIN_W ||
    width > PAGE_MAX_W ||
    !Number.isInteger(height) ||
    height < PAGE_MIN_H ||
    height > PAGE_MAX_H
  ) {
    return {
      error: `Canvas must be ${PAGE_MIN_W * PAGE_GRID}–${PAGE_MAX_W * PAGE_GRID} wide and ${PAGE_MIN_H * PAGE_GRID}–${PAGE_MAX_H * PAGE_GRID} tall in 8px steps`,
    };
  }
  const source = value.widgets;
  if (!Array.isArray(source)) return { error: "No controls in this file" };
  const widgets: PanelWidget[] = [];
  const placed: Geometry[] = [];
  const counts: Partial<Record<PanelWidgetKind, number>> = {};
  for (let index = 0; index < source.length; index += 1) {
    if (index >= 100) return { error: "Too many controls (100 max)" };
    const kind = (source[index] as Record<string, unknown> | null)?.kind;
    let id = `widget${index + 1}`;
    if (isPanelKind(kind)) {
      const n = (counts[kind] ?? 0) + 1;
      counts[kind] = n;
      id = `${kind}${n}`;
    }
    const widget = strictWidget(source[index], index, width, height, placed, id);
    if ("error" in widget) return { error: widget.error };
    placed.push({ x: widget.x, y: widget.y, w: widget.w, h: widget.h });
    widgets.push(widget);
  }
  return {
    page: {
      version: 1,
      id: "imported",
      name: text(value.name, 40).trim() || "Imported page",
      width,
      height,
      widgets,
    },
  };
}

/// Appends an imported page under a fresh id and unique name.
export function addPanelPageFromFile(
  book: PanelBook,
  page: PanelPage,
): { book: PanelBook; page: PanelPage } {
  const id = pageId(book);
  const stored: PanelPage = {
    ...page,
    id,
    name: uniquePageName(book, page.name || "Imported page"),
  };
  return {
    book: { ...book, activeId: id, pages: [...book.pages, stored] },
    page: stored,
  };
}

export function updatePanelWidget(
  page: PanelPage,
  id: string,
  patch: PanelWidgetPatch,
): PanelPage {
  return {
    ...page,
    widgets: page.widgets.map((widget) =>
      widget.id === id
        ? panelWidgetSpec(widget.kind).applyPatch(widget, patch)
        : widget,
    ),
  };
}

export function removePanelWidget(page: PanelPage, id: string): PanelPage {
  return { ...page, widgets: page.widgets.filter((widget) => widget.id !== id) };
}

export function resizePanelPage(
  page: PanelPage,
  width: number,
  height: number,
): PanelPage | null {
  if (!Number.isFinite(width) || !Number.isFinite(height)) return null;
  const nextW = clamp(Math.round(width), PAGE_MIN_W, PAGE_MAX_W);
  const nextH = clamp(Math.round(height), PAGE_MIN_H, PAGE_MAX_H);
  if (
    page.widgets.some(
      (widget) => widget.x + widget.w > nextW || widget.y + widget.h > nextH,
    )
  ) {
    return null;
  }
  return { ...page, width: nextW, height: nextH };
}

export function movePanelWidget(
  page: PanelPage,
  id: string,
  x: number,
  y: number,
  w: number,
  h: number,
): PanelPage | null {
  const current = page.widgets.find((widget) => widget.id === id);
  if (!current) return null;
  const min = widgetMinSize(current.kind);
  const nextW = clamp(Math.round(w), min.w, page.width);
  const nextH = clamp(Math.round(h), min.h, page.height);
  const next = {
    x: clamp(Math.round(x), 0, page.width - nextW),
    y: clamp(Math.round(y), 0, page.height - nextH),
    w: nextW,
    h: nextH,
  };
  if (page.widgets.some((widget) => widget.id !== id && overlaps(next, widget))) {
    return null;
  }
  return {
    ...page,
    widgets: page.widgets.map((widget) =>
      widget.id === id ? { ...widget, ...next } : widget,
    ),
  };
}

function nextId(page: PanelPage, kind: PanelWidgetKind): string {
  let n = 1;
  const ids = new Set(page.widgets.map((widget) => widget.id));
  while (ids.has(`${kind}${n}`)) n += 1;
  return `${kind}${n}`;
}

/// First free spot at or after `(fromX, fromY)`, scanning rows.
function freeSpot(
  page: PanelPage,
  w: number,
  h: number,
  fromX: number,
  fromY: number,
): { x: number; y: number } | null {
  for (let y = Math.max(0, fromY); y <= page.height - h; y += 1) {
    for (
      let x = y === fromY ? Math.max(0, fromX) : 0;
      x <= page.width - w;
      x += 1
    ) {
      const candidate = { x, y, w, h };
      if (page.widgets.some((widget) => overlaps(candidate, widget))) continue;
      return { x, y };
    }
  }
  return null;
}

export function addPanelWidget(
  page: PanelPage,
  kind: PanelWidgetKind,
): { page: PanelPage; id: string } | null {
  const spec = panelWidgetSpec(kind);
  const spot = freeSpot(page, spec.defaultW, spec.defaultH, 0, 0);
  if (!spot) return null;
  const id = nextId(page, kind);
  const widget = spec.create(id, spot.x, spot.y, spec.defaultW, spec.defaultH);
  return { page: { ...page, widgets: [...page.widgets, widget] }, id };
}

/// Copies `widget` next to its current spot when that area is free.
export function pastePanelWidget(
  page: PanelPage,
  widget: PanelWidget,
): { page: PanelPage; id: string } | null {
  const spec = panelWidgetSpec(widget.kind);
  const spot = freeSpot(page, widget.w, widget.h, widget.x + 1, widget.y + 1);
  if (!spot) return null;
  const id = nextId(page, widget.kind);
  const next = spec.create(id, spot.x, spot.y, widget.w, widget.h);
  const merged = spec.applyPatch(next, {
    label: widget.label,
    ...(widget.kind === "button"
      ? { action: widget.action, script: widget.script }
      : {}),
    ...(widget.kind === "value"
      ? { source: widget.source, digits: widget.digits, bands: widget.bands }
      : {}),
    ...(widget.kind === "toggle" ? { sysvar: widget.sysvar } : {}),
    ...(widget.kind === "numeric"
      ? { sysvar: widget.sysvar, digits: widget.digits, step: widget.step }
      : {}),
    ...(widget.kind === "lamp"
      ? {
          source: widget.source,
          threshold: widget.threshold,
          color: widget.color,
        }
      : {}),
    ...(widget.kind === "bar" || widget.kind === "gauge"
      ? {
          source: widget.source,
          min: widget.min,
          max: widget.max,
          digits: widget.digits,
          bands: widget.bands,
        }
      : {}),
  });
  return { page: { ...page, widgets: [...page.widgets, merged] }, id };
}

export function duplicatePanelWidget(
  page: PanelPage,
  id: string,
): { page: PanelPage; id: string } | null {
  const widget = page.widgets.find((entry) => entry.id === id);
  if (!widget) return null;
  return pastePanelWidget(page, widget);
}

/// Kept for the default-page helpers and older callers.
export function addPanelButton(
  page: PanelPage,
): { page: PanelPage; id: string } | null {
  return addPanelWidget(page, "button");
}