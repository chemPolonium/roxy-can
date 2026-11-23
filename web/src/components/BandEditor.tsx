import { useEffect, useState } from "preact/hooks";
import { PaletteSwatches } from "./PaletteSwatches";
import { finalBand, normaliseBands, type Band } from "../bands";

interface Props {
  bands: Band[];
  colors: string[];
  onChange: (bands: Band[]) => void;
  showText?: boolean;
  showColor?: boolean;
  label?: string;
  maxBands?: number;
}

/// Shared < / <= band editor used by Monitor, State and Panel.
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

  const commit = (next: Band[]) => {
    const sorted = normaliseBands(next);
    setDraft(sorted);
    onChange(sorted);
  };
  const update = (index: number, patch: Partial<Band>) => {
    commit(
      draft.map((band, at) => (at === index ? { ...band, ...patch } : band)),
    );
  };
  const add = () => {
    if (draft.length >= maxBands) return;
    const finite = draft.filter((band) => band.upper !== null);
    const last = finite[finite.length - 1]?.upper ?? 0;
    const next: Band = {
      upper: last + 1,
      inclusive: false,
      color: draft.length % Math.max(1, colors.length),
    };
    commit([...draft.filter((band) => band.upper !== null), next, finalBand()]);
  };
  const remove = (index: number) => {
    if (draft[index].upper === null) return;
    commit(draft.filter((_, at) => at !== index));
  };



  return (
    <div class="band-editor">
      <div class="band-editor-head">
        <span>{label}</span>
        <span>{draft.length}/{maxBands}</span>
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
          <input
            class="band-bound"
            type="number"
            value={band.upper ?? ""}
            disabled={band.upper === null}
            placeholder="∞"
            aria-label={`Band ${index + 1} upper bound`}
            onInput={(event) => {
              const raw = (event.target as HTMLInputElement).value;
              update(index, {
                upper: raw.trim() === "" ? null : Number(raw),
              });
            }}
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

