import { useEffect, useState } from "preact/hooks";
import type { MonitorRow } from "../types";
import { palette, type Theme } from "../theme";
import {
  addCutBand,
  cutError,
  cutIsInvalid,
  fitBandColors,
  rangeText,
  removeCutBand,
} from "../bandCuts";
import { PaletteSwatches } from "./PaletteSwatches";

interface Props {
  row: MonitorRow;
  theme: Theme;
  onApply: (digits: number, cuts: number[], colors: number[]) => void;
  onClose: () => void;
}

/** Right-click a Monitor row: decimals, cut points and one color per band. */
export function MonitorRowModal({ row, theme, onApply, onClose }: Props) {
  const colors = palette(theme).monitors;
  const [digits, setDigits] = useState(row.digits);
  const [cuts, setCuts] = useState(() => row.cuts.map(String));
  const [bands, setBands] = useState(() =>
    fitBandColors(row.colors, row.cuts.length + 1),
  );

  useEffect(() => {
    const close = (event: KeyboardEvent) => {
      if (event.key === "Escape") onClose();
    };
    window.addEventListener("keydown", close);
    return () => window.removeEventListener("keydown", close);
  }, [onClose]);

  const parsed = cuts.map(Number);
  const error = cutError(parsed);
  const invalidCut = (index: number) => cutIsInvalid(parsed, index);

  const setCut = (index: number, value: string) => {
    setCuts((old) => {
      const next = [...old];
      next[index] = value;
      return next;
    });
  };
  const setBandColor = (index: number, color: number) => {
    setBands((old) => {
      const next = [...old];
      next[index] = color;
      return next;
    });
  };
  const addCut = () => {
    const next = addCutBand(
      cuts.map(Number),
      bands,
      bands[bands.length - 1] ?? 0,
    );
    setCuts(next.cuts.map(String));
    setBands(next.colors);
  };
  const removeCut = (cutIndex: number) => {
    const next = removeCutBand(cuts.map(Number), bands, cutIndex);
    setCuts(next.cuts.map(String));
    setBands(next.colors);
  };

  return (
    <div class="modal-backdrop" onMouseDown={onClose}>
      <div
        class="state-rule-modal"
        role="dialog"
        aria-modal="true"
        aria-label="Monitor row"
        onMouseDown={(event) => event.stopPropagation()}
      >
        <header class="modal-head">
          <h2>
            Monitor row — {row.label} · {row.bus}
            {row.unit ? ` · ${row.unit}` : ""}
          </h2>
          <button type="button" onClick={onClose}>
            Close
          </button>
        </header>
        <div class="state-rule-body">
          <p class="hint">
            Right-clicked from Monitor. Cut points split the value axis; each
            band keeps its own palette color, and the value uses that color
            while it is inside the band. Values are in {row.unit || "raw units"}.
          </p>
          <div class="monitor-style-row">
            <span>Decimals</span>
            {[0, 1, 2, 3].map((value) => (
              <button
                key={value}
                type="button"
                class={`mini monitor-style-button ${
                  digits === value ? "on" : ""
                }`}
                aria-pressed={digits === value}
                onClick={() => setDigits(value)}
              >
                {value}
              </button>
            ))}
          </div>
          <div class="state-band-list">
            {cuts.map((_, index) => (
              <div class="monitor-band-edit" key={index}>
                <span class="state-band-range">
                  {rangeText(parsed, index)}
                </span>
                <label class="monitor-band-cut">
                  {index > 0 && (
                    <>
                      cut
                      <input
                        class={invalidCut(index - 1) ? "invalid" : ""}
                        aria-invalid={invalidCut(index - 1)}
                        value={cuts[index - 1]}
                        inputMode="decimal"
                        onInput={(event) =>
                          setCut(
                            index - 1,
                            (event.target as HTMLInputElement).value,
                          )
                        }
                      />
                    </>
                  )}
                </label>
                <PaletteSwatches
                  colors={colors}
                  value={bands[index] ?? 0}
                  compact
                  onChange={(slot) => setBandColor(index, slot)}
                />
                {index > 0 ? (
                  <button
                    type="button"
                    class="mini monitor-band-remove"
                    title="Remove this cut (merge with the band above)"
                    onClick={() => removeCut(index - 1)}
                  >
                    −
                  </button>
                ) : (
                  <span />
                )}
              </div>
            ))}
            <div class="monitor-band-edit">
              <span class="state-band-range">
                {rangeText(parsed, cuts.length)}
              </span>
              <label class="monitor-band-cut">
                {cuts.length > 0 && (
                  <>
                    cut
                    <input
                      class={invalidCut(cuts.length - 1) ? "invalid" : ""}
                      aria-invalid={invalidCut(cuts.length - 1)}
                      value={cuts[cuts.length - 1]}
                      inputMode="decimal"
                      onInput={(event) =>
                        setCut(
                          cuts.length - 1,
                          (event.target as HTMLInputElement).value,
                        )
                      }
                    />
                  </>
                )}
              </label>
              <PaletteSwatches
                colors={colors}
                value={bands[cuts.length] ?? 0}
                compact
                onChange={(slot) => setBandColor(cuts.length, slot)}
              />
              {cuts.length > 0 ? (
                <button
                  type="button"
                  class="mini monitor-band-remove"
                  title="Remove this cut (merge with the band above)"
                  onClick={() => removeCut(cuts.length - 1)}
                >
                  −
                </button>
              ) : (
                <span />
              )}
            </div>
          </div>
          {error && (
            <p class="sv-error monitor-cut-error" role="alert">
              {error}
            </p>
          )}
          <div class="state-rule-actions">
            <button type="button" onClick={addCut}>
              + Cut
            </button>
            <button
              type="button"
              onClick={() => {
                setCuts([]);
                setBands([bands[0] ?? 0]);
              }}
            >
              Clear bands
            </button>
            <span class="spacer" />
            <button
              type="button"
              class="primary"
              disabled={!!error}
              onClick={() =>
                onApply(digits, parsed, fitBandColors(bands, cuts.length + 1))
              }
            >
              Apply
            </button>
            <button type="button" onClick={onClose}>
              Cancel
            </button>
          </div>
        </div>
      </div>
    </div>
  );
}
