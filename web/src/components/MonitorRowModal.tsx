import { useEffect, useState } from "preact/hooks";
import type { MonitorRow } from "../types";
import { palette, type Theme } from "../theme";
import { cutError, fitBandColors } from "../bandCuts";
import { CutColorEditor } from "./CutColorEditor";
import { ModalHeader } from "./ModalHeader";

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
  const [cuts, setCuts] = useState(row.cuts);
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

  const error = cutError(cuts);

  return (
    <div class="modal-backdrop" onMouseDown={onClose}>
      <div
        class="state-rule-modal"
        role="dialog"
        aria-modal="true"
        aria-label="Monitor row"
        onMouseDown={(event) => event.stopPropagation()}
      >
        <ModalHeader
          title={`Monitor row — ${row.label} · ${row.bus}${
            row.unit ? ` · ${row.unit}` : ""
          }`}
          onClose={onClose}
        />
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
          <CutColorEditor
            label="Color cuts"
            cuts={cuts}
            colors={bands}
            palette={colors}
            strict={false}
            onChange={(nextCuts, nextColors) => {
              setCuts(nextCuts);
              setBands(nextColors);
            }}
            onClear={() => {
              setCuts([]);
              setBands([bands[0] ?? 0]);
            }}
          />
          <div class="state-rule-actions">
            <span class="spacer" />
            <button
              type="button"
              class="primary"
              disabled={!!error}
              onClick={() =>
                onApply(digits, cuts, fitBandColors(bands, cuts.length + 1))
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
