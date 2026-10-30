import { useEffect, useRef, useState } from "preact/hooks";
import type {
  ArxmlScan,
  Bus,
  BusDescription,
  Hardware,
  HwChannel,
  ReplayMap,
} from "../types";
import { FilePick } from "./FilePick";

interface Props {
  buses: Bus[];
  replayMap: ReplayMap;
  realBus: boolean;
  hardware: Hardware | null;
  loading: boolean;
  error: string;
  onError: (message: string) => void;
  onRefresh: () => void;
  onAddBus: (kind: "can" | "flexray" | "ethernet") => void;
  onRemoveBus: (bus: number) => void;
  onRenameBus: (bus: number, name: string) => void;
  onSetRates: (bus: number, arb: number, fd: number) => void;
  onAddDescription: (
    bus: number,
    kind: string,
    path: string,
    cluster?: string,
  ) => void;
  onRemoveDescription: (
    bus: number,
    kind: string,
    path: string,
    cluster?: string,
  ) => void;
  arxmlScan: ArxmlScan | null;
  onScanArxml: (path: string) => void;
  onSetNodeRole: (bus: number, node: string, simulated: boolean) => void;
  onRemoveFr: (bus: number) => void;
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

function slotLabel(bus: Bus): string {
  return bus.kind === "flexray"
    ? `FR${bus.index}`
    : bus.kind === "ethernet"
      ? `ETH${bus.index}`
      : `CAN${bus.index + 1}`;
}

function busKey(bus: Bus): string {
  return `${bus.kind}:${bus.index}`;
}

function rateOptions(standard: number[], current: number): number[] {
  const values = standard.includes(current)
    ? [...standard]
    : [...standard, current];
  return values.sort((a, b) => a - b);
}

function DescriptionList({
  bus,
  kind,
  descriptions,
  scan,
  onScan,
  onAdd,
  onRemove,
  onError,
  accept,
  allowCluster,
}: {
  bus: number;
  kind: string;
  descriptions: BusDescription[];
  scan: ArxmlScan | null;
  onScan: (path: string) => void;
  onAdd: (bus: number, kind: string, path: string, cluster?: string) => void;
  onRemove: (bus: number, kind: string, path: string, cluster?: string) => void;
  onError: (message: string) => void;
  accept: string;
  allowCluster: boolean;
}) {
  const [pending, setPending] = useState<string | null>(null);
  const matches =
    pending && scan?.path === pending
      ? scan.clusters.filter((cluster) => cluster.kind === kind)
      : [];

  useEffect(() => {
    if (!pending || !scan || scan.path !== pending) return;
    const usable = scan.clusters.filter((cluster) => cluster.kind === kind);
    if (usable.length === 1 && allowCluster) {
      onAdd(bus, kind, pending, usable[0].name);
      setPending(null);
    } else if (usable.length === 0) {
      onError(
        `That ARXML declares no ${kind === "can" ? "CAN" : kind === "flexray" ? "FlexRay" : "Ethernet"} cluster`,
      );
      setPending(null);
    }
  }, [pending, scan, kind, bus, allowCluster, onAdd, onError]);

  const pick = (path: string) => {
    if (path.toLowerCase().endsWith(".arxml") && allowCluster) {
      setPending(path);
      onScan(path);
      return;
    }
    onAdd(bus, kind, path);
  };

  return (
    <div class="bus-descriptions">
      <span class="bus-descriptions-label">Descriptions</span>
      <div class="bus-description-list">
        {descriptions.length === 0 && (
          <span class="hint">None bound; frames stay raw.</span>
        )}
        {descriptions.map((desc) => (
          <span
            class="description-chip"
            key={`${desc.kind}:${desc.path}:${desc.cluster ?? ""}`}
            title={desc.cluster ? `${desc.path} · ${desc.cluster}` : desc.path}
          >
            <em>{desc.kind.toUpperCase()}</em>
            {fileName(desc.path)}
            {desc.cluster && <span class="description-cluster">{desc.cluster}</span>}
            <button
              type="button"
              class="description-remove"
              aria-label={`Remove ${fileName(desc.path)}`}
              onClick={() =>
                onRemove(bus, kind, desc.path, desc.cluster || undefined)
              }
            >
              ×
            </button>
          </span>
        ))}
      </div>
      {pending && (
        <div class="arxml-chooser" role="group" aria-label="Choose a cluster">
          <span title={pending}>
            {fileName(pending)}: choose the cluster for this bus
          </span>
          {matches.map((cluster) => (
            <button
              type="button"
              key={cluster.name}
              onClick={() => {
                onAdd(bus, kind, pending, cluster.name);
                setPending(null);
              }}
            >
              {cluster.name}
            </button>
          ))}
          <button type="button" class="mini" onClick={() => setPending(null)}>
            Cancel
          </button>
        </div>
      )}
      <FilePick
        label="Add description…"
        accept={accept}
        onError={onError}
        onPick={pick}
      />
    </div>
  );
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
  scan,
  onScan,
  onAddDescription,
  onRemoveDescription,
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
  scan: ArxmlScan | null;
  onScan: (path: string) => void;
  onAddDescription: (
    bus: number,
    kind: string,
    path: string,
    cluster?: string,
  ) => void;
  onRemoveDescription: (
    bus: number,
    kind: string,
    path: string,
    cluster?: string,
  ) => void;
  onSetNodeRole: (bus: number, node: string, simulated: boolean) => void;
  onError: (message: string) => void;
}) {
  const can = bus.can;
  const nodes = can?.nodes ?? [];
  const attached = can?.attached ?? null;
  const [name, setName] = useState(bus.name);
  const nameRef = useRef(bus.name);
  const [arb, setArb] = useState(can?.bitrateKbps ?? 0);
  const [fd, setFd] = useState(can?.fdDataKbps ?? 0);
  const ratesRef = useRef({ arb: can?.bitrateKbps ?? 0, fd: can?.fdDataKbps ?? 0 });
  const [channelChoice, setChannelChoice] = useState("");
  const selected =
    channels.find((channel) => channelKey(channel) === channelChoice) ?? null;

  useEffect(() => {
    setName(bus.name);
    nameRef.current = bus.name;
  }, [bus.name]);
  useEffect(() => {
    setArb(can?.bitrateKbps ?? 0);
    setFd(can?.fdDataKbps ?? 0);
    ratesRef.current = { arb: can?.bitrateKbps ?? 0, fd: can?.fdDataKbps ?? 0 };
  }, [can?.bitrateKbps ?? 0, can?.fdDataKbps ?? 0]);

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
  const simulated = nodes.filter((node) => node.simulated).length;
  const role = attached
    ? realBus
      ? `Real · ${attached.driver} ch${attached.adapter}`
      : "Attached · sim mode"
    : nodes.length > 0
      ? `${simulated}/${nodes.length} simulated`
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
            attached ? (realBus ? "real" : "attached") : "sim"
          }`}
          title={
            attached
              ? realBus
                ? "Attached adapter is used on the wire"
                : "Adapter configured; Real bus is off"
              : nodes.length > 0
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
          {bus.descriptions.length > 0
            ? `${bus.descriptions.length} description${
                bus.descriptions.length === 1 ? "" : "s"
              }`
            : "no description"}
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

      <DescriptionList
        bus={bus.index}
        kind="can"
        descriptions={bus.descriptions}
        scan={scan}
        onScan={onScan}
        onAdd={onAddDescription}
        onRemove={onRemoveDescription}
        onError={onError}
        accept=".dbc,.arxml"
        allowCluster
      />

      {nodes.length > 0 && (
        <div class="bus-nodes">
          <span class="bus-nodes-label">Nodes</span>
          <div class="bus-node-list">
            {nodes.map((node) => (
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

      {attached ? (
        <div class="bus-attached">
          <span class="attach-dot" />
          <strong>
            {attached.driver} ch {attached.adapter}
          </strong>
          <span>
            {attached.kbps} kbit/s
            {attached.fd ? " · FD" : ""}
            {attached.canTx ? " · TX" : " · RX only"}
          </span>
          {attached.txFail && (
            <span class="attach-warn">
              TX refused {attached.txFail.count}×:{" "}
              {attached.txFail.reason}
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
                : "Pick the hardware channel for this bus"
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
  onAddDescription,
  onRemoveDescription,
  arxmlScan,
  onScanArxml,
  onSetNodeRole,
  onRemoveFr,
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
  const selectedBus = buses.find((bus) => busKey(bus) === selected) ?? null;
  const selectedCanBus = selectedBus?.kind === "can" ? selectedBus : null;
  const selectedFrBus = selectedBus?.kind === "flexray" ? selectedBus : null;
  const selectedEthBus = selectedBus?.kind === "ethernet" ? selectedBus : null;

  const wireFor = (bus: Bus) => {
    if (bus.kind === "can") {
      const attachment = bus.can?.attached;
      if (!attachment) {
        return { text: "—", cls: "none", title: "No adapter attached" };
      }
      return {
        text: `${attachment.driver} ch${attachment.adapter} · ${
          attachment.canTx ? "TX" : "RX"
        }${realBus ? "" : " · off"}`,
        cls: realBus ? "live" : "off",
        title: realBus
          ? "Attached adapter is on the wire"
          : "Adapter configured; Real bus is off",
      };
    }
    if (bus.kind === "flexray") {
      const detail = bus.flexray;
      if (!detail?.attached) {
        return { text: "—", cls: "none", title: "No FlexRay port attached" };
      }
      return {
        text: `V ch${detail.channel} · RX${realBus ? "" : " · off"}`,
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
    if (buses.length === 0) {
      setSelected(null);
      return;
    }
    if (!buses.some((bus) => busKey(bus) === selected)) {
      setSelected(busKey(buses[0]));
    }
  }, [buses, selected]);

  const selectBus = (bus: Bus) => {
    const key = busKey(bus);
    setSelected(key);
    setRowMenu(null);
    setTimeout(() => {
      const id = bus.kind === "can" ? `bus-card-${bus.index}` : `proto-${key}`;
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
    const close = () => setRowMenu(null);
    window.addEventListener("resize", close);
    window.addEventListener("scroll", close, true);
    return () => {
      window.removeEventListener("resize", close);
      window.removeEventListener("scroll", close, true);
    };
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
  const attachedCount = buses.filter(
    (bus) => bus.can?.attached || bus.flexray?.attached,
  ).length;

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
              {buses.length} bus{buses.length === 1 ? "" : "s"} ·{" "}
              {attachedCount} attached
            </span>
            <div class="add-cluster" ref={addRef}>
              <button
                type="button"
                aria-haspopup="menu"
                aria-expanded={addOpen}
                onClick={() => setAddOpen((open) => !open)}
              >
                Add bus
              </button>
              {addOpen && (
                <div class="add-cluster-menu" role="menu" aria-label="Add bus">
                  {[
                    ["can", "CAN bus"],
                    ["flexray", "FlexRay bus"],
                    ["ethernet", "Ethernet bus"],
                  ].map(([kind, label]) => (
                    <button
                      type="button"
                      role="menuitem"
                      key={kind}
                      onClick={() => {
                        setAddOpen(false);
                        onAddBus(kind as "can" | "flexray" | "ethernet");
                      }}
                    >
                      {label}
                    </button>
                  ))}
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

        {buses.length > 0 && (
          <section class="buses-clusters" aria-label="Buses">
            <table>
              <thead>
                <tr>
                  <th>Kind</th>
                  <th>Bus</th>
                  <th>Frames</th>
                  <th>Wire</th>
                  <th>Source</th>
                  <th aria-label="Actions" />
                </tr>
              </thead>
              <tbody>
                {buses.map((bus) => {
                  const key = busKey(bus);
                  const isSelected = key === selected;
                  return (
                    <tr
                      key={key}
                      class={isSelected ? "selected" : ""}
                      tabIndex={0}
                      aria-selected={isSelected}
                      title={`cluster #${bus.id}`}
                      onClick={() => selectBus(bus)}
                      onKeyDown={(event) => {
                        if (event.key === "Enter" || event.key === " ") {
                          event.preventDefault();
                          selectBus(bus);
                        }
                      }}
                    >
                      <td>
                        {bus.kind === "flexray"
                          ? "FlexRay"
                          : bus.kind === "ethernet"
                            ? "Ethernet"
                            : "CAN"}
                      </td>
                      <td>
                        {bus.name}
                        {slotLabel(bus) !== bus.name && (
                          <span class="cluster-slot">{slotLabel(bus)}</span>
                        )}
                      </td>
                      <td>{bus.frames}</td>
                      <td
                        class={`wire ${wireFor(bus).cls}`}
                        title={wireFor(bus).title}
                      >
                        {wireFor(bus).text}
                      </td>
                      <td title={bus.source}>{fileName(bus.source)}</td>
                      <td class="row-actions">
                        <button
                          type="button"
                          class="row-menu-button"
                          aria-haspopup="menu"
                          aria-expanded={rowMenu === key}
                          aria-label={`Actions for ${bus.name}`}
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
                            {bus.kind === "can" && (
                              <button
                                type="button"
                                role="menuitem"
                                onClick={() => {
                                  setRowMenu(null);
                                  document
                                    .getElementById(`bus-name-${bus.index}`)
                                    ?.focus();
                                }}
                              >
                                Rename
                              </button>
                            )}
                            {bus.kind === "can" && (
                              <button
                                type="button"
                                role="menuitem"
                                onClick={() => {
                                  setRowMenu(null);
                                  document
                                    .getElementById(`bus-attach-${bus.index}`)
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
                                  bus.kind === "can"
                                    ? `Remove ${bus.name}? Its attachments, signals and trace rows for this bus are removed.`
                                    : `Remove ${bus.name}?`;
                                if (window.confirm(message)) {
                                  if (bus.kind === "can") onRemoveBus(bus.index);
                                  else if (bus.kind === "flexray") onRemoveFr(bus.index);
                                  else onRemoveEth(bus.index);
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
            <p class="clusters-hint">
              Select a bus to edit its rates, description and hardware attachment.
            </p>
          </section>
        )}

        <div class="buses-tools">
          <label
            class="real-toggle"
            title="Applies to every bus with an attached adapter"
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
              <strong>No buses in this session</strong>
              <span>Add a bus or load a project.</span>
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
              scan={arxmlScan}
              onScan={onScanArxml}
              onAddDescription={onAddDescription}
              onRemoveDescription={onRemoveDescription}
              onSetNodeRole={onSetNodeRole}
              onError={onError}
            />
          )}

          {selectedFrBus && (
          <section class="bus-proto">
            <h3>FlexRay</h3>
            <div
              class={`proto-row${selected === busKey(selectedFrBus) ? " selected" : ""}`}
              id={`proto-${busKey(selectedFrBus)}`}
              onClick={() => setSelected(busKey(selectedFrBus))}
            >
              <strong>{selectedFrBus.name}</strong>
              <span>{selectedFrBus.flexray?.cluster || "unnamed cluster"}</span>
              <em>{selectedFrBus.frames} frames</em>
              {selectedFrBus.flexray?.attached ? (
                <>
                  <em class="fr-watch" title="Receive-only Vector FlexRay watch">
                    ch{selectedFrBus.flexray.channel} · RX
                  </em>
                  <button
                    type="button"
                    class="mini"
                    onClick={() => onDetachFr(selectedFrBus.index)}
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
                    if (Number.isFinite(index)) onAttachFr(selectedFrBus.index, index);
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
              <DescriptionList
                bus={selectedFrBus.index}
                kind="flexray"
                descriptions={selectedFrBus.descriptions}
                scan={arxmlScan}
                onScan={onScanArxml}
                onAdd={onAddDescription}
                onRemove={onRemoveDescription}
                onError={onError}
                accept=".xml,.arxml,.fibex"
                allowCluster
              />
              <button
                type="button"
                class="bus-remove"
                onClick={() => onRemoveFr(selectedFrBus.index)}
              >
                Remove bus
              </button>
            </div>
          </section>
          )}

          {selectedEthBus && (
          <section class="bus-proto">
            <h3>Ethernet</h3>
            <div
              class={`proto-row${selected === busKey(selectedEthBus) ? " selected" : ""}`}
              id={`proto-${busKey(selectedEthBus)}`}
              onClick={() => setSelected(busKey(selectedEthBus))}
            >
              <strong>{selectedEthBus.name}</strong>
              <span>
                {selectedEthBus.ethernet?.clusters ?? 0} clusters ·{" "}
                {selectedEthBus.frames} frames
              </span>
              <DescriptionList
                bus={selectedEthBus.index}
                kind="ethernet"
                descriptions={selectedEthBus.descriptions}
                scan={arxmlScan}
                onScan={onScanArxml}
                onAdd={onAddDescription}
                onRemove={onRemoveDescription}
                onError={onError}
                accept=".arxml"
                allowCluster
              />
              <button
                type="button"
                class="bus-remove"
                onClick={() => onRemoveEth(selectedEthBus.index)}
              >
                Remove bus
              </button>
            </div>
          </section>
          )}

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
                  targets: buses
                    .filter((bus) => bus.kind === "can")
                    .map((bus) => ({ id: bus.index, label: bus.name })),
                  channel: (raw: number) => `Channel ${raw + 1}`,
                },
                {
                  kind: "fr",
                  title: "FlexRay channels in the log",
                  rows: replayMap.fr,
                  targets: buses
                    .filter((bus) => bus.kind === "flexray")
                    .map((bus) => ({ id: bus.index, label: bus.name })),
                  channel: (raw: number) => `#${raw}`,
                },
                {
                  kind: "eth",
                  title: "Ethernet channels in the log",
                  rows: replayMap.eth,
                  targets: buses
                    .filter((bus) => bus.kind === "ethernet")
                    .map((bus) => ({ id: bus.index, label: bus.name })),
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
