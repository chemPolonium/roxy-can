import { useEffect, useRef, useState } from "preact/hooks";
import { fetchScripts } from "../api";
import type { Bus, Command, Script } from "../types";
import { FilePick } from "./FilePick";

interface Props {
  buses: Bus[];
  onCommand: (command: Command) => void;
  onClose: () => void;
}

const TEMPLATE = `on start {
    print("node online");
}

on timer 1000 {
    // set_sig("EngineSpeed", 1000);
}
`;

function busName(buses: Bus[], channel: number): string {
  return buses[channel]?.name ?? `CAN${channel + 1}`;
}

export function ScriptsModal({ buses, onCommand, onClose }: Props) {
  const [scripts, setScripts] = useState<Script[]>([]);
  const [drafts, setDrafts] = useState<Record<number, string>>({});
  const [dirty, setDirty] = useState<Set<number>>(new Set());
  const [selected, setSelected] = useState<number | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState("");
  const [reload, setReload] = useState(0);
  const editing = useRef<Set<number>>(new Set());
  const selectNewest = useRef(false);
  const [name, setName] = useState("");
  const [savePath, setSavePath] = useState("");
  const [newOpen, setNewOpen] = useState(false);
  const [newName, setNewName] = useState("");
  const [newChannel, setNewChannel] = useState(0);
  const [newAttach, setNewAttach] = useState("");

  useEffect(() => {
    const close = (event: KeyboardEvent) => {
      if (event.key === "Escape") onClose();
    };
    window.addEventListener("keydown", close);
    return () => window.removeEventListener("keydown", close);
  }, [onClose]);

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
        script.id === current.id ? { ...script, source: draft } : script,
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

  return (
    <div class="modal-backdrop" onMouseDown={onClose}>
      <div
        class="scripts-modal"
        role="dialog"
        aria-modal="true"
        aria-label="Node scripts"
        onMouseDown={(event) => event.stopPropagation()}
      >
        <header class="modal-head">
          <h2>Node Scripts</h2>
          <div class="picker-head-right">
            <span>{scripts.length} scripts</span>
            <button type="button" onClick={() => setNewOpen((open) => !open)}>
              + New script
            </button>
            <button type="button" onClick={onClose}>
              Close
            </button>
          </div>
        </header>

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
                {(buses[newChannel]?.nodes ?? []).map((node) => (
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
                <textarea
                  class="script-textarea"
                  spellcheck={false}
                  value={draft}
                  placeholder="on start { … }"
                  onInput={(event) =>
                    edit((event.target as HTMLTextAreaElement).value)
                  }
                />
                <div class="script-actions">
                  <button
                    type="button"
                    class="primary"
                    disabled={!dirty.has(current.id)}
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
                    onClick={() => {
                      if (window.confirm(`Remove ${current.name}?`)) {
                        onCommand({ type: "remove_script", id: current.id });
                        setSelected(null);
                        setTimeout(() => setReload((tick) => tick + 1), 500);
                      }
                    }}
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
                <p class="hint">
                  Scripts are C-like and recompile on every start; Apply while
                  measuring restarts the node. No syntax highlighting in the
                  web editor yet.
                </p>
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
  );
}
