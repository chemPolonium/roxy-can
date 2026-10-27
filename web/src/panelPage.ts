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

export type PanelWidgetKind = "button" | "value" | "toggle" | "numeric";

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

/// Read-only value tile: signal key, `derived:Name` or `sysvar:ns::name`.
export interface PanelValue extends Geometry {
  id: string;
  kind: "value";
  label: string;
  source: string;
  digits: number;
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

export type PanelWidget = PanelButton | PanelValue | PanelToggle | PanelNumeric;

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
}

export type PanelFieldKey = keyof PanelWidgetPatch;

export interface PanelFieldSpec {
  key: PanelFieldKey;
  label: string;
  kind: "text" | "select" | "list" | "digits" | "number";
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

const VALUE_FIELDS: PanelFieldSpec[] = [
  { key: "label", label: "Label", kind: "text", maxLength: 80 },
  {
    key: "source",
    label: "Source",
    kind: "list",
    maxLength: 120,
    listId: "page-sources",
    placeholder: "signal key / derived:Name / sysvar:ns::name",
  },
  { key: "digits", label: "Decimals", kind: "digits" },
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

export const PANEL_WIDGET_SPECS: Record<PanelWidgetKind, PanelWidgetSpec> = {
  button: BUTTON_SPEC,
  value: VALUE_SPEC,
  toggle: TOGGLE_SPEC,
  numeric: NUMERIC_SPEC,
};

export const PANEL_WIDGET_KINDS: { kind: PanelWidgetKind; label: string }[] = [
  { kind: "button", label: BUTTON_SPEC.label },
  { kind: "value", label: VALUE_SPEC.label },
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

function sanitise(
  raw: unknown,
  width: number,
  height: number,
  used: Set<string>,
): PanelWidget | null {
  if (!raw || typeof raw !== "object") return null;
  const value = raw as Record<string, unknown>;
  const kind = value.kind;
  if (
    kind !== "button" &&
    kind !== "value" &&
    kind !== "toggle" &&
    kind !== "numeric"
  ) {
    return null;
  }
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
  const widget = spec.create(value.id, geometry.x, geometry.y, w, h);
  return spec.applyPatch(widget, {
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
  });
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
      ? { source: widget.source, digits: widget.digits }
      : {}),
    ...(widget.kind === "toggle" ? { sysvar: widget.sysvar } : {}),
    ...(widget.kind === "numeric"
      ? { sysvar: widget.sysvar, digits: widget.digits, step: widget.step }
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