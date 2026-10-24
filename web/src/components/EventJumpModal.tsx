import { useEffect, useMemo, useState } from "preact/hooks";
import type { SignalSeries } from "../types";
import { conditionEvents } from "../derived";

export function EventJumpModal({
  series,
  nowS,
  onJump,
  onClose,
}: {
  series: SignalSeries[];
  nowS: number;
  onJump: (t: number) => void;
  onClose: () => void;
}) {
  const [expression, setExpression] = useState("");

  useEffect(() => {
    const escape = (event: KeyboardEvent) => {
      if (event.key === "Escape") onClose();
    };
    window.addEventListener("keydown", escape);
    return () => window.removeEventListener("keydown", escape);
  }, [onClose]);

  const scan = useMemo(
    () =>
      expression.trim()
        ? conditionEvents(expression, series)
        : { times: [] as number[], error: null },
    [expression, series],
  );
  const next = scan.times.find((time) => time > nowS + 1e-6);
  const previous = [...scan.times].reverse().find((time) => time < nowS - 1e-6);
  const names = [...new Set(series.map((signal) => signal.name))].slice(0, 60);

  const insert = (name: string) => {
    setExpression((old) => `${old}${old.trim() ? " " : ""}[${name}]`);
  };

  return (
    <div class="modal-backdrop" onMouseDown={onClose}>
      <div
        class="event-modal"
        role="dialog"
        aria-modal="true"
        aria-label="Signal events"
        onMouseDown={(event) => event.stopPropagation()}
      >
        <header class="modal-head">
          <h2>Signal events</h2>
          <button type="button" onClick={onClose}>
            Close
          </button>
        </header>
        <div class="event-body">
          <p class="hint">
            Find rising edges of a condition, for example{" "}
            <code>[VehicleSpeed] &gt; 30</code>. The same grammar as derived
            channels is used, including {"[CAN1:signal]"}.
          </p>
          <input
            type="text"
            autoFocus
            value={expression}
            placeholder="[VehicleSpeed] > 30"
            spellcheck={false}
            onInput={(event) =>
              setExpression((event.target as HTMLInputElement).value)
            }
          />
          <div class="event-picks">
            {names.map((name) => (
              <button
                type="button"
                class="mini"
                key={name}
                onClick={() => insert(name)}
              >
                {name}
              </button>
            ))}
          </div>
          {scan.error ? (
            <p class="modal-error">{scan.error}</p>
          ) : (
            expression.trim() && (
              <p class="event-summary">
                {scan.times.length} rising edge
                {scan.times.length === 1 ? "" : "s"} in the current signal
                history
              </p>
            )
          )}
          <div class="event-actions">
            <button
              type="button"
              disabled={!previous}
              title={
                previous
                  ? `Jump to ${previous.toFixed(3)} s`
                  : "No earlier edge in the signal history"
              }
              onClick={() => previous && onJump(previous)}
            >
              ← Previous
            </button>
            <button
              type="button"
              class="primary"
              disabled={!next}
              title={
                next
                  ? `Jump to ${next.toFixed(3)} s`
                  : "No later edge in the signal history"
              }
              onClick={() => next && onJump(next)}
            >
              Next →
            </button>
            <span class="spacer" />
            <button type="button" onClick={onClose}>
              Close
            </button>
          </div>
        </div>
      </div>
    </div>
  );
}
