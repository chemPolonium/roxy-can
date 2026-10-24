import { useEffect, useRef, useState } from "preact/hooks";
import type {
  Bus,
  Catalog,
  CatalogSignal,
  FrBus,
  FrTxEntry,
  ReplayBlock,
  TxEntry,
  TxSource,
} from "../types";
import { FilePick } from "./FilePick";

interface Props {
  entries: TxEntry[];
  frTx: FrTxEntry[];
  blocks: ReplayBlock[];
  buses: Bus[];
  frBuses: FrBus[];
  catalog: Catalog | null;
  catalogLoading: boolean;
  catalogError: string;
  onToggle: (entry: TxEntry, on: boolean) => void;
  onCycle: (entry: TxEntry, cycleUs: number) => void;
  onFd: (entry: TxEntry, on: boolean) => void;
  onHex: (entry: TxEntry, text: string) => void;
  onSource: (entry: TxEntry, src: TxSource) => void;
  onClearSource: (entry: TxEntry, name: string) => void;
  onFrToggle: (entry: FrTxEntry, on: boolean) => void;
  onFrCycle: (entry: FrTxEntry, cycleUs: number) => void;
  onFrHex: (entry: FrTxEntry, text: string) => void;
  onFrAdd: (bus: number, slot: number) => void;
  onFrRemove: (entry: FrTxEntry) => void;
  onAdd: (ch: number, id: number) => void;
  onRemove: (entry: TxEntry) => void;
  onSimulate: (ch: number) => void;
  onBlockAdd: (
    channel: number,
    name: string,
    path: string,
    node: string | null,
  ) => void;
  onBlockToggle: (id: number, on: boolean) => void;
  onBlockRemove: (id: number) => void;
  onClose: () => void;
}

const KINDS = ["Ramp", "Sine", "Step", "Random", "Triangle", "Counter"];

function NumField({
  value,
  onCommit,
  title,
}: {
  value: number;
  onCommit: (value: number) => void;
  title?: string;
}) {
  const [text, setText] = useState(String(value));
  const [editing, setEditing] = useState(false);
  useEffect(() => {
    if (!editing) setText(String(value));
  }, [value, editing]);
  const commit = () => {
    setEditing(false);
    const parsed = Number(text);
    if (Number.isFinite(parsed)) {
      if (parsed !== value) onCommit(parsed);
    } else {
      setText(String(value));
    }
  };
  return (
    <input
      class="gen-src-num"
      value={text}
      title={title}
      inputMode="decimal"
      onFocus={() => setEditing(true)}
      onInput={(event) => setText((event.target as HTMLInputElement).value)}
      onBlur={commit}
      onKeyDown={(event) => {
        if (event.key === "Enter") (event.target as HTMLInputElement).blur();
        if (event.key === "Escape") {
          setText(String(value));
          (event.target as HTMLInputElement).blur();
        }
      }}
    />
  );
}

function SeqField({
  seq,
  onCommit,
}: {
  seq: number[];
  onCommit: (seq: number[]) => void;
}) {
  const label = seq.join(", ");
  const [text, setText] = useState(label);
  const [editing, setEditing] = useState(false);
  useEffect(() => {
    if (!editing) setText(label);
  }, [label, editing]);
  const commit = () => {
    setEditing(false);
    const parts = text
      .split(/[\s,;]+/)
      .map((part) => part.trim())
      .filter(Boolean);
    const values: number[] = [];
    for (const part of parts) {
      const parsed = Number(part);
      if (!Number.isFinite(parsed)) {
        setText(label);
        return;
      }
      values.push(parsed);
    }
    if (values.join(",") !== seq.join(",")) onCommit(values);
  };
  return (
    <input
      class="gen-src-seq"
      value={text}
      placeholder="comma-separated values"
      title="Values walked at equal duration; empty toggles Lo/Hi"
      onFocus={() => setEditing(true)}
      onInput={(event) => setText((event.target as HTMLInputElement).value)}
      onBlur={commit}
      onKeyDown={(event) => {
        if (event.key === "Enter") (event.target as HTMLInputElement).blur();
      }}
    />
  );
}

function SourceRow({
  src,
  onChange,
  onRemove,
}: {
  src: TxSource;
  onChange: (src: TxSource) => void;
  onRemove: () => void;
}) {
  const patch = (next: Partial<TxSource>) => onChange({ ...src, ...next });
  return (
    <div class="gen-src-row">
      <strong class="gen-src-name">{src.name}</strong>
      <select
        value={src.kind}
        title="Waveform shape"
        onChange={(event) =>
          patch({ kind: (event.target as HTMLSelectElement).value })
        }
      >
        {KINDS.map((kind) => (
          <option key={kind} value={kind}>
            {kind}
          </option>
        ))}
      </select>
      <label>
        lo
        <NumField value={src.lo} onCommit={(lo) => patch({ lo })} />
      </label>
      <label>
        hi
        <NumField value={src.hi} onCommit={(hi) => patch({ hi })} />
      </label>
      <label title="One full cycle">
        period
        <NumField
          value={src.periodUs / 1000}
          onCommit={(ms) => patch({ periodUs: Math.max(0, Math.round(ms * 1000)) })}
        />
        ms
      </label>
      <label title="Shift onto the simulation clock">
        phase
        <NumField
          value={src.phaseUs / 1000}
          onCommit={(ms) => patch({ phaseUs: Math.max(0, Math.round(ms * 1000)) })}
        />
        ms
      </label>
      {src.kind === "Random" && (
        <label title="Redraw interval; 0 = every frame">
          redraw
          <NumField
            value={src.redrawUs / 1000}
            onCommit={(ms) =>
              patch({ redrawUs: Math.max(0, Math.round(ms * 1000)) })
            }
          />
          ms
        </label>
      )}
      {src.kind === "Step" && (
        <label class="gen-src-seq-label">
          seq
          <SeqField seq={src.seq} onCommit={(seq) => patch({ seq })} />
        </label>
      )}
      <button
        type="button"
        class="gen-src-remove"
        title="Stop driving this signal"
        onClick={onRemove}
      >
        ×
      </button>
    </div>
  );
}

function SourceEditor({
  entry,
  signals,
  onSource,
  onClearSource,
}: {
  entry: TxEntry;
  signals: CatalogSignal[];
  onSource: (entry: TxEntry, src: TxSource) => void;
  onClearSource: (entry: TxEntry, name: string) => void;
}) {
  const [name, setName] = useState("");
  const available = signals.filter(
    (signal) => !entry.srcs.some((src) => src.name === signal.name),
  );
  const add = () => {
    const signal = signals.find((candidate) => candidate.name === name);
    if (!signal) return;
    onSource(entry, {
      name: signal.name,
      kind: "Ramp",
      lo: signal.min,
      hi: signal.max,
      periodUs: 1_000_000,
      phaseUs: 0,
      seq: [],
      seed: 0x5eed1234,
      redrawUs: 0,
    });
    setName("");
  };
  return (
    <div class="gen-srcs">
      {entry.srcs.map((src) => (
        <SourceRow
          key={src.name}
          src={src}
          onChange={(next) => onSource(entry, next)}
          onRemove={() => onClearSource(entry, src.name)}
        />
      ))}
      {signals.length === 0 ? (
        <div class="gen-src-empty">
          Load a DBC for this message to drive its signals.
        </div>
      ) : (
        <div class="gen-src-add">
          <select
            value={name}
            onChange={(event) => setName((event.target as HTMLSelectElement).value)}
          >
            <option value="">
              {available.length === 0 ? "All signals driven" : "Add source…"}
            </option>
            {available.map((signal) => (
              <option key={signal.name} value={signal.name}>
                {signal.name} ({signal.min}..{signal.max}
                {signal.unit ? ` ${signal.unit}` : ""})
              </option>
            ))}
          </select>
          <button type="button" disabled={!name} onClick={add}>
            + Source
          </button>
        </div>
      )}
    </div>
  );
}

function hexId(id: number): string {
  return `0x${id.toString(16).toUpperCase().padStart(3, "0")}`;
}

function EntryRow({
  entry,
  signals,
  onToggle,
  onCycle,
  onFd,
  onHex,
  onSource,
  onClearSource,
  onRemove,
}: {
  entry: TxEntry;
  signals: CatalogSignal[];
  onToggle: (entry: TxEntry, on: boolean) => void;
  onCycle: (entry: TxEntry, cycleUs: number) => void;
  onFd: (entry: TxEntry, on: boolean) => void;
  onHex: (entry: TxEntry, text: string) => void;
  onSource: (entry: TxEntry, src: TxSource) => void;
  onClearSource: (entry: TxEntry, name: string) => void;
  onRemove: (entry: TxEntry) => void;
}) {
  const [cycle, setCycle] = useState((entry.cycleUs / 1000).toString());
  const cycleRef = useRef(cycle);
  const [hex, setHex] = useState(entry.dataText);
  const hexRef = useRef(hex);
  const [sourcesOpen, setSourcesOpen] = useState(false);

  useEffect(() => {
    const next = (entry.cycleUs / 1000).toString();
    setCycle(next);
    cycleRef.current = next;
  }, [entry.cycleUs]);
  useEffect(() => {
    setHex(entry.dataText);
    hexRef.current = entry.dataText;
  }, [entry.dataText]);

  const commitCycle = () => {
    const ms = Number(cycleRef.current);
    if (!Number.isFinite(ms) || ms < 0) {
      const reset = (entry.cycleUs / 1000).toString();
      setCycle(reset);
      cycleRef.current = reset;
      return;
    }
    onCycle(entry, Math.round(ms * 1000));
  };

  const commitHex = () => {
    const text = hexRef.current.trim();
    if (text && text !== entry.dataText) {
      onHex(entry, text);
    }
  };

  return (
    <div class={`gen-row ${entry.active ? "on" : ""}`}>
      <div class="gen-line">
        <label class="gen-active" title="Transmit this entry">
          <input
            type="checkbox"
            checked={entry.active}
            onChange={(event) =>
              onToggle(entry, (event.target as HTMLInputElement).checked)
            }
          />
          On
        </label>
        <strong class="gen-name">{entry.name || hexId(entry.id)}</strong>
        <span class="gen-id">{hexId(entry.id)}</span>
        {entry.node && <span class="gen-node">{entry.node}</span>}
        {!entry.gateOpen && (
          <span class="gen-warn" title="The owning DBC node is not Simulated">
            node not simulated
          </span>
        )}
        {entry.muted && <span class="gen-warn">muted</span>}
        <button
          type="button"
          class={`mini gen-src-toggle ${entry.srcs.length ? "on" : ""}`}
          title="Edit the waveforms driving this entry's signals"
          onClick={() => setSourcesOpen((open) => !open)}
        >
          Sources{entry.srcs.length ? ` ${entry.srcs.length}` : ""}
        </button>
        <label class="gen-cycle" title="0 = event-triggered">
          Cycle
          <input
            value={cycle}
            onInput={(event) => {
              cycleRef.current = (event.target as HTMLInputElement).value;
              setCycle(cycleRef.current);
            }}
            onBlur={commitCycle}
            onKeyDown={(event) => {
              if (event.key === "Enter") {
                (event.target as HTMLInputElement).blur();
              }
            }}
          />
          ms
        </label>
        <label class="gen-fd">
          <input
            type="checkbox"
            checked={entry.fd}
            onChange={(event) =>
              onFd(entry, (event.target as HTMLInputElement).checked)
            }
          />
          FD
        </label>
        <span class="spacer" />
        <button
          type="button"
          class="bus-remove"
          onClick={() => onRemove(entry)}
        >
          Remove
        </button>
      </div>
      <div class="gen-line gen-data">
        <input
          class="gen-hex"
          value={hex}
          spellcheck={false}
          placeholder="Hex payload"
          onInput={(event) => {
            hexRef.current = (event.target as HTMLInputElement).value;
            setHex(hexRef.current);
          }}
          onBlur={commitHex}
          onKeyDown={(event) => {
            if (event.key === "Enter") {
              (event.target as HTMLInputElement).blur();
            }
          }}
        />
        {entry.sentText && <span class="gen-sent">sent {entry.sentText}</span>}
      </div>
      {sourcesOpen && (
        <SourceEditor
          entry={entry}
          signals={signals}
          onSource={onSource}
          onClearSource={onClearSource}
        />
      )}
    </div>
  );
}

function BusGroup({
  bus,
  entries,
  catalog,
  catalogLoading,
  catalogError,
  onToggle,
  onCycle,
  onFd,
  onHex,
  onSource,
  onClearSource,
  onAdd,
  onRemove,
  onSimulate,
}: {
  bus: Bus;
  entries: TxEntry[];
  catalog: Catalog | null;
  catalogLoading: boolean;
  catalogError: string;
  onToggle: (entry: TxEntry, on: boolean) => void;
  onCycle: (entry: TxEntry, cycleUs: number) => void;
  onFd: (entry: TxEntry, on: boolean) => void;
  onHex: (entry: TxEntry, text: string) => void;
  onSource: (entry: TxEntry, src: TxSource) => void;
  onClearSource: (entry: TxEntry, name: string) => void;
  onAdd: (ch: number, id: number) => void;
  onRemove: (entry: TxEntry) => void;
  onSimulate: (ch: number) => void;
}) {
  const [addId, setAddId] = useState("");
  const existing = new Set(entries.map((entry) => entry.id));
  const catalogMessages = catalog?.channels[bus.index]?.messages ?? [];
  const messages = catalogMessages.filter((message) => !existing.has(message.id));
  const activeCount = entries.filter((entry) => entry.active).length;

  return (
    <section class="gen-bus">
      <div class="gen-bus-head">
        <strong>{bus.name}</strong>
        <span class="gen-meta">
          {entries.length} entries · {activeCount} on
        </span>
        <button
          type="button"
          class="mini"
          disabled={bus.dbcPaths.length === 0}
          title="Set every DBC node on this bus to Simulated; entries still need On to transmit"
          onClick={() => onSimulate(bus.index)}
        >
          Simulate nodes
        </button>
        <span class="spacer" />
        <select
          value={addId}
          disabled={catalogLoading || messages.length === 0}
          onChange={(event) =>
            setAddId((event.target as HTMLSelectElement).value)
          }
        >
          <option value="">
            {catalogLoading
              ? "Loading messages…"
              : catalogError
                ? "Catalog unavailable"
                : catalogMessages.length === 0
                  ? "No DBC messages"
                  : messages.length === 0
                    ? "All messages added"
                    : "Add message…"}
          </option>
          {messages.map((message) => (
            <option key={message.id} value={message.id}>
              {hexId(message.id)} {message.name}
            </option>
          ))}
        </select>
        <button
          type="button"
          disabled={!addId}
          title="Add a message entry to this bus"
          onClick={() => {
            if (addId) {
              onAdd(bus.index, Number(addId));
              setAddId("");
            }
          }}
        >
          Add entry
        </button>
      </div>
      {entries.length === 0 ? (
        <div class="gen-empty">
          No generator entries on this bus. Add a DBC message above.
        </div>
      ) : (
        entries.map((entry) => (
          <EntryRow
            key={`${entry.channel}:${entry.id}`}
            entry={entry}
            signals={
              catalogMessages.find((message) => message.id === entry.id)
                ?.signals ?? []
            }
            onToggle={onToggle}
            onCycle={onCycle}
            onFd={onFd}
            onHex={onHex}
            onSource={onSource}
            onClearSource={onClearSource}
            onRemove={onRemove}
          />
        ))
      )}
    </section>
  );
}

function FrEntryRow({
  entry,
  onToggle,
  onCycle,
  onHex,
  onRemove,
}: {
  entry: FrTxEntry;
  onToggle: (entry: FrTxEntry, on: boolean) => void;
  onCycle: (entry: FrTxEntry, cycleUs: number) => void;
  onHex: (entry: FrTxEntry, text: string) => void;
  onRemove: (entry: FrTxEntry) => void;
}) {
  const [cycle, setCycle] = useState((entry.cycleUs / 1000).toString());
  const cycleRef = useRef(cycle);
  const [hex, setHex] = useState(entry.dataText);
  const hexRef = useRef(hex);
  useEffect(() => {
    const next = (entry.cycleUs / 1000).toString();
    setCycle(next);
    cycleRef.current = next;
  }, [entry.cycleUs]);
  useEffect(() => {
    setHex(entry.dataText);
    hexRef.current = entry.dataText;
  }, [entry.dataText]);
  return (
    <div class={`gen-row ${entry.active ? "on" : ""}`}>
      <div class="gen-line">
        <label class="gen-active">
          <input
            type="checkbox"
            checked={entry.active}
            onChange={(event) =>
              onToggle(entry, (event.target as HTMLInputElement).checked)
            }
          />
          On
        </label>
        <strong class="gen-name">
          {entry.name || `slot ${entry.slot}`}
        </strong>
        <span class="gen-id">slot {entry.slot}</span>
        {entry.node && <span class="gen-node">{entry.node}</span>}
        {entry.muted && <span class="gen-warn">muted</span>}
        {entry.undescribed && <span class="gen-warn">undescribed</span>}
        <label class="gen-cycle" title="0 = event-triggered">
          Cycle
          <input
            value={cycle}
            onInput={(event) => {
              cycleRef.current = (event.target as HTMLInputElement).value;
              setCycle(cycleRef.current);
            }}
            onBlur={() => {
              const ms = Number(cycleRef.current);
              if (Number.isFinite(ms) && ms >= 0) {
                onCycle(entry, Math.round(ms * 1000));
              } else {
                setCycle((entry.cycleUs / 1000).toString());
              }
            }}
            onKeyDown={(event) => {
              if (event.key === "Enter") {
                (event.target as HTMLInputElement).blur();
              }
            }}
          />
          ms
        </label>
        <span class="spacer" />
        <button
          type="button"
          class="bus-remove"
          onClick={() => onRemove(entry)}
        >
          Remove
        </button>
      </div>
      <div class="gen-line gen-data">
        <input
          class="gen-hex"
          value={hex}
          spellcheck={false}
          placeholder="Hex payload"
          onInput={(event) => {
            hexRef.current = (event.target as HTMLInputElement).value;
            setHex(hexRef.current);
          }}
          onBlur={() => {
            const text = hexRef.current.trim();
            if (text && text !== entry.dataText) onHex(entry, text);
          }}
          onKeyDown={(event) => {
            if (event.key === "Enter") {
              (event.target as HTMLInputElement).blur();
            }
          }}
        />
        {entry.sentText && <span class="gen-sent">sent {entry.sentText}</span>}
      </div>
    </div>
  );
}

function FrBusGroup({
  bus,
  entries,
  onToggle,
  onCycle,
  onHex,
  onAdd,
  onRemove,
}: {
  bus: FrBus;
  entries: FrTxEntry[];
  onToggle: (entry: FrTxEntry, on: boolean) => void;
  onCycle: (entry: FrTxEntry, cycleUs: number) => void;
  onHex: (entry: FrTxEntry, text: string) => void;
  onAdd: (bus: number, slot: number) => void;
  onRemove: (entry: FrTxEntry) => void;
}) {
  const [slot, setSlot] = useState("");
  const parsed = Number(slot);
  return (
    <section class="gen-bus">
      <div class="gen-bus-head">
        <strong>{bus.name}</strong>
        <span class="gen-meta">
          {bus.cluster || "unnamed cluster"} · {entries.length} entries
        </span>
        <span class="spacer" />
        <input
          class="gen-slot"
          value={slot}
          placeholder="slot"
          inputMode="numeric"
          onInput={(event) => setSlot((event.target as HTMLInputElement).value)}
        />
        <button
          type="button"
          disabled={!Number.isInteger(parsed) || parsed <= 0}
          onClick={() => {
            onAdd(bus.bus, parsed);
            setSlot("");
          }}
        >
          Add slot
        </button>
      </div>
      {entries.length === 0 ? (
        <div class="gen-empty">
          No FlexRay entries on this bus. Add a slot the description
          schedules.
        </div>
      ) : (
        entries.map((entry) => (
          <FrEntryRow
            key={`${entry.bus}:${entry.slot}`}
            entry={entry}
            onToggle={onToggle}
            onCycle={onCycle}
            onHex={onHex}
            onRemove={onRemove}
          />
        ))
      )}
    </section>
  );
}

export function GeneratorModal({
  entries,
  frTx,
  blocks,
  buses,
  frBuses,
  catalog,
  catalogLoading,
  catalogError,
  onToggle,
  onCycle,
  onFd,
  onHex,
  onSource,
  onClearSource,
  onFrToggle,
  onFrCycle,
  onFrHex,
  onFrAdd,
  onFrRemove,
  onAdd,
  onRemove,
  onSimulate,
  onBlockAdd,
  onBlockToggle,
  onBlockRemove,
  onClose,
}: Props) {
  const [blockName, setBlockName] = useState("");
  const [blockNode, setBlockNode] = useState("");
  const [blockBus, setBlockBus] = useState(buses[0]?.index ?? 0);
  useEffect(() => {
    if (!buses.some((bus) => bus.index === blockBus)) {
      setBlockBus(buses[0]?.index ?? 0);
    }
  }, [buses, blockBus]);
  useEffect(() => {
    const close = (event: KeyboardEvent) => {
      if (event.key === "Escape") onClose();
    };
    window.addEventListener("keydown", close);
    return () => window.removeEventListener("keydown", close);
  }, [onClose]);

  const active = entries.filter((entry) => entry.active).length;

  return (
    <div class="modal-backdrop" onMouseDown={onClose}>
      <div
        class="generator-modal"
        role="dialog"
        aria-modal="true"
        aria-label="Interactive generator"
        onMouseDown={(event) => event.stopPropagation()}
      >
        <header class="modal-head">
          <h2>Interactive Generator</h2>
          <div class="picker-head-right">
            <span>
              {entries.length} entries · {active} on
            </span>
            <button type="button" onClick={onClose}>
              Close
            </button>
          </div>
        </header>
        <div class="gen-body">
          {buses.map((bus) => (
            <BusGroup
              key={bus.index}
              bus={bus}
              entries={entries.filter((entry) => entry.channel === bus.index)}
              catalog={catalog}
              catalogLoading={catalogLoading}
              catalogError={catalogError}
              onToggle={onToggle}
              onCycle={onCycle}
              onFd={onFd}
              onHex={onHex}
              onSource={onSource}
              onClearSource={onClearSource}
              onAdd={onAdd}
              onRemove={onRemove}
              onSimulate={onSimulate}
            />
          ))}
          {frBuses.map((bus) => (
            <FrBusGroup
              key={`fr${bus.bus}`}
              bus={bus}
              entries={frTx.filter((entry) => entry.bus === bus.bus)}
              onToggle={onFrToggle}
              onCycle={onFrCycle}
              onHex={onFrHex}
              onAdd={onFrAdd}
              onRemove={onFrRemove}
            />
          ))}

          <section class="gen-block">
            <h3>Replay blocks</h3>
            {blocks.length === 0 ? (
              <p class="hint">
                Replay a recorded ASC/BLF into a simulated bus, optionally
                filtering to the frames one DBC node sends.
              </p>
            ) : (
              blocks.map((block) => (
                <div
                  class={`gen-block-row ${block.enabled ? "on" : ""}`}
                  key={block.id}
                >
                  <label>
                    <input
                      type="checkbox"
                      checked={block.enabled}
                      onChange={(event) =>
                        onBlockToggle(
                          block.id,
                          (event.target as HTMLInputElement).checked,
                        )
                      }
                    />
                    On
                  </label>
                  <strong>{block.name}</strong>
                  <span>
                    {buses.find((bus) => bus.index === block.channel)?.name ??
                      `CAN${block.channel + 1}`}
                    {" · "}
                    {block.nodeFilter ?? "all ids"}
                  </span>
                  <em>
                    {block.enabled ? `${block.frames} frames` : "off"}
                  </em>
                  <span class="gen-block-path" title={block.path}>
                    {block.path.split(/[\\/]/).pop()}
                  </span>
                  {block.error && (
                    <span class="gen-block-error" title={block.error}>
                      load failed
                    </span>
                  )}
                  <button
                    type="button"
                    class="bus-remove"
                    onClick={() => onBlockRemove(block.id)}
                  >
                    Remove
                  </button>
                </div>
              ))
            )}
            <div class="gen-block-add">
              <input
                value={blockName}
                placeholder="Block name"
                onInput={(event) =>
                  setBlockName((event.target as HTMLInputElement).value)
                }
              />
              <select
                value={blockBus}
                onChange={(event) =>
                  setBlockBus(Number((event.target as HTMLSelectElement).value))
                }
              >
                {buses.map((bus) => (
                  <option key={bus.index} value={bus.index}>
                    {bus.name}
                  </option>
                ))}
              </select>
              <input
                value={blockNode}
                placeholder="DBC node (optional)"
                onInput={(event) =>
                  setBlockNode((event.target as HTMLInputElement).value)
                }
              />
              <FilePick
                label="Add from log…"
                accept=".asc,.blf"
                onPick={(path) => {
                  onBlockAdd(
                    blockBus,
                    blockName.trim(),
                    path,
                    blockNode.trim() || null,
                  );
                  setBlockName("");
                  setBlockNode("");
                }}
              />
            </div>
          </section>
        </div>
        <p class="hint picker-hint">
          Entries transmit only when On and their DBC node is Simulated. On a
          Real bus they also need an attached adapter. Cycle 0 is
          event-triggered.
        </p>
      </div>
    </div>
  );
}
