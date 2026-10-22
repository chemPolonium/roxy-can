import { useEffect, useMemo, useState } from "preact/hooks";
import type { Catalog, CatalogMessage, CatalogSignal } from "../types";

interface Props {
  catalog: Catalog | null;
  loading: boolean;
  error: string;
  selected: Set<string>;
  title?: string;
  onToggle: (
    ch: number,
    message: CatalogMessage,
    signal: CatalogSignal,
    on: boolean,
  ) => void;
  onClose: () => void;
}

function hexId(message: CatalogMessage): string {
  const width = message.extended ? 8 : 3;
  return `0x${message.id.toString(16).toUpperCase().padStart(width, "0")}`;
}

export function SignalPicker({
  catalog,
  loading,
  error,
  selected,
  title = "Signals",
  onToggle,
  onClose,
}: Props) {
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
      .map((channel, ch) => {
        const channelHit =
          needle.length > 0 && channel.name.toLowerCase().includes(needle);
        const messages = channel.messages
          .map((message) => {
            const messageHit =
              channelHit ||
              message.name.toLowerCase().includes(needle) ||
              hexId(message).toLowerCase().includes(needle);
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
        return { ch, name: channel.name, messages };
      })
      .filter((channel) => channel.messages.length > 0);
  }, [catalog, query]);

  return (
    <div class="modal-backdrop" onMouseDown={onClose}>
      <div
        class="picker-modal"
        role="dialog"
        aria-modal="true"
        aria-label="Signal selection"
        onMouseDown={(event) => event.stopPropagation()}
      >
        <header class="modal-head">
          <h2>{title}</h2>
          <div class="picker-head-right">
            <span>{selected.size} selected</span>
            <button type="button" onClick={onClose}>
              Close
            </button>
          </div>
        </header>
        <div class="picker-tools">
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
        <div class="picker-body">
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
                  : "Load a DBC into the project first."}
              </span>
            </div>
          )}
          {!loading &&
            !error &&
            groups.map((group) => (
              <section class="picker-channel" key={group.ch}>
                <h4>{group.name}</h4>
                {group.messages.map(({ message, signals }) => (
                  <div
                    class="picker-message"
                    key={`${group.ch}:${message.id}:${message.extended}`}
                  >
                    <div class="picker-message-head">
                      <span>{message.name}</span>
                      <em>{hexId(message)}</em>
                    </div>
                    {signals.map((signal) => {
                      const on = selected.has(signal.key);
                      return (
                        <label
                          class={`picker-signal ${on ? "on" : ""}`}
                          key={signal.key}
                        >
                          <input
                            type="checkbox"
                            checked={on}
                            onChange={() =>
                              onToggle(group.ch, message, signal, !on)
                            }
                          />
                          <span>{signal.name}</span>
                          <em>
                            {signal.unit || "no unit"} · {signal.min}..{signal.max}
                          </em>
                        </label>
                      );
                    })}
                  </div>
                ))}
              </section>
            ))}
        </div>
        <p class="hint picker-hint">
          Selection is session-only for now. Unselecting removes the bus
          subscription and its curve.
        </p>
      </div>
    </div>
  );
}
