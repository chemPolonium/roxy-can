export const PAGE_GRID = 8;
export const PAGE_WIDTH = 160;
export const PAGE_HEIGHT = 90;
export const PAGE_MIN_W = 40;
export const PAGE_MIN_H = 20;
export const PAGE_MAX_W = 400;
export const PAGE_MAX_H = 300;

/// What a Panel button does: apply its signal writes, send one CAN frame, or
/// run a named script handler (`on panel_action("…")`). Simulation controls
/// live in the top bar, not on panel buttons.
export type PanelActionKind = "script" | "none" | "send";

/// One frame a button sends once, in the protocol's own address terms.
export type PanelFrameSend = (
  | {
      protocol: "can";
      channel: number;
      id: number;
      extended: boolean;
      data: string;
    }
  | { protocol: "fr"; bus: number; slot: number; data: string }
  | { protocol: "eth"; bus: number; frame: string; data: string }
) & {
  /// CAN address selector: a manually typed raw frame or a catalog message.
  /// Missing means the editor infers it from whether the ID matches.
  source?: "raw" | "catalog";
  /// Raw sends `data`; physical encodes `values` against the catalog frame.
  /// Missing means raw (older pages).
  mode?: "raw" | "physical";
  /// Physical values keyed by catalog signal name. Unlisted signals keep the
  /// database's start value.
  values?: PanelFrameValue[];
};

export interface PanelFrameValue {
  name: string;
  value: number;
}

export const PANEL_MAX_FRAME_VALUES = 128;

export const PANEL_MAX_FRAME_BYTES = 2048;

/// Parses space/comma separated or contiguous hex into bytes. `null` means
/// malformed or larger than the shared safety cap; each protocol applies its
/// own tighter length rule in `panelFrameProblem`.
export function parsePanelHex(text: string): number[] | null {
  const clean = text.replace(/[\s,]+/g, "");
  if (clean === "") return [];
  if (clean.length % 2 !== 0 || !/^[0-9a-fA-F]+$/.test(clean)) return null;
  const bytes: number[] = [];
  for (let i = 0; i < clean.length; i += 2) {
    bytes.push(Number.parseInt(clean.slice(i, i + 2), 16));
  }
  return bytes.length <= PANEL_MAX_FRAME_BYTES ? bytes : null;
}

export function formatPanelHex(bytes: number[]): string {
  return bytes
    .map((byte) => byte.toString(16).toUpperCase().padStart(2, "0"))
    .join(" ");
}

const CAN_FD_LENGTHS = [0, 1, 2, 3, 4, 5, 6, 7, 8, 12, 16, 20, 24, 32, 48, 64];

/// Why the frame cannot be sent, or `null` when its syntax is valid. Bus
/// numbers and Ethernet frame lengths still have to match the catalog.
export function panelFrameProblem(frame: PanelFrameSend): string | null {
  const physical = frame.mode === "physical";
  const parsed = parsePanelHex(frame.data);
  if (!physical && parsed === null) return "Data must be hex bytes";
  const bytes = parsed ?? [];
  if (physical) {
    for (const entry of frame.values ?? []) {
      if (!entry.name.trim()) return "Select a signal to send";
      if (!Number.isFinite(entry.value)) {
        return `Value for ${entry.name} must be a number`;
      }
    }
  }
  if (frame.protocol === "can") {
    if (!Number.isInteger(frame.channel) || frame.channel < 0 || frame.channel > 255) {
      return "Bus must be a CAN channel";
    }
    if (!Number.isInteger(frame.id) || frame.id < 0) {
      return "ID must be a non-negative integer";
    }
    if (frame.extended ? frame.id > 0x1fffffff : frame.id > 0x7ff) {
      return frame.extended
        ? "Extended IDs stop at 0x1FFFFFFF"
        : "Standard IDs stop at 0x7FF";
    }
    if (!physical && !CAN_FD_LENGTHS.includes(bytes.length)) {
      return "Data must be 0-8, 12, 16, 20, 24, 32, 48 or 64 bytes";
    }
    return null;
  }
  if (frame.protocol === "fr") {
    if (!Number.isInteger(frame.bus) || frame.bus < 0 || frame.bus > 255) {
      return "Bus must be a FlexRay channel";
    }
    if (!Number.isInteger(frame.slot) || frame.slot < 1 || frame.slot > 2047) {
      return "Slot must be 1..2047";
    }
    if (!physical && bytes.length > 254) return "FlexRay data must be at most 254 bytes";
    return null;
  }
  if (!Number.isInteger(frame.bus) || frame.bus < 0 || frame.bus > 255) {
    return "Bus must be an Ethernet channel";
  }
  if (!frame.frame.trim()) return "Select an Ethernet frame";
  return null;
}

export type PanelWidgetKind =
  | "button"
  | "value"
  | "toggle"
  | "numeric"
  | "lamp"
  | "bar"
  | "gauge";

/// Pages saved before Numeric and Setpoint were merged still carry this kind;
/// it loads as a Numeric with `target: "signal"`.
const LEGACY_SETPOINT_KIND = "setpoint";
type RawPanelKind = PanelWidgetKind | typeof LEGACY_SETPOINT_KIND;

interface Geometry {
  x: number;
  y: number;
  w: number;
  h: number;
}

/// One signal value a Button's preset writes.
export interface PanelWrite {
  source: string;
  value: number;
}

export interface PanelButton extends Geometry {
  id: string;
  kind: "button";
  label: string;
  action: PanelActionKind;
  /// Handler name for `action === "script"`.
  script?: string;
  /// Value handed to a script handler that declares a parameter.
  argument?: number;
  /// Signals applied before the action; each is attempted independently.
  writes?: PanelWrite[];
  /// The one-shot CAN frame for `action === "send"`.
  frame?: PanelFrameSend;
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

export type PanelNumericTarget = "sysvar" | "signal";

/// Editable setpoint: writes a system variable or a DBC signal, clamped by
/// whichever backend owns the range.
export interface PanelNumeric extends Geometry {
  id: string;
  kind: "numeric";
  label: string;
  /// Where the committed value goes; absent means a system variable (pages
  /// saved before the merge have no target).
  target?: PanelNumericTarget;
  sysvar: string;
  /// Catalog signal key (`channel:id:signal`) used when target is `signal`.
  source: string;
  digits: number;
  step: number;
  /// Optional script action fired with the committed value.
  panelAction?: string;
}

export function panelNumericTarget(widget: PanelNumeric): PanelNumericTarget {
  return widget.target === "signal" ? "signal" : "sysvar";
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
  argument?: number;
  writes?: PanelWrite[];
  frame?: PanelFrameSend;
  panelAction?: string;
  source?: string;
  digits?: number;
  sysvar?: string;
  step?: number;
  bands?: PanelBand[];
  threshold?: number;
  color?: PanelBandColor;
  min?: number;
  max?: number;
  target?: PanelNumericTarget;
}

export type PanelFieldKey = keyof PanelWidgetPatch;

export interface PanelFieldSpec {
  key: PanelFieldKey;
  label: string;
  kind: "text" | "select" | "list" | "digits" | "number" | "bands" | "writes" | "frame";
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

function readPanelKind(value: unknown): RawPanelKind | null {
  if (value === LEGACY_SETPOINT_KIND) return LEGACY_SETPOINT_KIND;
  return isPanelKind(value) ? value : null;
}

function widgetKindFor(kind: RawPanelKind): PanelWidgetKind {
  return kind === LEGACY_SETPOINT_KIND ? "numeric" : kind;
}

export const PANEL_MAX_WRITES = 16;

/// Keeps rows that name a signal and carry a finite value; malformed rows in
/// a saved page are dropped the same way invalid bands are.
export function normalisePanelWrites(raw: unknown): PanelWrite[] | undefined {
  if (!Array.isArray(raw)) return undefined;
  const writes: PanelWrite[] = [];
  for (const entry of raw.slice(0, PANEL_MAX_WRITES)) {
    if (!entry || typeof entry !== "object") continue;
    const value = entry as Record<string, unknown>;
    const source = text(value.source, 120);
    const number = Number(value.value);
    if (!source || !Number.isFinite(number)) continue;
    writes.push({ source, value: number });
  }
  return writes.length > 0 ? writes : undefined;
}

/// Stored physical values: unique non-empty names with finite numbers.
function normalisePanelFrameValues(raw: unknown): PanelFrameValue[] {
  if (!Array.isArray(raw)) return [];
  const values: PanelFrameValue[] = [];
  for (const entry of raw.slice(0, PANEL_MAX_FRAME_VALUES)) {
    if (!entry || typeof entry !== "object") continue;
    const value = entry as Record<string, unknown>;
    const name = text(value.name, 120);
    const number = Number(value.value);
    if (!name || !Number.isFinite(number)) continue;
    if (values.some((existing) => existing.name === name)) continue;
    values.push({ name, value: number });
  }
  return values;
}

/// A stored frame is kept only when its data parses and the address fits the
/// chosen frame class; otherwise the button has no configured message.
function normalisePanelFrame(raw: unknown): PanelFrameSend | undefined {
  if (!raw || typeof raw !== "object") return undefined;
  const value = raw as Record<string, unknown>;
  const protocol =
    value.protocol === "fr"
      ? "fr"
      : value.protocol === "eth"
        ? "eth"
        : value.protocol === "can" || value.channel !== undefined
          ? "can"
          : value.slot !== undefined
            ? "fr"
            : value.frame !== undefined
              ? "eth"
              : null;
  if (!protocol) return undefined;
  const data = parsePanelHex(text(value.data, 8192));
  if (data === null) return undefined;
  const hex = formatPanelHex(data);
  const mode = value.mode === "physical" ? "physical" : "raw";
  const source =
    value.source === "raw" || value.source === "catalog"
      ? value.source
      : undefined;
  const values = normalisePanelFrameValues(value.values);
  const frame: PanelFrameSend =
    protocol === "can"
      ? {
          protocol,
          channel: Number(value.channel),
          id: Number(value.id),
          extended: Boolean(value.extended),
          data: hex,
          source,
          mode,
          values,
        }
      : protocol === "fr"
        ? {
            protocol,
            bus: Number(value.bus),
            slot: Number(value.slot),
            data: hex,
            source,
            mode,
            values,
          }
        : {
            protocol,
            bus: Number(value.bus),
            frame: text(value.frame, 120),
            data: hex,
            source,
            mode,
            values,
          };
  return panelFrameProblem(frame) ? undefined : frame;
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

function isActionKind(value: unknown): value is PanelActionKind {
  return value === "script" || value === "none" || value === "send";
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
    label: "On click",
    kind: "select",
    options: [
      { value: "none", label: "Write signals" },
      { value: "send", label: "Send message" },
      { value: "script", label: "Script action" },
    ],
  },
  {
    key: "script",
    label: "Script action",
    kind: "list",
    listId: "page-actions",
    maxLength: 80,
    placeholder: 'on panel_action("…")',
  },
  {
    key: "argument",
    label: "Argument",
    kind: "number",
    placeholder: "0",
  },
  {
    key: "writes",
    label: "Signal writes",
    kind: "writes",
  },
  {
    key: "frame",
    label: "Message",
    kind: "frame",
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
    key: "target",
    label: "Target",
    kind: "select",
    options: [
      { value: "sysvar", label: "System variable" },
      { value: "signal", label: "Signal" },
    ],
  },
  {
    key: "sysvar",
    label: "System variable",
    kind: "list",
    maxLength: 120,
    listId: "page-sysvars",
    placeholder: "namespace::name",
  },
  {
    key: "source",
    label: "Signal",
    kind: "list",
    maxLength: 120,
    listId: "page-write",
    placeholder: "channel:id:signal",
  },
  { key: "digits", label: "Decimals", kind: "digits" },
  {
    key: "step",
    label: "Step",
    kind: "number",
    placeholder: "1",
  },
  {
    key: "panelAction",
    label: "Script action",
    kind: "list",
    listId: "page-actions",
    maxLength: 80,
    placeholder: 'on panel_action("…")',
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
    action: "none",
    argument: 0,
    x,
    y,
    w,
    h,
  }),
  fieldValue: (widget, key) => {
    if (widget.kind !== "button") return "";
    if (key === "action") return widget.action;
    if (key === "script") return widget.script ?? "";
    if (key === "argument") return widget.argument ?? 0;
    if (key === "writes") return widget.writes?.length ?? 0;
    if (key === "frame") return widget.frame ? 1 : 0;
    if (key === "label") return widget.label;
    return "";
  },
  applyPatch: (widget, patch) => {
    if (widget.kind !== "button") return widget;
    const argument = Number(patch.argument);
    const action =
      patch.action !== undefined && isActionKind(patch.action)
        ? patch.action
        : widget.action;
    let writes = patch.writes === undefined ? widget.writes : patch.writes;
    let frame = patch.frame === undefined ? widget.frame : patch.frame;
    if (patch.action !== undefined && action !== widget.action) {
      if (action === "none") frame = undefined;
      else writes = undefined;
      if (action !== "send") frame = undefined;
    }
    return {
      ...widget,
      label: patch.label === undefined ? widget.label : patch.label.slice(0, 80),
      action,
      script:
        patch.script === undefined
          ? widget.script
          : patch.script.slice(0, 80),
      argument:
        patch.argument === undefined || !Number.isFinite(argument)
          ? widget.argument
          : argument,
      writes,
      frame,
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
  label: "Setpoint",
  minW: 14,
  minH: 5,
  defaultW: 18,
  defaultH: 6,
  fields: NUMERIC_FIELDS,
  create: (id, x, y, w, h) => ({
    id,
    kind: "numeric",
    label: "Setpoint",
    target: "sysvar",
    sysvar: "",
    source: "",
    digits: 1,
    step: 1,
    panelAction: "",
    x,
    y,
    w,
    h,
  }),
  fieldValue: (widget, key) => {
    if (widget.kind !== "numeric") return "";
    if (key === "target") return panelNumericTarget(widget);
    if (key === "sysvar") return widget.sysvar;
    if (key === "source") return widget.source;
    if (key === "digits") return widget.digits;
    if (key === "step") return widget.step;
    if (key === "panelAction") return widget.panelAction ?? "";
    if (key === "label") return widget.label;
    return "";
  },
  applyPatch: (widget, patch) => {
    if (widget.kind !== "numeric") return widget;
    const step = Number(patch.step);
    return {
      ...widget,
      label: patch.label === undefined ? widget.label : patch.label.slice(0, 80),
      target:
        patch.target === "signal" || patch.target === "sysvar"
          ? patch.target
          : widget.target,
      sysvar:
        patch.sysvar === undefined
          ? widget.sysvar
          : patch.sysvar.slice(0, 120),
      source:
        patch.source === undefined
          ? widget.source
          : patch.source.slice(0, 120),
      digits:
        patch.digits === undefined
          ? widget.digits
          : clamp(Math.round(patch.digits), 0, 3),
      step:
        patch.step === undefined || !Number.isFinite(step) || step <= 0
          ? widget.step
          : Math.min(step, 1e9),
      panelAction:
        patch.panelAction === undefined
          ? widget.panelAction
          : patch.panelAction.slice(0, 80),
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
  return {
    version: 1,
    id: "page1",
    name: "Overview",
    width: PAGE_WIDTH,
    height: PAGE_HEIGHT,
    widgets: [],
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

function widgetPatch(
  value: Record<string, unknown>,
  rawKind: RawPanelKind,
): PanelWidgetPatch {
  const kind = widgetKindFor(rawKind);
  // A button has exactly one payload: signal writes, one message, or a
  // script. A saved combination keeps the writes and narrows to that mode.
  const writes = kind === "button" ? normalisePanelWrites(value.writes) : undefined;
  let action = value.action as PanelActionKind;
  if (writes && action !== "none") action = "none";
  const frame =
    kind === "button" && action === "send"
      ? normalisePanelFrame(value.frame)
      : undefined;
  return {
    label: text(value.label, 80),
    ...(kind === "button"
      ? {
          action,
          script: text(value.script, 80),
          argument: Number(value.argument),
          writes: action === "none" ? writes : undefined,
          frame,
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
      ? rawKind === LEGACY_SETPOINT_KIND
        ? {
            target: "signal" as const,
            source: text(value.source, 120),
            digits: clamp(Math.round(Number(value.digits) || 0), 0, 3),
            step: Number(value.step),
          }
        : {
            target: value.target === "signal" ? ("signal" as const) : ("sysvar" as const),
            sysvar: text(value.sysvar, 120),
            source: text(value.source, 120),
            digits: clamp(Math.round(Number(value.digits) || 0), 0, 3),
            step: Number(value.step),
            panelAction: text(value.panelAction, 80),
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
  kind: RawPanelKind,
  id: string,
  geometry: Geometry,
  value: Record<string, unknown>,
): PanelWidget {
  const spec = panelWidgetSpec(widgetKindFor(kind));
  const widget = spec.create(
    id,
    geometry.x,
    geometry.y,
    geometry.w,
    geometry.h,
  );
  return spec.applyPatch(widget, widgetPatch(value, kind));
}

function sanitise(
  raw: unknown,
  width: number,
  height: number,
  used: Set<string>,
): PanelWidget | null {
  if (!raw || typeof raw !== "object") return null;
  const value = raw as Record<string, unknown>;
  const rawKind = readPanelKind(value.kind);
  if (!rawKind) return null;
  if (typeof value.id !== "string" || value.id.length === 0) return null;
  if (used.has(value.id)) return null;
  const x = Math.round(Number(value.x));
  const y = Math.round(Number(value.y));
  const w = Math.round(Number(value.w));
  const h = Math.round(Number(value.h));
  if (![x, y, w, h].every(Number.isFinite)) return null;
  const spec = panelWidgetSpec(widgetKindFor(rawKind));
  if (w < spec.minW || h < spec.minH) return null;
  if (w > width || h > height) return null;
  const geometry = {
    x: clamp(x, 0, width - w),
    y: clamp(y, 0, height - h),
    w,
    h,
  };
  if (widgetKindFor(rawKind) === "button" && !isActionKind(value.action)) {
    return null;
  }
  if (
    widgetKindFor(rawKind) === "button" &&
    value.action === "send" &&
    !normalisePanelFrame(value.frame)
  ) {
    return null;
  }
  return buildWidget(rawKind, value.id, geometry, value);
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
  const rawKind = readPanelKind(value.kind);
  if (!rawKind) {
    return { error: `${where}: unknown type "${String(value.kind ?? "")}"` };
  }
  const kind = widgetKindFor(rawKind);
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
  if (kind === "button" && value.writes !== undefined) {
    if (!Array.isArray(value.writes)) {
      return { error: `${where}: signal writes must be a list` };
    }
    if (value.writes.length > PANEL_MAX_WRITES) {
      return { error: `${where}: at most ${PANEL_MAX_WRITES} signal writes` };
    }
    if (value.writes.length > 0 && value.action !== "none") {
      return {
        error: `${where}: signal writes need the "Write signals" action`,
      };
    }
    for (const entry of value.writes) {
      if (!entry || typeof entry !== "object") {
        return { error: `${where}: each signal write needs a source and value` };
      }
      const write = entry as Record<string, unknown>;
      if (typeof write.source !== "string" || typeof write.value !== "number" || !Number.isFinite(write.value)) {
        return { error: `${where}: each signal write needs a source and a finite value` };
      }
    }
  }
  if (kind === "button" && value.action === "send" && !normalisePanelFrame(value.frame)) {
    return {
      error: `${where}: Send message needs a valid bus, ID and hex data`,
    };
  }
  if (kind === "button" && value.action !== "send" && value.frame !== undefined) {
    return { error: `${where}: a message belongs to Send message` };
  }
  return buildWidget(rawKind, id, { x, y, w, h }, value);
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
    const rawKind = readPanelKind(
      (source[index] as Record<string, unknown> | null)?.kind,
    );
    let id = `widget${index + 1}`;
    if (rawKind) {
      const kind = widgetKindFor(rawKind);
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
      ? {
          action: widget.action,
          script: widget.script,
          argument: widget.argument,
          writes: widget.writes,
          frame: widget.frame,
        }
      : {}),
    ...(widget.kind === "value"
      ? { source: widget.source, digits: widget.digits, bands: widget.bands }
      : {}),
    ...(widget.kind === "toggle" ? { sysvar: widget.sysvar } : {}),
    ...(widget.kind === "numeric"
      ? {
          target: panelNumericTarget(widget),
          sysvar: widget.sysvar,
          source: widget.source,
          digits: widget.digits,
          step: widget.step,
          panelAction: widget.panelAction,
        }
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