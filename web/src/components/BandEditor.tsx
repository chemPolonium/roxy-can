import { useEffect, useState } from "preact/hooks";
import { PaletteSwatches } from "./PaletteSwatches";
import {
  finalBand,
  normaliseBands,
  parseBandBound,
  type Band,
} from "../bands";

interface Props {
  bands: Band[];
  colors: string[];
  onChange: (bands: Band[]) => void;
  showText?: boolean;
  showColor?: boolean;
  label?: string;
  maxBands?: number;
}

/// Bound input that keeps its own text while focused, so typing a number
/// never reorders the rows or turns an emptied field into the open band.
/// The value is parsed and committed on blur/Enter; invalid or blank text
/// falls back to the last committed bound.
function BoundInput({
  value,
  disabled,
  ariaLabel,
  onCommit,
}: {
  value: number | null;
  disabled: boolean;
  ariaLabel: string;
  onCommit: (value: number) => void;
}) {
  const textOf = (bound: number | null) => (bound === null ? "" : String(bound));
  const [text, setText] = useState(textOf(value));
  const [editing, setEditing] = useState(false);

  useEffect(() => {
    if (!editing) setText(textOf(value));
  }, [value, editing]);

  const commit = () => {
    setEditing(false);
    const parsed = parseBandBound(text);
    if (parsed !== null && parsed !== value) onCommit(parsed);
  };

  return (
    <input
      class="band-bound"
      type="number"
      step="any"
      value={text}
      disabled={disabled}
      placeholder="∞"
      aria-label={ariaLabel}
      onFocus={() => setEditing(true)}
      onInput={(event) => setText((event.target as HTMLInputElement).value)}
      onBlur={commit}
      onKeyDown={(event) => {
        if (event.key === "Enter") event.currentTarget.blur();
        if (event.key === "Escape") {
          setText(textOf(value));
          event.currentTarget.blur();
        }
      }}
    />
  );
}

/// Band-set editor: one `<` / `≤` upper bound per row plus the open final
/// band. Rows keep their edit order while a bound is being typed; a committed
/// bound re-sorts the list.
export function BandEditor({
  bands,
  colors,
  onChange,
  showText = true,
  showColor = true,
  label = "Bands",
  maxBands = 64,
}: Props) {
  const [draft, setDraft] = useState(bands);
  useEffect(() => {
    setDraft(bands);
  }, [bands]);

  /// Local edit plus an optional re-sort. Sorting only happens on a
  /// committed bound (blur/Enter) or a row add/remove, never while typing.
  const commit = (next: Band[], resort = false) => {
    const result = resort ? normaliseBands(next) : next;
    setDraft(result);
    onChange(result);
  };
  const update = (index: number, patch: Partial<Band>, resort = false) => {
    commit(
      draft.map((band, at) => (at === index ? { ...band, ...patch } : band)),
      resort,
    );
  };
  const add = () => {
    if (draft.length >= maxBands) return;
    const finite = draft
      .map((band) => band.upper)
      .filter((upper): upper is number => upper !== null);
    const last = finite.length ? Math.max(...finite) : 0;
    const next: Band = {
      upper: last + 1,
      inclusive: false,
      color: draft.length % Math.max(1, colors.length),
    };
    commit([...draft.filter((band) => band.upper !== null), next, finalBand()], true);
  };
  const remove = (index: number) => {
    if (draft[index].upper === null) return;
    commit(
      draft.filter((_, at) => at !== index),
      true,
    );
  };

  return (
    <div class="band-editor">
      <div class="band-editor-head">
        <span>{label}</span>
        <span>
          {draft.length}/{maxBands}
        </span>
      </div>
      {draft.map((band, index) => (
        <div class="band-row" key={index}>
          <select
            class="band-mode"
            value={band.inclusive ? "le" : "lt"}
            disabled={band.upper === null}
            aria-label={`Band ${index + 1} comparison`}
            onChange={(event) =>
              update(index, {
                inclusive: (event.target as HTMLSelectElement).value === "le",
              })
            }
          >
            <option value="lt">&lt;</option>
            <option value="le">≤</option>
          </select>
          <BoundInput
            value={band.upper}
            disabled={band.upper === null}
            ariaLabel={`Band ${index + 1} upper bound`}
            onCommit={(upper) => update(index, { upper }, true)}
          />
          {showText && (
            <input
              class="band-text"
              value={band.text ?? ""}
              maxLength={80}
              placeholder="Label"
              aria-label={`Band ${index + 1} text`}
              onInput={(event) =>
                update(index, {
                  text: (event.target as HTMLInputElement).value || undefined,
                })
              }
            />
          )}
          {showColor && (
            <>
              <PaletteSwatches
                colors={colors}
                value={band.color ?? null}
                compact
                onChange={(color) => update(index, { color })}
              />
              {band.color != null && (
                <button
                  type="button"
                  class="mini band-clear"
                  title="Remove colour"
                  aria-label="Remove colour"
                  onClick={() => update(index, { color: undefined })}
                >
                  ×
                </button>
              )}
            </>
          )}
          <button
            type="button"
            class="mini"
            disabled={band.upper === null}
            title={band.upper === null ? "Open-ended band" : "Remove band"}
            aria-label={`Remove band ${index + 1}`}
            onClick={() => remove(index)}
          >
            ×
          </button>
        </div>
      ))}
      <div>
        <button
          type="button"
          class="mini"
          disabled={draft.length >= maxBands}
          onClick={add}
        >
          + Band
        </button>
      </div>
    </div>
  );
}
