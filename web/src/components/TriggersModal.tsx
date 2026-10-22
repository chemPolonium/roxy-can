import { useEffect } from "preact/hooks";
import type { TriggerEntry } from "../types";

interface Props {
  triggers: TriggerEntry[];
  onAdd: (kind: string) => void;
  onToggle: (index: number, on: boolean) => void;
  onRemove: (index: number) => void;
  onRearm: () => void;
  onClose: () => void;
}

const KINDS: [string, string][] = [
  ["signal", "+ Signal"],
  ["id", "+ ID"],
  ["error", "+ Error frames"],
  ["timeout", "+ Timeout"],
  ["sysvar", "+ SysVar"],
  ["fr_frame", "+ FR frame"],
  ["fr_signal", "+ FR signal"],
];

function timeText(us: number): string {
  if (us === 0) return "—";
  return `${(us / 1e6).toFixed(3)} s`;
}

export function TriggersModal({
  triggers,
  onAdd,
  onToggle,
  onRemove,
  onRearm,
  onClose,
}: Props) {
  useEffect(() => {
    const close = (event: KeyboardEvent) => {
      if (event.key === "Escape") onClose();
    };
    window.addEventListener("keydown", close);
    return () => window.removeEventListener("keydown", close);
  }, [onClose]);

  return (
    <div class="modal-backdrop" onMouseDown={onClose}>
      <div
        class="trigger-modal"
        role="dialog"
        aria-modal="true"
        aria-label="Triggers"
        onMouseDown={(event) => event.stopPropagation()}
      >
        <header class="modal-head">
          <h2>Triggers</h2>
          <div class="picker-head-right">
            <span>
              {triggers.length} rules · {triggers.filter((t) => t.enabled).length}{" "}
              enabled
            </span>
            <button type="button" onClick={onRearm}>
              Re-arm latched
            </button>
            <button type="button" onClick={onClose}>
              Close
            </button>
          </div>
        </header>

        <div class="trig-actions">
          {KINDS.map(([kind, label]) => (
            <button
              type="button"
              key={kind}
              onClick={() => onAdd(kind)}
            >
              {label}
            </button>
          ))}
        </div>

        <div class="trig-body">
          {triggers.length === 0 && (
            <div class="empty">
              <strong>No trigger rules</strong>
              <span>Add a default rule above; editing its condition and action is next.</span>
            </div>
          )}
          {triggers.map((trigger, index) => (
            <div class={`trig-row ${trigger.enabled ? "" : "off"}`} key={index}>
              <label class="gen-active">
                <input
                  type="checkbox"
                  checked={trigger.enabled}
                  onChange={(event) =>
                    onToggle(
                      index,
                      (event.target as HTMLInputElement).checked,
                    )
                  }
                />
                On
              </label>
              <span class="trig-summary" title={trigger.summary}>
                {trigger.summary}
              </span>
              {trigger.level && (
                <span class="trig-level" title="Condition is currently true" />
              )}
              <span class="trig-action">{trigger.action}</span>
              <span class="trig-fired">{trigger.fired} fired</span>
              <span class="trig-last">{timeText(trigger.lastFireUs)}</span>
              <button
                type="button"
                class="bus-remove"
                onClick={() => onRemove(index)}
              >
                Remove
              </button>
            </div>
          ))}
        </div>

        <p class="hint picker-hint">
          Default rules are created against the live snapshot. Condition/action
          editing and pre/post-trigger limits are the next slice.
        </p>
      </div>
    </div>
  );
}
