import { useEffect, useState } from "preact/hooks";
import type {
  Bus,
  Catalog,
  DerivedDef,
  TriggerEdit,
  TriggerEntry,
  TriggerLimits,
  TxEntry,
} from "../types";

interface Props {
  triggers: TriggerEntry[];
  limits: TriggerLimits;
  buses: Bus[];
  txEntries: TxEntry[];
  catalog: Catalog | null;
  catalogLoading: boolean;
  derived: DerivedDef[];
  onAdd: (kind: string) => void;
  onToggle: (index: number, on: boolean) => void;
  onRemove: (index: number) => void;
  onRearm: () => void;
  onEdit: (edit: TriggerEdit) => void;
  onLimits: (limits: TriggerLimits) => void;
  onClose: () => void;
}

const KINDS: [string, string][] = [
  ["signal", "+ Signal"],
  ["id", "+ ID"],
  ["error", "+ Error frames"],
  ["timeout", "+ Timeout"],
  ["sysvar", "+ SysVar"],
  ["derived", "+ Derived"],
  ["fr_frame", "+ FR frame"],
  ["fr_signal", "+ FR signal"],
];

const ACTIONS: [string, string][] = [
  ["start_recording", "Start recording"],
  ["stop_recording", "Stop recording"],
  ["send", "Send generator entry"],
  ["clear_trace", "Clear trace"],
  ["insert_marker", "Insert marker"],
];

function hexId(id: number): string {
  return `0x${id.toString(16).toUpperCase().padStart(3, "0")}`;
}

function parseHex(text: string): number {
  const value = Number.parseInt(text.trim().replace(/^0x/i, ""), 16);
  return Number.isFinite(value) ? value : 0;
}

function timeText(us: number): string {
  if (us === 0) return "—";
  return `${(us / 1e6).toFixed(3)} s`;
}

interface Draft {
  index: number;
  kind: string;
  ch: number;
  idText: string;
  ext: boolean;
  signal: string;
  threshold: string;
  rising: boolean;
  bus: number;
  slot: number;
  key: string;
  derivedId: string;
  action: string;
  sendCh: number;
  sendId: number;
}

function draftOf(index: number, trigger: TriggerEntry): Draft {
  return {
    index,
    kind: trigger.kind,
    ch: trigger.ch,
    idText: trigger.id ? trigger.id.toString(16).toUpperCase() : "100",
    ext: trigger.ext,
    signal: trigger.signal,
    threshold: trigger.threshold.toString(),
    rising: trigger.rising,
    bus: trigger.bus,
    slot: trigger.slot,
    key: trigger.varKey || trigger.signal,
    derivedId: trigger.derivedId || "",
    action: trigger.actionKind,
    sendCh: trigger.sendCh,
    sendId: trigger.sendId,
  };
}

function TriggerEditor({
  draft,
  buses,
  txEntries,
  catalog,
  catalogLoading,
  derived,
  onChange,
  onCancel,
  onSave,
}: {
  draft: Draft;
  buses: Bus[];
  txEntries: TxEntry[];
  catalog: Catalog | null;
  catalogLoading: boolean;
  derived: DerivedDef[];
  onChange: (next: Draft) => void;
  onCancel: () => void;
  onSave: () => void;
}) {
  const set = (patch: Partial<Draft>) => onChange({ ...draft, ...patch });
  const messages = catalog?.channels[draft.ch]?.messages ?? [];
  const id = parseHex(draft.idText);
  const message =
    messages.find(
      (candidate) => candidate.id === id && candidate.extended === draft.ext,
    ) ?? null;
  const signals = message?.signals ?? [];
  const needsSignal = draft.kind === "signal" || draft.kind === "fr_signal";
  const isFr = draft.kind === "fr_frame" || draft.kind === "fr_signal";
  const isDerived = draft.kind === "derived";
  const needsThreshold =
    draft.kind === "signal" ||
    draft.kind === "fr_signal" ||
    draft.kind === "sysvar" ||
    isDerived;
  const needsTarget = !isFr && draft.kind !== "sysvar" && !isDerived;
  const canSave =
    (draft.kind !== "sysvar" || draft.key.trim().length > 0) &&
    (!isDerived || draft.derivedId.length > 0) &&
    (!needsSignal || draft.signal.trim().length > 0);

  const chooseMessage = (messageId: number, extended: boolean) => {
    const chosen = messages.find(
      (candidate) => candidate.id === messageId && candidate.extended === extended,
    );
    set({
      idText: messageId.toString(16).toUpperCase(),
      ext: extended,
      signal: chosen?.signals[0]?.name ?? draft.signal,
    });
  };

  const sendKey = `${draft.sendCh}:${draft.sendId}`;

  return (
    <div class="trig-editor">
      <div class="trig-editor-head">
        <strong>
          Edit rule {draft.index + 1} · {draft.kind}
        </strong>
        <span class="spacer" />
        <button type="button" onClick={onCancel}>
          Cancel
        </button>
        <button type="button" class="primary" disabled={!canSave} onClick={onSave}>
          Save rule
        </button>
      </div>

      <div class="trig-fields">
        {needsTarget && !isFr && (
          <>
            <label class="trig-field">
              Bus
              <select
                value={draft.ch}
                onChange={(event) =>
                  set({ ch: Number((event.target as HTMLSelectElement).value) })
                }
              >
                {buses.map((bus) => (
                  <option key={bus.index} value={bus.index}>
                    {bus.name}
                  </option>
                ))}
              </select>
            </label>
            <label class="trig-field">
              ID
              <input
                value={draft.idText}
                class="trig-hex"
                onInput={(event) =>
                  set({ idText: (event.target as HTMLInputElement).value })
                }
              />
            </label>
          </>
        )}

        {draft.kind === "signal" && (
          <>
            <label class="trig-field">
              Message
              <select
                value={message ? String(message.id) : ""}
                disabled={catalogLoading || messages.length === 0}
                onChange={(event) => {
                  const chosen = messages.find(
                    (candidate) =>
                      candidate.id ===
                      Number((event.target as HTMLSelectElement).value),
                  );
                  if (chosen) chooseMessage(chosen.id, chosen.extended);
                }}
              >
                <option value="">
                  {catalogLoading ? "Loading…" : "Pick a message…"}
                </option>
                {messages.map((candidate) => (
                  <option key={candidate.id} value={candidate.id}>
                    {hexId(candidate.id)} {candidate.name}
                  </option>
                ))}
              </select>
            </label>
            <label class="trig-field trig-wide">
              Signal
              <select
                value={draft.signal}
                disabled={signals.length === 0}
                onChange={(event) =>
                  set({
                    signal: (event.target as HTMLSelectElement).value,
                  })
                }
              >
                <option value="">Pick a signal…</option>
                {signals.map((signal) => (
                  <option key={signal.name} value={signal.name}>
                    {signal.name}
                  </option>
                ))}
                {draft.signal &&
                  !signals.some((signal) => signal.name === draft.signal) && (
                    <option value={draft.signal}>{draft.signal}</option>
                  )}
              </select>
            </label>
          </>
        )}

        {isFr && (
          <>
            <label class="trig-field">
              FR bus
              <input
                type="number"
                min="0"
                value={draft.bus}
                onInput={(event) =>
                  set({ bus: Number((event.target as HTMLInputElement).value) })
                }
              />
            </label>
            <label class="trig-field">
              Slot
              <input
                type="number"
                min="0"
                value={draft.slot}
                onInput={(event) =>
                  set({ slot: Number((event.target as HTMLInputElement).value) })
                }
              />
            </label>
            {draft.kind === "fr_signal" && (
              <label class="trig-field">
                Signal
                <input
                  value={draft.signal}
                  onInput={(event) =>
                    set({ signal: (event.target as HTMLInputElement).value })
                  }
                />
              </label>
            )}
          </>
        )}

        {draft.kind === "sysvar" && (
          <label class="trig-field trig-wide">
            Variable
            <input
              value={draft.key}
              placeholder="Namespace.Variable"
              onInput={(event) =>
                set({ key: (event.target as HTMLInputElement).value })
              }
            />
          </label>
        )}

        {isDerived && (
          <label class="trig-field trig-wide">
            Derived channel
            <select
              value={draft.derivedId}
              onChange={(event) =>
                set({
                  derivedId: (event.target as HTMLSelectElement).value,
                })
              }
            >
              <option value="">
                {derived.length === 0 ? "No derived channels yet" : "Pick a channel…"}
              </option>
              {derived.map((definition) => (
                <option key={definition.id} value={definition.id}>
                  {definition.name} · {definition.expression}
                </option>
              ))}
              {draft.derivedId &&
                !derived.some((definition) => definition.id === draft.derivedId) && (
                  <option value={draft.derivedId}>{draft.derivedId}</option>
                )}
            </select>
          </label>
        )}

        {needsThreshold && (
          <>
            <label class="trig-field">
              Threshold
              <input
                type="number"
                step="any"
                value={draft.threshold}
                onInput={(event) =>
                  set({
                    threshold: (event.target as HTMLInputElement).value,
                  })
                }
              />
            </label>
            <label class="trig-field">
              Edge
              <select
                value={draft.rising ? "1" : "0"}
                onChange={(event) =>
                  set({
                    rising: (event.target as HTMLSelectElement).value === "1",
                  })
                }
              >
                <option value="1">crossing up ≥</option>
                <option value="0">crossing down ≤</option>
              </select>
            </label>
          </>
        )}

        <label class="trig-field trig-wide">
          Action
          <select
            value={draft.action}
            onChange={(event) =>
              set({ action: (event.target as HTMLSelectElement).value })
            }
          >
            {ACTIONS.map(([value, label]) => (
              <option key={value} value={value}>
                {label}
              </option>
            ))}
          </select>
        </label>

        {draft.action === "send" && (
          <>
            {txEntries.length > 0 ? (
              <label class="trig-field trig-wide">
                Entry
                <select
                  value={sendKey}
                  onChange={(event) => {
                    const [ch, id] = (
                      event.target as HTMLSelectElement
                    ).value.split(":");
                    set({ sendCh: Number(ch), sendId: Number(id) });
                  }}
                >
                  {txEntries.map((entry) => (
                    <option
                      key={`${entry.channel}:${entry.id}`}
                      value={`${entry.channel}:${entry.id}`}
                    >
                      {entry.bus} {hexId(entry.id)} {entry.name}
                    </option>
                  ))}
                </select>
              </label>
            ) : (
              <>
                <label class="trig-field">
                  Send bus
                  <input
                    type="number"
                    min="0"
                    value={draft.sendCh}
                    onInput={(event) =>
                      set({
                        sendCh: Number(
                          (event.target as HTMLInputElement).value,
                        ),
                      })
                    }
                  />
                </label>
                <label class="trig-field">
                  Send ID
                  <input
                    value={draft.sendId.toString(16)}
                    class="trig-hex"
                    onInput={(event) =>
                      set({
                        sendId: parseHex(
                          (event.target as HTMLInputElement).value,
                        ),
                      })
                    }
                  />
                </label>
              </>
            )}
          </>
        )}
      </div>
    </div>
  );
}

export function TriggersModal({
  triggers,
  limits,
  buses,
  txEntries,
  catalog,
  catalogLoading,
  derived,
  onAdd,
  onToggle,
  onRemove,
  onRearm,
  onEdit,
  onLimits,
  onClose,
}: Props) {
  const [draft, setDraft] = useState<Draft | null>(null);
  const [pre, setPre] = useState(String(limits.preFrames));
  const [post, setPost] = useState(String(limits.postFrames));
  const [cap, setCap] = useState(String(limits.markerCap));

  useEffect(() => {
    setPre(String(limits.preFrames));
    setPost(String(limits.postFrames));
    setCap(String(limits.markerCap));
  }, [limits.preFrames, limits.postFrames, limits.markerCap]);

  useEffect(() => {
    const close = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        if (draft) setDraft(null);
        else onClose();
      }
    };
    window.addEventListener("keydown", close);
    return () => window.removeEventListener("keydown", close);
  }, [draft, onClose]);

  const save = () => {
    if (!draft) return;
    onEdit({
      index: draft.index,
      kind: draft.kind,
      ch: draft.ch,
      id: parseHex(draft.idText),
      ext: draft.ext,
      signal: draft.signal.trim(),
      threshold: Number(draft.threshold) || 0,
      rising: draft.rising,
      bus: draft.bus,
      slot: draft.slot,
      key: draft.key.trim(),
      derivedId: draft.derivedId,
      action: draft.action,
      sendCh: draft.sendCh,
      sendId: draft.sendId,
    });
    setDraft(null);
  };

  return (
    <div class="modal-backdrop" onMouseDown={onClose}>
      <div
        class="trigger-modal"
        role="dialog"
        aria-modal="true"
        aria-label="Triggers"
        onMouseDown={(event) => event.stopPropagation()}
      >
        <header class="modal-head">
          <h2>Triggers</h2>
          <div class="picker-head-right">
            <span>
              {triggers.length} rules ·{" "}
              {triggers.filter((trigger) => trigger.enabled).length} enabled
            </span>
            <button type="button" onClick={onRearm}>
              Re-arm latched
            </button>
            <button type="button" onClick={onClose}>
              Close
            </button>
          </div>
        </header>

        <div class="trig-actions">
          {KINDS.map(([kind, label]) => (
            <button type="button" key={kind} onClick={() => onAdd(kind)}>
              {label}
            </button>
          ))}
          <span class="spacer" />
          <label class="trig-limit">
            pre
            <input
              type="number"
              min="0"
              value={pre}
              onInput={(event) =>
                setPre((event.target as HTMLInputElement).value)
              }
            />
          </label>
          <label class="trig-limit">
            post
            <input
              type="number"
              min="0"
              value={post}
              onInput={(event) =>
                setPost((event.target as HTMLInputElement).value)
              }
            />
          </label>
          <label class="trig-limit">
            markers
            <input
              type="number"
              min="8"
              value={cap}
              onInput={(event) =>
                setCap((event.target as HTMLInputElement).value)
              }
            />
          </label>
          <button
            type="button"
            onClick={() =>
              onLimits({
                preFrames: Math.max(0, Number(pre) || 0),
                postFrames: Math.max(0, Number(post) || 0),
                markerCap: Math.max(8, Number(cap) || 8),
              })
            }
          >
            Apply limits
          </button>
        </div>

        <div class="trig-body">
          {draft && (
            <TriggerEditor
              draft={draft}
              buses={buses}
              txEntries={txEntries}
              catalog={catalog}
              catalogLoading={catalogLoading}
              derived={derived}
              onChange={setDraft}
              onCancel={() => setDraft(null)}
              onSave={save}
            />
          )}
          {triggers.length === 0 && (
            <div class="empty">
              <strong>No trigger rules</strong>
              <span>Add a default rule above, then edit its condition.</span>
            </div>
          )}
          {triggers.map((trigger, index) => (
            <div class={`trig-row ${trigger.enabled ? "" : "off"}`} key={index}>
              <label class="gen-active">
                <input
                  type="checkbox"
                  checked={trigger.enabled}
                  onChange={(event) =>
                    onToggle(
                      index,
                      (event.target as HTMLInputElement).checked,
                    )
                  }
                />
                On
              </label>
              <span class="trig-summary" title={trigger.summary}>
                {trigger.summary}
              </span>
              {trigger.level && (
                <span class="trig-level" title="Condition is currently true" />
              )}
              <span class="trig-action">{trigger.action}</span>
              <span class="trig-fired">{trigger.fired} fired</span>
              <span class="trig-last">{timeText(trigger.lastFireUs)}</span>
              <button
                type="button"
                class="mini"
                onClick={() => setDraft(draftOf(index, trigger))}
              >
                Edit
              </button>
              <button
                type="button"
                class="bus-remove"
                onClick={() => onRemove(index)}
              >
                Remove
              </button>
            </div>
          ))}
        </div>

        <p class="hint picker-hint">
          Edits are applied on Save. Appearance rules latch until Re-arm; the
          pre/post/marker limits apply to trigger-started recordings.
        </p>
      </div>
    </div>
  );
}
