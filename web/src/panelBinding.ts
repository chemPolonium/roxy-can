import type { ScriptInfo, SignalSeries, SysVar } from "./types";
import {
  parseSysVarRef,
  type PanelActionKind,
  type PanelButton,
  type PanelNumeric,
  type PanelToggle,
  type PanelValue,
  type PanelWidget,
} from "./panelPage";

export interface PanelView {
  signals: SignalSeries[];
  sysvars: SysVar[];
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
  widget: PanelValue,
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
  if (widget.kind === "value") {
    return panelValueCapability(widget, resolvePanelSource(widget.source, view));
  }
  if (widget.kind === "numeric") return panelNumberCapability(widget, view);
  return panelToggleCapability(widget, view);
}