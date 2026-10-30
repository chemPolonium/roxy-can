import { useEffect, useRef, useState } from "preact/hooks";
import type {
  Bus,
  Cluster,
  EthBus,
  FrBus,
  Hardware,
  HwChannel,
  ReplayMap,
} from "../types";
import { FilePick } from "./FilePick";

interface Props {
  buses: Bus[];
  clusters: Cluster[];
  frBuses: FrBus[];
  ethBuses: EthBus[];
  replayMap: ReplayMap;
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
  onImportArxml: (path: string) => void;
  onSetNodeRole: (bus: number, node: string, simulated: boolean) => void;
  onLoadFr: (path: string) => void;
  onRemoveFr: (bus: number) => void;
  onLoadEth: (path: string) => void;
  onRemoveEth: (bus: number) => void;
  onAttach: (bus: number, driver: string, index: number) => void;
  onDetach: (bus: number) => void;
  onAttachFr: (bus: number, index: number) => void;
  onDetachFr: (bus: number) => void;
  onSetReplay: (kind: string, raw: number, target: number | null) => void;
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

function slotLabel(cluster: Cluster): string {
  return cluster.kind === "flexray"
    ? `FR${cluster.bus}`
    : cluster.kind === "ethernet"
      ? `ETH${cluster.bus}`
      : `CAN${cluster.bus + 1}`;
}

function clusterKey(cluster: Cluster): string {
  return `${cluster.kind}:${cluster.bus}`;
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
  realBus,
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
  realBus: boolean;
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
  const simulated = bus.nodes.filter((node) => node.simulated).length;
  const role = bus.attached
    ? realBus
      ? `Real · ${bus.attached.driver} ch${bus.attached.adapter}`
      : "Attached · sim mode"
    : bus.nodes.length > 0
      ? `${simulated}/${bus.nodes.length} simulated`
      : "No hardware";

  return (
    <section class="bus-card" id={`bus-card-${bus.index}`}>
      <div class="bus-head">
        <input
          class="bus-name-input"
          id={`bus-name-${bus.index}`}
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
        <span
          class={`bus-role ${
            bus.attached ? (realBus ? "real" : "attached") : "sim"
          }`}
          title={
            bus.attached
              ? realBus
                ? "Attached adapter is used on the wire"
                : "Adapter configured; Real bus is off"
              : bus.nodes.length > 0
                ? "These nodes generate traffic in simulation"
                : "No adapter attached and no DBC nodes"
          }
        >
          {role}
        </span>
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
          placeholder="Host path to a CAN database (.dbc / ARXML)"
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
          Load database
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
            id={`bus-attach-${bus.index}`}
            value={channelChoice}
            disabled={channels.length === 0 || loading}
            title={
              channels.length === 0
                ? "Plug in the adapter, then use Refresh hardware"
                : "Pick the hardware channel for this cluster"
            }
            onChange={(event) =>
              setChannelChoice((event.target as HTMLSelectElement).value)
            }
          >
            <option value="">
              {channels.length === 0
                ? "No CAN hardware found"
                : "Select a hardware channel…"}
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
            title={
              selected
                ? "Attach this channel using the configured rates"
                : channels.length === 0
                  ? "No adapter found; plug it in and use Refresh hardware"
                  : "Select a hardware channel first"
            }
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
            {selected
              ? "Uses this bus's configured arbitration/FD rates."
              : channels.length === 0
                ? "Plug in the adapter, then use Refresh hardware."
                : "Pick a hardware channel above, then Attach."}
          </span>
        </div>
      )}
    </section>
  );
}

export function BusesModal({
  buses,
  clusters,
  frBuses,
  ethBuses,
  replayMap,
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
  onImportArxml,
  onSetNodeRole,
  onLoadFr,
  onRemoveFr,
  onLoadEth,
  onRemoveEth,
  onAttach,
  onDetach,
  onAttachFr,
  onDetachFr,
  onSetReplay,
  onSetReal,
  onOpenScripts,
  onClose,
}: Props) {
  const [addOpen, setAddOpen] = useState(false);
  const addRef = useRef<HTMLDivElement>(null);
  const [selected, setSelected] = useState<string | null>(null);
  const [rowMenu, setRowMenu] = useState<string | null>(null);
  const [menuPos, setMenuPos] = useState({ top: 0, left: 0 });
  const rowMenuRef = useRef<HTMLDivElement>(null);
  const selectedCluster = clusters.find((cluster) => clusterKey(cluster) === selected) ?? null;
  const selectedCanBus =
    selectedCluster?.kind === "can"
      ? (buses.find((bus) => bus.index === selectedCluster.bus) ?? null)
      : null;

  const wireFor = (cluster: Cluster) => {
    if (cluster.kind === "can") {
      const bus = buses.find((candidate) => candidate.index === cluster.bus);
      if (!bus?.attached) {
        return { text: "—", cls: "none", title: "No adapter attached" };
      }
      const attachment = bus.attached;
      const live = realBus;
      return {
        text: `${attachment.driver} ch${attachment.adapter} · ${
          attachment.canTx ? "TX" : "RX"
        }${live ? "" : " · off"}`,
        cls: live ? "live" : "off",
        title: live
          ? "Attached adapter is on the wire"
          : "Adapter configured; Real bus is off",
      };
    }
    if (cluster.kind === "flexray") {
      const bus = frBuses.find((candidate) => candidate.bus === cluster.bus);
      if (!bus?.attached) {
        return { text: "—", cls: "none", title: "No FlexRay port attached" };
      }
      return {
        text: `V ch${bus.channel} · RX${realBus ? "" : " · off"}`,
        cls: realBus ? "live" : "off",
        title: realBus
          ? "Receive-only FlexRay watch"
          : "Port configured; Real bus is off",
      };
    }
    return {
      text: "session",
      cls: "none",
      title: "Ethernet frames stay in this session; there is no send port",
    };
  };

  useEffect(() => {
    if (clusters.length === 0) {
      setSelected(null);
      return;
    }
    if (!clusters.some((cluster) => clusterKey(cluster) === selected)) {
      setSelected(clusterKey(clusters[0]));
    }
  }, [clusters, selected]);

  const selectCluster = (cluster: Cluster) => {
    const key = clusterKey(cluster);
    setSelected(key);
    setRowMenu(null);
    setTimeout(() => {
      const id =
        cluster.kind === "can"
          ? `bus-card-${cluster.bus}`
          : `proto-${key}`;
      document.getElementById(id)?.scrollIntoView({ block: "nearest" });
    }, 0);
  };

  useEffect(() => {
    if (!rowMenu) return;
    rowMenuRef.current
      ?.querySelector<HTMLButtonElement>('[role="menuitem"]')
      ?.focus();
  }, [rowMenu]);

  useEffect(() => {
    if (!rowMenu) return;
    const close = (event: MouseEvent) => {
      if (!rowMenuRef.current?.contains(event.target as Node)) setRowMenu(null);
    };
    const escape = (event: KeyboardEvent) => {
      if (event.key === "Escape") setRowMenu(null);
    };
    document.addEventListener("mousedown", close);
    document.addEventListener("keydown", escape);
    return () => {
      document.removeEventListener("mousedown", close);
      document.removeEventListener("keydown", escape);
    };
  }, [rowMenu]);
  useEffect(() => {
    if (!addOpen) return;
    const close = (event: MouseEvent) => {
      if (!addRef.current?.contains(event.target as Node)) setAddOpen(false);
    };
    document.addEventListener("mousedown", close);
    return () => document.removeEventListener("mousedown", close);
  }, [addOpen]);
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
              {clusters.length} cluster{clusters.length === 1 ? "" : "s"} ·{" "}
              {attachedCount} attached
            </span>
            <div class="add-cluster" ref={addRef}>
              <button
                type="button"
                aria-haspopup="menu"
                aria-expanded={addOpen}
                onClick={() => setAddOpen((open) => !open)}
              >
                Add cluster
              </button>
              {addOpen && (
                <div class="add-cluster-menu" role="menu" aria-label="Add cluster">
                  <button
                    type="button"
                    role="menuitem"
                    onClick={() => {
                      setAddOpen(false);
                      onAddBus();
                    }}
                  >
                    CAN cluster
                  </button>
                  <FilePick
                    menuItem
                    class="add-cluster-item"
                    label="ARXML matrix…"
                    accept=".arxml"
                    onError={onError}
                    onPick={(path) => {
                      setAddOpen(false);
                      onImportArxml(path);
                    }}
                  />
                  <FilePick
                    menuItem
                    class="add-cluster-item"
                    label="FlexRay FIBEX…"
                    accept=".xml,.arxml,.fibex"
                    onError={onError}
                    onPick={(path) => {
                      setAddOpen(false);
                      onLoadFr(path);
                    }}
                  />
                  <FilePick
                    menuItem
                    class="add-cluster-item"
                    label="Ethernet ARXML…"
                    accept=".arxml"
                    onError={onError}
                    onPick={(path) => {
                      setAddOpen(false);
                      onLoadEth(path);
                    }}
                  />
                </div>
              )}
            </div>
            <button type="button" onClick={onOpenScripts}>
              Scripts…
            </button>
            <button type="button" onClick={onClose}>
              Close
            </button>
          </div>
        </header>

        {clusters.length > 0 && (
          <section class="buses-clusters" aria-label="Clusters">
            <table>
              <thead>
                <tr>
                  <th>Kind</th>
                  <th>Cluster</th>
                  <th>Frames</th>
                  <th>Wire</th>
                  <th>Source</th>
                  <th aria-label="Actions" />
                </tr>
              </thead>
              <tbody>
                {clusters.map((cluster) => {
                  const key = clusterKey(cluster);
                  const isSelected = key === selected;
                  return (
                    <tr
                      key={key}
                      class={isSelected ? "selected" : ""}
                      tabIndex={0}
                      aria-selected={isSelected}
                      title={`cluster #${cluster.id}`}
                      onClick={() => selectCluster(cluster)}
                      onKeyDown={(event) => {
                        if (event.key === "Enter" || event.key === " ") {
                          event.preventDefault();
                          selectCluster(cluster);
                        }
                      }}
                    >
                      <td>
                        {cluster.kind === "flexray"
                          ? "FlexRay"
                          : cluster.kind === "ethernet"
                            ? "Ethernet"
                            : "CAN"}
                      </td>
                      <td>
                        {cluster.name}
                        {slotLabel(cluster) !== cluster.name && (
                          <span class="cluster-slot">{slotLabel(cluster)}</span>
                        )}
                      </td>
                      <td>{cluster.frames}</td>
                      <td
                        class={`wire ${wireFor(cluster).cls}`}
                        title={wireFor(cluster).title}
                      >
                        {wireFor(cluster).text}
                      </td>
                      <td title={cluster.source}>{fileName(cluster.source)}</td>
                      <td class="row-actions">
                        <button
                          type="button"
                          class="row-menu-button"
                          aria-haspopup="menu"
                          aria-expanded={rowMenu === key}
                          aria-label={`Actions for ${cluster.name}`}
                          onClick={(event) => {
                            event.stopPropagation();
                            setSelected(key);
                            const rect = (
                              event.currentTarget as HTMLButtonElement
                            ).getBoundingClientRect();
                            setMenuPos({
                              top: rect.bottom + 2,
                              left: Math.max(8, rect.right - 140),
                            });
                            setRowMenu(rowMenu === key ? null : key);
                          }}
                        >
                          ⋯
                        </button>
                        {rowMenu === key && (
                          <div
                            class="row-menu"
                            role="menu"
                            ref={rowMenuRef}
                            style={{ top: `${menuPos.top}px`, left: `${menuPos.left}px` }}
                            onClick={(event) => event.stopPropagation()}
                            onKeyDown={(event) => {
                              const menu = event.currentTarget;
                              const items = [
                                ...menu.querySelectorAll<HTMLButtonElement>(
                                  '[role="menuitem"]',
                                ),
                              ];
                              const at = items.indexOf(
                                document.activeElement as HTMLButtonElement,
                              );
                              if (event.key === "Escape") {
                                event.preventDefault();
                                event.stopPropagation();
                                setRowMenu(null);
                                menu.parentElement
                                  ?.querySelector<HTMLButtonElement>(
                                    ".row-menu-button",
                                  )
                                  ?.focus();
                                return;
                              }
                              const next =
                                event.key === "ArrowDown"
                                  ? (at + 1) % items.length
                                  : event.key === "ArrowUp"
                                    ? (at - 1 + items.length) % items.length
                                    : event.key === "Home"
                                      ? 0
                                      : event.key === "End"
                                        ? items.length - 1
                                        : -1;
                              if (next >= 0) {
                                event.preventDefault();
                                event.stopPropagation();
                                items[next]?.focus();
                              }
                            }}
                          >
                            {cluster.kind === "can" && (
                              <button
                                type="button"
                                role="menuitem"
                                onClick={() => {
                                  setRowMenu(null);
                                  document
                                    .getElementById(`bus-name-${cluster.bus}`)
                                    ?.focus();
                                }}
                              >
                                Rename
                              </button>
                            )}
                            {cluster.kind === "can" && (
                              <button
                                type="button"
                                role="menuitem"
                                onClick={() => {
                                  setRowMenu(null);
                                  document
                                    .getElementById(`bus-attach-${cluster.bus}`)
                                    ?.focus();
                                }}
                              >
                                Attach…
                              </button>
                            )}
                            <button
                              type="button"
                              role="menuitem"
                              class="danger"
                              onClick={() => {
                                setRowMenu(null);
                                const message =
                                  cluster.kind === "can"
                                    ? `Remove ${cluster.name}? Its attachments, signals and trace rows for this bus are removed.`
                                    : `Remove ${cluster.name}?`;
                                if (window.confirm(message)) {
                                  if (cluster.kind === "can") onRemoveBus(cluster.bus);
                                  else if (cluster.kind === "flexray") onRemoveFr(cluster.bus);
                                  else onRemoveEth(cluster.bus);
                                }
                              }}
                            >
                              Remove
                            </button>
                          </div>
                        )}
                      </td>
                    </tr>
                  );
                })}
              </tbody>
            </table>
          </section>
        )}

        <div class="buses-tools">
          <label
            class="real-toggle"
            title="Applies to every cluster with an attached adapter"
          >
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
              ? "Every attached adapter receives and transmits on the wire."
              : "Simulated: configured adapters stay attached but neither receive nor transmit."}
          </span>
          <button
            type="button"
            class="hw-refresh"
            disabled={loading}
            onClick={onRefresh}
            title="Re-enumerate hardware channels"
          >
            Refresh hardware
          </button>
          <span class={`hw-state ${error ? "bad" : ""}`}>
            {loading
              ? "Probing hardware…"
              : error
                ? error
                : `${channels.length} CAN hardware channels`}
          </span>
        </div>

        <div class="buses-body">
          {buses.length === 0 && (
            <div class="empty">
              <strong>No clusters in this session</strong>
              <span>Add a cluster or load a project.</span>
            </div>
          )}
          {selectedCanBus && (
            <BusCard
              key={selectedCanBus.index}
              bus={selectedCanBus}
              channels={channels}
              realBus={realBus}
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
          )}

          <section class="bus-proto">
            <h3>FlexRay</h3>
            {frBuses.map((bus) => (
              <div
                class={`proto-row${selected === `flexray:${bus.bus}` ? " selected" : ""}`}
                id={`proto-flexray:${bus.bus}`}
                key={bus.bus}
                onClick={() => setSelected(`flexray:${bus.bus}`)}
              >
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
          </section>

          <section class="bus-proto">
            <h3>Ethernet</h3>
            {ethBuses.map((bus) => (
              <div
                class={`proto-row${selected === `ethernet:${bus.bus}` ? " selected" : ""}`}
                id={`proto-ethernet:${bus.bus}`}
                key={bus.bus}
                onClick={() => setSelected(`ethernet:${bus.bus}`)}
              >
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
          </section>

          <section class="bus-proto replay-map">
            <h3>Replay channel map</h3>
            {replayMap.can.length === 0 &&
            replayMap.fr.length === 0 &&
            replayMap.eth.length === 0 ? (
              <p class="hint">
                Replay a log and the channels it contains are listed here.
              </p>
            ) : (
              [
                {
                  kind: "can",
                  title: "CAN channels in the log",
                  rows: replayMap.can,
                  targets: buses.map((bus) => ({
                    id: bus.index,
                    label: bus.name,
                  })),
                  channel: (raw: number) => `Channel ${raw + 1}`,
                },
                {
                  kind: "fr",
                  title: "FlexRay channels in the log",
                  rows: replayMap.fr,
                  targets: frBuses.map((bus) => ({
                    id: bus.bus,
                    label: bus.name,
                  })),
                  channel: (raw: number) => `#${raw}`,
                },
                {
                  kind: "eth",
                  title: "Ethernet channels in the log",
                  rows: replayMap.eth,
                  targets: ethBuses.map((bus) => ({
                    id: bus.bus,
                    label: bus.name,
                  })),
                  channel: (raw: number) => `#${raw}`,
                },
              ].map((group) =>
                group.rows.length === 0 ? null : (
                  <div class="replay-group" key={group.kind}>
                    <span class="replay-group-title">{group.title}</span>
                    {group.rows.map((row) => {
                      const value =
                        row.target == null
                          ? "none"
                          : row.target === row.raw
                            ? "raw"
                            : `b${row.target}`;
                      return (
                        <div
                          class="replay-row"
                          key={`${group.kind}:${row.raw}`}
                          title="Which project bus replays this log channel"
                        >
                          <span>{group.channel(row.raw)}</span>
                          <em>{row.count} frames</em>
                          <select
                            value={value}
                            onChange={(event) => {
                              const next = (
                                event.target as HTMLSelectElement
                              ).value;
                              onSetReplay(
                                group.kind,
                                row.raw,
                                next === "raw"
                                  ? row.raw
                                  : next === "none"
                                    ? null
                                    : Number(next.slice(1)),
                              );
                            }}
                          >
                            <option value="raw">
                              As-is ({group.channel(row.raw)})
                            </option>
                            {group.targets.map((target) => (
                              <option
                                key={target.id}
                                value={`b${target.id}`}
                              >
                                {target.label}
                              </option>
                            ))}
                            <option value="none">No replay</option>
                          </select>
                        </div>
                      );
                    })}
                  </div>
                ),
              )
            )}
          </section>
        </div>

        <p class="hint picker-hint">
          Add/remove, names, rates and database paths are host-side edits. Removing a
          cluster stops measurement and may remap the remaining CAN channels.
        </p>
      </div>
    </div>
  );
}
