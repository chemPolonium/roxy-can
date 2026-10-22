import { useEffect, useRef, useState } from "preact/hooks";
import type { Bus, Catalog, TxEntry } from "../types";

interface Props {
  entries: TxEntry[];
  buses: Bus[];
  catalog: Catalog | null;
  catalogLoading: boolean;
  catalogError: string;
  onToggle: (entry: TxEntry, on: boolean) => void;
  onCycle: (entry: TxEntry, cycleUs: number) => void;
  onFd: (entry: TxEntry, on: boolean) => void;
  onHex: (entry: TxEntry, text: string) => void;
  onAdd: (ch: number, id: number) => void;
  onRemove: (entry: TxEntry) => void;
  onSimulate: (ch: number) => void;
  onClose: () => void;
}

function hexId(id: number): string {
  return `0x${id.toString(16).toUpperCase().padStart(3, "0")}`;
}

function EntryRow({
  entry,
  onToggle,
  onCycle,
  onFd,
  onHex,
  onRemove,
}: {
  entry: TxEntry;
  onToggle: (entry: TxEntry, on: boolean) => void;
  onCycle: (entry: TxEntry, cycleUs: number) => void;
  onFd: (entry: TxEntry, on: boolean) => void;
  onHex: (entry: TxEntry, text: string) => void;
  onRemove: (entry: TxEntry) => void;
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
        {entry.sources > 0 && (
          <span class="gen-badge">{entry.sources} src</span>
        )}
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
          title="Set every DBC node on this bus to Simulated"
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
            onToggle={onToggle}
            onCycle={onCycle}
            onFd={onFd}
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
  buses,
  catalog,
  catalogLoading,
  catalogError,
  onToggle,
  onCycle,
  onFd,
  onHex,
  onAdd,
  onRemove,
  onSimulate,
  onClose,
}: Props) {
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
              onAdd={onAdd}
              onRemove={onRemove}
              onSimulate={onSimulate}
            />
          ))}
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
