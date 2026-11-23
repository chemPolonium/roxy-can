import { useEffect, useState } from "preact/hooks";
import type { BandSet, MonitorRow } from "../types";
import { ModalHeader } from "./ModalHeader";

interface Props {
  row: MonitorRow;
  bandSets: BandSet[];
  onApply: (digits: number, bands: MonitorRow["bands"], bandSetId: string | null) => void;
  onClose: () => void;
}

/** Right-click a Monitor row: decimals and colour bands. */
export function MonitorRowModal({ row, bandSets, onApply, onClose }: Props) {
  const [digits, setDigits] = useState(row.digits);
  const [bandSetId, setBandSetId] = useState(row.bandSetId ?? "");

  useEffect(() => {
    const close = (event: KeyboardEvent) => {
      if (event.key === "Escape") onClose();
    };
    window.addEventListener("keydown", close);
    return () => window.removeEventListener("keydown", close);
  }, [onClose]);

  const error = "";

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
            Pick a project BandSet for this row, or keep the numeric value.
            Edit sets in Variables → Band sets.
          </p>
          <div class="monitor-style-row">
            <span>Bands</span>
            <select
              value={bandSetId}
              onChange={(event) => {
                const id = (event.target as HTMLSelectElement).value;
                setBandSetId(id);
              }}
            >
              <option value="">No bands</option>
              {bandSets.map((set) => (
                <option key={set.id} value={set.id}>
                  {set.name}
                </option>
              ))}
            </select>
          </div>
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
          {error && <span class="sv-error">{error}</span>}
          <div class="state-rule-actions">
            <span class="spacer" />
            <button
              type="button"
              class="primary"
              disabled={!!error}
              onClick={() => onApply(digits, row.bands, bandSetId || null)}
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
