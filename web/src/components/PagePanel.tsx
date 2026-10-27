import { useEffect, useRef, useState } from "preact/hooks";
import {
  PAGE_GRID,
  PAGE_MAX_H,
  PAGE_MAX_W,
  PAGE_MIN_H,
  PAGE_MIN_W,
  PANEL_WIDGET_KINDS,
  activePanelPage,
  addPanelPage,
  addPanelWidget,
  duplicatePanelPage,
  duplicatePanelWidget,
  formatPanelValue,
  movePanelWidget,
  panelWidgetSpec,
  pastePanelWidget,
  removePanelPage,
  removePanelWidget,
  renamePanelPage,
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
  type PanelNumeric,
} from "../panelPage";
import {
  clampPanelNumber,
  findPanelSysVar,
  panelWidgetCapability,
  resolvePanelSource,
  type PanelActionContext,
  type PanelView,
} from "../panelBinding";

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

const EMPTY_HISTORY: History = { past: [], future: [] };

function NumericControl({
  widget,
  value,
  min,
  max,
  unit,
  disabled,
  reason,
  onCommit,
}: {
  widget: PanelNumeric;
  value: number | null;
  min: number | null;
  max: number | null;
  unit: string;
  disabled: boolean;
  reason: string;
  onCommit: (value: number) => void;
}) {
  const [draft, setDraft] = useState("");
  const [editing, setEditing] = useState(false);
  useEffect(() => {
    if (!editing) {
      setDraft(value === null ? "" : value.toFixed(widget.digits));
    }
  }, [value, editing, widget.digits]);
  const commit = () => {
    setEditing(false);
    const parsed = draft.trim() === "" ? Number.NaN : Number(draft);
    if (!Number.isFinite(parsed)) {
      setDraft(value === null ? "" : value.toFixed(widget.digits));
      return;
    }
    let next = clampPanelNumber(parsed, min, max);
    onCommit(next);
  };
  return (
    <div class="page-numeric" title={disabled ? reason : widget.sysvar}>
      {widget.label && <span class="page-numeric-label">{widget.label}</span>}
      <span class="page-numeric-row">
        <input
          type="number"
          value={draft}
          step={widget.step}
          disabled={disabled}
          onFocus={() => setEditing(true)}
          onInput={(event) => setDraft((event.target as HTMLInputElement).value)}
          onBlur={commit}
          onKeyDown={(event) => {
            if (event.key === "Enter") {
              (event.target as HTMLInputElement).blur();
            } else if (event.key === "Escape") {
              setDraft(value === null ? "" : value.toFixed(widget.digits));
              (event.target as HTMLInputElement).blur();
            }
          }}
        />
        {unit && <em>{unit}</em>}
      </span>
    </div>
  );
}

export function PagePanel({
  book,
  revision,
  toast,
  sourceOptions,
  sysvarOptions,
  view,
  actionContext,
  onBook,
  onRun,
  onToggle,
  onSetNumber,
  onNotice,
}: {
  book: PanelBook;
  revision: number;
  toast?: string;
  sourceOptions: string[];
  sysvarOptions: string[];
  view: PanelView;
  actionContext: PanelActionContext;
  onBook: (book: PanelBook) => void;
  onRun: (widget: PanelButton) => void;
  onToggle: (widget: PanelToggle, on: boolean) => void;
  onSetNumber: (widget: PanelNumeric, value: number) => void;
  onNotice: (message: string) => void;
}) {
  const page = activePanelPage(book);
  const onPage = (next: PanelPage) => onBook(updatePanelPage(book, next));
  const [editing, setEditing] = useState(false);
  const [fullscreen, setFullscreen] = useState(false);
  const [selected, setSelected] = useState<string | null>(null);
  const [renaming, setRenaming] = useState<{ id: string; name: string } | null>(
    null,
  );
  const renameCanceled = useRef(false);
  const [drag, setDrag] = useState<DragState | null>(null);
  const [history, setHistory] = useState<History>(EMPTY_HISTORY);
  const [clipboard, setClipboard] = useState<PanelWidget | null>(null);
  const [sizeW, setSizeW] = useState(String(page.width * PAGE_GRID));
  const [sizeH, setSizeH] = useState(String(page.height * PAGE_GRID));

  useEffect(() => {
    setSizeW(String(page.width * PAGE_GRID));
    setSizeH(String(page.height * PAGE_GRID));
  }, [page.width, page.height]);

  useEffect(() => {
    setHistory(EMPTY_HISTORY);
    setClipboard(null);
    setSelected(null);
    setRenaming(null);
  }, [revision, page.id]);

  useEffect(() => {
    if (!editing) setSelected(null);
  }, [editing]);

  useEffect(() => {
    if (!editing) return;
    const onKey = (event: KeyboardEvent) => {
      if (event.key !== "Escape") return;
      if (selected) {
        setSelected(null);
      } else {
        setEditing(false);
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [editing, selected]);

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

  const removeSelected = () => {
    if (!selectedWidget) return;
    commit(removePanelWidget(page, selectedWidget.id));
    setSelected(null);
  };

  const duplicateSelected = () => {
    if (!selectedWidget) return;
    const copy = duplicatePanelWidget(page, selectedWidget.id);
    if (!copy) {
      onNotice("No free space for a copy");
      return;
    }
    commit(copy.page);
    setSelected(copy.id);
  };

  const copySelected = () => {
    if (selectedWidget) setClipboard({ ...selectedWidget });
  };

  const pasteClipboard = () => {
    if (!clipboard) return;
    const copy = pastePanelWidget(page, clipboard);
    if (!copy) {
      onNotice("No free space for a paste");
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
    if (!editing) return;
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

  const clearPageUi = () => {
    setSelected(null);
    setHistory(EMPTY_HISTORY);
    setClipboard(null);
    setRenaming(null);
  };

  const switchPage = (id: string) => {
    if (drag || id === page.id) return;
    clearPageUi();
    onBook(selectPanelPage(book, id));
  };

  const newPage = () => {
    const added = addPanelPage(book);
    clearPageUi();
    onBook(added.book);
  };

  const copyPage = () => {
    const copy = duplicatePanelPage(book, page.id);
    if (!copy) {
      onNotice("Cannot duplicate this page");
      return;
    }
    clearPageUi();
    onBook(copy.book);
  };

  const deletePage = () => {
    if (book.pages.length <= 1) {
      onNotice("Keep at least one page");
      return;
    }
    clearPageUi();
    onBook(removePanelPage(book, page.id));
  };

  const commitRename = () => {
    if (!renaming) return;
    if (renameCanceled.current) {
      renameCanceled.current = false;
      setRenaming(null);
      return;
    }
    onBook(renamePanelPage(book, renaming.id, renaming.name));
    setRenaming(null);
  };

  const add = (kind: PanelWidgetKind) => {
    const added = addPanelWidget(page, kind);
    if (!added) {
      onNotice("No free space on this page");
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
      onNotice("Canvas width and height must be numbers");
      resetSize();
      return;
    }
    const next = resizePanelPage(
      page,
      Math.round(w / PAGE_GRID),
      Math.round(h / PAGE_GRID),
    );
    if (!next) {
      onNotice("Move controls inside the new canvas first");
      resetSize();
      return;
    }
    commit(next);
    setSizeW(String(next.width * PAGE_GRID));
    setSizeH(String(next.height * PAGE_GRID));
  };

  const capabilityOf = (widget: PanelWidget) =>
    panelWidgetCapability(widget, view, actionContext);

  const control = (widget: PanelWidget) => {
    const capability = capabilityOf(widget);
    if (widget.kind === "value") {
      const resolved = resolvePanelSource(widget.source, view);
      return (
        <div
          class={`page-value ${capability.enabled ? "" : "missing"}`}
          title={capability.enabled ? widget.source : capability.reason}
        >
          <span class="page-value-head">
            {widget.label && (
              <span class="page-value-label">{widget.label}</span>
            )}
            {resolved.unit && <em>{resolved.unit}</em>}
          </span>
          <span class="page-value-line">
            <strong>{formatPanelValue(resolved.value, widget.digits)}</strong>
          </span>
        </div>
      );
    }
    if (widget.kind === "numeric") {
      const variable = findPanelSysVar(widget.sysvar, view);
      const disabled = editing || !capability.enabled;
      return (
        <NumericControl
          widget={widget}
          value={variable?.value ?? null}
          min={variable?.min ?? null}
          max={variable?.max ?? null}
          unit={variable?.unit ?? ""}
          disabled={disabled}
          reason={capability.reason}
          onCommit={(value) => {
            if (!editing && capability.enabled) onSetNumber(widget, value);
          }}
        />
      );
    }
    if (widget.kind === "toggle") {
      const variable = findPanelSysVar(widget.sysvar, view);
      const on = (variable?.value ?? 0) !== 0;
      return (
        <button
          type="button"
          class={`page-toggle ${on ? "on" : ""} ${
            capability.enabled ? "" : "disabled"
          }`}
          title={capability.enabled ? widget.sysvar : capability.reason}
          onClick={() => {
            if (!editing && capability.enabled) onToggle(widget, !on);
          }}
        >
          <span class="page-toggle-label">
            {widget.label || widget.sysvar || "Toggle"}
          </span>
          <span class="page-toggle-state">{on ? "ON" : "OFF"}</span>
        </button>
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
          (widget.action === "script"
            ? widget.script || "Script"
            : panelWidgetSpec(widget.kind).fields
                .find((field) => field.key === "action")
                ?.options?.find((option) => option.value === widget.action)
                ?.label) ||
          "Button"}
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
    const value = field(widget, fieldSpec.key);
    const update = (patch: PanelWidgetPatch) =>
      commit(updatePanelWidget(page, widget.id, patch));
    if (fieldSpec.kind === "select") {
      return (
        <label>
          {fieldSpec.label}
          <select
            value={String(value)}
            onFocus={pushHistory}
            onChange={(event) =>
              update({
                [fieldSpec.key]: (event.target as HTMLSelectElement)
                  .value as never,
              } as PanelWidgetPatch)
            }
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
      return (
        <label>
          {fieldSpec.label}
          <input
            type="number"
            step="any"
            value={String(value)}
            placeholder={fieldSpec.placeholder}
            onFocus={pushHistory}
            onInput={(event) =>
              update({
                [fieldSpec.key]: Number(
                  (event.target as HTMLInputElement).value,
                ),
              } as PanelWidgetPatch)
            }
          />
        </label>
      );
    }
    return (
      <label>
        {fieldSpec.label}
        <input
          list={fieldSpec.listId}
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
      <div class={`panel-head ${editing ? "editing" : ""}`}>
        <div class="panel-title">Panel</div>
        <span class="meta">
          {page.width * PAGE_GRID}×{page.height * PAGE_GRID} · {PAGE_GRID}px grid
        </span>
        <div class="page-tabs">
          <div class="page-tab-list" role="tablist" aria-label="Panel pages">
            {book.pages.map((entry) =>
              renaming?.id === entry.id ? (
                <input
                  class="page-tab-rename"
                  autoFocus
                  value={renaming.name}
                  maxLength={40}
                  onInput={(event) =>
                    setRenaming({
                      id: entry.id,
                      name: (event.target as HTMLInputElement).value,
                    })
                  }
                  onBlur={commitRename}
                  onKeyDown={(event) => {
                    if (event.key === "Enter") {
                      event.preventDefault();
                      commitRename();
                    } else if (event.key === "Escape") {
                      event.preventDefault();
                      event.stopPropagation();
                      renameCanceled.current = true;
                      setRenaming(null);
                    }
                  }}
                />
              ) : (
                <button
                  type="button"
                  role="tab"
                  aria-selected={entry.id === page.id}
                  class={`page-tab ${entry.id === page.id ? "on" : ""}`}
                  key={entry.id}
                  title={editing ? "Double-click to rename" : entry.name}
                  onClick={() => switchPage(entry.id)}
                  onDblClick={() => {
                    if (editing) {
                      renameCanceled.current = false;
                      setRenaming({ id: entry.id, name: entry.name });
                    }
                  }}
                >
                  {entry.name}
                </button>
              ),
            )}
          </div>
          {editing && (
            <div class="page-tab-actions">
              <button
                type="button"
                class="mini"
                title="New page"
                onClick={newPage}
              >
                +
              </button>
              <button
                type="button"
                class="mini"
                title="Duplicate this page"
                onClick={copyPage}
              >
                Duplicate
              </button>
              <button
                type="button"
                class="mini"
                title="Delete this page"
                disabled={book.pages.length <= 1}
                onClick={deletePage}
              >
                Delete
              </button>
            </div>
          )}
        </div>
        <div class="panel-tools">
          {editing && (
            <>
              {PANEL_WIDGET_KINDS.map((entry) => (
                <button
                  type="button"
                  class="mini"
                  key={entry.kind}
                  onClick={() => add(entry.kind)}
                >
                  + {entry.label}
                </button>
              ))}
              <button
                type="button"
                class="mini"
                disabled={history.past.length === 0}
                title="Undo (Ctrl+Z)"
                onClick={undo}
              >
                Undo
              </button>
              <button
                type="button"
                class="mini"
                disabled={history.future.length === 0}
                title="Redo (Ctrl+Shift+Z / Ctrl+Y)"
                onClick={redo}
              >
                Redo
              </button>
            </>
          )}
          <button
            type="button"
            class={`mini ${editing ? "on" : ""}`}
            onClick={() => setEditing((old) => !old)}
          >
            {editing ? "Done" : "Edit"}
          </button>
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
      <div class="page-body">
        <div
          class="page-viewport"
          onPointerDown={() => editing && setSelected(null)}
        >
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
          >
            {page.widgets.length === 0 && (
              <div class="page-empty">
                <strong>No controls yet</strong>
                <span>Use Edit, then + Button, + Value or + Toggle.</span>
              </div>
            )}
            {page.widgets.map((widget) => (
              <div
                key={widget.id}
                class={`page-widget ${editing ? "editing" : ""} ${
                  selected === widget.id ? "selected" : ""
                }`}
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
            {selectedWidget ? (
              <>
                <div class="page-inspector-title">
                  {panelWidgetSpec(selectedWidget.kind).label}
                </div>
            {panelWidgetSpec(selectedWidget.kind).fields.map((spec) =>
              inspectorField(selectedWidget, spec.key),
            )}
            <datalist id="page-sources">
              {sourceOptions.map((option) => (
                <option value={option} key={option} />
              ))}
            </datalist>
            <datalist id="page-sysvars">
              {sysvarOptions.map((option) => (
                <option value={option} key={option} />
              ))}
            </datalist>
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
                <div class="page-inspector-title">Panel</div>
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
          </aside>
        )}
      </div>
      {fullscreen && toast && (
        <div class="page-toast" role="status">
          {toast}
        </div>
      )}
    </section>
  );
}