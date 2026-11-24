import { useEffect, useRef, useState } from "preact/hooks";
import {
  PAGE_GRID,
  PAGE_MAX_H,
  PAGE_MAX_W,
  PAGE_MIN_H,
  PAGE_MIN_W,
  PANEL_WIDGET_KINDS,
  activePanelPage,
  addPanelWidget,
  duplicatePanelWidget,
  formatPanelValue,
  movePanelWidget,
  overlappingPanelWidgets,
  panelSetpointTarget,
  panelWidgetSpec,
  pastePanelWidget,
  removePanelWidget,
  resizePanelPage,
  selectPanelPage,
  updatePanelPage,
  updatePanelWidget,
  widgetMinSize,
  type PanelBook,
  type PanelPage,
  type PanelWidget,
  type PanelWidgetKind,
  type PanelWidgetPatch,
  type PanelToggle,
  type PanelButton,
  type PanelSetpoint,
} from "../panelPage";
import {
  findPanelSysVar,
  panelLampLit,
  panelMeterRatio,
  panelWidgetCapability,
  resolvePanelSource,
  type PanelActionContext,
  type PanelView,
} from "../panelBinding";
import { bandAt, type Band } from "../bands";
import { palette, type Theme } from "../theme";
import { copyText } from "../clipboard";
import { ContextMenu } from "./ContextMenu";
import { FrameField } from "./PanelFrameField";
import { NumberField } from "./NumberField";
import { PaletteSwatches } from "./PaletteSwatches";
import type { SysVar } from "../types";

interface DragState {
  kind: "move" | "resize";
  id: string;
  pointerX: number;
  pointerY: number;
  x: number;
  y: number;
  w: number;
  h: number;
}

interface History {
  past: PanelPage[];
  future: PanelPage[];
}

type PageMenu =
  | { kind: "widget"; x: number; y: number; id: string }
  | { kind: "canvas"; x: number; y: number };

/// The signal a read-only or setpoint cell watches, for "Copy signal key".
function panelSignalKey(widget: PanelWidget): string {
  if (
    widget.kind === "value" ||
    widget.kind === "lamp" ||
    widget.kind === "bar" ||
    widget.kind === "gauge"
  ) {
    return widget.source.trim();
  }
  if (widget.kind === "setpoint" && panelSetpointTarget(widget) === "signal") {
    return widget.source.trim();
  }
  return "";
}

function panelWidgetSysVar(
  widget: PanelWidget,
  view: PanelView,
): SysVar | undefined {
  if (widget.kind === "toggle" || widget.kind === "setpoint") {
    return findPanelSysVar(widget.sysvar, view);
  }
  if (
    widget.kind === "value" ||
    widget.kind === "lamp" ||
    widget.kind === "bar" ||
    widget.kind === "gauge"
  ) {
    const source = widget.source.trim();
    return source.startsWith("sysvar:")
      ? findPanelSysVar(source.slice("sysvar:".length), view)
      : undefined;
  }
  return undefined;
}

function panelWidgetBands(widget: PanelWidget, view: PanelView): Band[] {
  if ("labelSet" in widget) {
    if (widget.labelSet === null) return [];
    if (widget.labelSet) {
      return view.bandSets?.find((set) => set.id === widget.labelSet)?.bands ?? [];
    }
  }
  const variable = panelWidgetSysVar(widget, view);
  if (variable) {
    const chosen = variable.bandSetId ?? null;
    return chosen
      ? (view.bandSets?.find((set) => set.id === chosen)?.bands ?? [])
      : [];
  }
  if (widget.kind === "value" || widget.kind === "bar" || widget.kind === "gauge") {
    return [];
  }
  if (widget.kind === "lamp") {
    return [];
  }
  return [];
}

const EMPTY_HISTORY: History = { past: [], future: [] };

import { GaugeView, SetpointControl, SuggestionInput, SysVarSetpointControl } from "./PanelControls";
import { BandSetPicker } from "./BandSetPicker";
export function PagePanel({
  book,
  revision,
  toast,
  sourceOptions,
  sysvarOptions,
  actionNames,
  view,
  actionContext,
  theme,
  editing,
  onEditing,
  onBook,
  onRun,
  onToggle,
  onSetNumber,
  onWrite,
  onNotice,
}: {
  book: PanelBook;
  revision: number;
  toast?: string;
  sourceOptions: string[];
  sysvarOptions: string[];
  actionNames: string[];
  view: PanelView;
  actionContext: PanelActionContext;
  theme: Theme;
  editing: boolean;
  onEditing: (on: boolean) => void;
  onBook: (book: PanelBook) => void;
  onRun: (widget: PanelButton) => void;
  onToggle: (widget: PanelToggle, on: boolean) => void;
  onSetNumber: (widget: PanelSetpoint, value: number) => void;
  onWrite: (widget: PanelSetpoint, value: number) => void;
  onNotice: (message: string) => void;
}) {
  const page = activePanelPage(book);
  const monitorColors = palette(theme).monitors;
  const overlapping = overlappingPanelWidgets(page);
  const onPage = (next: PanelPage) => onBook(updatePanelPage(book, next));
  const [fullscreen, setFullscreen] = useState(false);
  const [selected, setSelected] = useState<string | null>(null);
  const [drag, setDrag] = useState<DragState | null>(null);
  const [history, setHistory] = useState<History>(EMPTY_HISTORY);
  const [clipboard, setClipboard] = useState<PanelWidget | null>(null);
  const [sizeW, setSizeW] = useState(String(page.width * PAGE_GRID));
  const [sizeH, setSizeH] = useState(String(page.height * PAGE_GRID));
  const [addOpen, setAddOpen] = useState(false);
  const [addPos, setAddPos] = useState({ x: 0, y: 0 });
  const addTrigger = useRef<HTMLButtonElement>(null);
  const [menu, setMenu] = useState<PageMenu | null>(null);
  const [pageNotice, setPageNotice] = useState("");
  const noticeError = (message: string) => {
    setPageNotice(message);
    onNotice(message);
  };
  const noticeDone = (message: string) => {
    setPageNotice("");
    onNotice(message);
  };

  useEffect(() => {
    setSizeW(String(page.width * PAGE_GRID));
    setSizeH(String(page.height * PAGE_GRID));
  }, [page.width, page.height]);

  useEffect(() => {
    setHistory(EMPTY_HISTORY);
    setClipboard(null);
    setSelected(null);
    setMenu(null);
    setAddOpen(false);
    setPageNotice("");
  }, [revision, page.id]);

  useEffect(() => {
    if (!editing) setSelected(null);
  }, [editing]);

  useEffect(() => {
    if (!editing) setAddOpen(false);
  }, [editing]);

  useEffect(() => {
    if (!menu) return;
    const onKey = (event: KeyboardEvent) => {
      if (event.key !== "Escape") return;
      // Capture phase: the editor's Esc handler must not leave Edit too.
      event.stopImmediatePropagation();
      setMenu(null);
    };
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  }, [menu]);

  useEffect(() => {
    if (!editing) return;
    const onKey = (event: KeyboardEvent) => {
      if (event.key !== "Escape") return;
      if (selected) {
        setSelected(null);
      } else {
        onEditing(false);
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [editing, selected, onEditing]);

  useEffect(() => {
    if (!fullscreen || editing) return;
    const onKey = (event: KeyboardEvent) => {
      if (event.key !== "Escape") return;
      if (document.querySelector(".modal-backdrop")) return;
      setFullscreen(false);
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [fullscreen, editing]);

  useEffect(() => {
    if (!drag) return;
    const min = widgetMinSize(
      page.widgets.find((widget) => widget.id === drag.id)?.kind ?? "button",
    );
    const move = (event: PointerEvent) => {
      const dx = Math.round((event.clientX - drag.pointerX) / PAGE_GRID);
      const dy = Math.round((event.clientY - drag.pointerY) / PAGE_GRID);
      const next =
        drag.kind === "move"
          ? movePanelWidget(
              page,
              drag.id,
              drag.x + dx,
              drag.y + dy,
              drag.w,
              drag.h,
            )
          : movePanelWidget(
              page,
              drag.id,
              drag.x,
              drag.y,
              Math.max(min.w, drag.w + dx),
              Math.max(min.h, drag.h + dy),
            );
      if (next) onPage(next);
    };
    const up = () => setDrag(null);
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", up);
    window.addEventListener("pointercancel", up);
    return () => {
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", up);
      window.removeEventListener("pointercancel", up);
    };
  }, [drag, page, onPage]);

  const selectedWidget = page.widgets.find((widget) => widget.id === selected);

  const pushHistory = () => {
    setHistory((old) => ({
      past: [...old.past.slice(-49), page],
      future: [],
    }));
  };

  const commit = (next: PanelPage) => {
    if (next === page) return;
    pushHistory();
    onPage(next);
  };

  const undo = () => {
    if (history.past.length === 0) return;
    const previous = history.past[history.past.length - 1];
    setHistory({
      past: history.past.slice(0, -1),
      future: [page, ...history.future].slice(0, 50),
    });
    onPage(previous);
    setSelected(null);
  };

  const redo = () => {
    if (history.future.length === 0) return;
    const next = history.future[0];
    setHistory({
      past: [...history.past.slice(-49), page],
      future: history.future.slice(1),
    });
    onPage(next);
    setSelected(null);
  };

  const removeWidget = (id: string) => {
    commit(removePanelWidget(page, id));
    setSelected(null);
  };

  const duplicateWidget = (id: string) => {
    const copy = duplicatePanelWidget(page, id);
    if (!copy) {
      noticeError("No free space for a copy");
      return;
    }
    commit(copy.page);
    setSelected(copy.id);
  };

  const copyWidget = (id: string) => {
    const widget = page.widgets.find((entry) => entry.id === id);
    if (widget) setClipboard({ ...widget });
  };

  const copySignalKey = (id: string) => {
    const widget = page.widgets.find((entry) => entry.id === id);
    const key = widget ? panelSignalKey(widget) : "";
    if (!key) return;
    void copyText(key).then((copied) =>
      copied ? noticeDone(`Copied ${key}`) : noticeError("Could not copy the signal key"),
    );
  };

  const removeSelected = () => {
    if (selectedWidget) removeWidget(selectedWidget.id);
  };

  const duplicateSelected = () => {
    if (selectedWidget) duplicateWidget(selectedWidget.id);
  };

  const copySelected = () => {
    if (selectedWidget) copyWidget(selectedWidget.id);
  };

  const pasteClipboard = () => {
    if (!clipboard) return;
    const copy = pastePanelWidget(page, clipboard);
    if (!copy) {
      noticeError("No free space for a paste");
      return;
    }
    commit(copy.page);
    setSelected(copy.id);
  };

  useEffect(() => {
    if (!editing) return;
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
      if (event.key === "Delete" || event.key === "Backspace") {
        if (!selectedWidget) return;
        event.preventDefault();
        removeSelected();
        return;
      }
      if (!(event.ctrlKey || event.metaKey)) return;
      const key = event.key.toLowerCase();
      if (key === "c") {
        if (!selectedWidget) return;
        event.preventDefault();
        copySelected();
      } else if (key === "v") {
        event.preventDefault();
        pasteClipboard();
      } else if (key === "z" && !event.shiftKey) {
        event.preventDefault();
        undo();
      } else if (key === "y" || (key === "z" && event.shiftKey)) {
        event.preventDefault();
        redo();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [editing, selectedWidget, clipboard, history, page]);

  const startDrag = (
    event: PointerEvent,
    id: string,
    kind: "move" | "resize",
  ) => {
    if (!editing || event.button !== 0) return;
    const widget = page.widgets.find((entry) => entry.id === id);
    if (!widget) return;
    event.preventDefault();
    event.stopPropagation();
    pushHistory();
    setSelected(id);
    setDrag({
      kind,
      id,
      pointerX: event.clientX,
      pointerY: event.clientY,
      x: widget.x,
      y: widget.y,
      w: widget.w,
      h: widget.h,
    });
  };

  // ←/→ walk the Panel pages even though the tabs now live in the top
  // switch. Typing, menus and dialogs keep their own key handling.
  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (!["ArrowLeft", "ArrowRight", "Home", "End"].includes(event.key)) {
        return;
      }
      const tag = (event.target as HTMLElement).tagName;
      if (
        tag === "INPUT" ||
        tag === "TEXTAREA" ||
        tag === "SELECT" ||
        document.querySelector(".modal-backdrop, .row-menu, .app-menu")
      ) {
        return;
      }
      const index = book.pages.findIndex((entry) => entry.id === page.id);
      if (index < 0 || book.pages.length < 2) return;
      const last = book.pages.length - 1;
      const next =
        event.key === "Home"
          ? 0
          : event.key === "End"
            ? last
            : (index + (event.key === "ArrowRight" ? 1 : -1) + book.pages.length) %
              book.pages.length;
      const target = book.pages[next];
      if (!target) return;
      event.preventDefault();
      setSelected(null);
      setHistory(EMPTY_HISTORY);
      setClipboard(null);
      onBook(selectPanelPage(book, target.id));
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [book, page.id, onBook]);

  const add = (kind: PanelWidgetKind) => {
    const added = addPanelWidget(page, kind);
    if (!added) {
      noticeError("No free space on this page");
      return;
    }
    commit(added.page);
    setSelected(added.id);
  };

  const resetSize = () => {
    setSizeW(String(page.width * PAGE_GRID));
    setSizeH(String(page.height * PAGE_GRID));
  };

  const applySize = () => {
    const w = Number(sizeW);
    const h = Number(sizeH);
    if (!Number.isFinite(w) || !Number.isFinite(h)) {
      noticeError("Canvas width and height must be numbers");
      resetSize();
      return;
    }
    const next = resizePanelPage(
      page,
      Math.round(w / PAGE_GRID),
      Math.round(h / PAGE_GRID),
    );
    if (!next) {
      noticeError("Move controls inside the new canvas first");
      resetSize();
      return;
    }
    commit(next);
    setSizeW(String(next.width * PAGE_GRID));
    setSizeH(String(next.height * PAGE_GRID));
  };

  const menuWidget =
    menu?.kind === "widget"
      ? page.widgets.find((widget) => widget.id === menu.id)
      : undefined;
  const menuSignal = menuWidget ? panelSignalKey(menuWidget) : "";
  // Flip above the click when the menu would run off the bottom; the canvas
  // menu is the tallest (Paste, seven controls, Undo/Redo).
  const menuHeight =
    menu?.kind === "canvas" ? 330 : menu?.kind === "widget" ? 150 : 150;
  const menuTop = menu
    ? menu.y + menuHeight > window.innerHeight - 8
      ? Math.max(8, menu.y - menuHeight)
      : menu.y
    : 0;
  const menuLeft = menu ? Math.min(menu.x, window.innerWidth - 200) : 0;

  const capabilityOf = (widget: PanelWidget) =>
    panelWidgetCapability(widget, view, actionContext);

  const control = (widget: PanelWidget) => {
    const capability = capabilityOf(widget);
    if (widget.kind === "value") {
      const resolved = resolvePanelSource(widget.source, view);
      const entry = bandAt(panelWidgetBands(widget, view), resolved.value);
      const state = entry?.text || resolved.label || null;
      const color =
        entry?.color != null
          ? monitorColors[Math.abs(entry.color) % monitorColors.length]
          : undefined;
      return (
        <div
          class={`page-value ${capability.enabled ? "" : "missing"}`}
          title={
            capability.enabled
              ? `${widget.source}${
                  state
                    ? ` · ${state} · value ${formatPanelValue(
                        resolved.value,
                        widget.digits,
                      )}`
                    : ""
                }`
              : capability.reason
          }
        >
          <span class="page-value-head">
            {widget.label && (
              <span class="page-value-label">{widget.label}</span>
            )}
            {resolved.unit && <em>{resolved.unit}</em>}
          </span>
          <span class="page-value-line">
            <strong class={state ? "state" : undefined} style={color ? { color } : undefined}>
              {state ?? formatPanelValue(resolved.value, widget.digits)}
            </strong>
          </span>
        </div>
      );
    }
    if (widget.kind === "setpoint") {
      if (panelSetpointTarget(widget) === "signal") {
        const target = (view.writeTargets ?? []).find(
          (entry) => entry.key === widget.source,
        );
        const resolved = resolvePanelSource(widget.source, view);
        return (
          <SetpointControl
            widget={widget}
            value={resolved.value}
            min={target?.min ?? null}
            max={target?.max ?? null}
            unit={target?.unit ?? resolved.unit}
            disabled={editing || !capability.enabled}
            reason={capability.reason}
            onWrite={(value) => {
              if (!editing && capability.enabled) onWrite(widget, value);
            }}
          />
        );
      }
      const variable = findPanelSysVar(widget.sysvar, view);
      const disabled = editing || !capability.enabled;
      const state =
        bandAt(panelWidgetBands(widget, view), variable?.value ?? null)?.text ||
        null;
      return (
        <SysVarSetpointControl
          widget={widget}
          value={variable?.value ?? null}
          min={variable?.min ?? null}
          max={variable?.max ?? null}
          unit={variable?.unit ?? ""}
          state={state}
          disabled={disabled}
          reason={capability.reason}
          title={state ? `${widget.sysvar} · ${state}` : widget.sysvar}
          onCommit={(value) => {
            if (!editing && capability.enabled) onSetNumber(widget, value);
          }}
        />
      );
    }
    if (widget.kind === "toggle") {
      const variable = findPanelSysVar(widget.sysvar, view);
      const on = (variable?.value ?? 0) !== 0;
      const entry = bandAt(
        panelWidgetBands(widget, view),
        variable?.value ?? null,
      );
      const state = entry?.text || null;
      const color =
        entry?.color != null
          ? monitorColors[Math.abs(entry.color) % monitorColors.length]
          : undefined;
      return (
        <button
          type="button"
          class={`page-toggle ${on ? "on" : ""} ${
            capability.enabled ? "" : "disabled"
          }`}
          style={on && color ? { borderColor: color, color } : undefined}
          title={
            capability.enabled
              ? `${widget.sysvar}${state ? ` · ${state}` : ""}`
              : capability.reason
          }
          onClick={() => {
            if (!editing && capability.enabled) onToggle(widget, !on);
          }}
        >
          <span class="page-toggle-label">
            {widget.label || widget.sysvar || "Toggle"}
          </span>
          <span class="page-toggle-state">{state ?? (on ? "ON" : "OFF")}</span>
        </button>
      );
    }
    if (widget.kind === "lamp") {
      const resolved = resolvePanelSource(widget.source, view);
      const entry = bandAt(panelWidgetBands(widget, view), resolved.value);
      const color =
        entry?.color != null
          ? monitorColors[Math.abs(entry.color) % monitorColors.length]
          : monitorColors[Math.abs(widget.color) % monitorColors.length];
      const lit = capability.enabled && panelLampLit(widget, resolved.value);
      return (
        <div
          class={`page-lamp ${lit ? "on" : ""} ${
            capability.enabled ? "" : "missing"
          }`}
          title={
            capability.enabled
              ? `${widget.source} · ${
                  entry?.text ||
                  resolved.label ||
                  formatPanelValue(resolved.value, 1)
                }`
              : capability.reason
          }
        >
          <span
            class="page-lamp-dot"
            style={
              lit
                ? {
                    background: color,
                    boxShadow: `0 0 7px ${color}`,
                  }
                : undefined
            }
          />
          <span class="page-lamp-label">
            {widget.label || widget.source || "Lamp"}
          </span>
        </div>
      );
    }
    if (widget.kind === "bar") {
      const resolved = resolvePanelSource(widget.source, view);
      const entry = bandAt(panelWidgetBands(widget, view), resolved.value);
      const state = entry?.text || resolved.label || null;
      const color =
        entry?.color != null
          ? monitorColors[Math.abs(entry.color) % monitorColors.length]
          : "var(--accent)";
      const ratio = capability.enabled
        ? panelMeterRatio(resolved.value, widget.min, widget.max)
        : null;
      return (
        <div
          class={`page-bar ${capability.enabled ? "" : "missing"}`}
          title={
            capability.enabled
              ? `${widget.source} · ${formatPanelValue(
                  widget.min,
                  0,
                )}..${formatPanelValue(widget.max, 0)}`
              : capability.reason
          }
        >
          <span class="page-bar-head">
            <span class="page-bar-label">{widget.label || "Bar"}</span>
            <span class="page-bar-value">
              {state ?? formatPanelValue(resolved.value, widget.digits)}
              {resolved.unit && <em>{resolved.unit}</em>}
            </span>
          </span>
          <span class="page-bar-track">
            <span
              class="page-bar-fill"
              style={{
                width: `${(ratio ?? 0) * 100}%`,
                background: color,
              }}
            />
          </span>
        </div>
      );
    }
    if (widget.kind === "gauge") {
      const resolved = resolvePanelSource(widget.source, view);
      const entry = bandAt(panelWidgetBands(widget, view), resolved.value);
      const color =
        entry?.color != null
          ? monitorColors[Math.abs(entry.color) % monitorColors.length]
          : "var(--accent)";
      return (
        <div
          class={`page-gauge ${capability.enabled ? "" : "missing"}`}
          title={capability.enabled ? widget.source : capability.reason}
        >
          <span class="page-gauge-label">{widget.label || "Gauge"}</span>
          <GaugeView
            widget={widget}
            value={resolved.value}
            unit={resolved.unit}
            state={entry?.text || resolved.label || null}
            color={color}
          />
        </div>
      );
    }
    return (
      <button
        type="button"
        class={`page-button ${capability.enabled ? "" : "disabled"}`}
        title={capability.enabled ? widget.label : capability.reason}
        onClick={() => {
          if (!editing && capability.enabled) onRun(widget);
        }}
      >
        {widget.label ||
          (widget.action === "send"
            ? "Send"
            : widget.action === "script"
              ? widget.script || "Script"
              : "Button")}
      </button>
    );
  };

  const field = (widget: PanelWidget, key: keyof PanelWidgetPatch) =>
    panelWidgetSpec(widget.kind).fieldValue(widget, key);

  const inspectorField = (widget: PanelWidget, key: string) => {
    const spec = panelWidgetSpec(widget.kind);
    const fieldSpec = spec.fields.find((field) => field.key === key);
    if (!fieldSpec) return null;
    if (key === "script" && field(widget, "action") !== "script") return null;
    if (key === "argument" && field(widget, "action") !== "script") return null;
    if (key === "frame" && field(widget, "action") !== "send") return null;
    if (
      widget.kind === "setpoint" &&
      key === "sysvar" &&
      field(widget, "target") === "signal"
    ) {
      return null;
    }
    if (
      widget.kind === "setpoint" &&
      key === "source" &&
      field(widget, "target") !== "signal"
    ) {
      return null;
    }
    const value = field(widget, fieldSpec.key);
    const update = (patch: PanelWidgetPatch) =>
      commit(updatePanelWidget(page, widget.id, patch));
    if (fieldSpec.kind === "frame") {
      return (
        <FrameField
          frame={widget.kind === "button" ? widget.frame : undefined}
          canBuses={view.frameBuses ?? []}
          frBuses={view.frBuses ?? []}
          ethBuses={view.ethBuses ?? []}
          ethFrames={view.ethFrames ?? []}
          frameDefs={view.frameDefs ?? []}
          someipServices={view.someipServices ?? []}
          onFocus={pushHistory}
          onChange={(frame) => update({ frame })}
        />
      );
    }
    if (fieldSpec.kind === "color") {
      const current =
        widget.kind === "lamp" ? widget.color : 0;
      return (
        <PaletteSwatches
          colors={monitorColors}
          value={current}
          onFocus={pushHistory}
          onChange={(slot) => update({ color: slot })}
        />
      );
    }
    if (fieldSpec.key === "labelSet") {
      if (!("labelSet" in widget)) return null;
      if (
        widget.kind === "setpoint" &&
        panelSetpointTarget(widget) !== "sysvar"
      ) {
        return null;
      }
      const variable = panelWidgetSysVar(widget, view);
      const inlineSource =
        widget.kind === "value" ||
        widget.kind === "lamp" ||
        widget.kind === "bar" ||
        widget.kind === "gauge";
      if ((!variable && !inlineSource) || (view.bandSets ?? []).length === 0) {
        return null;
      }
      const chosen =
        widget.labelSet === null ? "none" : (widget.labelSet ?? "");
      const fallback = variable?.bandSetId
        ? view.bandSets?.find((set) => set.id === variable.bandSetId)
        : undefined;
      return (
        <label>
          Bands
          <BandSetPicker
            value={chosen}
            ariaLabel="Bands"
            onOpen={pushHistory}
            choices={[
              { value: "", label: `Default · ${fallback?.name ?? "none"}` },
              { value: "none", label: "No bands" },
              ...(view.bandSets ?? []).map((set) => ({
                value: set.id,
                label: set.name,
                hint: set.id,
              })),
            ]}
            onChange={(next) =>
              update({
                labelSet:
                  next === "" ? undefined : next === "none" ? null : next,
              })
            }
          />
        </label>
      );
    }
    if (fieldSpec.kind === "select") {
      return (
        <label>
          {fieldSpec.label}
          <select
            value={String(value)}
            onFocus={pushHistory}
            onChange={(event) => {
              const next = (event.target as HTMLSelectElement).value;
              if (
                fieldSpec.key === "action" &&
                next === "send" &&
                widget.kind === "button" &&
                !widget.frame
              ) {
                update({
                  action: "send",
                  frame: {
                    protocol: "can",
                    channel: view.frameBuses?.[0]?.index ?? 0,
                    id: 0x7df,
                    extended: false,
                    data: "00",
                  },
                });
                return;
              }
              update({
                [fieldSpec.key]: next,
              } as PanelWidgetPatch);
            }}
          >
            {fieldSpec.options?.map((option) => (
              <option value={option.value} key={option.value}>
                {option.label}
              </option>
            ))}
          </select>
        </label>
      );
    }
    if (fieldSpec.kind === "digits") {
      return (
        <label>
          {fieldSpec.label}
          <select
            value={String(value)}
            onFocus={pushHistory}
            onChange={(event) =>
              update({
                [fieldSpec.key]: Number(
                  (event.target as HTMLSelectElement).value,
                ),
              } as PanelWidgetPatch)
            }
          >
            {[0, 1, 2, 3].map((digits) => (
              <option value={digits} key={digits}>
                {digits}
              </option>
            ))}
          </select>
        </label>
      );
    }
    if (fieldSpec.kind === "number") {
      const numeric = Number(value);
      return (
        <label>
          {fieldSpec.label}
          <NumberField
            value={Number.isFinite(numeric) ? numeric : 0}
            placeholder={fieldSpec.placeholder}
            onFocus={pushHistory}
            onCommit={(next) =>
              update({ [fieldSpec.key]: next } as PanelWidgetPatch)
            }
          />
        </label>
      );
    }
    if (fieldSpec.kind === "list") {
      return (
        <label>
          {fieldSpec.label}
          <SuggestionInput
            value={String(value)}
            options={
              fieldSpec.listId === "page-sysvars"
                ? sysvarOptions
                : fieldSpec.listId === "page-actions"
                  ? actionNames
                  : fieldSpec.listId === "page-write"
                    ? (view.writeTargets ?? []).map((target) => target.key)
                    : sourceOptions
            }
            placeholder={fieldSpec.placeholder}
            maxLength={fieldSpec.maxLength}
            onFocus={pushHistory}
            onCommit={(next) =>
              update({ [fieldSpec.key]: next } as PanelWidgetPatch)
            }
          />
        </label>
      );
    }
    return (
      <label>
        {fieldSpec.label}
        <input
          value={String(value)}
          maxLength={fieldSpec.maxLength}
          placeholder={fieldSpec.placeholder}
          onFocus={pushHistory}
          onInput={(event) =>
            update({
              [fieldSpec.key]: (event.target as HTMLInputElement).value,
            } as PanelWidgetPatch)
          }
        />
      </label>
    );
  };

  return (
    <section
      class={`panel page-panel ${fullscreen ? "page-fullscreen" : ""}`}
    >
      <div class={`panel-head dock-head ${editing ? "editing" : ""}`}>
        <span class="meta">
          {page.width * PAGE_GRID}×{page.height * PAGE_GRID} · {PAGE_GRID}px grid
        </span>
        <div class="panel-tools">
          {editing && (
            <>
              <button
                ref={addTrigger}
                type="button"
                class="mini"
                aria-haspopup="menu"
                aria-expanded={addOpen}
                title="Add a control"
                onClick={(event) => {
                  const rect = (
                    event.currentTarget as HTMLButtonElement
                  ).getBoundingClientRect();
                  setAddPos({ x: rect.left, y: rect.bottom + 4 });
                  setAddOpen((open) => !open);
                }}
              >
                + Add
              </button>
              {addOpen && (
                <ContextMenu
                  x={addPos.x}
                  y={addPos.y}
                  label="Add control"
                  items={PANEL_WIDGET_KINDS.map((entry) => ({
                    label: entry.label,
                    onSelect: () => add(entry.kind),
                  }))}
                  onClose={() => {
                    setAddOpen(false);
                    addTrigger.current?.focus();
                  }}
                />
              )}
              <button
                type="button"
                class="mini icon-mini"
                disabled={history.past.length === 0}
                title="Undo (Ctrl+Z)"
                aria-label="Undo"
                onClick={undo}
              >
                ↶
              </button>
              <button
                type="button"
                class="mini icon-mini"
                disabled={history.future.length === 0}
                title="Redo (Ctrl+Shift+Z / Ctrl+Y)"
                aria-label="Redo"
                onClick={redo}
              >
                ↷
              </button>
            </>
          )}
          <button
            type="button"
            class={`mini ${fullscreen ? "on" : ""}`}
            title={fullscreen ? "Exit web fullscreen" : "Fill the browser window"}
            onClick={() => setFullscreen((old) => !old)}
          >
            {fullscreen ? "Exit" : "Fullscreen"}
          </button>
        </div>
      </div>
      {pageNotice && (
        <div class="page-notice" role="alert">
          {pageNotice}
        </div>
      )}
      <div class="page-body">
        <div
          class="page-viewport"
          onPointerDown={() => editing && setSelected(null)}
        >
          {editing && overlapping.size > 0 && (
            <div class="page-overlap-warning" role="alert">
              Controls overlap. Move one to separate them.
            </div>
          )}
          <div
            class={`page-canvas ${editing ? "editing" : ""}`}
            style={{
              width: `${page.width * PAGE_GRID}px`,
              height: `${page.height * PAGE_GRID}px`,
            }}
            onPointerDown={(event) => {
              if (event.target === event.currentTarget && editing) {
                setSelected(null);
              }
            }}
            onContextMenu={(event) => {
              if (!editing) return;
              event.preventDefault();
              setSelected(null);
              setMenu({
                kind: "canvas",
                x: event.clientX,
                y: event.clientY,
              });
            }}
          >
            {page.widgets.length === 0 && (
              <div class="page-empty">
                <strong>No controls yet</strong>
                <span>
                  {editing
                    ? "Use + Add to place a control."
                    : "Press Edit, then + Add to place a control."}
                </span>
              </div>
            )}
            {page.widgets.map((widget) => (
              <div
                key={widget.id}
                class={`page-widget ${editing ? "editing" : ""} ${
                  selected === widget.id ? "selected" : ""
                } ${overlapping.has(widget.id) ? "overlapping" : ""}`}
                style={{
                  left: `${widget.x * PAGE_GRID}px`,
                  top: `${widget.y * PAGE_GRID}px`,
                  width: `${widget.w * PAGE_GRID}px`,
                  height: `${widget.h * PAGE_GRID}px`,
                }}
                onPointerDown={(event) => {
                  if ((event.target as HTMLElement).closest(".page-resize")) {
                    startDrag(event, widget.id, "resize");
                  } else {
                    startDrag(event, widget.id, "move");
                  }
                }}
                onContextMenu={(event) => {
                  if (!editing) return;
                  event.preventDefault();
                  event.stopPropagation();
                  setSelected(widget.id);
                  setMenu({
                    kind: "widget",
                    id: widget.id,
                    x: event.clientX,
                    y: event.clientY,
                  });
                }}
              >
                {control(widget)}
                {editing && selected === widget.id && (
                  <span
                    class="page-resize"
                    title="Resize on the 8px grid"
                  />
                )}
              </div>
            ))}
          </div>
        </div>
        {editing && (
          <aside class="page-inspector">
            <div class="page-inspector-head">
              <span class="page-inspector-title">Properties</span>
              <span class="meta">
                {selectedWidget
                  ? panelWidgetSpec(selectedWidget.kind).label
                  : "Panel"}
              </span>
            </div>
            <div class="page-inspector-body">
            {selectedWidget ? (
              <>
            {panelWidgetSpec(selectedWidget.kind).fields.map((spec) =>
              inspectorField(selectedWidget, spec.key),
            )}
            <div class="page-inspector-meta">
              {selectedWidget.w}×{selectedWidget.h} cells · x{" "}
              {selectedWidget.x} · y {selectedWidget.y}
            </div>
            <div class="page-inspector-actions">
              <button
                type="button"
                class="mini"
                title="Duplicate (Ctrl+C / Ctrl+V)"
                onClick={duplicateSelected}
              >
                Duplicate
              </button>
              <button type="button" class="mini" onClick={removeSelected}>
                Delete
              </button>
            </div>
              </>
            ) : (
              <>
                <label>
                  Width (px)
                  <input
                    type="number"
                    step={PAGE_GRID}
                    min={PAGE_MIN_W * PAGE_GRID}
                    max={PAGE_MAX_W * PAGE_GRID}
                    value={sizeW}
                    onInput={(event) =>
                      setSizeW((event.target as HTMLInputElement).value)
                    }
                    onKeyDown={(event) => {
                      if (event.key === "Enter") {
                        event.preventDefault();
                        applySize();
                      } else if (event.key === "Escape") {
                        event.preventDefault();
                        event.stopPropagation();
                        resetSize();
                        (event.target as HTMLInputElement).blur();
                      }
                    }}
                  />
                </label>
                <label>
                  Height (px)
                  <input
                    type="number"
                    step={PAGE_GRID}
                    min={PAGE_MIN_H * PAGE_GRID}
                    max={PAGE_MAX_H * PAGE_GRID}
                    value={sizeH}
                    onInput={(event) =>
                      setSizeH((event.target as HTMLInputElement).value)
                    }
                    onKeyDown={(event) => {
                      if (event.key === "Enter") {
                        event.preventDefault();
                        applySize();
                      } else if (event.key === "Escape") {
                        event.preventDefault();
                        event.stopPropagation();
                        resetSize();
                        (event.target as HTMLInputElement).blur();
                      }
                    }}
                  />
                </label>
                <button type="button" class="mini" onClick={applySize}>
                  Apply size
                </button>
                <div class="page-inspector-meta">
                  {page.widgets.length} control
                  {page.widgets.length === 1 ? "" : "s"} · {page.width}×
                  {page.height} cells · {PAGE_GRID}px grid
                </div>
              </>
            )}
            </div>
          </aside>
        )}
      </div>
      {menu && (
        <ContextMenu
          x={menuLeft}
          y={menuTop}
          label="Panel actions"
          maxHeight={`calc(100vh - ${menuTop + 8}px)`}
          onClose={() => setMenu(null)}
        >
          {menu.kind === "widget" && (
            <>
              <button
                type="button"
                role="menuitem"
                onClick={() => {
                  setMenu(null);
                  duplicateWidget(menu.id);
                }}
              >
                Duplicate
              </button>
              <button
                type="button"
                role="menuitem"
                onClick={() => {
                  setMenu(null);
                  copyWidget(menu.id);
                }}
              >
                Copy
              </button>
              {menuSignal && (
                <button
                  type="button"
                  role="menuitem"
                  onClick={() => {
                    setMenu(null);
                    copySignalKey(menu.id);
                  }}
                >
                  Copy signal key
                </button>
              )}
              <div class="page-menu-sep" role="separator" />
              <button
                type="button"
                role="menuitem"
                onClick={() => {
                  setMenu(null);
                  removeWidget(menu.id);
                }}
              >
                Delete
              </button>
            </>
          )}
          {menu.kind === "canvas" && (
            <>
              <button
                type="button"
                role="menuitem"
                disabled={!clipboard}
                onClick={() => {
                  setMenu(null);
                  pasteClipboard();
                }}
              >
                Paste
              </button>
              <div class="page-menu-sep" role="separator" />
              {PANEL_WIDGET_KINDS.map((entry) => (
                <button
                  type="button"
                  role="menuitem"
                  key={entry.kind}
                  onClick={() => {
                    setMenu(null);
                    add(entry.kind);
                  }}
                >
                  Add {entry.label}
                </button>
              ))}
              <div class="page-menu-sep" role="separator" />
              <button
                type="button"
                role="menuitem"
                disabled={history.past.length === 0}
                onClick={() => {
                  setMenu(null);
                  undo();
                }}
              >
                Undo
              </button>
              <button
                type="button"
                role="menuitem"
                disabled={history.future.length === 0}
                onClick={() => {
                  setMenu(null);
                  redo();
                }}
              >
                Redo
              </button>
            </>
          )}
        </ContextMenu>
      )}
      {fullscreen && toast && (
        <div class="page-toast" role="status">
          {toast}
        </div>
      )}
    </section>
  );
}
