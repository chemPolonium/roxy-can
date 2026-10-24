import { useEffect, useState } from "preact/hooks";
import type { SignalSeries } from "../types";
import {
  validateDerivedExpression,
  type DerivedDef,
} from "../derived";

export function DerivedModal({
  defs,
  series,
  onSave,
  onRemove,
  onRule,
  onClose,
}: {
  defs: DerivedDef[];
  series: SignalSeries[];
  onSave: (definition: DerivedDef) => void;
  onRemove: (id: string) => void;
  onRule?: (definition: DerivedDef) => void;
  onClose: () => void;
}) {
  const [name, setName] = useState("");
  const [unit, setUnit] = useState("");
  const [expression, setExpression] = useState("");
  const error =
    expression.trim() === ""
      ? null
      : validateDerivedExpression(expression, series);
  const names = [...new Set(series.map((signal) => signal.name))].slice(0, 80);

  useEffect(() => {
    const escape = (event: KeyboardEvent) => {
      if (event.key === "Escape") onClose();
    };
    window.addEventListener("keydown", escape);
    return () => window.removeEventListener("keydown", escape);
  }, [onClose]);

  const insert = (signalName: string) => {
    setExpression((old) => `${old}${old.trim() ? " " : ""}[${signalName}]`);
  };

  const save = () => {
    if (!name.trim() || !expression.trim() || error) return;
    onSave({
      id: `${Date.now().toString(36)}${Math.random().toString(36).slice(2, 6)}`,
      name: name.trim(),
      unit: unit.trim(),
      expression: expression.trim(),
      cuts: [],
      names: [],
    });
    setName("");
    setUnit("");
    setExpression("");
  };

  return (
    <div class="modal-backdrop" onMouseDown={onClose}>
      <div
        class="derived-modal"
        role="dialog"
        aria-modal="true"
        aria-label="Derived signals"
        onMouseDown={(event) => event.stopPropagation()}
      >
        <header class="modal-head">
          <h2>Derived signals</h2>
          <button type="button" onClick={onClose}>
            Close
          </button>
        </header>
        <div class="derived-body">
          {defs.length > 0 && (
            <div class="derived-list">
              {defs.map((definition) => (
                <div class="derived-item" key={definition.id}>
                  <span class="derived-item-name">
                    {definition.name}
                    {definition.unit && <em>{definition.unit}</em>}
                  </span>
                  <code>{definition.expression}</code>
                  {onRule && (
                    <button
                      type="button"
                      class="mini"
                      title="Edit the State threshold bands"
                      onClick={() => onRule(definition)}
                    >
                      {definition.cuts.length > 0 ? "Bands…" : "Add bands…"}
                    </button>
                  )}
                  <button
                    type="button"
                    class="mini"
                    onClick={() => onRemove(definition.id)}
                  >
                    Remove
                  </button>
                </div>
              ))}
            </div>
          )}
          <div class="derived-form">
            <label>
              Name
              <input
                value={name}
                placeholder="Speed error"
                onInput={(event) =>
                  setName((event.target as HTMLInputElement).value)
                }
              />
            </label>
            <label>
              Unit
              <input
                value={unit}
                placeholder="km/h"
                onInput={(event) =>
                  setUnit((event.target as HTMLInputElement).value)
                }
              />
            </label>
            <label class="derived-expression">
              Expression
              <input
                value={expression}
                placeholder="[VehicleSpeed] - [WheelSpeedFL]"
                spellcheck={false}
                onInput={(event) =>
                  setExpression((event.target as HTMLInputElement).value)
                }
                onKeyDown={(event) => {
                  if (event.key === "Enter") save();
                }}
              />
            </label>
            <p class={`hint ${error ? "derived-error" : ""}`}>
              {error ??
                "Use [signal] or [CAN1:signal]; supports + - * /, abs(), sqrt(), min(), max()."}
            </p>
            <div class="derived-picks">
              {names.map((signalName) => (
                <button
                  type="button"
                  class="mini"
                  key={signalName}
                  onClick={() => insert(signalName)}
                >
                  {signalName}
                </button>
              ))}
            </div>
            <div class="derived-actions">
              <span class="hint">
                The core samples every derived channel; definitions and State
                bands travel with the project.
              </span>
              <span class="spacer" />
              <button type="button" onClick={onClose}>
                Cancel
              </button>
              <button
                type="button"
                class="primary"
                disabled={!name.trim() || !expression.trim() || Boolean(error)}
                onClick={save}
              >
                Add
              </button>
            </div>
          </div>
        </div>
      </div>
    </div>
  );
}
