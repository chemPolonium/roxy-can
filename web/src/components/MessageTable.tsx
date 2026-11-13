import { Fragment } from "preact";
import { useLayoutEffect, useRef, useState } from "preact/hooks";
import type { ComponentChildren } from "preact";
import type { Message } from "../types";

function hexId(message: Message): string {
  const width = message.extended ? 8 : 3;
  return `0x${message.id.toString(16).toUpperCase().padStart(width, "0")}`;
}

export function MessageTable({
  rows,
  filtered = false,
  onExpand,
  empty,
}: {
  rows: Message[];
  filtered?: boolean;
  onExpand?: (key: string, on: boolean) => void;
  empty?: ComponentChildren;
}) {
  const wrapRef = useRef<HTMLDivElement>(null);
  const [showBus, setShowBus] = useState(true);
  useLayoutEffect(() => {
    const element = wrapRef.current;
    if (!element) return;
    const update = () => setShowBus(element.clientWidth >= 480);
    update();
    const observer = new ResizeObserver(update);
    observer.observe(element);
    return () => observer.disconnect();
  }, [rows.length]);
  if (rows.length === 0) {
    if (empty && !filtered) return <>{empty}</>;
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
    onExpand?.(message.key, (message.signals?.length ?? 0) === 0);
  };

  return (
    <div class="table-scroll" ref={wrapRef}>
      <table class="data-table message-table">
        <colgroup>
          {showBus && <col style={{ width: "46px" }} />}
          <col style={{ width: "70px" }} />
          <col />
          <col style={{ width: "80px" }} />
          <col />
        </colgroup>
        <thead>
          <tr>
            {showBus && <th>Bus</th>}
            <th>ID</th>
            <th>Name</th>
            <th class="num">Cycle</th>
            <th class="num" title="Raw payload on a message row; decoded values on its signal rows">Data</th>
          </tr>
        </thead>
        <tbody>
          {rows.map((message) => {
            const open = (message.signals?.length ?? 0) > 0;
            const expandable = message.signalCount > 0;
            return (
              <Fragment key={message.key}>
                <tr
                  class={`message-row ${expandable ? "expandable" : ""}`}
                  onClick={() => toggle(message)}
                >
                  {showBus && <td>{message.busName}</td>}
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
                          message.e2e.repeats + message.e2e.jumps > 0 ||
                          message.e2e.crcFail > 0
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
                          message.e2e.crcChecked > 0
                            ? `CRC ${message.e2e.crcOk ? "OK" : "FAIL"} · ${message.e2e.crcFail}/${message.e2e.crcChecked} failed`
                            : "CRC position/profile not configured",
                          "Counter continuity and CRC check; authentication keys are not used.",
                        ].join("\n")}
                      >
                        E2E P{message.e2e.profile}
                        {message.e2e.crcChecked > 0 &&
                          (message.e2e.crcOk ? " ✓" : " ✗")}
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
                        <td colSpan={showBus ? 5 : 4} class="message-signal-empty">
                          Loading signals…
                        </td>
                      </tr>
                    ) : (
                      message.signals?.map((signal, index) => (
                        <tr
                          class="message-signal-row"
                          key={`${message.key}:${signal.name}:${index}`}
                        >
                          {showBus && <td />}
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
