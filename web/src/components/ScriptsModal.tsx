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
import { FilePick } from "./FilePick";
import { ConfirmDialog } from "./ConfirmDialog";
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
  const [dirty, setDirty] = useState<Set<number>>(new Set());
  const [selected, setSelected] = useState<number | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState("");
  const [functions, setFunctions] = useState<ScriptFunctionHelp[]>([]);
  const [signature, setSignature] = useState<SignatureHint | null>(null);
  const [helpOpen, setHelpOpen] = useState(false);
  const [reload, setReload] = useState(0);
  const editing = useRef<Set<number>>(new Set());
  const selectNewest = useRef(false);
  const [name, setName] = useState("");
  const [savePath, setSavePath] = useState("");
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
  const breakpoints = selected != null ? (breakpointMap[selected] ?? []) : [];
  const debugPaused =
    debugArmed && debugView?.paused === true && debugView.node === current?.id;
  const debugLine = debugPaused ? (debugView?.line ?? null) : null;

  const issueLine =
    current?.issue && selected != null && !dirty.has(selected)
      ? current.issue.line
      : null;

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
    setSavePath(current?.filePath || (current ? `${current.name}.rxcan` : ""));
    // Only reset when the selection changes; polling must not overwrite
    // the name or path boxes while the user types.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [selected]);

  const edit = (value: string) => {
    if (selected == null) return;
    editing.current.add(selected);
    setDrafts((currentDrafts) => ({ ...currentDrafts, [selected]: value }));
    setDirty((currentDirty) => new Set(currentDirty).add(selected));
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
    const issue = current?.issue;
    if (!issue) return;
    editorApi.current?.revealLine(issue.line, issue.col ?? 1);
  };

  const apply = () => {
    if (!current) return;
    onCommand({ type: "set_script_source", id: current.id, source: draft });
    editing.current.delete(current.id);
    setDirty((currentDirty) => {
      const next = new Set(currentDirty);
      next.delete(current.id);
      return next;
    });
    setScripts((list) =>
      list.map((script) =>
        script.id === current.id
          ? { ...script, source: draft, issue: null }
          : script,
      ),
    );
  };

  const revert = () => {
    if (!current) return;
    editing.current.delete(current.id);
    setDrafts((currentDrafts) => ({ ...currentDrafts, [current.id]: current.source }));
    setDirty((currentDirty) => {
      const next = new Set(currentDirty);
      next.delete(current.id);
      return next;
    });
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

  const issue = current?.issue ?? null;
  const showIssue = !!issue && selected != null && !dirty.has(selected);
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
          : current && dirty.has(current.id)
            ? "Apply changes before debugging."
            : "F9 toggles a breakpoint on the caret line.";
  const scriptTests = current?.tests ?? [];
  const failedTests = scriptTests.filter((test) => test.failures > 0).length;

  return (
    <>
    <div class="modal-backdrop" onMouseDown={onClose}>
      <div
        class="scripts-modal"
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
                  {dirty.has(script.id) && <i>•</i>}
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
                      disabled={!current.enabled || dirty.has(current.id)}
                      title={
                        !current.enabled
                          ? "Enable the script first"
                          : dirty.has(current.id)
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
                        if (
                          selected != null &&
                          dirty.has(selected) &&
                          !debugPaused
                        ) {
                          apply();
                        }
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
                      <strong>编辑器帮助</strong>
                      <button
                        type="button"
                        onClick={() => setHelpOpen(false)}
                      >
                        关闭
                      </button>
                    </div>
                    <p>
                      脚本是类 C 语言，每次启动都会重新编译；测量中点击 Apply
                      会重启该节点。Tab 缩进，Shift+Tab 反缩进。补全跟随光标，Tab
                      接受选中项；Alt+/ 打开列表，方向键选择后 Enter
                      接受。已知函数会显示参数。括号和双引号自动配对，Backspace
                      删除整级缩进。Ctrl+/ 注释行，Alt+↑/↓ 移动行，Shift+Alt+↓
                      复制行，Ctrl+F 查找，Ctrl+S 应用。F9 切换断点；Debug
                      会暂停节点，方便检查局部变量和调用栈。
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
                {showIssue && issue && (
                  <button type="button" class="script-issue" onClick={jumpToIssue}>
                    <strong>
                      line {issue.line}
                      {issue.col ? `:${issue.col}` : ""}
                    </strong>
                    <span>{issue.message}</span>
                  </button>
                )}
                <div class="script-actions">
                  <button
                    type="button"
                    class="primary"
                    disabled={!dirty.has(current.id) || debugPaused}
                    title={
                      debugPaused
                        ? "Continue or stop the debugger first"
                        : undefined
                    }
                    onClick={apply}
                  >
                    Apply
                  </button>
                  <button
                    type="button"
                    disabled={!dirty.has(current.id)}
                    onClick={revert}
                  >
                    Revert
                  </button>
                  <button
                    type="button"
                    onClick={() => edit(TEMPLATE)}
                  >
                    Template
                  </button>
                  <FilePick
                    label="Load file…"
                    accept=".rxcan,.txt"
                    onText={(text, name) => {
                      edit(text);
                      setSavePath(name);
                    }}
                    onError={setError}
                  />
                  <button
                    type="button"
                    class="danger"
                    disabled={debugPaused}
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
                  <input
                    class="script-path"
                    value={savePath}
                    placeholder="save path (.rxcan)"
                    onInput={(event) =>
                      setSavePath((event.target as HTMLInputElement).value)
                    }
                  />
                  <button
                    type="button"
                    disabled={!savePath.trim()}
                    onClick={() =>
                      onCommand({
                        type: "save_script_file",
                        path: savePath.trim(),
                        source: draft,
                      })
                    }
                  >
                    Save file
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
