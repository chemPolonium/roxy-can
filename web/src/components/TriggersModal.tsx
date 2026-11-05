import { useEffect, useState } from "preact/hooks";
import { FilePick } from "./FilePick";
import { checkTriggers } from "../triggerCheck";
import { MenuButton } from "./MenuButton";
import type {
  Bus,
  Catalog,
  DerivedDef,
  FrTxEntry,
  ScriptInfo,
  SysVar,
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
  frEntries: FrTxEntry[];
  catalog: Catalog | null;
  catalogLoading: boolean;
  derived: DerivedDef[];
  sysvars: SysVar[];
  scripts: ScriptInfo[];
  onAdd: (kind: string) => void;
  onToggle: (index: number, on: boolean) => void;
  onRemove: (index: number) => void;
  onRearm: () => void;
  onEdit: (edit: TriggerEdit) => void;
  onLimits: (limits: TriggerLimits) => void;
  onRearmOne: (index: number) => void;
  onExport: () => void;
  onImport: (text: string) => void;
  onClose: () => void;
}

const KIND_GROUPS: { title: string; kinds: [string, string][] }[] = [
  {
    title: "CAN",
    kinds: [
      ["signal", "Signal crossing"],
      ["id", "Message present"],
      ["error", "Error frame"],
      ["timeout", "Cycle timeout"],
      ["payload", "Payload byte"],
      ["e2e", "E2E gap/jump"],
    ],
  },
  {
    title: "FlexRay",
    kinds: [
      ["fr_frame", "Frame present"],
      ["fr_signal", "Signal crossing"],
      ["fr_timeout", "Slot timeout"],
    ],
  },
  {
    title: "Ethernet",
    kinds: [
      ["eth_flow", "Flow present"],
      ["someip", "SOME/IP service"],
    ],
  },
  {
    title: "Global",
    kinds: [
      ["sysvar", "System variable"],
      ["derived", "Derived signal"],
    ],
  },
];


const ACTIONS: [string, string][] = [
  ["start_recording", "Start recording"],
  ["stop_recording", "Stop recording"],
  ["send", "Send CAN entry"],
  ["send_fr", "Send FlexRay entry"],
  ["send_eth", "Send Ethernet frame"],
  ["send_someip", "Send SOME/IP message"],
  ["clear_trace", "Clear trace"],
  ["insert_marker", "Insert marker"],
  ["set_sysvar", "Set system variable"],
  ["script", "Run script action"],
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
  sysVarKey: string;
  sysVarValue: string;
  scriptName: string;
  scriptValue: string;
  sendFrame: string;
  sendData: string;
  someipMethodText: string;
  someipClientText: string;
  someipSessionText: string;
  someipTypeText: string;
  someipTransport: "udp" | "tcp";
  someipHost: string;
  someipPortText: string;
  someipValuesText: string;
  condSomeipMethodText: string;
  condSomeipTypeText: string;
  offsetText: string;
  maskText: string;
  valueText: string;
  vlanText: string;
  preFramesText: string;
  postFramesText: string;
  cooldownMs: string;
  maxFires: string;
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
    sysVarKey: trigger.sysVarKey || "",
    sysVarValue: (trigger.sysVarValue ?? 0).toString(),
    scriptName: trigger.scriptName || "",
    scriptValue:
      trigger.scriptValue == null ? "" : trigger.scriptValue.toString(),
    sendFrame: trigger.sendFrame || "",
    sendData: trigger.sendData || "",
    someipMethodText: (trigger.someipMethod ?? 0).toString(16),
    someipClientText: (trigger.someipClient ?? 0).toString(16),
    someipSessionText: (trigger.someipSession ?? 0).toString(16),
    someipTypeText: (trigger.someipType ?? 2).toString(16),
    someipTransport: trigger.someipTransport ?? "udp",
    someipHost: trigger.someipHost ?? "127.0.0.1",
    someipPortText: (trigger.someipPort ?? 30490).toString(),
    someipValuesText: Object.entries(trigger.someipValues ?? {})
      .map(([name, value]) => `${name}=${value}`)
      .join("; "),
    condSomeipMethodText:
      trigger.condSomeipMethod == null
        ? ""
        : trigger.condSomeipMethod.toString(16),
    condSomeipTypeText:
      trigger.condSomeipType == null
        ? ""
        : trigger.condSomeipType.toString(16),
    offsetText: (trigger.offset ?? 0).toString(),
    maskText: (trigger.mask ?? 0xff).toString(16).toUpperCase(),
    valueText: (trigger.value ?? 0).toString(16).toUpperCase(),
    vlanText: trigger.vlan == null ? "" : trigger.vlan.toString(),
    preFramesText:
      trigger.preFrames == null ? "" : trigger.preFrames.toString(),
    postFramesText:
      trigger.postFrames == null ? "" : trigger.postFrames.toString(),
    cooldownMs: (trigger.cooldownMs ?? 0).toString(),
    maxFires: (trigger.maxFires ?? 0).toString(),
  };
}

function TriggerEditor({
  draft,
  buses,
  txEntries,
  frEntries,
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
  frEntries: FrTxEntry[];
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
  const isFr =
    draft.kind === "fr_frame" ||
    draft.kind === "fr_signal" ||
    draft.kind === "fr_timeout";
  const isDerived = draft.kind === "derived";
  const isEth = draft.kind === "eth_flow";
  const isPayload = draft.kind === "payload";
  const needsThreshold =
    draft.kind === "signal" ||
    draft.kind === "fr_signal" ||
    draft.kind === "sysvar" ||
    isDerived;
  const needsTarget = !isFr && !isEth && draft.kind !== "sysvar" && !isDerived;
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
      <div class="trig-editor-summary">
        <span>
          {KIND_GROUPS.flatMap((group) => group.kinds).find(
            ([kind]) => kind === draft.kind,
          )?.[1] ?? draft.kind}
        </span>
        <span aria-hidden="true">→</span>
        <span>
          {ACTIONS.find(([action]) => action === draft.action)?.[1] ??
            draft.action}
        </span>
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
              {draft.kind === "someip" ? "Service" : "ID"}
              <input
                value={draft.idText}
                class="trig-hex"
                onInput={(event) =>
                  set({ idText: (event.target as HTMLInputElement).value })
                }
              />
            </label>
            {draft.kind === "someip" && (
              <>
                <label class="trig-field">
                  Method (blank = any)
                  <input
                    value={draft.condSomeipMethodText}
                    class="trig-hex"
                    placeholder="any"
                    onInput={(event) =>
                      set({
                        condSomeipMethodText: (
                          event.target as HTMLInputElement
                        ).value,
                      })
                    }
                  />
                </label>
                <label class="trig-field">
                  Type (blank = any)
                  <input
                    value={draft.condSomeipTypeText}
                    class="trig-hex"
                    placeholder="any"
                    onInput={(event) =>
                      set({
                        condSomeipTypeText: (
                          event.target as HTMLInputElement
                        ).value,
                      })
                    }
                  />
                </label>
              </>
            )}
          </>
        )}

        {isPayload && (
          <>
            <label class="trig-field">
              Byte
              <input
                type="number"
                min="0"
                max="63"
                value={draft.offsetText}
                onInput={(event) =>
                  set({ offsetText: (event.target as HTMLInputElement).value })
                }
              />
            </label>
            <label class="trig-field">
              Mask
              <input
                value={draft.maskText}
                class="trig-hex"
                onInput={(event) =>
                  set({ maskText: (event.target as HTMLInputElement).value })
                }
              />
            </label>
            <label class="trig-field">
              Equals
              <input
                value={draft.valueText}
                class="trig-hex"
                onInput={(event) =>
                  set({ valueText: (event.target as HTMLInputElement).value })
                }
              />
            </label>
          </>
        )}

        {isEth && (
          <>
            <label class="trig-field">
              Bus
              <select
                value={draft.bus}
                onChange={(event) =>
                  set({ bus: Number((event.target as HTMLSelectElement).value) })
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
              EtherType
              <input
                value={draft.idText}
                class="trig-hex"
                onInput={(event) =>
                  set({ idText: (event.target as HTMLInputElement).value })
                }
              />
            </label>
            <label class="trig-field">
              VLAN
              <input
                type="number"
                min="0"
                max="4095"
                value={draft.vlanText}
                placeholder="any"
                onInput={(event) =>
                  set({ vlanText: (event.target as HTMLInputElement).value })
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

        {draft.action === "send_fr" && (
          <>
            {frEntries.length > 0 ? (
              <label class="trig-field trig-wide">
                Entry
                <select
                  value={`${draft.sendCh}:${draft.sendId}`}
                  onChange={(event) => {
                    const [bus, slot] = (
                      event.target as HTMLSelectElement
                    ).value.split(":");
                    set({ sendCh: Number(bus), sendId: Number(slot) });
                  }}
                >
                  {frEntries.map((entry) => (
                    <option
                      key={`${entry.bus}:${entry.slot}`}
                      value={`${entry.bus}:${entry.slot}`}
                    >
                      {entry.busName} slot {entry.slot} {entry.name}
                    </option>
                  ))}
                </select>
              </label>
            ) : (
              <>
                <label class="trig-field">
                  FR bus
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
                  Slot
                  <input
                    type="number"
                    min="0"
                    value={draft.sendId}
                    onInput={(event) =>
                      set({
                        sendId: Number(
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

        {draft.action === "send_eth" && (
          <>
            <label class="trig-field">
              Bus
              <select
                value={draft.sendCh}
                onChange={(event) =>
                  set({
                    sendCh: Number(
                      (event.target as HTMLSelectElement).value,
                    ),
                  })
                }
              >
                {buses
                  .filter((bus) => bus.kind === "ethernet")
                  .map((bus) => (
                    <option key={bus.index} value={bus.index}>
                      {bus.name}
                    </option>
                  ))}
              </select>
            </label>
            <label class="trig-field">
              Frame
              <input
                value={draft.sendFrame}
                placeholder="frame name"
                onInput={(event) =>
                  set({
                    sendFrame: (event.target as HTMLInputElement).value,
                  })
                }
              />
            </label>
            <label class="trig-field trig-wide">
              Payload (hex)
              <input
                value={draft.sendData}
                class="trig-hex"
                placeholder="AA BB CC"
                onInput={(event) =>
                  set({
                    sendData: (event.target as HTMLInputElement).value,
                  })
                }
              />
            </label>
          </>
        )}

        {draft.action === "send_someip" && (
          <>
            <label class="trig-field">
              Bus
              <select
                value={draft.sendCh}
                onChange={(event) =>
                  set({
                    sendCh: Number(
                      (event.target as HTMLSelectElement).value,
                    ),
                  })
                }
              >
                {buses
                  .filter((bus) => bus.kind === "ethernet")
                  .map((bus) => (
                    <option key={bus.index} value={bus.index}>
                      {bus.name}
                    </option>
                  ))}
              </select>
            </label>
            <label class="trig-field">
              Service
              <input
                value={draft.sendId}
                onInput={(event) =>
                  set({
                    sendId: Number(
                      (event.target as HTMLInputElement).value,
                    ),
                  })
                }
              />
            </label>
            <label class="trig-field">
              Method
              <input
                value={draft.someipMethodText}
                onChange={(event) =>
                  set({
                    someipMethodText: (
                      event.target as HTMLInputElement
                    ).value,
                  })
                }
              />
            </label>
            <label class="trig-field">
              Client
              <input
                value={draft.someipClientText}
                onChange={(event) =>
                  set({
                    someipClientText: (
                      event.target as HTMLInputElement
                    ).value,
                  })
                }
              />
            </label>
            <label class="trig-field">
              Session (0 = auto)
              <input
                value={draft.someipSessionText}
                onChange={(event) =>
                  set({
                    someipSessionText: (
                      event.target as HTMLInputElement
                    ).value,
                  })
                }
              />
            </label>
            <label class="trig-field">
              Type (0=req, 0x80=resp)
              <input
                value={draft.someipTypeText}
                onChange={(event) =>
                  set({
                    someipTypeText: (event.target as HTMLInputElement).value,
                  })
                }
              />
            </label>
            <label class="trig-field">
              Transport
              <select
                value={draft.someipTransport}
                onChange={(event) =>
                  set({
                    someipTransport: (event.target as HTMLSelectElement).value as
                      | "udp"
                      | "tcp",
                  })
                }
              >
                <option value="udp">UDP</option>
                <option value="tcp">TCP</option>
              </select>
            </label>
            {draft.someipTransport === "tcp" && (
              <>
                <label class="trig-field">
                  TCP host
                  <input
                    value={draft.someipHost}
                    onChange={(event) =>
                      set({
                        someipHost: (event.target as HTMLInputElement).value,
                      })
                    }
                  />
                </label>
                <label class="trig-field">
                  TCP port
                  <input
                    type="number"
                    value={draft.someipPortText}
                    onChange={(event) =>
                      set({
                        someipPortText: (
                          event.target as HTMLInputElement
                        ).value,
                      })
                    }
                  />
                </label>
              </>
            )}
            <label class="trig-field">
              Fields (name=value; …)
              <input
                value={draft.someipValuesText}
                placeholder="speed=12.5; wheel[0]=1"
                spellcheck={false}
                onInput={(event) =>
                  set({
                    someipValuesText: (
                      event.target as HTMLInputElement
                    ).value,
                  })
                }
              />
            </label>
            <label class="trig-field">
              Payload (hex)
              <input
                value={draft.sendData}
                placeholder="01 02 03"
                onInput={(event) =>
                  set({
                    sendData: (event.target as HTMLInputElement).value,
                  })
                }
              />
            </label>
          </>
        )}

        {draft.action === "set_sysvar" && (
          <>
            <label class="trig-field trig-wide">
              Variable
              <input
                value={draft.sysVarKey}
                placeholder="Namespace::Variable"
                onInput={(event) =>
                  set({ sysVarKey: (event.target as HTMLInputElement).value })
                }
              />
            </label>
            <label class="trig-field">
              Value
              <input
                type="number"
                step="any"
                value={draft.sysVarValue}
                onInput={(event) =>
                  set({
                    sysVarValue: (event.target as HTMLInputElement).value,
                  })
                }
              />
            </label>
          </>
        )}

        {draft.action === "script" && (
          <>
            <label class="trig-field trig-wide">
              Action name
              <input
                value={draft.scriptName}
                placeholder='on panel_action("…")'
                onInput={(event) =>
                  set({ scriptName: (event.target as HTMLInputElement).value })
                }
              />
            </label>
            <label class="trig-field">
              Argument
              <input
                type="number"
                step="any"
                value={draft.scriptValue}
                placeholder="optional"
                onInput={(event) =>
                  set({ scriptValue: (event.target as HTMLInputElement).value })
                }
              />
            </label>
          </>
        )}

        <details class="trig-advanced">
          <summary>Advanced limits and recording context</summary>
          <div class="trig-advanced-fields">
            <label class="trig-field">
              Cooldown ms
              <input
                type="number"
                min="0"
                value={draft.cooldownMs}
                title="Minimum gap between two fires; 0 disables the gate"
                onInput={(event) =>
                  set({ cooldownMs: (event.target as HTMLInputElement).value })
                }
              />
            </label>
            <label class="trig-field">
              Max fires
              <input
                type="number"
                min="0"
                value={draft.maxFires}
                title="Stop after this many fires; 0 means no limit"
                onInput={(event) =>
                  set({ maxFires: (event.target as HTMLInputElement).value })
                }
              />
            </label>
            <label class="trig-field">
              Pre frames
              <input
                type="number"
                min="0"
                value={draft.preFramesText}
                placeholder="inherit"
                title="Pre-trigger frames for this rule; blank inherits the global limit"
                onInput={(event) =>
                  set({
                    preFramesText: (event.target as HTMLInputElement).value,
                  })
                }
              />
            </label>
            <label class="trig-field">
              Post frames
              <input
                type="number"
                min="0"
                value={draft.postFramesText}
                placeholder="inherit"
                title="Post-roll frames for this rule; blank inherits the global limit"
                onInput={(event) =>
                  set({
                    postFramesText: (event.target as HTMLInputElement).value,
                  })
                }
              />
            </label>
          </div>
        </details>
      </div>
    </div>
  );
}

export function TriggersModal({
  triggers,
  limits,
  buses,
  txEntries,
  frEntries,
  catalog,
  catalogLoading,
  derived,
  sysvars,
  scripts,
  onAdd,
  onToggle,
  onRemove,
  onRearm,
  onRearmOne,
  onEdit,
  onLimits,
  onExport,
  onImport,
  onClose,
}: Props) {
  const [draft, setDraft] = useState<Draft | null>(null);
  const [error, setError] = useState("");
  const [pre, setPre] = useState(String(limits.preFrames));
  const [post, setPost] = useState(String(limits.postFrames));
  const [cap, setCap] = useState(String(limits.markerCap));
  const [check, setCheck] = useState<string[] | null>(null);

  useEffect(() => {
    setPre(String(limits.preFrames));
    setPost(String(limits.postFrames));
    setCap(String(limits.markerCap));
  }, [limits.preFrames, limits.postFrames, limits.markerCap]);

  useEffect(() => {
    const close = (event: KeyboardEvent) => {
      if (event.key !== "Escape") return;
      if (draft) setDraft(null);
      else onClose();
    };
    window.addEventListener("keydown", close);
    return () => window.removeEventListener("keydown", close);
  }, [draft, onClose]);

  const save = () => {
    if (!draft) return;
    const sysVarValue = Number(draft.sysVarValue);
    const scriptValue =
      draft.scriptValue.trim() === "" ? null : Number(draft.scriptValue);
    const cooldownMs = Math.max(0, Math.round(Number(draft.cooldownMs) || 0));
    const maxFires = Math.max(0, Math.round(Number(draft.maxFires) || 0));
    const sendData = draft.sendData.replace(/[^0-9a-fA-F]/g, "").toUpperCase();
    if (draft.action === "send_eth") {
      if (!draft.sendFrame.trim()) {
        setError("Pick the Ethernet frame this action sends");
        return;
      }
      if (sendData.length % 2 !== 0) {
        setError("Ethernet payload must be whole hex bytes");
        return;
      }
    }
    if (draft.action === "set_sysvar" && !Number.isFinite(sysVarValue)) {
      setError("Variable value must be a number");
      return;
    }
    if (
      draft.action === "script" &&
      scriptValue !== null &&
      !Number.isFinite(scriptValue)
    ) {
      setError("Script argument must be a number or left blank");
      return;
    }
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
      cooldownMs,
      maxFires,
      preFrames:
        draft.preFramesText.trim() === ""
          ? null
          : Math.max(0, Math.round(Number(draft.preFramesText) || 0)),
      postFrames:
        draft.postFramesText.trim() === ""
          ? null
          : Math.max(0, Math.round(Number(draft.postFramesText) || 0)),
      sysVarKey: draft.sysVarKey.trim(),
      sysVarValue: Number.isFinite(sysVarValue) ? sysVarValue : null,
      scriptName: draft.scriptName.trim(),
      scriptValue,
      sendFrame: draft.sendFrame.trim(),
      sendData,
      someipMethod: parseHex(draft.someipMethodText) & 0xffff,
      someipClient: parseHex(draft.someipClientText) & 0xffff,
      someipSession: parseHex(draft.someipSessionText) & 0xffff,
      someipType: parseHex(draft.someipTypeText) & 0xff,
      someipTransport: draft.someipTransport,
      someipHost: draft.someipHost.trim(),
      someipPort: Math.max(1, Math.min(65535, Math.round(Number(draft.someipPortText) || 30490))),
      someipValues: Object.fromEntries(
        draft.someipValuesText
          .split(";")
          .map((part) => part.trim())
          .filter(Boolean)
          .map((part) => {
            const [name, raw] = part.split("=");
            return [name.trim(), Number(raw)];
          })
          .filter(([name, value]) => name !== "" && Number.isFinite(value)),
      ),
      condSomeipMethod:
        draft.condSomeipMethodText.trim() === ""
          ? undefined
          : parseHex(draft.condSomeipMethodText) & 0xffff,
      condSomeipType:
        draft.condSomeipTypeText.trim() === ""
          ? undefined
          : parseHex(draft.condSomeipTypeText) & 0xff,
      offset: Math.max(0, Math.round(Number(draft.offsetText) || 0)),
      mask: parseHex(draft.maskText) & 0xff,
      value: parseHex(draft.valueText) & 0xff,
      vlan:
        draft.vlanText.trim() === ""
          ? null
          : Math.max(0, Math.round(Number(draft.vlanText) || 0)),
    });
    setError("");
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
            <MenuButton
              label="Add rule ▾"
              buttonClass="primary"
              menuClass="app-menu-trig"
              menuLabel="Add rule"
            >
              {(close) =>
                KIND_GROUPS.map((group) => (
                  <div key={group.title}>
                    <div class="app-menu-group">{group.title}</div>
                    {group.kinds.map(([kind, label]) => (
                      <button
                        type="button"
                        role="menuitem"
                        class="app-menu-item"
                        key={kind}
                        onClick={() => {
                          onAdd(kind);
                          close();
                        }}
                      >
                        {label}
                      </button>
                    ))}
                  </div>
                ))
              }
            </MenuButton>
            <MenuButton
              label="More ▾"
              menuClass="app-menu-trig"
              menuLabel="Rule tools"
            >
              {(close) => (
                <>
                  <button
                    type="button"
                    role="menuitem"
                    class="app-menu-item"
                    onClick={() => {
                      setCheck(
                        checkTriggers(triggers, {
                          buses,
                          sysvars,
                          derived,
                          tx: txEntries,
                          frTx: frEntries,
                          scripts,
                          catalog,
                        }),
                      );
                      close();
                    }}
                  >
                    Check rules…
                  </button>
                  <button
                    type="button"
                    role="menuitem"
                    class="app-menu-item"
                    onClick={() => {
                      onRearm();
                      close();
                    }}
                  >
                    Re-arm latched
                  </button>
                  <button
                    type="button"
                    role="menuitem"
                    class="app-menu-item"
                    onClick={() => {
                      onExport();
                      close();
                    }}
                  >
                    Export rules…
                  </button>
                  <FilePick
                    label="Import rules…"
                    menuItem
                    class="app-menu-item"
                    accept=".rxtriggers,.json,application/json"
                    onText={(text) => {
                      onImport(text);
                      close();
                    }}
                    onError={(message) => setError(message)}
                  />
                </>
              )}
            </MenuButton>
            <button type="button" onClick={onClose}>
              Close
            </button>
          </div>
        </header>

        <details class="trig-defaults">
          <summary>
            Recording limits · pre {limits.preFrames} / post{" "}
            {limits.postFrames} / markers {limits.markerCap}
          </summary>
          <div class="trig-limits">
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
              Apply
            </button>
          </div>
        </details>

        {check && (
          <div class="trig-check">
            {check.length === 0 ? (
              <p>All rule targets resolve.</p>
            ) : (
              check.map((line) => <p key={line}>{line}</p>)
            )}
          </div>
        )}

        <div class="trig-body">
          {draft && (
            <TriggerEditor
              draft={draft}
              buses={buses}
              txEntries={txEntries}
              frEntries={frEntries}
              catalog={catalog}
              catalogLoading={catalogLoading}
              derived={derived}
              onChange={setDraft}
              onCancel={() => setDraft(null)}
              onSave={save}
            />
          )}
          {draft && error && <p class="modal-error">{error}</p>}
          {triggers.length === 0 && (
            <div class="empty">
              <strong>No trigger rules</strong>
              <span>Use Add rule above, then edit its condition.</span>
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
              <span class="trig-fired">
                {trigger.maxFires > 0
                  ? `${trigger.fired}/${trigger.maxFires} fired`
                  : `${trigger.fired} fired`}
              </span>
              <span class="trig-last">{timeText(trigger.lastFireUs)}</span>
              <button
                type="button"
                class="mini trig-icon"
                title="Clear this rule's latch and fire count"
                aria-label="Re-arm rule"
                onClick={() => onRearmOne(index)}
              >
                ↺
              </button>
              <button
                type="button"
                class="mini"
                onClick={() => {
                  setError("");
                  setDraft(draftOf(index, trigger));
                }}
              >
                Edit
              </button>
              <button
                type="button"
                class="bus-remove trig-icon"
                title="Remove this rule"
                aria-label="Remove rule"
                onClick={() => onRemove(index)}
              >
                ×
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
