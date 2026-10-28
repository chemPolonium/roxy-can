import type { ScriptInfo, SignalSeries, SysVar } from "./types";
import {
  panelNumericTarget,
  parseSysVarRef,
  type PanelActionKind,
  type PanelBand,
  type PanelBandColor,
  type PanelButton,
  type PanelLamp,
  type PanelNumeric,
  type PanelToggle,
  type PanelWidget,
} from "./panelPage";

export interface PanelView {
  signals: SignalSeries[];
  sysvars: SysVar[];
  /// Writable CAN signals from the catalog, for setpoint controls.
  writeTargets?: PanelWriteTarget[];
}

/// One signal a Panel setpoint may write: the catalog key plus its DBC range.
export interface PanelWriteTarget {
  key: string;
  /// Declared bounds; null when the description names no usable range, which
  /// the write path treats as unbounded rather than as 0..0.
  min: number | null;
  max: number | null;
  unit: string;
}

export interface PanelSourceValue {
  value: number | null;
  unit: string;
  /// False when the source is empty or names nothing the session knows.
  known: boolean;
}

const EMPTY: PanelSourceValue = { value: null, unit: "", known: false };

/// Latest value of a signal key, `derived:Name` or `sysvar:ns::name`.
export function resolvePanelSource(
  source: string,
  view: PanelView,
): PanelSourceValue {
  const trimmed = source.trim();
  if (!trimmed) return EMPTY;
  if (trimmed.startsWith("sysvar:")) {
    const ref = parseSysVarRef(trimmed.slice("sysvar:".length));
    const variable = ref
      ? view.sysvars.find(
          (entry) =>
            entry.namespace === ref.namespace && entry.name === ref.name,
        )
      : undefined;
    if (!variable) return EMPTY;
    return { value: variable.value, unit: variable.unit, known: true };
  }
  const series = trimmed.startsWith("derived:")
    ? view.signals.find(
        (entry) =>
          entry.key.startsWith("derived:") &&
          entry.name === trimmed.slice("derived:".length),
      )
    : view.signals.find((entry) => entry.key === trimmed);
  if (!series) return EMPTY;
  if (series.points.length === 0) {
    return { value: null, unit: series.unit, known: true };
  }
  return {
    value: series.points[series.points.length - 1][1],
    unit: series.unit,
    known: true,
  };
}

/// Colour band the value falls into: the first "up to" it fits, or null.
export function panelValueBand(
  value: number | null,
  bands?: PanelBand[],
): PanelBandColor | null {
  if (value === null || !Number.isFinite(value) || !bands?.length) return null;
  for (const band of bands) {
    if (band.upTo === null || value <= band.upTo) return band.color;
  }
  return null;
}

/// 0..1 fill for a meter; null when the range is empty or the value is absent.
export function panelMeterRatio(
  value: number | null,
  min: number,
  max: number,
): number | null {
  if (value === null || !Number.isFinite(value)) return null;
  if (!Number.isFinite(min) || !Number.isFinite(max) || max <= min) return null;
  return Math.min(1, Math.max(0, (value - min) / (max - min)));
}

/// Whether a lamp is lit: source value at or above its threshold.
export function panelLampLit(widget: PanelLamp, value: number | null): boolean {
  return value !== null && Number.isFinite(value) && value >= widget.threshold;
}

/// Fill path for the gauge's 100 × 64 viewBox, from the left end clockwise.
/// The arc never exceeds 180°, so the large-arc flag stays 0.
export function panelGaugeArc(ratio: number): string {
  const clamped = Math.min(1, Math.max(0, ratio));
  const angle = Math.PI + Math.PI * clamped;
  const x = 50 + 44 * Math.cos(angle);
  const y = 50 + 44 * Math.sin(angle);
  return `M6 50 A44 44 0 0 1 ${x.toFixed(2)} ${y.toFixed(2)}`;
}

export function findPanelSysVar(ref: string, view: PanelView): SysVar | undefined {
  const parts = parseSysVarRef(ref);
  if (!parts) return undefined;
  return view.sysvars.find(
    (entry) => entry.namespace === parts.namespace && entry.name === parts.name,
  );
}

/// Numeric setpoints clamp into the sysvar's declared bounds.
export function clampPanelNumber(
  value: number,
  min: number | null,
  max: number | null,
): number {
  let next = value;
  if (min !== null && Number.isFinite(min)) next = Math.max(min, next);
  if (max !== null && Number.isFinite(max)) next = Math.min(max, next);
  return next;
}

export function panelSourceOptions(
  view: PanelView,
  derivedNames: string[],
): string[] {
  const options: string[] = [];
  for (const signal of view.signals) options.push(signal.key);
  for (const name of derivedNames) options.push(`derived:${name}`);
  for (const variable of view.sysvars) {
    options.push(`sysvar:${variable.namespace}::${variable.name}`);
  }
  return options;
}

/// Case-insensitive substring match for the source/sysvar suggestion list.
export function filterPanelOptions(
  options: string[],
  query: string,
  limit = 100,
): string[] {
  const needle = query.trim().toLowerCase();
  const matches = needle
    ? options.filter((option) => option.toLowerCase().includes(needle))
    : options;
  return matches.slice(0, limit);
}

/// Names declared by every running, enabled, error-free script.
export function runningPanelActions(scripts: ScriptInfo[]): Set<string> {
  const actions = new Set<string>();
  for (const script of scripts) {
    if (!script.enabled || !script.running || script.errored) continue;
    for (const name of script.panelActions) actions.add(name);
  }
  return actions;
}

export interface PanelCapability {
  enabled: boolean;
  reason: string;
}

export interface PanelActionContext {
  online: boolean;
  measuring: boolean;
  paused: boolean;
  scriptActions: Set<string>;
}

/// Whether a button can act now, with the reason to show on hover.
export function panelButtonCapability(
  widget: PanelButton,
  ctx: PanelActionContext,
): PanelCapability {
  if (!ctx.online) return { enabled: false, reason: "No connection to the bus core" };
  if (widget.action === "script") {
    const name = widget.script?.trim() ?? "";
    if (!name) return { enabled: false, reason: "No script action name" };
    if (!ctx.measuring) {
      return { enabled: false, reason: `"${name}" needs a running measurement` };
    }
    if (!ctx.scriptActions.has(name)) {
      return { enabled: false, reason: `No running script handles "${name}"` };
    }
    return { enabled: true, reason: "" };
  }
  const action = widget.action as Exclude<PanelActionKind, "script">;
  if (action === "start") {
    return ctx.measuring
      ? { enabled: false, reason: "Measurement is already running" }
      : { enabled: true, reason: "" };
  }
  if (!ctx.measuring) {
    return { enabled: false, reason: "Start the measurement first" };
  }
  if (action === "pause" && ctx.paused) {
    return { enabled: false, reason: "Trace is already paused" };
  }
  if (action === "resume" && !ctx.paused) {
    return { enabled: false, reason: "Trace is not paused" };
  }
  return { enabled: true, reason: "" };
}

export function panelValueCapability(
  widget: { source: string },
  value: PanelSourceValue,
): PanelCapability {
  if (!widget.source.trim()) {
    return { enabled: false, reason: "No source bound" };
  }
  if (!value.known) {
    return { enabled: false, reason: `Unknown source "${widget.source}"` };
  }
  return { enabled: true, reason: "" };
}

/// A setpoint writes only while the measurement runs, into a signal the
/// catalog knows.
export function panelSetpointCapability(
  source: string,
  view: PanelView,
  ctx: PanelActionContext,
): PanelCapability {
  if (!ctx.online) return { enabled: false, reason: "No connection to the bus core" };
  const trimmed = source.trim();
  if (!trimmed) return { enabled: false, reason: "No signal bound" };
  if (trimmed.startsWith("derived:")) {
    return { enabled: false, reason: "Derived signals are read-only" };
  }
  if (!(view.writeTargets ?? []).some((target) => target.key === trimmed)) {
    return { enabled: false, reason: `Unknown signal "${trimmed}"` };
  }
  if (!ctx.measuring) {
    return { enabled: false, reason: "Start the measurement first" };
  }
  return { enabled: true, reason: "" };
}

export function panelToggleCapability(
  widget: PanelToggle,
  view: PanelView,
): PanelCapability {
  if (!widget.sysvar.trim()) {
    return { enabled: false, reason: "No system variable bound" };
  }
  const variable = findPanelSysVar(widget.sysvar, view);
  if (!variable) {
    return { enabled: false, reason: `Unknown sysvar "${widget.sysvar}"` };
  }
  return { enabled: true, reason: "" };
}

export function panelNumberCapability(
  widget: PanelNumeric,
  view: PanelView,
): PanelCapability {
  return panelToggleCapability(
    { ...widget, kind: "toggle" },
    view,
  );
}

export function panelWidgetCapability(
  widget: PanelWidget,
  view: PanelView,
  ctx: PanelActionContext,
): PanelCapability {
  if (widget.kind === "button") return panelButtonCapability(widget, ctx);
  if (
    widget.kind === "value" ||
    widget.kind === "lamp" ||
    widget.kind === "bar" ||
    widget.kind === "gauge"
  ) {
    return panelValueCapability(widget, resolvePanelSource(widget.source, view));
  }
  if (widget.kind === "numeric") {
    return panelNumericTarget(widget) === "signal"
      ? panelSetpointCapability(widget.source, view, ctx)
      : panelNumberCapability(widget, view);
  }
  return panelToggleCapability(widget, view);
}