import { useEffect, useState } from "preact/hooks";
import type { Bus, Hardware, HwChannel } from "../types";

interface Props {
  buses: Bus[];
  realBus: boolean;
  hardware: Hardware | null;
  loading: boolean;
  error: string;
  onRefresh: () => void;
  onAttach: (bus: number, driver: string, index: number) => void;
  onDetach: (bus: number) => void;
  onSetReal: (real: boolean) => void;
  onClose: () => void;
}

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

export function BusesModal({
  buses,
  realBus,
  hardware,
  loading,
  error,
  onRefresh,
  onAttach,
  onDetach,
  onSetReal,
  onClose,
}: Props) {
  const [choices, setChoices] = useState<Record<number, string>>({});

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
          <span class="spacer" />
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
              <span>Load a project with at least one bus.</span>
            </div>
          )}
          {buses.map((bus) => {
            const preferred = choices[bus.index] ?? "";
            const selected =
              channels.find((channel) => channelKey(channel) === preferred) ??
              null;
            return (
              <section class="bus-card" key={bus.index}>
                <div class="bus-head">
                  <strong>{bus.name}</strong>
                  <span class="bus-rates">
                    {bus.bitrateKbps} kbit/s
                    {bus.fdDataKbps > 0
                      ? ` · FD data ${bus.fdDataKbps} kbit/s`
                      : " · classic CAN"}
                  </span>
                  <span class="bus-dbcs">
                    {bus.dbcPaths.length > 0
                      ? bus.dbcPaths.map(fileName).join(", ")
                      : "no database"}
                  </span>
                </div>

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
                    <button
                      type="button"
                      onClick={() => onDetach(bus.index)}
                    >
                      Detach
                    </button>
                  </div>
                ) : (
                  <div class="bus-attach">
                    <select
                      value={preferred}
                      disabled={channels.length === 0 || loading}
                      onChange={(event) =>
                        setChoices((old) => ({
                          ...old,
                          [bus.index]: (event.target as HTMLSelectElement).value,
                        }))
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
          })}
        </div>

        <p class="hint picker-hint">
          Add/remove buses and load DBCs are the next slice. Attachments survive
          the Simulated/Real bus switch.
        </p>
      </div>
    </div>
  );
}
