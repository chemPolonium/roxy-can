import { useEffect, useState } from "preact/hooks";
import type { BandSet, StateRule } from "../types";
import type { Band } from "../bands";
import { BandSetPicker } from "./BandSetPicker";
import { ModalHeader } from "./ModalHeader";

interface Props {
  signal: string;
  rule: StateRule | null;
  bandSets: BandSet[];
  onApply: (bands: Band[], bandSetId: string | null) => void;
  onClear: () => void;
  onClose: () => void;
}

export function StateRuleModal({
  signal,
  rule,
  bandSets,
  onApply,
  onClear,
  onClose,
}: Props) {
  const [bandSetId, setBandSetId] = useState(rule?.bandSetId ?? "");
  const bands =
    bandSets.find((set) => set.id === bandSetId)?.bands ?? rule?.bands ?? [];
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
        aria-label="State bands"
        onMouseDown={(event) => event.stopPropagation()}
      >
        <ModalHeader title={`State bands — ${signal}`} onClose={onClose} />
        <div class="state-rule-body">
          <p class="hint">
            Bands use &lt; or ≤ against an upper bound; the last band is
            open-ended. Text and colour are optional.
          </p>
          <div class="state-band-list">
            <label>
              Project BandSet
              <BandSetPicker
                value={bandSetId}
                ariaLabel="Project BandSet"
                choices={[
                  { value: "", label: "Inline / none" },
                  ...bandSets.map((set) => ({
                    value: set.id,
                    label: set.name,
                    hint: set.id,
                  })),
                ]}
                onChange={setBandSetId}
              />
            </label>
          </div>
          <div class="state-rule-actions">
            <button type="button" onClick={onClear}>
              Default bands
            </button>
            <span class="spacer" />
            {error && <span class="sv-error">{error}</span>}
            <button type="button" onClick={onClose}>
              Cancel
            </button>
            <button
              type="button"
              class="primary"
              disabled={!!error}
              onClick={() => onApply(bands, bandSetId || null)}
            >
              Apply
            </button>
          </div>
        </div>
      </div>
    </div>
  );
}
