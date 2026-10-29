import { useState } from "preact/hooks";
import {
  validateDerivedExpression,
  type DerivedDef,
  type DerivedReference,
} from "../derived";

/// The computed-channel editor. It is embedded in the global Variables
/// window, so references come from the whole catalog plus the existing
/// computed channels instead of whatever happens to be plotted.
export function DerivedPanel({
  defs,
  references,
  onSave,
  onRemove,
  onRule,
}: {
  defs: DerivedDef[];
  references: DerivedReference[];
  onSave: (definition: DerivedDef) => void;
  onRemove: (id: string) => void;
  onRule?: (definition: DerivedDef) => void;
}) {
  const [name, setName] = useState("");
  const [unit, setUnit] = useState("");
  const [expression, setExpression] = useState("");
  const error =
    expression.trim() === ""
      ? null
      : validateDerivedExpression(expression, references);
  const counts = new Map<string, number>();
  const busCounts = new Map<string, number>();
  for (const reference of references) {
    counts.set(reference.name, (counts.get(reference.name) ?? 0) + 1);
    const busKey = `${reference.bus}:${reference.name}`;
    busCounts.set(busKey, (busCounts.get(busKey) ?? 0) + 1);
  }
  const computedRefs = references.filter(
    (reference) => reference.bus === "derived",
  );
  const picks = [
    ...computedRefs,
    ...references
      .filter((reference) => reference.bus !== "derived")
      .slice(0, Math.max(0, 80 - computedRefs.length)),
  ].map((reference) => {
    const sharedName = (counts.get(reference.name) ?? 0) > 1;
    const sharedBus =
      (busCounts.get(`${reference.bus}:${reference.name}`) ?? 0) > 1;
    const label = !sharedName
      ? reference.name
      : sharedBus && reference.frame
        ? `${reference.bus}:${reference.frame}:${reference.name}`
        : `${reference.bus}:${reference.name}`;
    return { key: reference.key, label };
  });

  const insert = (reference: string) => {
    setExpression((old) => `${old}${old.trim() ? " " : ""}[${reference}]`);
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
    <div class="derived-panel">
      {defs.length > 0 && (
        <div class="derived-list">
          {defs.map((definition) => (
            <div class="derived-item" key={definition.id}>
              <span class="derived-item-name">
                {definition.name}
                {definition.unit && <em>{definition.unit}</em>}
              </span>
              <code>{definition.expression}</code>
              {definition.error && (
                <span class="derived-error" title={definition.error}>
                  error
                </span>
              )}
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
            "Use [signal], [CAN1:signal] or [CAN2:0x0C7:signal] when one bus repeats a name; supports + - * /, abs(), sqrt(), min(), max()."}
        </p>
        <div class="derived-picks">
          {picks.map((pick) => (
            <button
              type="button"
              class="mini"
              key={pick.key}
              onClick={() => insert(pick.label)}
            >
              {pick.label}
            </button>
          ))}
          {references.length === 0 && (
            <span class="hint">
              Load a description file to reference signals here.
            </span>
          )}
        </div>
        <div class="derived-actions">
          <span class="hint">
            The core samples every computed channel; definitions and State
            bands travel with the project.
          </span>
          <span class="spacer" />
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
  );
}
