import { useEffect, useMemo, useRef, useState } from "preact/hooks";
import { ModalHeader } from "./ModalHeader";
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
  /// Batch frame actions. Without it each target falls back to onToggle.
  onToggleMany?: (
    channel: CatalogChannel,
    message: CatalogMessage,
    signals: CatalogSignal[],
    on: boolean,
  ) => void | Promise<void>;
  /// Ask before a frame's All would add more than this many chart lanes.
  chartBatchLimit?: number;
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
  /// Label for pickers that only add/remove a computed channel (Monitor /
  /// Data / State) instead of Plot/Read.
  derivedToggleLabel?: string;
  onClose: () => void;
}

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
  onToggleMany,
  chartBatchLimit,
  onReadToggle,
  onCopyTo,
  derived = [],
  onManageDerived,
  onDerivedToggle,
  onDerivedReadToggle,
  derivedToggleLabel = "Add",
  onClose,
}: Props) {
  const [query, setQuery] = useState("");
  const [expanded, setExpanded] = useState<Set<string>>(new Set());
  const [collapsedSearch, setCollapsedSearch] = useState<Set<string>>(
    new Set(),
  );
  const [selectedOnly, setSelectedOnly] = useState(false);
  const [pendingBulk, setPendingBulk] = useState<{
    channel: CatalogChannel;
    message: CatalogMessage;
    signals: CatalogSignal[];
    on: boolean;
  } | null>(null);
  const bulkConfirm = useRef<HTMLButtonElement>(null);

  useEffect(() => {
    if (pendingBulk) bulkConfirm.current?.focus();
  }, [pendingBulk]);

  useEffect(() => {
    // A new search starts with every matching message open; clearing it
    // returns to the frames the user had explicitly expanded.
    setCollapsedSearch(new Set());
  }, [query]);

  useEffect(() => {
    const close = (event: KeyboardEvent) => {
      if (event.key !== "Escape") return;
      if (pendingBulk) {
        setPendingBulk(null);
        return;
      }
      onClose();
    };
    window.addEventListener("keydown", close);
    return () => window.removeEventListener("keydown", close);
  }, [onClose, pendingBulk]);

  const groups = useMemo(() => {
    if (!catalog) return [];
    const needle = query.trim().toLowerCase();
    const active = (key: string) => selected.has(key) || !!read?.has(key);
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
            let signals = messageHit
              ? message.signals
              : message.signals.filter(
                  (signal) =>
                    signal.name.toLowerCase().includes(needle) ||
                    signal.unit.toLowerCase().includes(needle) ||
                    (signal.comment ?? "").toLowerCase().includes(needle),
                );
            if (selectedOnly) {
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
  }, [catalog, query, kinds, selectedOnly, selected, read]);

  const selectedEntries = useMemo(() => {
    if (!catalog) return [];
    const out: {
      channel: CatalogChannel;
      message: CatalogMessage;
      signal: CatalogSignal;
      readOnly: boolean;
    }[] = [];
    for (const channel of catalog.channels) {
      for (const message of channel.messages) {
        for (const signal of message.signals) {
          const plotted = selected.has(signal.key);
          const reading = !!read?.has(signal.key);
          if (plotted || reading) {
            out.push({
              channel,
              message,
              signal,
              readOnly: reading && !plotted,
            });
          }
        }
      }
    }
    return out;
  }, [catalog, selected, read]);

  const selectedDerived = useMemo(
    () =>
      derived.filter(
        (definition) =>
          selected.has(`derived:${definition.id}`) ||
          !!read?.has(`derived:${definition.id}`),
      ),
    [derived, selected, read],
  );

  const computed = useMemo(() => {
    const needle = query.trim().toLowerCase();
    return derived.filter(
      (definition) =>
        !needle ||
        definition.name.toLowerCase().includes(needle) ||
        definition.expression.toLowerCase().includes(needle),
    );
  }, [derived, query]);

  const visibleComputed = selectedOnly
    ? computed.filter(
        (definition) =>
          selected.has(`derived:${definition.id}`) ||
          !!read?.has(`derived:${definition.id}`),
      )
    : computed;

  const selectedCount = selectedEntries.length + selectedDerived.length;
  const querying = query.trim().length > 0;
  const visibleSelected = selectedEntries.slice(0, 16);
  const visibleDerived = selectedDerived.slice(0, 8);
  const hiddenSelected =
    selectedEntries.length -
    visibleSelected.length +
    (selectedDerived.length - visibleDerived.length);

  const messageOpen = (key: string) =>
    selectedOnly ||
    (querying ? !collapsedSearch.has(key) : expanded.has(key));

  const toggleMessage = (key: string) => {
    if (querying) {
      setCollapsedSearch((old) => toggleSet(old, key));
    } else {
      setExpanded((old) => toggleSet(old, key));
    }
  };

  const removeSelected = (entry: (typeof selectedEntries)[number]) => {
    if (selected.has(entry.signal.key)) {
      onToggle(entry.channel, entry.message, entry.signal, false);
    } else if (entry.readOnly) {
      onReadToggle?.(entry.channel, entry.message, entry.signal, false);
    }
  };

  const removeDerived = (definition: DerivedDef) => {
    const key = `derived:${definition.id}`;
    if (selected.has(key)) {
      onDerivedToggle?.(definition, false);
    } else if (read?.has(key)) {
      onDerivedReadToggle?.(definition, false);
    }
  };

  const commitBulk = (
    channel: CatalogChannel,
    message: CatalogMessage,
    signals: CatalogSignal[],
    on: boolean,
  ) => {
    if (signals.length === 0) return;
    if (onToggleMany) {
      void onToggleMany(channel, message, signals, on);
      return;
    }
    for (const signal of signals) {
      if (on) {
        onToggle(channel, message, signal, true);
      } else if (selected.has(signal.key)) {
        onToggle(channel, message, signal, false);
      } else if (read?.has(signal.key)) {
        onReadToggle?.(channel, message, signal, false);
      }
    }
  };

  const requestBulk = (
    channel: CatalogChannel,
    message: CatalogMessage,
    signals: CatalogSignal[],
    on: boolean,
  ) => {
    setPendingBulk(null);
    if (signals.length === 0) return;
    if (on && chartBatchLimit != null && signals.length > chartBatchLimit) {
      setPendingBulk({ channel, message, signals, on });
      return;
    }
    commitBulk(channel, message, signals, on);
  };

  return (
    <div class="modal-backdrop" onMouseDown={onClose}>
      <div
        class="picker-modal"
        role="dialog"
        aria-modal="true"
        aria-label="Signal selection"
        onMouseDown={(event) => event.stopPropagation()}
      >
        <ModalHeader title={title} onClose={onClose}>
          <span>
            {read
              ? `${selected.size} plotted · ${read.size} read`
              : `${selected.size} selected`}
          </span>
        </ModalHeader>
        <div class="picker-tools">
          <input
            type="search"
            autoFocus
            value={query}
            placeholder="Filter bus, message or signal…"
            onInput={(event) => {
              setQuery((event.target as HTMLInputElement).value);
              setPendingBulk(null);
            }}
          />
          <button
            type="button"
            class={`mini ${selectedOnly ? "on" : ""}`}
            aria-pressed={selectedOnly}
            title="Show only plotted or read signals"
            onClick={() => {
              setSelectedOnly((old) => !old);
              setPendingBulk(null);
            }}
          >
            Selected only
          </button>
        </div>
        {pendingBulk && (
          <div class="picker-batch" role="alert">
            <span>
              Add <strong>{pendingBulk.signals.length}</strong>{" "}
              {pendingBulk.signals.length === 1 ? "signal" : "signals"} from{" "}
              <strong>
                {pendingBulk.message.name ||
                  messageId(pendingBulk.channel, pendingBulk.message)}
              </strong>{" "}
              to the chart? Each one becomes a lane.
            </span>
            <button type="button" onClick={() => setPendingBulk(null)}>
              Cancel
            </button>
            <button
              ref={bulkConfirm}
              type="button"
              class="primary"
              onClick={() => {
                const bulk = pendingBulk;
                setPendingBulk(null);
                commitBulk(
                  bulk.channel,
                  bulk.message,
                  bulk.signals,
                  bulk.on,
                );
              }}
            >
              Add {pendingBulk.signals.length}
            </button>
          </div>
        )}
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
          {!loading && !error && groups.length === 0 && visibleComputed.length === 0 && selectedCount === 0 && (
            <div class="empty">
              <strong>
                {selectedOnly
                  ? "No selected signals"
                  : query
                    ? "No matching signals"
                    : "No signals available"}
              </strong>
              <span>
                {selectedOnly
                  ? "Select a signal from a frame first, then turn on Selected only."
                  : query
                    ? "Try a different bus, message or signal name."
                    : "Load a description file into the project first."}
              </span>
            </div>
          )}
          {!loading && !error && selectedCount > 0 && (
            <section class="picker-selected">
              <div class="picker-selected-head">
                <h4>Selected</h4>
                <span>{selectedCount}</span>
              </div>
              <div class="picker-selected-list">
                {visibleSelected.map((entry) => (
                  <button
                    key={entry.signal.key}
                    type="button"
                    class="picker-chip"
                    title={`${entry.message.name} / ${entry.signal.name}`}
                    onClick={() => removeSelected(entry)}
                  >
                    <span>{entry.signal.name}</span>
                    <small>{entry.message.name}</small>
                    {entry.readOnly && <small>read</small>}
                    <b>×</b>
                  </button>
                ))}
                {visibleDerived.map((definition) => (
                  <button
                    key={definition.id}
                    type="button"
                    class="picker-chip"
                    title={definition.expression}
                    onClick={() => removeDerived(definition)}
                  >
                    <span>{definition.name}</span>
                    <small>computed</small>
                    <b>×</b>
                  </button>
                ))}
                {hiddenSelected > 0 && (
                  <button
                    type="button"
                    class="picker-chip more"
                    onClick={() => setSelectedOnly(true)}
                  >
                    +{hiddenSelected} more
                  </button>
                )}
              </div>
            </section>
          )}
          {!loading &&
            !error &&
            groups.map(({ channel, messages }) => (
              <section
                class="picker-channel"
                key={`${channel.kind}:${channel.index}`}
              >
                <h4>{channel.name}</h4>
                {messages.map(({ message, signals, key, total, active }) => {
                  const open = messageOpen(key);
                  const addTargets = signals.filter(
                    (signal) => !selected.has(signal.key),
                  );
                  const activeSignals = signals.filter(
                    (signal) =>
                      selected.has(signal.key) || !!read?.has(signal.key),
                  );
                  return (
                    <div class="picker-message" key={key}>
                      <div class="picker-message-head">
                        <button
                          type="button"
                          class="picker-message-toggle"
                          aria-expanded={open}
                          onClick={() => toggleMessage(key)}
                        >
                          <span class="picker-message-title">
                            <span class="picker-chevron">
                              {open ? "▾" : "▸"}
                            </span>
                            {message.name}
                          </span>
                          <span class="picker-message-meta">
                            <em>{messageId(channel, message)}</em>
                            <small>
                              {signals.length}
                              {querying || selectedOnly
                                ? ` / ${total}`
                                : ""}{" "}
                              signals
                            </small>
                            {active > 0 && <small>{active} selected</small>}
                          </span>
                        </button>
                        {open && (
                          <span class="picker-message-actions">
                            <button
                              type="button"
                              class="mini picker-frame-action"
                              disabled={addTargets.length === 0}
                              title={`Add all ${addTargets.length} visible signals from this frame`}
                              onClick={() =>
                                requestBulk(
                                  channel,
                                  message,
                                  addTargets,
                                  true,
                                )
                              }
                            >
                              All
                            </button>
                            <button
                              type="button"
                              class="mini picker-frame-action"
                              disabled={activeSignals.length === 0}
                              title={`Remove all ${activeSignals.length} active signals from this frame`}
                              onClick={() =>
                                requestBulk(
                                  channel,
                                  message,
                                  activeSignals,
                                  false,
                                )
                              }
                            >
                              None
                            </button>
                          </span>
                        )}
                      </div>
                      {open &&
                        signals.map((signal) => {
                      const on = selected.has(signal.key);
                      const reading = !!read?.has(signal.key);
                      return (
                        <div
                          class={`picker-signal ${on ? "on" : ""} ${
                            reading ? "read" : ""
                          } ${onReadToggle ? "with-modes" : ""}`}
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
                          {signal.group && (
                            <button
                              type="button"
                              class="picker-group"
                              title={`Signal group "${signal.group}": click to select every member`}
                              onClick={(event) => {
                                event.stopPropagation();
                                for (const member of signals) {
                                  if (
                                    member.group === signal.group &&
                                    !selected.has(member.key)
                                  ) {
                                    onToggle(channel, message, member, true);
                                  }
                                }
                              }}
                            >
                              {signal.group}
                            </button>
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
                  );
                })}
              </section>
            ))}
          {!loading && !error && visibleComputed.length > 0 && (
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
              {visibleComputed.map((definition) => {
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
                    ) : onDerivedToggle ? (
                      <button
                        type="button"
                        class={`mini ${on ? "on" : ""}`}
                        aria-pressed={on}
                        onClick={() => onDerivedToggle(definition, !on)}
                      >
                        {on ? "Remove" : derivedToggleLabel}
                      </button>
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
