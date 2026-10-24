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
    return catalog.channels
      .map((channel) => {
        const channelHit =
          needle.length > 0 && channel.name.toLowerCase().includes(needle);
        const messages = channel.messages
          .map((message) => {
            const messageHit =
              channelHit ||
              message.name.toLowerCase().includes(needle) ||
              messageId(channel, message).toLowerCase().includes(needle);
            const signals = messageHit
              ? message.signals
              : message.signals.filter(
                  (signal) =>
                    signal.name.toLowerCase().includes(needle) ||
                    signal.unit.toLowerCase().includes(needle),
                );
            return { message, signals };
          })
          .filter((entry) => entry.signals.length > 0);
        return { channel, messages };
      })
      .filter((group) => group.messages.length > 0);
  }, [catalog, query]);

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
              <strong>{query ? "No matching signals" : "No signals available"}</strong>
              <span>
                {query
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
                {messages.map(({ message, signals }) => (
                  <div
                    class="dbc-message"
                    key={`${channel.kind}:${channel.index}:${message.id}`}
                  >
                    <div class="dbc-message-head">
                      <span>{message.name}</span>
                      <em>{messageId(channel, message)}</em>
                    </div>
                    {signals.map((signal) => {
                      const onChart = charted.has(signal.key);
                      const onMonitor = watched.has(signal.key);
                      return (
                        <div class="dbc-signal" key={signal.key}>
                          <span class="dbc-signal-name" title={signal.name}>
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
                ))}
              </section>
            ))}
        </div>
      </aside>
    </>
  );
}
