import { Fragment } from "preact";
import { useState } from "preact/hooks";
import type { Message } from "../types";

function hexId(message: Message): string {
  const width = message.extended ? 8 : 3;
  return `0x${message.id.toString(16).toUpperCase().padStart(width, "0")}`;
}

export function MessageTable({
  rows,
  filtered = false,
  onExpand,
}: {
  rows: Message[];
  filtered?: boolean;
  onExpand?: (key: string, on: boolean) => void;
}) {
  const [expanded, setExpanded] = useState<Set<string>>(new Set());
  if (rows.length === 0) {
    return (
      <div class="empty">
        <strong>{filtered ? "No matching messages" : "No messages seen"}</strong>
        <span>
          {filtered
            ? "Try a different bus, name or id."
            : "Traffic will appear here once the bus runs."}
        </span>
      </div>
    );
  }

  const toggle = (message: Message) => {
    if (message.signalCount === 0) return;
    const open = !expanded.has(message.key);
    setExpanded((old) => {
      const next = new Set(old);
      if (open) next.add(message.key);
      else next.delete(message.key);
      return next;
    });
    onExpand?.(message.key, open);
  };

  return (
    <div class="table-scroll">
      <table class="data-table message-table">
        <colgroup>
          <col style={{ width: "46px" }} />
          <col style={{ width: "70px" }} />
          <col />
          <col style={{ width: "80px" }} />
          <col />
        </colgroup>
        <thead>
          <tr>
            <th>Bus</th>
            <th>ID</th>
            <th>Name</th>
            <th class="num">Cycle</th>
            <th class="num" title="Raw payload on a message row; decoded values on its signal rows">Data</th>
          </tr>
        </thead>
        <tbody>
          {rows.map((message) => {
            const open = expanded.has(message.key);
            const expandable = message.signalCount > 0;
            return (
              <Fragment key={message.key}>
                <tr
                  class={`message-row ${expandable ? "expandable" : ""}`}
                  onClick={() => toggle(message)}
                >
                  <td>{message.busName}</td>
                  <td class="mono">{message.addr || hexId(message)}</td>
                  <td class="name" title={message.name}>
                    {expandable ? (
                      <button
                        type="button"
                        class="msg-expand"
                        aria-expanded={open}
                        title={
                          open ? "Hide decoded signals" : "Show decoded signals"
                        }
                        onClick={(event) => {
                          event.stopPropagation();
                          toggle(message);
                        }}
                      >
                        {open ? "▾" : "▸"}
                      </button>
                    ) : (
                      <span class="msg-expand-space" />
                    )}
                    <span class="msg-name">
                      {message.name || <span class="dash">—</span>}
                    </span>
                    {message.e2e && (
                      <span
                        class={`e2e-tag ${
                          message.e2e.repeats + message.e2e.jumps > 0
                            ? "bad"
                            : ""
                        }`}
                        title={[
                          `E2E Profile ${message.e2e.profile}${
                            message.e2e.dataId != null
                              ? `, Data ID ${message.e2e.dataId}`
                              : ""
                          }`,
                          `Counter ${message.e2e.counter ?? "—"} · repeats ${message.e2e.repeats} · gaps ${message.e2e.gaps} · jumps ${message.e2e.jumps}`,
                          "Counter continuity only; authentication stays unverified (no CRC key).",
                        ].join("\n")}
                      >
                        E2E P{message.e2e.profile}
                      </span>
                    )}
                  </td>
                  <td class="num mono">
                    {message.cycleMs > 0
                      ? `${message.cycleMs.toFixed(1)} ms`
                      : "event"}
                  </td>
                  <td class="num mono data" title={message.data}>
                    {message.data || <span class="dash">—</span>}
                  </td>
                </tr>
                {open && message.signalCount > 0 && (
                  <>
                    {(message.signals?.length ?? 0) === 0 ? (
                      <tr class="message-signal-row" key={`${message.key}:loading`}>
                        <td colSpan={5} class="message-signal-empty">
                          Loading signals…
                        </td>
                      </tr>
                    ) : (
                      message.signals?.map((signal, index) => (
                        <tr
                          class="message-signal-row"
                          key={`${message.key}:${signal.name}:${index}`}
                        >
                          <td />
                          <td />
                          <td class="name message-signal-name">
                            <span>↳ {signal.name}</span>
                          </td>
                          <td
                            class="num mono message-signal-value"
                            title={`raw ${signal.raw}`}
                          >
                            {signal.value}
                          </td>
                          <td class="mono data message-signal-unit">
                            {signal.unit && <em>{signal.unit}</em>}
                            {signal.label && <small>{signal.label}</small>}
                          </td>
                        </tr>
                      ))
                    )}
                  </>
                )}
              </Fragment>
            );
          })}
        </tbody>
      </table>
    </div>
  );
}
