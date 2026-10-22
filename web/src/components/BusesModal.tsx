import { useEffect, useRef, useState } from "preact/hooks";
import type { Bus, Hardware, HwChannel } from "../types";

interface Props {
  buses: Bus[];
  realBus: boolean;
  hardware: Hardware | null;
  loading: boolean;
  error: string;
  onRefresh: () => void;
  onAddBus: () => void;
  onRemoveBus: (bus: number) => void;
  onRenameBus: (bus: number, name: string) => void;
  onSetRates: (bus: number, arb: number, fd: number) => void;
  onLoadDbc: (bus: number, path: string) => void;
  onSetNodeRole: (bus: number, node: string, simulated: boolean) => void;
  onAttach: (bus: number, driver: string, index: number) => void;
  onDetach: (bus: number) => void;
  onSetReal: (real: boolean) => void;
  onClose: () => void;
}

const ARB_RATES = [10, 50, 100, 125, 250, 500, 800, 1000];
const FD_RATES = [0, 1000, 2000, 4000, 5000, 8000];

function channelKey(channel: HwChannel): string {
  return `${channel.driver}:${channel.index}`;
}

function channelLabel(channel: HwChannel): string {
  const tag = channel.driver === "Vector" ? "V" : "K";
  return `[${tag}] ${channel.name}${channel.detail ? ` · ${channel.detail}` : ""}`;
}

function fileName(path: string): string {
  const cut = Math.max(path.lastIndexOf("\\"), path.lastIndexOf("/"));
  return cut >= 0 ? path.slice(cut + 1) : path;
}

function rateOptions(standard: number[], current: number): number[] {
  const values = standard.includes(current)
    ? [...standard]
    : [...standard, current];
  return values.sort((a, b) => a - b);
}

function BusCard({
  bus,
  channels,
  loading,
  onAttach,
  onDetach,
  onRemove,
  onRename,
  onSetRates,
  onLoadDbc,
  onSetNodeRole,
}: {
  bus: Bus;
  channels: HwChannel[];
  loading: boolean;
  onAttach: (bus: number, driver: string, index: number) => void;
  onDetach: (bus: number) => void;
  onRemove: (bus: number) => void;
  onRename: (bus: number, name: string) => void;
  onSetRates: (bus: number, arb: number, fd: number) => void;
  onLoadDbc: (bus: number, path: string) => void;
  onSetNodeRole: (bus: number, node: string, simulated: boolean) => void;
}) {
  const [name, setName] = useState(bus.name);
  const nameRef = useRef(bus.name);
  const [dbc, setDbc] = useState("");
  const [arb, setArb] = useState(bus.bitrateKbps);
  const [fd, setFd] = useState(bus.fdDataKbps);
  const ratesRef = useRef({ arb: bus.bitrateKbps, fd: bus.fdDataKbps });
  const [channelChoice, setChannelChoice] = useState("");
  const selected =
    channels.find((channel) => channelKey(channel) === channelChoice) ?? null;

  useEffect(() => {
    setName(bus.name);
    nameRef.current = bus.name;
  }, [bus.name]);
  useEffect(() => {
    setArb(bus.bitrateKbps);
    setFd(bus.fdDataKbps);
    ratesRef.current = { arb: bus.bitrateKbps, fd: bus.fdDataKbps };
  }, [bus.bitrateKbps, bus.fdDataKbps]);

  const commitName = () => {
    const value = nameRef.current.trim();
    if (!value) {
      setName(bus.name);
      nameRef.current = bus.name;
      return;
    }
    if (value !== bus.name) {
      onRename(bus.index, value);
    }
  };

  const changeRates = (next: { arb: number; fd: number }) => {
    ratesRef.current = next;
    setArb(next.arb);
    setFd(next.fd);
    onSetRates(bus.index, next.arb, next.fd);
  };

  return (
    <section class="bus-card">
      <div class="bus-head">
        <input
          class="bus-name-input"
          value={name}
          aria-label="Bus name"
          onInput={(event) => {
            nameRef.current = (event.target as HTMLInputElement).value;
            setName(nameRef.current);
          }}
          onBlur={commitName}
          onKeyDown={(event) => {
            if (event.key === "Enter") (event.target as HTMLInputElement).blur();
            if (event.key === "Escape") {
              setName(bus.name);
              nameRef.current = bus.name;
              (event.target as HTMLInputElement).blur();
            }
          }}
        />
        <label class="rate-field">
          Arb
          <select
            value={arb}
            onChange={(event) => {
              const value = Number((event.target as HTMLSelectElement).value);
              changeRates({ ...ratesRef.current, arb: value });
            }}
          >
            {rateOptions(ARB_RATES, arb).map((value) => (
              <option key={value} value={value}>
                {value} kbit/s
              </option>
            ))}
          </select>
        </label>
        <label class="rate-field">
          FD data
          <select
            value={fd}
            onChange={(event) => {
              const value = Number((event.target as HTMLSelectElement).value);
              changeRates({ ...ratesRef.current, fd: value });
            }}
          >
            {rateOptions(FD_RATES, fd).map((value) => (
              <option key={value} value={value}>
                {value === 0 ? "classic" : `${value} kbit/s`}
              </option>
            ))}
          </select>
        </label>
        <span class="bus-dbcs">
          {bus.dbcPaths.length > 0
            ? bus.dbcPaths.map(fileName).join(", ")
            : "no database"}
        </span>
        <button
          type="button"
          class="bus-remove"
          onClick={() => {
            if (
              window.confirm(
                `Remove ${bus.name}? Its attachments, signals and trace rows for this bus are removed.`,
              )
            ) {
              onRemove(bus.index);
            }
          }}
        >
          Remove
        </button>
      </div>

      <div class="bus-dbc-row">
        <span>Database</span>
        <input
          value={dbc}
          placeholder="Host path to a .dbc or ARXML"
          onInput={(event) => setDbc((event.target as HTMLInputElement).value)}
          onKeyDown={(event) => {
            if (event.key === "Enter" && dbc.trim()) {
              onLoadDbc(bus.index, dbc.trim());
            }
          }}
        />
        <button
          type="button"
          disabled={!dbc.trim()}
          onClick={() => onLoadDbc(bus.index, dbc.trim())}
        >
          Load DBC
        </button>
      </div>

      {bus.nodes.length > 0 && (
        <div class="bus-nodes">
          <span class="bus-nodes-label">Nodes</span>
          <div class="bus-node-list">
            {bus.nodes.map((node) => (
              <label
                class={`node-chip ${node.simulated ? "on" : ""}`}
                key={node.name}
                title={`${node.entries} generator ${
                  node.entries === 1 ? "entry" : "entries"
                }; click to ${
                  node.simulated ? "take it offline" : "simulate it"
                }`}
              >
                <input
                  type="checkbox"
                  checked={node.simulated}
                  onChange={(event) =>
                    onSetNodeRole(
                      bus.index,
                      node.name,
                      (event.target as HTMLInputElement).checked,
                    )
                  }
                />
                {node.name}
                {node.entries > 0 && <em>{node.entries}</em>}
              </label>
            ))}
          </div>
        </div>
      )}

      {bus.attached ? (
        <div class="bus-attached">
          <span class="attach-dot" />
          <strong>
            {bus.attached.driver} ch {bus.attached.adapter}
          </strong>
          <span>
            {bus.attached.kbps} kbit/s
            {bus.attached.fd ? " · FD" : ""}
            {bus.attached.canTx ? " · TX" : " · RX only"}
          </span>
          {bus.attached.txFail && (
            <span class="attach-warn">
              TX refused {bus.attached.txFail.count}×:{" "}
              {bus.attached.txFail.reason}
            </span>
          )}
          <span class="spacer" />
          <button type="button" onClick={() => onDetach(bus.index)}>
            Detach
          </button>
        </div>
      ) : (
        <div class="bus-attach">
          <select
            value={channelChoice}
            disabled={channels.length === 0 || loading}
            onChange={(event) =>
              setChannelChoice((event.target as HTMLSelectElement).value)
            }
          >
            <option value="">
              {channels.length === 0
                ? "No CAN hardware found"
                : "Select a channel…"}
            </option>
            {channels.map((channel) => (
              <option key={channelKey(channel)} value={channelKey(channel)}>
                {channelLabel(channel)}
              </option>
            ))}
          </select>
          <button
            type="button"
            class="primary"
            disabled={!selected || loading}
            onClick={() => {
              if (selected) {
                setChannelChoice(channelKey(selected));
                onAttach(bus.index, selected.driver, selected.index);
              }
            }}
          >
            Attach
          </button>
          <span class="hint">
            Uses this bus's configured arbitration/FD rates.
          </span>
        </div>
      )}
    </section>
  );
}

export function BusesModal({
  buses,
  realBus,
  hardware,
  loading,
  error,
  onRefresh,
  onAddBus,
  onRemoveBus,
  onRenameBus,
  onSetRates,
  onLoadDbc,
  onSetNodeRole,
  onAttach,
  onDetach,
  onSetReal,
  onClose,
}: Props) {
  useEffect(() => {
    const close = (event: KeyboardEvent) => {
      if (event.key === "Escape") onClose();
    };
    window.addEventListener("keydown", close);
    return () => window.removeEventListener("keydown", close);
  }, [onClose]);

  const channels = hardware?.channels ?? [];
  const attachedCount = buses.filter((bus) => bus.attached).length;

  return (
    <div class="modal-backdrop" onMouseDown={onClose}>
      <div
        class="buses-modal"
        role="dialog"
        aria-modal="true"
        aria-label="Buses and hardware"
        onMouseDown={(event) => event.stopPropagation()}
      >
        <header class="modal-head">
          <h2>Buses</h2>
          <div class="picker-head-right">
            <span>
              {buses.length} buses · {attachedCount} attached
            </span>
            <button type="button" onClick={onAddBus}>
              Add bus
            </button>
            <button type="button" disabled={loading} onClick={onRefresh}>
              Refresh hardware
            </button>
            <button type="button" onClick={onClose}>
              Close
            </button>
          </div>
        </header>

        <div class="buses-tools">
          <label class="real-toggle">
            <input
              type="checkbox"
              checked={realBus}
              onChange={(event) =>
                onSetReal((event.target as HTMLInputElement).checked)
              }
            />
            Real bus
          </label>
          <span class="hint">
            {realBus
              ? "Attached hardware receives and transmits on the wire."
              : "Simulated: attachments stay configured but neither receive nor transmit."}
          </span>
          <span class={`hw-state ${error ? "bad" : ""}`}>
            {loading
              ? "Probing hardware…"
              : error
                ? error
                : `${channels.length} CAN channels`}
          </span>
        </div>

        <div class="buses-body">
          {buses.length === 0 && (
            <div class="empty">
              <strong>No buses in this session</strong>
              <span>Add a bus or load a project.</span>
            </div>
          )}
          {buses.map((bus) => (
            <BusCard
              key={bus.index}
              bus={bus}
              channels={channels}
              loading={loading}
              onAttach={onAttach}
              onDetach={onDetach}
              onRemove={onRemoveBus}
              onRename={onRenameBus}
              onSetRates={onSetRates}
              onLoadDbc={onLoadDbc}
              onSetNodeRole={onSetNodeRole}
            />
          ))}
        </div>

        <p class="hint picker-hint">
          Add/remove, names, rates and DBC paths are host-side edits. Removing a
          bus stops measurement and remaps the remaining channels.
        </p>
      </div>
    </div>
  );
}
