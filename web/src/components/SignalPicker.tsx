import { useEffect, useMemo, useState } from "preact/hooks";
import type {
  Catalog,
  CatalogChannel,
  CatalogMessage,
  CatalogSignal,
} from "../types";
import type { DerivedDef } from "../derived";

interface Props {
  catalog: Catalog | null;
  loading: boolean;
  error: string;
  selected: Set<string>;
  watched?: Set<string>;
  /// Read-only signal set: sampled for cursor values, never drawn.
  read?: Set<string>;
  title?: string;
  kinds?: string[];
  onToggle: (
    channel: CatalogChannel,
    message: CatalogMessage,
    signal: CatalogSignal,
    on: boolean,
  ) => void;
  onReadToggle?: (
    channel: CatalogChannel,
    message: CatalogMessage,
    signal: CatalogSignal,
    on: boolean,
  ) => void;
  onCopyTo?: (target: "monitor" | "data" | "state") => void;
  derived?: DerivedDef[];
  onManageDerived?: () => void;
  onDerivedToggle?: (definition: DerivedDef, on: boolean) => void;
  onDerivedReadToggle?: (definition: DerivedDef, on: boolean) => void;
  onClose: () => void;
}

function messageId(channel: CatalogChannel, message: CatalogMessage): string {
  if (channel.kind === "fr") return `Slot ${message.id}`;
  if (channel.kind === "eth") return "frame";
  const width = message.extended ? 8 : 3;
  return `0x${message.id.toString(16).toUpperCase().padStart(width, "0")}`;
}

export function SignalPicker({
  catalog,
  loading,
  error,
  selected,
  watched,
  read,
  title = "Signals",
  kinds,
  onToggle,
  onReadToggle,
  onCopyTo,
  derived = [],
  onManageDerived,
  onDerivedToggle,
  onDerivedReadToggle,
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
      .filter((channel) => !kinds || kinds.includes(channel.kind))
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
  }, [catalog, query, kinds]);

  const computed = useMemo(() => {
    const needle = query.trim().toLowerCase();
    return derived.filter(
      (definition) =>
        !needle ||
        definition.name.toLowerCase().includes(needle) ||
        definition.expression.toLowerCase().includes(needle),
    );
  }, [derived, query]);

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
            <span>
              {read
                ? `${selected.size} plotted · ${read.size} read`
                : `${selected.size} selected`}
            </span>
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
          {!loading && !error && groups.length === 0 && computed.length === 0 && (
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
                class="picker-channel"
                key={`${channel.kind}:${channel.index}`}
              >
                <h4>{channel.name}</h4>
                {messages.map(({ message, signals }) => (
                  <div
                    class="picker-message"
                    key={`${channel.kind}:${channel.index}:${message.id}:${message.extended}`}
                  >
                    <div class="picker-message-head">
                      <span>{message.name}</span>
                      <em>{messageId(channel, message)}</em>
                    </div>
                    {signals.map((signal) => {
                      const on = selected.has(signal.key);
                      const reading = !!read?.has(signal.key);
                      return (
                        <div
                          class={`picker-signal ${on ? "on" : ""} ${
                            reading ? "read" : ""
                          }`}
                          key={signal.key}
                        >
                          {onReadToggle ? (
                            <button
                              type="button"
                              class="picker-name"
                              title="Click to plot · Ctrl-click to toggle Read"
                              onClick={(event) => {
                                if (event.ctrlKey || event.metaKey) {
                                  if (!on) {
                                    onReadToggle(
                                      channel,
                                      message,
                                      signal,
                                      !reading,
                                    );
                                  }
                                  return;
                                }
                                onToggle(channel, message, signal, !on);
                              }}
                            >
                              <span>{signal.name}</span>
                            </button>
                          ) : (
                            <label class="picker-plot">
                              <input
                                type="checkbox"
                                checked={on}
                                onChange={() =>
                                  onToggle(channel, message, signal, !on)
                                }
                              />
                              <span>{signal.name}</span>
                            </label>
                          )}
                          {watched?.has(signal.key) && (
                            <small class="picker-mark">mon</small>
                          )}
                          <em>
                            {signal.unit || "no unit"} · {signal.min}..{signal.max}
                          </em>
                          {onReadToggle && (
                            <div
                              class="picker-modes"
                              role="group"
                              aria-label="Signal mode"
                            >
                              <button
                                type="button"
                                class={`picker-mode ${on ? "on" : ""}`}
                                aria-pressed={on}
                                title={on ? "Stop plotting" : "Plot this signal"}
                                onClick={() =>
                                  onToggle(channel, message, signal, !on)
                                }
                              >
                                Plot
                              </button>
                              <button
                                type="button"
                                class={`picker-mode ${reading ? "on" : ""}`}
                                aria-pressed={reading}
                                title={
                                  reading
                                    ? "Remove from the cursor readout"
                                    : on
                                      ? "Switch to Read-only (stop plotting)"
                                      : "Read at cursors without plotting"
                                }
                                onClick={() =>
                                  onReadToggle(
                                    channel,
                                    message,
                                    signal,
                                    !reading,
                                  )
                                }
                              >
                                Read
                              </button>
                            </div>
                          )}
                        </div>
                      );
                    })}
                  </div>
                ))}
              </section>
            ))}
          {!loading && !error && computed.length > 0 && (
            <section class="picker-channel picker-computed">
              <h4>
                Computed
                {onManageDerived && (
                  <button
                    type="button"
                    class="mini"
                    onClick={onManageDerived}
                  >
                    Manage…
                  </button>
                )}
              </h4>
              {computed.map((definition) => {
                const key = `derived:${definition.id}`;
                const on = selected.has(key);
                const reading = !!read?.has(key);
                return (
                  <div
                    class={`picker-derived-row ${on ? "on" : ""} ${
                      reading ? "read" : ""
                    }`}
                    key={definition.id}
                  >
                    <span>{definition.name}</span>
                    <code>{definition.expression}</code>
                    {onDerivedToggle && onDerivedReadToggle ? (
                      <div
                        class="picker-modes"
                        role="group"
                        aria-label="Computed channel mode"
                      >
                        <button
                          type="button"
                          class={`picker-mode ${on ? "on" : ""}`}
                          aria-pressed={on}
                          title={on ? "Stop plotting" : "Plot this channel"}
                          onClick={() => onDerivedToggle(definition, !on)}
                        >
                          Plot
                        </button>
                        <button
                          type="button"
                          class={`picker-mode ${reading ? "on" : ""}`}
                          aria-pressed={reading}
                          title={
                            reading
                              ? "Remove from the cursor readout"
                              : on
                                ? "Switch to Read-only (stop plotting)"
                                : "Read at cursors without plotting"
                          }
                          onClick={() =>
                            onDerivedReadToggle(definition, !reading)
                          }
                        >
                          Read
                        </button>
                      </div>
                    ) : (
                      <small>computed</small>
                    )}
                  </div>
                );
              })}
            </section>
          )}
        </div>
        {onCopyTo && (
          <div class="picker-copy">
            <span>Copy this selection to</span>
            <button
              type="button"
              disabled={selected.size === 0}
              onClick={() => onCopyTo("monitor")}
            >
              Monitor
            </button>
            <button
              type="button"
              disabled={selected.size === 0}
              onClick={() => onCopyTo("data")}
            >
              Data
            </button>
            <button
              type="button"
              disabled={selected.size === 0}
              onClick={() => onCopyTo("state")}
            >
              State
            </button>
          </div>
        )}
        <p class="hint picker-hint">
          {read
            ? "Plot / Read sets each signal's role; click the active one to clear. Plotted signals also appear at cursors; Ctrl-click a name to toggle Read."
            : "Selection is session-only for now. Unselecting removes the bus subscription and its curve."}
        </p>
      </div>
    </div>
  );
}
