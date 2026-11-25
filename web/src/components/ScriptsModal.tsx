import { useEffect, useRef, useState } from "preact/hooks";
import { ModalHeader } from "./ModalHeader";
import { lazy, Suspense } from "preact/compat";
import {
  fetchDebug,
  fetchScriptHelp,
  fetchScripts,
  sendDebug,
} from "../api";
import type {
  Bus,
  Catalog,
  Command,
  DebugView,
  DerivedDef,
  Script,
  ScriptFunctionHelp,
  SysVar,
} from "../types";
import { useFileBrowser } from "./FileBrowser";
import { ConfirmDialog } from "./ConfirmDialog";
import { UiIcon } from "./icons";
import type { ScriptEditorApi } from "../scriptEditor";
import type { Theme } from "../theme";
import {
  buildCandidates,
  callAtCaret,
  localSymbols,
  signatureArgs,
} from "../scriptCompletions";

export {
  buildCandidates,
  callAtCaret,
  localSymbols,
  signatureArgs,
} from "../scriptCompletions";

// CodeMirror is only needed once a script is edited; keep it out of the
// initial shell bundle.
const ScriptCodeEditor = lazy(() =>
  import("./ScriptCodeEditor").then((module) => ({
    default: module.ScriptCodeEditor,
  })),
);

interface Props {
  buses: Bus[];
  catalog: Catalog | null;
  derived: DerivedDef[];
  sysvars: SysVar[];
  theme: Theme;
  onCommand: (command: Command) => void;
  onClose: () => void;
}


interface SignatureHint {
  name: string;
  args: string[];
  active: number;
  left: number;
  top: number;
}


const TEMPLATE = `on start {
    print("node online");
}

on timer 1000 {
    // let cmd = frame(EngineStatus);
    // cmd.EngineSpeed = 1000;
    // send_frame(cmd);
}
`;


function busName(buses: Bus[], channel: number): string {
  return buses[channel]?.name ?? `CAN${channel + 1}`;
}

export function ScriptsModal({
  buses,
  catalog,
  derived,
  sysvars,
  theme,
  onCommand,
  onClose,
}: Props) {
  const [scripts, setScripts] = useState<Script[]>([]);
  const [drafts, setDrafts] = useState<Record<number, string>>({});
  const [checkedDrafts, setCheckedDrafts] = useState<Record<number, string>>({});
  const pickHost = useFileBrowser();
  const [selected, setSelected] = useState<number | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState("");
  const [functions, setFunctions] = useState<ScriptFunctionHelp[]>([]);
  const [signature, setSignature] = useState<SignatureHint | null>(null);
  const [helpOpen, setHelpOpen] = useState(false);
  const [fullscreen, setFullscreen] = useState(false);
  const [reload, setReload] = useState(0);
  const editing = useRef<Set<number>>(new Set());
  const selectNewest = useRef(false);
  const [name, setName] = useState("");
  const [newOpen, setNewOpen] = useState(false);
  const [confirmRemove, setConfirmRemove] = useState(false);
  const [newName, setNewName] = useState("");
  const [newChannel, setNewChannel] = useState(0);
  const [newAttach, setNewAttach] = useState("");
  const [debugArmed, setDebugArmed] = useState(false);
  const [debugView, setDebugView] = useState<DebugView | null>(null);
  const [breakpointMap, setBreakpointMap] = useState<Record<number, number[]>>(
    {},
  );
  const [debugError, setDebugError] = useState("");
  const editorApi = useRef<ScriptEditorApi | null>(null);
  const debugArmedRef = useRef(false);

  useEffect(() => {
    let alive = true;
    void fetchScriptHelp().then((items) => {
      if (alive) setFunctions(items);
    });
    return () => {
      alive = false;
    };
  }, []);

  useEffect(() => {
    // A different script has a different caret context.
    setSignature(null);
    if (debugArmedRef.current) {
      void sendDebug({ action: "stop" }).catch(() => {});
    }
    setDebugArmed(false);
    setDebugView(null);
    setDebugError("");
  }, [selected]);

  useEffect(() => {
    debugArmedRef.current = debugArmed;
  }, [debugArmed]);

  // In fullscreen Esc leaves the editor first; only a second Esc closes the
  // dialog. The capture listener wins over the app's global Escape handler.
  useEffect(() => {
    if (!fullscreen) return;
    const onKey = (event: KeyboardEvent) => {
      if (event.key !== "Escape") return;
      event.preventDefault();
      event.stopImmediatePropagation();
      setFullscreen(false);
    };
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  }, [fullscreen]);

  useEffect(
    () => () => {
      if (debugArmedRef.current) {
        void sendDebug({ action: "stop" }).catch(() => {});
      }
    },
    [],
  );

  useEffect(() => {
    if (!debugArmed) return;
    let alive = true;
    const poll = async () => {
      try {
        const view = await fetchDebug();
        if (!alive) return;
        setDebugView(view);
        if (view.note) setDebugError(view.note);
        if (!view.enabled) setDebugArmed(false);
      } catch (e) {
        if (alive) setDebugError(String(e));
      }
    };
    void poll();
    const timer = setInterval(poll, 400);
    return () => {
      alive = false;
      clearInterval(timer);
    };
  }, [debugArmed]);

  useEffect(() => {
    const close = (event: KeyboardEvent) => {
      if (event.defaultPrevented) return;
      // CodeMirror owns Escape while its search panel or completion list is
      // open; the first Escape dismisses that, not the whole editor.
      if (
        document.querySelector(".cm-search") ||
        document.querySelector(".cm-tooltip-autocomplete")
      ) {
        return;
      }
      if (event.key === "Escape" && helpOpen) {
        setHelpOpen(false);
        return;
      }
      if (event.key === "Escape") onClose();
    };
    window.addEventListener("keydown", close);
    return () => window.removeEventListener("keydown", close);
  }, [helpOpen, onClose]);

  useEffect(() => {
    let alive = true;
    const load = async () => {
      try {
        const next = await fetchScripts();
        if (!alive) return;
        setScripts(next);
        setDrafts((current) => {
          const merged = { ...current };
          for (const script of next) {
            if (!editing.current.has(script.id)) merged[script.id] = script.source;
          }
          return merged;
        });
        setSelected((current) => {
          if (selectNewest.current) {
            selectNewest.current = false;
            const newest = next.reduce<Script | null>(
              (best, script) => (best == null || script.id > best.id ? script : best),
              null,
            );
            return newest?.id ?? null;
          }
          return current != null && next.some((script) => script.id === current)
            ? current
            : (next[0]?.id ?? null);
        });
        setError("");
      } catch (e) {
        if (alive) setError(String(e));
      } finally {
        if (alive) setLoading(false);
      }
    };
    void load();
    const timer = setInterval(load, 1200);
    return () => {
      alive = false;
      clearInterval(timer);
    };
  }, [reload]);

  const current = scripts.find((script) => script.id === selected) ?? null;
  const draft = selected != null ? (drafts[selected] ?? current?.source ?? "") : "";
  const fileDirty = !!current && draft !== current.fileSource;
  const applyDirty = !!current && draft !== current.source;
  const checked = !!current && current.checkedSource === draft && !current.checkIssue;
  const checkAttempt = !!current && checkedDrafts[current.id] === draft;
  const checkIssue = checkAttempt ? (current?.checkIssue ?? null) : null;
  const checkSummary = checked ? current?.checkSummary ?? null : null;
  const activeIssue = checkIssue ?? (!applyDirty ? (current?.issue ?? null) : null);
  const breakpoints = selected != null ? (breakpointMap[selected] ?? []) : [];
  const debugPaused =
    debugArmed && debugView?.paused === true && debugView.node === current?.id;
  const debugLine = debugPaused ? (debugView?.line ?? null) : null;
  const issueLine = activeIssue?.line ?? null;

  useEffect(() => {
    if (debugLine != null) editorApi.current?.revealLine(debugLine);
  }, [debugLine]);

  const revealLine = (line: number) => {
    editorApi.current?.revealLine(line);
  };

  const toggleBreakpoint = (line: number) => {
    if (selected == null) return;
    const key = selected;
    const next = breakpoints.includes(line)
      ? breakpoints.filter((item) => item !== line)
      : [...breakpoints, line].sort((a, b) => a - b);
    setBreakpointMap((map) => ({ ...map, [key]: next }));
    if (debugArmedRef.current) {
      void sendDebug({ action: "set_breakpoints", lines: next }).catch((e) =>
        setDebugError(String(e)),
      );
    }
  };

  const armDebugger = async () => {
    if (!current) return;
    setDebugError("");
    try {
      const view = await sendDebug({
        action: "arm",
        node_id: current.id,
        lines: breakpoints,
      });
      setDebugView(view);
      setDebugArmed(true);
    } catch (e) {
      setDebugError(String(e));
    }
  };

  const stopDebugger = async () => {
    setDebugArmed(false);
    setDebugError("");
    try {
      setDebugView(await sendDebug({ action: "stop" }));
    } catch (e) {
      setDebugError(String(e));
    }
  };

  const runDebug = async (action: "continue" | "step" | "pause") => {
    try {
      setDebugView(await sendDebug({ action }));
    } catch (e) {
      setDebugError(String(e));
    }
  };

  useEffect(() => {
    setName(current?.name ?? "");
    // Only reset when the selection changes; polling must not overwrite
    // the name box while the user types.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [selected]);

  const edit = (value: string) => {
    if (selected == null) return;
    editing.current.add(selected);
    setDrafts((currentDrafts) => ({ ...currentDrafts, [selected]: value }));
    setCheckedDrafts((current) => {
      const next = { ...current };
      delete next[selected];
      return next;
    });
  };

  const complete = (text: string, caret: number, explicit: boolean) =>
    buildCandidates(
      text,
      caret,
      explicit,
      functions,
      derived,
      sysvars,
      catalog,
      current?.channel ?? 0,
    );

  const updateSignature = (
    text: string,
    caret: number,
    coords: { left: number; top: number } | null,
  ) => {
    if (!coords) {
      setSignature(null);
      return;
    }
    const call = callAtCaret(text, caret);
    if (!call) {
      setSignature(null);
      return;
    }
    const builtin = functions.find((item) => item.name === call.name);
    const local = builtin
      ? null
      : localSymbols(text, caret).find(
          (symbol) => symbol.kind === "fn" && symbol.name === call.name,
        );
    if (!builtin && !local) {
      setSignature(null);
      return;
    }
    setSignature({
      name: call.name,
      args: builtin ? signatureArgs(builtin.usage) : (local?.params ?? []),
      active: call.active,
      left: coords.left,
      top: coords.top + 2,
    });
  };

  const jumpToIssue = () => {
    if (!activeIssue) return;
    editorApi.current?.revealLine(activeIssue.line, activeIssue.col ?? 1);
  };

  const check = () => {
    if (!current || debugPaused) return;
    setCheckedDrafts((previous) => ({ ...previous, [current.id]: draft }));
    onCommand({ type: "check_script", id: current.id, source: draft });
  };

  const load = async () => {
    if (!current || debugPaused) return;
    try {
      const path = await pickHost({
        mode: "file",
        title: "Load .rxcan script",
        accept: [".rxcan"],
        initialPath: current.filePath || null,
      });
      if (!path) return;
      editing.current.delete(current.id);
      setCheckedDrafts((previous) => {
        const next = { ...previous };
        delete next[current.id];
        return next;
      });
      onCommand({ type: "load_script", id: current.id, path });
    } catch (e) {
      setError(String(e));
    }
  };

  const apply = () => {
    if (!current || !current.filePath || !checked || debugPaused) return;
    onCommand({ type: "apply_script", id: current.id, source: draft });
    editing.current.delete(current.id);
  };

  const save = () => {
    if (!current?.filePath || !fileDirty || debugPaused) return;
    onCommand({ type: "save_script", id: current.id, source: draft });
  };

  const saveAs = async () => {
    if (!current || debugPaused) return;
    try {
      const path = await pickHost({
        mode: "save",
        title: "Save script as",
        accept: [".rxcan"],
        initialPath: current.filePath || null,
        initialName: `${current.name}.rxcan`,
      });
      if (!path) return;
      onCommand({
        type: "save_script_as",
        id: current.id,
        path,
        source: draft,
      });
    } catch (e) {
      setError(String(e));
    }
  };

  const revert = () => {
    if (!current) return;
    editing.current.delete(current.id);
    setDrafts((currentDrafts) => ({
      ...currentDrafts,
      [current.id]: current.fileSource || current.source,
    }));
  };

  const add = () => {
    const channel = newChannel;
    const [busText, node] = newAttach.split(":");
    const attached_bus = busText === "" ? null : Number(busText);
    onCommand({
      type: "add_script",
      name: newName.trim() || `Node ${scripts.length + 1}`,
      channel,
      attached_bus,
      attached_node: attached_bus == null ? null : node,
    });
    setNewOpen(false);
    setNewName("");
    setNewAttach("");
    selectNewest.current = true;
    setTimeout(() => setReload((tick) => tick + 1), 700);
  };

  const debugStatus = debugError
    ? debugError
    : debugPaused
      ? `Paused at line ${debugLine}${
          debugView?.function ? ` in ${debugView.function}` : ""
        }. The node is blocked until Continue, Step or Stop.`
      : debugArmed
        ? "Armed — waiting for a breakpoint. Start or restart the measurement."
        : current && !current.enabled
          ? "Enable the script to debug it."
          : current && applyDirty
            ? "Apply changes before debugging."
            : "F9 toggles a breakpoint on the caret line.";
  const scriptTests = current?.tests ?? [];
  const failedTests = scriptTests.filter((test) => test.failures > 0).length;

  return (
    <>
    <div class="modal-backdrop" onMouseDown={onClose}>
      <div
        class={`scripts-modal${fullscreen ? " fullscreen" : ""}`}
        role="dialog"
        aria-modal="true"
        aria-label="Node scripts"
        onMouseDown={(event) => event.stopPropagation()}
      >
        <ModalHeader title="Node Scripts" onClose={onClose}>
          <span>{scripts.length} scripts</span>
          <button type="button" onClick={() => setNewOpen((open) => !open)}>
            + New script
          </button>
          <button
            type="button"
            class="mini icon-btn"
            aria-label={
              fullscreen ? "Collapse the editor" : "Expand the editor"
            }
            title={
              fullscreen
                ? "Leave the full-screen editor (Esc)"
                : "Expand the editor to fill the screen"
            }
            onClick={() => setFullscreen((value) => !value)}
          >
            <UiIcon name={fullscreen ? "collapse" : "expand"} />
          </button>
        </ModalHeader>

        {newOpen && (
          <form
            class="script-new"
            onSubmit={(event) => {
              event.preventDefault();
              add();
            }}
          >
            <input
              value={newName}
              placeholder={`Node ${scripts.length + 1}`}
              onInput={(event) =>
                setNewName((event.target as HTMLInputElement).value)
              }
            />
            <label>
              Bus
              <select
                value={newChannel}
                onChange={(event) => {
                  setNewChannel(Number((event.target as HTMLSelectElement).value));
                  setNewAttach("");
                }}
              >
                {buses.map((bus) => (
                  <option key={bus.index} value={bus.index}>
                    {bus.name}
                  </option>
                ))}
              </select>
            </label>
            <label>
              Bind to
              <select
                value={newAttach}
                onChange={(event) =>
                  setNewAttach((event.target as HTMLSelectElement).value)
                }
              >
                <option value="">unbound script</option>
                {(buses[newChannel]?.can?.nodes ?? []).map((node) => (
                  <option key={node.name} value={`${newChannel}:${node.name}`}>
                    {node.name}
                  </option>
                ))}
              </select>
            </label>
            <button type="submit">Add</button>
          </form>
        )}

        <div class="scripts-body">
          <aside class="scripts-list">
            {scripts.map((script) => (
              <button
                type="button"
                key={script.id}
                class={`script-item ${script.id === selected ? "on" : ""} ${
                  script.errored ? "bad" : ""
                }`}
                onClick={() => setSelected(script.id)}
              >
                <strong>
                  {script.name}
                  {script.id === selected && fileDirty && <i>•</i>}
                </strong>
                <span>
                  {busName(buses, script.channel)}
                  {script.attachedNode ? ` · ${script.attachedNode}` : ""}
                </span>
                <span class="script-chips">
                  {!script.enabled && <em>disabled</em>}
                  {script.running && <em class="ok">running</em>}
                  {script.errored && <em class="bad">error</em>}
                  {script.tests.length > 0 && (
                    <em
                      class={
                        script.tests.some((test) => test.failures > 0)
                          ? "bad"
                          : "ok"
                      }
                    >
                      {script.tests.filter((test) => test.failures === 0).length}
                      /{script.tests.length} tests
                    </em>
                  )}
                </span>
              </button>
            ))}
            {!loading && scripts.length === 0 && (
              <div class="script-empty">
                No scripts yet. Add one and bind it to a DBC node.
              </div>
            )}
            {loading && <div class="script-empty">Loading scripts…</div>}
            {error && <div class="script-empty bad">{error}</div>}
          </aside>

          <section class="scripts-editor">
            {current ? (
              <>
                <div class="script-meta">
                  <label>
                    Name
                    <input
                      value={name}
                      onInput={(event) =>
                        setName((event.target as HTMLInputElement).value)
                      }
                      onBlur={() => {
                        const value = name.trim();
                        if (value && value !== current.name) {
                          onCommand({
                            type: "set_script_name",
                            id: current.id,
                            name: value,
                          });
                        }
                      }}
                    />
                  </label>
                  <span>
                    {busName(buses, current.channel)}
                    {current.attachedNode
                      ? ` · bound to ${current.attachedNode}`
                      : " · unbound"}
                  </span>
                  <label class="script-enable">
                    <input
                      type="checkbox"
                      checked={current.enabled}
                      onChange={(event) =>
                        onCommand({
                          type: "set_script_enabled",
                          id: current.id,
                          on: (event.target as HTMLInputElement).checked,
                        })
                      }
                    />
                    Enabled
                  </label>
                </div>
                <div class={`script-debug-bar ${debugArmed ? "on" : ""}`}>
                  {debugArmed ? (
                    <>
                      <button
                        type="button"
                        class="primary"
                        disabled={!debugPaused}
                        onClick={() => void runDebug("continue")}
                      >
                        Continue
                      </button>
                      <button
                        type="button"
                        disabled={!debugPaused}
                        onClick={() => void runDebug("step")}
                      >
                        Step
                      </button>
                      <button
                        type="button"
                        disabled={debugPaused}
                        onClick={() => void runDebug("pause")}
                      >
                        Pause
                      </button>
                      <button
                        type="button"
                        disabled={debugPaused}
                        title={
                          debugPaused
                            ? "Continue or stop before restarting"
                            : "Apply the editor and restart this node"
                        }
                        onClick={apply}
                      >
                        Restart
                      </button>
                      <button type="button" onClick={() => void stopDebugger()}>
                        Stop
                      </button>
                    </>
                  ) : (
                    <button
                      type="button"
                      class="primary"
                      disabled={!current.enabled || applyDirty}
                      title={
                        !current.enabled
                          ? "Enable the script first"
                          : applyDirty
                            ? "Apply changes before debugging"
                            : "Pause on breakpoints; F9 toggles one"
                      }
                      onClick={() => void armDebugger()}
                    >
                      Debug
                    </button>
                  )}
                  <span class={`script-debug-status ${debugError ? "bad" : ""}`}>
                    {debugStatus}
                  </span>
                  <button
                    type="button"
                    class="mini icon-btn script-help-btn"
                    title="编辑器帮助"
                    aria-label="编辑器帮助"
                    aria-expanded={helpOpen}
                    onClick={() => setHelpOpen((open) => !open)}
                  >
                    ?
                  </button>
                </div>
                <div class="script-cm-wrap">
                  <Suspense
                    fallback={
                      <div class="script-cm-loading">Loading editor…</div>
                    }
                  >
                    <ScriptCodeEditor
                      doc={draft}
                      scriptId={selected}
                      theme={theme}
                      breakpoints={breakpoints}
                      debugLine={debugLine}
                      issueLine={issueLine}
                      completion={complete}
                      onChange={edit}
                      onToggleBreakpoint={toggleBreakpoint}
                      onApply={() => {
                        if (selected == null || debugPaused) return;
                        if (current?.filePath) save();
                        else void saveAs();
                      }}
                      onCursor={updateSignature}
                      apiRef={editorApi}
                    />
                  </Suspense>
                  {signature && (
                    <div
                      class="script-signature"
                      aria-hidden="true"
                      style={{
                        left: `${signature.left}px`,
                        top: `${signature.top}px`,
                      }}
                    >
                      {signature.name}(
                      {signature.args.map((arg, index) => (
                        <span key={index}>
                          {index > 0 ? ", " : ""}
                          <span
                            class={
                              index ===
                              Math.min(
                                signature.active,
                                signature.args.length - 1,
                              )
                                ? "on"
                                : ""
                            }
                          >
                            {arg}
                          </span>
                        </span>
                      ))}
                      )
                    </div>
                  )}
                </div>
                {helpOpen && (
                  <div
                    class="script-help-pop"
                    role="dialog"
                    aria-label="编辑器帮助"
                  >
                    <div class="script-help-head">
                      <strong>Editor help</strong>
                      <button
                        type="button"
                        onClick={() => setHelpOpen(false)}
                      >
                        Close
                      </button>
                    </div>
                    <p>
                      Scripts use a C-like language. Check only compiles and
                      reports diagnostics; Apply installs a checked draft into
                      the node and restarts it during a measurement. Save and
                      Save As only write the linked .rxcan file, without
                      applying. Load links an existing file without applying;
                      Enabled is a separate runtime switch. Tab indents,
                      Shift+Tab outdents, and Tab accepts a completion. Alt+/
                      opens the completion list; Enter accepts a selection.
                      Known functions show their arguments. Brackets and quotes
                      pair automatically, Backspace removes one indent level,
                      Ctrl+/ comments a line, Alt+↑/↓ moves a line,
                      Shift+Alt+↓ duplicates it, Ctrl+F searches and Ctrl+S
                      saves. F9 toggles a breakpoint; Debug pauses the node so
                      locals and the call stack can be inspected.
                    </p>
                  </div>
                )}
                {debugPaused && debugView && (
                  <div class="script-debug-panel">
                    <section>
                      <h4>Call stack</h4>
                      <div class="script-debug-rows">
                        {[...debugView.stack].reverse().map((frame, index) => (
                          <button
                            type="button"
                            key={`${frame.function}:${index}`}
                            class={index === 0 ? "on" : ""}
                            onClick={() => revealLine(frame.line)}
                          >
                            <strong>{frame.function}</strong>
                            <span>line {frame.line}</span>
                          </button>
                        ))}
                      </div>
                    </section>
                    <section>
                      <h4>Locals</h4>
                      <div class="script-debug-rows">
                        {debugView.locals.length === 0 ? (
                          <span class="script-debug-empty">None</span>
                        ) : (
                          debugView.locals.map((value, index) => (
                            <div
                              key={`${value.name}:${index}`}
                              class="script-debug-row"
                            >
                              <strong>{value.name}</strong>
                              <span>{value.value}</span>
                            </div>
                          ))
                        )}
                      </div>
                    </section>
                    <section>
                      <h4>Globals</h4>
                      <div class="script-debug-rows">
                        {debugView.globals.length === 0 ? (
                          <span class="script-debug-empty">None</span>
                        ) : (
                          debugView.globals.map((value, index) => (
                            <div
                              key={`${value.name}:${index}`}
                              class="script-debug-row"
                            >
                              <strong>{value.name}</strong>
                              <span>{value.value}</span>
                            </div>
                          ))
                        )}
                      </div>
                    </section>
                  </div>
                )}
                {activeIssue && (
                  <button
                    type="button"
                    class="script-issue"
                    onClick={jumpToIssue}
                  >
                    <strong>
                      line {activeIssue.line}
                      {activeIssue.col ? `:${activeIssue.col}` : ""}
                    </strong>
                    <span>{activeIssue.message}</span>
                  </button>
                )}
                <div class="script-state-row">
                  <span class={!current?.filePath || fileDirty ? "warn" : "ok"}>
                    {!current?.filePath
                      ? "No .rxcan linked — use Save As"
                      : fileDirty
                        ? "File not saved"
                        : "File saved"}
                  </span>
                  <span class={checked ? "ok" : "muted"}>
                    {checked ? "Checked" : "Not checked"}
                  </span>
                  <span class={applyDirty ? "warn" : "ok"}>
                    {applyDirty ? "Not applied" : "Applied"}
                  </span>
                  {checkSummary && (
                    <span class="script-check-summary">
                      {checkSummary.handlers} handlers · sends{" "}
                      {checkSummary.sends.join(", ") || "—"} · receives{" "}
                      {checkSummary.receives.join(", ") || "—"}
                    </span>
                  )}
                </div>
                {checkSummary?.lines.map((line, index) => (
                  <div class="script-check-line" key={index}>
                    {line}
                  </div>
                ))}
                <div class="script-actions">
                  <button
                    type="button"
                    disabled={!current || debugPaused}
                    title={
                      debugPaused
                        ? "Continue or stop the debugger first"
                        : undefined
                    }
                    onClick={check}
                  >
                    Check
                  </button>
                  <button
                    type="button"
                    class="primary"
                    disabled={
                      !current?.filePath ||
                      !checked ||
                      !applyDirty ||
                      debugPaused
                    }
                    title={
                      debugPaused
                        ? "Continue or stop the debugger first"
                        : !current?.filePath
                          ? "Save As a .rxcan file before applying"
                          : !checked
                            ? "Check the script before applying"
                            : !applyDirty
                              ? "Already applied"
                              : undefined
                    }
                    onClick={apply}
                  >
                    Apply
                  </button>
                  <button
                    type="button"
                    disabled={!fileDirty}
                    title={fileDirty ? undefined : "No unsaved file changes"}
                    onClick={revert}
                  >
                    Revert
                  </button>
                  <button type="button" onClick={() => edit(TEMPLATE)}>
                    Template
                  </button>
                  <button
                    type="button"
                    disabled={!current || debugPaused}
                    title={
                      debugPaused
                        ? "Continue or stop the debugger first"
                        : "Link an existing .rxcan file; this does not apply it"
                    }
                    onClick={() => void load()}
                  >
                    Load…
                  </button>
                  <button
                    type="button"
                    class="danger"
                    disabled={!current || debugPaused}
                    title={
                      debugPaused
                        ? "Continue or stop the debugger first"
                        : undefined
                    }
                    onClick={() => setConfirmRemove(true)}
                  >
                    Remove
                  </button>
                  <span class="spacer" />
                  <span class="script-file-path" title={current?.filePath ?? ""}>
                    {current?.filePath || "No file linked"}
                  </span>
                  <button
                    type="button"
                    disabled={!current?.filePath || !fileDirty || debugPaused}
                    title={
                      !current?.filePath
                        ? "Use Save As to choose a file"
                        : !fileDirty
                          ? "No unsaved file changes"
                          : debugPaused
                            ? "Continue or stop the debugger first"
                            : undefined
                    }
                    onClick={save}
                  >
                    Save
                  </button>
                  <button
                    type="button"
                    disabled={!current || debugPaused}
                    onClick={() => void saveAs()}
                  >
                    Save As…
                  </button>
                </div>
                {scriptTests.length > 0 && (
                  <div class="script-tests">
                    <header class={failedTests > 0 ? "bad" : "ok"}>
                      <strong>Tests</strong>
                      <span>
                        {scriptTests.length - failedTests}/{scriptTests.length}{" "}
                        passed
                        {failedTests > 0
                          ? ` · ${failedTests} failed`
                          : ""}
                      </span>
                    </header>
                    <div class="script-test-rows">
                      {scriptTests.map((test) => (
                        <button
                          type="button"
                          key={test.name}
                          class={`script-test ${
                            test.failures > 0 ? "bad" : "ok"
                          }`}
                          title={
                            test.line > 0
                              ? `Jump to line ${test.line}`
                              : undefined
                          }
                          onClick={() => {
                            if (test.line > 0) revealLine(test.line);
                          }}
                        >
                          <span class="script-test-state">
                            {test.failures > 0 ? "FAIL" : "PASS"}
                          </span>
                          <strong>{test.name}</strong>
                          <span class="script-test-meta">
                            {test.runs} run{test.runs === 1 ? "" : "s"}
                            {test.line > 0 ? ` · line ${test.line}` : ""}
                            {` · ${test.timeS.toFixed(3)} s`}
                          </span>
                        </button>
                      ))}
                    </div>
                  </div>
                )}
                <div class={`script-log ${current.errored ? "bad" : ""}`}>
                  {current.log.length === 0 ? (
                    <span class="script-empty-line">
                      No output yet; print() and runtime errors appear here.
                    </span>
                  ) : (
                    current.log.slice(-8).map((line, index) => (
                      <div key={index}>{line}</div>
                    ))
                  )}
                </div>
              </>
            ) : (
              <div class="script-empty">
                Select a script on the left, or add one above.
              </div>
            )}
          </section>
        </div>
      </div>
    </div>
    {confirmRemove && current && (
      <ConfirmDialog
        title="Remove script"
        message={`Remove ${current.name}? Its source and runtime state are discarded.`}
        danger
        onConfirm={() => {
          onCommand({ type: "remove_script", id: current.id });
          setSelected(null);
          setConfirmRemove(false);
          setTimeout(() => setReload((tick) => tick + 1), 500);
        }}
        onClose={() => setConfirmRemove(false)}
      />
    )}
    </>
  );
}
