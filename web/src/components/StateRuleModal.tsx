import { useEffect, useState } from "preact/hooks";
import type { StateRule } from "../types";
import type { Band } from "../bands";
import { validateBands } from "../bands";
import { palette, type Theme } from "../theme";
import { BandEditor } from "./BandEditor";
import { ModalHeader } from "./ModalHeader";

interface Props {
  signal: string;
  rule: StateRule | null;
  theme: Theme;
  onApply: (bands: Band[]) => void;
  onClear: () => void;
  onClose: () => void;
}

export function StateRuleModal({
  signal,
  rule,
  theme,
  onApply,
  onClear,
  onClose,
}: Props) {
  const [bands, setBands] = useState<Band[]>(
    rule?.bands?.length
      ? rule.bands
      : [{ upper: null, inclusive: false, text: "all" }],
  );
  const colors = palette(theme).monitors;
  useEffect(() => {
    const close = (event: KeyboardEvent) => {
      if (event.key === "Escape") onClose();
    };
    window.addEventListener("keydown", close);
    return () => window.removeEventListener("keydown", close);
  }, [onClose]);
  const error = validateBands(bands);
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
            <BandEditor bands={bands} colors={colors} onChange={setBands} />
          </div>
          <div class="state-rule-actions">
            <button type="button" onClick={onClear}>
              Default bands
            </button>
            <span class="spacer" />
            {error && <span class="sv-error">{error}</span>}
            <button
              type="button"
              class="primary"
              disabled={!!error}
              onClick={() => onApply(bands)}
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
