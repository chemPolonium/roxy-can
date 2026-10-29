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
  const [busKey, setBusKey] = useState("");
  const [frameKey, setFrameKey] = useState("");
  const busCounts = new Map<string, number>();
  const frameCounts = new Map<string, number>();
  const busMap = new Map<
    string,
    {
      label: string;
      frames: Map<string, { label: string; refs: DerivedReference[] }>;
    }
  >();
  for (const reference of references) {
    const busName = `${reference.bus}:${reference.name}`;
    busCounts.set(busName, (busCounts.get(busName) ?? 0) + 1);
    if (reference.frame) {
      const frameName = `${reference.bus}:${reference.frame}:${reference.name}`;
      frameCounts.set(frameName, (frameCounts.get(frameName) ?? 0) + 1);
    }
    let bus = busMap.get(reference.bus);
    if (!bus) {
      bus = {
        label: reference.bus === "derived" ? "Computed" : reference.bus,
        frames: new Map(),
      };
      busMap.set(reference.bus, bus);
    }
    const token =
      reference.bus === "derived"
        ? "computed"
        : (reference.frame ?? reference.frameId ?? "signals");
    let frame = bus.frames.get(token);
    if (!frame) {
      const label =
        reference.bus === "derived"
          ? "Computed channels"
          : token === "signals"
            ? "Signals"
            : token.replace(/^slot(\d+)$/i, "Slot $1");
      frame = { label, refs: [] };
      bus.frames.set(token, frame);
    }
    frame.refs.push(reference);
  }
  const buses = [...busMap.entries()].map(([key, bus]) => ({
    key,
    label: bus.label,
    frames: [...bus.frames.entries()].map(([frameKey, frame]) => ({
      key: frameKey,
      label: frame.label,
      refs: frame.refs,
    })),
  }));
  const activeBus = buses.find((bus) => bus.key === busKey) ?? buses[0];
  const activeFrame =
    activeBus?.frames.find((frame) => frame.key === frameKey) ??
    activeBus?.frames[0];

  /// Always insert the full path of what the three columns show: computed
  /// channels stay `derived:Name`, catalog signals are `Bus:Frame:Signal`
  /// even when a shorter form would already be unique.
  const insertToken = (reference: DerivedReference): string => {
    if (reference.bus === "derived") {
      return (busCounts.get(`derived:${reference.name}`) ?? 0) <= 1
        ? `derived:${reference.name}`
        : reference.key;
    }
    const token = reference.frame ?? reference.frameId;
    if (!token) return `${reference.bus}:${reference.name}`;
    const sameFrame =
      (frameCounts.get(
        `${reference.bus}:${reference.frame}:${reference.name}`,
      ) ?? 0) > 1;
    return `${reference.bus}:${sameFrame && reference.frameId ? reference.frameId : token}:${reference.name}`;
  };

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
            "Picking inserts the full path, e.g. [CAN1:EngineStatus:EngineSpeed] (frame name or id); computed channels use [derived:Name]. Supports + - * /, abs(), sqrt(), min(), max()."}
        </p>
        {references.length === 0 ? (
          <span class="hint">
            Load a description file to reference signals here.
          </span>
        ) : (
          <div class="derived-browser" role="group" aria-label="Signal browser">
            <div class="derived-browser-col">
              <div class="derived-browser-head">Bus / cluster</div>
              {buses.map((bus) => (
                <button
                  type="button"
                  key={bus.key}
                  class={activeBus?.key === bus.key ? "on" : ""}
                  onClick={() => {
                    setBusKey(bus.key);
                    setFrameKey("");
                  }}
                >
                  {bus.label}
                </button>
              ))}
            </div>
            <div class="derived-browser-col">
              <div class="derived-browser-head">Frame / message</div>
              {activeBus?.frames.map((frame) => (
                <button
                  type="button"
                  key={frame.key}
                  class={activeFrame?.key === frame.key ? "on" : ""}
                  onClick={() => setFrameKey(frame.key)}
                >
                  {frame.label}
                </button>
              ))}
            </div>
            <div class="derived-browser-col derived-browser-signals">
              <div class="derived-browser-head">Signal</div>
              {activeFrame?.refs.map((reference) => (
                <button
                  type="button"
                  key={reference.key}
                  title={`Insert [${insertToken(reference)}]`}
                  onClick={() => insert(insertToken(reference))}
                >
                  {reference.name}
                </button>
              ))}
            </div>
          </div>
        )}
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
