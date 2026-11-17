import { useEffect, useMemo, useState } from "preact/hooks";
import type {
  Catalog,
  CatalogChannel,
  CatalogMessage,
  CatalogSignal,
} from "../types";

function messageId(channel: CatalogChannel, message: CatalogMessage): string {
  if (channel.kind === "fr") return `Slot ${message.id}`;
  if (channel.kind === "eth") return "frame";
  const width = message.extended ? 8 : 3;
  return `0x${message.id.toString(16).toUpperCase().padStart(width, "0")}`;
}

function messageKey(
  channel: CatalogChannel,
  message: CatalogMessage,
): string {
  return `${channel.kind}:${channel.index}:${message.id}:${message.extended}`;
}

function toggleSet(set: Set<string>, key: string): Set<string> {
  const next = new Set(set);
  if (next.has(key)) next.delete(key);
  else next.add(key);
  return next;
}

export function DbcExplorer({
  catalog,
  loading,
  error,
  charted,
  watched,
  onAddChart,
  onAddMonitor,
  onClose,
}: {
  catalog: Catalog | null;
  loading: boolean;
  error: string;
  charted: Set<string>;
  watched: Set<string>;
  onAddChart: (
    channel: CatalogChannel,
    message: CatalogMessage,
    signal: CatalogSignal,
  ) => void;
  onAddMonitor: (
    channel: CatalogChannel,
    message: CatalogMessage,
    signal: CatalogSignal,
  ) => void;
  onClose: () => void;
}) {
  const [query, setQuery] = useState("");
  const [expanded, setExpanded] = useState<Set<string>>(new Set());
  const [collapsedSearch, setCollapsedSearch] = useState<Set<string>>(
    new Set(),
  );
  const [usedOnly, setUsedOnly] = useState(false);

  useEffect(() => {
    setCollapsedSearch(new Set());
  }, [query]);

  useEffect(() => {
    const close = (event: KeyboardEvent) => {
      if (event.key === "Escape") onClose();
    };
    window.addEventListener("keydown", close);
    return () => window.removeEventListener("keydown", close);
  }, [onClose]);

  const groups = useMemo(() => {
    if (!catalog) return [];
    const needle = query.trim().toLowerCase();
    const active = (key: string) => charted.has(key) || watched.has(key);
    return catalog.channels
      .map((channel) => {
        const channelHit =
          needle.length > 0 && channel.name.toLowerCase().includes(needle);
        const messages = channel.messages
          .map((message) => {
            const messageHit =
              channelHit ||
              message.name.toLowerCase().includes(needle) ||
              (message.comment ?? "").toLowerCase().includes(needle) ||
              messageId(channel, message).toLowerCase().includes(needle);
            let signals = messageHit
              ? message.signals
              : message.signals.filter(
                  (signal) =>
                    signal.name.toLowerCase().includes(needle) ||
                    signal.unit.toLowerCase().includes(needle) ||
                    (signal.comment ?? "").toLowerCase().includes(needle),
                );
            if (usedOnly) {
              signals = signals.filter((signal) => active(signal.key));
            }
            return {
              message,
              signals,
              key: messageKey(channel, message),
              total: message.signals.length,
              active: message.signals.filter((signal) => active(signal.key))
                .length,
            };
          })
          .filter((entry) => entry.signals.length > 0);
        return { channel, messages };
      })
      .filter((group) => group.messages.length > 0);
  }, [catalog, query, charted, watched, usedOnly]);

  const querying = query.trim().length > 0;
  const messageOpen = (key: string) =>
    usedOnly ||
    (querying ? !collapsedSearch.has(key) : expanded.has(key));
  const toggleMessage = (key: string) => {
    if (querying) setCollapsedSearch((old) => toggleSet(old, key));
    else setExpanded((old) => toggleSet(old, key));
  };

  return (
    <>
      <div class="dbc-scrim" onMouseDown={onClose} />
      <aside class="dbc-drawer" aria-label="DBC explorer">
        <header class="dbc-drawer-head">
          <strong>DBC explorer</strong>
          <button type="button" class="mini" onClick={onClose}>
            Close
          </button>
        </header>
        <div class="dbc-drawer-search">
          <input
            type="search"
            autoFocus
            value={query}
            placeholder="Filter bus, message or signal…"
            onInput={(event) =>
              setQuery((event.target as HTMLInputElement).value)
            }
          />
          <button
            type="button"
            class={`mini ${usedOnly ? "on" : ""}`}
            aria-pressed={usedOnly}
            title="Show only signals already on the chart or in Monitor"
            onClick={() => setUsedOnly((old) => !old)}
          >
            In use only
          </button>
        </div>
        <div class="dbc-drawer-body">
          {loading && (
            <div class="empty">
              <strong>Loading catalog…</strong>
            </div>
          )}
          {!loading && error && (
            <div class="empty">
              <strong>Catalog unavailable</strong>
              <span>{error}</span>
            </div>
          )}
          {!loading && !error && groups.length === 0 && (
            <div class="empty">
              <strong>
                {usedOnly
                  ? "No signals in use"
                  : query
                    ? "No matching signals"
                    : "No signals available"}
              </strong>
              <span>
                {usedOnly
                  ? "Add a signal to the chart or Monitor first."
                  : query
                    ? "Try a different bus, message or signal name."
                    : "Load a description file into the project first."}
              </span>
            </div>
          )}
          {!loading &&
            !error &&
            groups.map(({ channel, messages }) => (
              <section
                class="dbc-channel"
                key={`${channel.kind}:${channel.index}`}
              >
                <h4>{channel.name}</h4>
                {messages.map(({ message, signals, key, total, active }) => {
                  const open = messageOpen(key);
                  return (
                    <div class="dbc-message" key={key}>
                      <button
                        type="button"
                        class="dbc-message-head"
                        aria-expanded={open}
                        onClick={() => toggleMessage(key)}
                      >
                        <span class="dbc-message-title">
                          <span class="picker-chevron">
                            {open ? "▾" : "▸"}
                          </span>
                          {message.name}
                        </span>
                        <span class="dbc-message-meta">
                          <em>
                            {messageId(channel, message)}
                            {channel.kind === "can" && message.fd
                              ? " · CAN FD"
                              : ""}
                          </em>
                          <small>
                            {signals.length}
                            {querying || usedOnly ? ` / ${total}` : ""}{" "}
                            signals
                          </small>
                          {active > 0 && <small>{active} in use</small>}
                        </span>
                      </button>
                      {open &&
                        signals.map((signal) => {
                      const onChart = charted.has(signal.key);
                      const onMonitor = watched.has(signal.key);
                      return (
                        <div
                          class="dbc-signal"
                          key={signal.key}
                          draggable
                          title="Drag to the chart, Monitor or Data tab"
                          onDragStart={(event) => {
                            event.dataTransfer?.setData(
                              "application/x-roxy-signal",
                              signal.key,
                            );
                            if (event.dataTransfer) {
                              event.dataTransfer.effectAllowed = "copy";
                            }
                          }}
                        >
                          <span
                            class="dbc-signal-name"
                            title={
                              signal.comment
                                ? `${signal.name} — ${signal.comment}`
                                : signal.name
                            }
                          >
                            {signal.name}
                          </span>
                          <em>{signal.unit || "—"}</em>
                          <button
                            type="button"
                            class="mini"
                            disabled={onChart}
                            title={
                              onChart
                                ? "Already on the chart"
                                : "Add to the Signals chart"
                            }
                            onClick={() =>
                              onAddChart(channel, message, signal)
                            }
                          >
                            {onChart ? "Charted" : "+ Chart"}
                          </button>
                          <button
                            type="button"
                            class="mini"
                            disabled={onMonitor}
                            title={
                              onMonitor
                                ? "Already in Monitor"
                                : "Add to Monitor"
                            }
                            onClick={() =>
                              onAddMonitor(channel, message, signal)
                            }
                          >
                            {onMonitor ? "Watched" : "+ Mon"}
                          </button>
                        </div>
                      );
                    })}
                    </div>
                  );
                })}
              </section>
            ))}
        </div>
      </aside>
    </>
  );
}
