import { useEffect, useRef, useState } from "preact/hooks";
import type { Bus, EthBus, FrBus, Hardware, HwChannel } from "../types";
import { FilePick } from "./FilePick";

interface Props {
  buses: Bus[];
  frBuses: FrBus[];
  ethBuses: EthBus[];
  realBus: boolean;
  hardware: Hardware | null;
  loading: boolean;
  error: string;
  onError: (message: string) => void;
  onRefresh: () => void;
  onAddBus: () => void;
  onRemoveBus: (bus: number) => void;
  onRenameBus: (bus: number, name: string) => void;
  onSetRates: (bus: number, arb: number, fd: number) => void;
  onLoadDbc: (bus: number, path: string) => void;
  onSetNodeRole: (bus: number, node: string, simulated: boolean) => void;
  onLoadFr: (path: string) => void;
  onRemoveFr: (bus: number) => void;
  onLoadEth: (path: string) => void;
  onRemoveEth: (bus: number) => void;
  onAttach: (bus: number, driver: string, index: number) => void;
  onDetach: (bus: number) => void;
  onAttachFr: (bus: number, index: number) => void;
  onDetachFr: (bus: number) => void;
  onSetReal: (real: boolean) => void;
  onOpenScripts: () => void;
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
  onError,
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
  onError: (message: string) => void;
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
        <FilePick
          label="Choose…"
          accept=".dbc,.arxml"
          onError={onError}
          onPick={(path) => {
            setDbc(path);
            onLoadDbc(bus.index, path);
          }}
        />
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
  frBuses,
  ethBuses,
  realBus,
  hardware,
  loading,
  error,
  onError,
  onRefresh,
  onAddBus,
  onRemoveBus,
  onRenameBus,
  onSetRates,
  onLoadDbc,
  onSetNodeRole,
  onLoadFr,
  onRemoveFr,
  onLoadEth,
  onRemoveEth,
  onAttach,
  onDetach,
  onAttachFr,
  onDetachFr,
  onSetReal,
  onOpenScripts,
  onClose,
}: Props) {
  const [frPath, setFrPath] = useState("");
  const [ethPath, setEthPath] = useState("");
  useEffect(() => {
    const close = (event: KeyboardEvent) => {
      if (event.key === "Escape") onClose();
    };
    window.addEventListener("keydown", close);
    return () => window.removeEventListener("keydown", close);
  }, [onClose]);

  const channels = hardware?.channels ?? [];
  const frChannels = hardware?.frChannels ?? [];
  const attachedFr = frBuses.filter((bus) => bus.attached).length;
  const attachedCount =
    buses.filter((bus) => bus.attached).length + attachedFr;

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
            <button type="button" onClick={onOpenScripts}>
              Scripts…
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
              onError={onError}
            />
          ))}

          <section class="bus-proto">
            <h3>FlexRay</h3>
            {frBuses.map((bus) => (
              <div class="proto-row" key={bus.bus}>
                <strong>{bus.name}</strong>
                <span>{bus.cluster || "unnamed cluster"}</span>
                <em>{bus.frames} frames</em>
                {bus.attached ? (
                  <>
                    <em class="fr-watch" title="Receive-only Vector FlexRay watch">
                      ch{bus.channel} · RX
                    </em>
                    <button
                      type="button"
                      class="mini"
                      onClick={() => onDetachFr(bus.bus)}
                    >
                      Detach
                    </button>
                  </>
                ) : (
                  <select
                    class="fr-attach"
                    value=""
                    disabled={frChannels.length === 0}
                    title={
                      frChannels.length
                        ? "Attach this port in receive-only mode"
                        : hardware?.frError || "No Vector FlexRay channel found"
                    }
                    onChange={(event) => {
                      const index = Number(
                        (event.target as HTMLSelectElement).value,
                      );
                      if (Number.isFinite(index)) onAttachFr(bus.bus, index);
                    }}
                  >
                    <option value="">
                      {frChannels.length ? "Attach port…" : "No FR port"}
                    </option>
                    {frChannels.map((channel) => (
                      <option
                        key={`${channel.driver}:${channel.index}`}
                        value={channel.index}
                      >
                        [V] ch{channel.index}: {channel.name}
                        {channel.detail ? ` · ${channel.detail}` : ""}
                      </option>
                    ))}
                  </select>
                )}
                <span class="proto-path" title={bus.path}>
                  {fileName(bus.path)}
                </span>
                <button
                  type="button"
                  class="bus-remove"
                  onClick={() => onRemoveFr(bus.bus)}
                >
                  Remove
                </button>
              </div>
            ))}
            <form
              class="proto-add"
              onSubmit={(event) => {
                event.preventDefault();
                if (frPath.trim()) {
                  onLoadFr(frPath.trim());
                  setFrPath("");
                }
              }}
            >
              <input
                value={frPath}
                placeholder="Host path to a FIBEX / ARXML cluster"
                onInput={(event) =>
                  setFrPath((event.target as HTMLInputElement).value)
                }
              />
              <button type="submit" disabled={!frPath.trim()}>
                Load description
              </button>
              <FilePick
                label="Choose…"
                accept=".xml,.arxml,.fibex"
                onError={onError}
                onPick={(path) => {
                  setFrPath(path);
                  onLoadFr(path);
                }}
              />
            </form>
          </section>

          <section class="bus-proto">
            <h3>Ethernet</h3>
            {ethBuses.map((bus) => (
              <div class="proto-row" key={bus.bus}>
                <strong>{bus.name}</strong>
                <span>
                  {bus.clusters} clusters · {bus.frames} frames
                </span>
                <span class="proto-path" title={bus.path}>
                  {fileName(bus.path)}
                </span>
                <button
                  type="button"
                  class="bus-remove"
                  onClick={() => onRemoveEth(bus.bus)}
                >
                  Remove
                </button>
              </div>
            ))}
            <form
              class="proto-add"
              onSubmit={(event) => {
                event.preventDefault();
                if (ethPath.trim()) {
                  onLoadEth(ethPath.trim());
                  setEthPath("");
                }
              }}
            >
              <input
                value={ethPath}
                placeholder="Host path to an AUTOSAR ARXML matrix"
                onInput={(event) =>
                  setEthPath((event.target as HTMLInputElement).value)
                }
              />
              <button type="submit" disabled={!ethPath.trim()}>
                Load matrix
              </button>
              <FilePick
                label="Choose…"
                accept=".arxml,.xml"
                onError={onError}
                onPick={(path) => {
                  setEthPath(path);
                  onLoadEth(path);
                }}
              />
            </form>
          </section>
        </div>

        <p class="hint picker-hint">
          Add/remove, names, rates and DBC paths are host-side edits. Removing a
          bus stops measurement and remaps the remaining channels.
        </p>
      </div>
    </div>
  );
}
