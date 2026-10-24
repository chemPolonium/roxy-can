import { useEffect, useState } from "preact/hooks";
import type { StateRule } from "../types";

interface Props {
  signal: string;
  rule: StateRule | null;
  onApply: (cuts: number[], names: string[]) => void;
  onClear: () => void;
  onClose: () => void;
}

interface Draft {
  cuts: string[];
  names: string[];
}

function fromRule(rule: StateRule | null): Draft {
  return {
    cuts: (rule?.cuts ?? []).map(String),
    names: rule?.names?.length ? [...rule.names] : ["all"],
  };
}

/** "&#x3c; 1", "1 .. 5", ">= 5" for band `index`. */
function rangeText(cuts: number[], index: number): string {
  const lo = index === 0 ? null : cuts[index - 1];
  const hi = index === cuts.length ? null : cuts[index];
  if (lo == null && hi == null) return "all values";
  if (lo == null) return `< ${hi}`;
  if (hi == null) return `>= ${lo}`;
  return `${lo} .. ${hi}`;
}

export function StateRuleModal({
  signal,
  rule,
  onApply,
  onClear,
  onClose,
}: Props) {
  const [draft, setDraft] = useState<Draft>(fromRule(rule));
  useEffect(() => {
    const close = (event: KeyboardEvent) => {
      if (event.key === "Escape") onClose();
    };
    window.addEventListener("keydown", close);
    return () => window.removeEventListener("keydown", close);
  }, [onClose]);
  const parsed = draft.cuts.map((cut) => Number(cut));
  let error = "";
  if (parsed.some((cut) => !Number.isFinite(cut))) {
    error = "cuts must be numbers";
  } else if (parsed.some((cut, i) => i > 0 && cut <= parsed[i - 1])) {
    error = "cuts must be strictly ascending";
  }

  const setCut = (index: number, value: string) => {
    setDraft((current) => {
      const cuts = [...current.cuts];
      cuts[index] = value;
      return { ...current, cuts };
    });
  };
  const setName = (index: number, value: string) => {
    setDraft((current) => {
      const names = [...current.names];
      names[index] = value;
      return { ...current, names };
    });
  };
  const addCut = () => {
    setDraft((current) => {
      const last = Number(current.cuts[current.cuts.length - 1]);
      const cut = Number.isFinite(last) ? String(last + 1) : "0";
      return {
        cuts: [...current.cuts, cut],
        names: [...current.names, `state ${current.names.length}`],
      };
    });
  };
  const removeCut = (cutIndex: number) => {
    setDraft((current) => ({
      cuts: current.cuts.filter((_, index) => index !== cutIndex),
      names: current.names.filter((_, index) => index !== cutIndex + 1),
    }));
  };

  return (
    <div class="modal-backdrop" onMouseDown={onClose}>
      <div
        class="state-rule-modal"
        role="dialog"
        aria-modal="true"
        aria-label="State bands"
        onMouseDown={(event) => event.stopPropagation()}
      >
        <header class="modal-head">
          <h2>State bands — {signal}</h2>
          <button type="button" onClick={onClose}>
            Close
          </button>
        </header>
        <div class="state-rule-body">
          <p class="hint">
            Ascending cut points split the value axis; every band carries its
            own name and takes a palette color. Default bands name the
            quantized values.
          </p>
          <div class="state-band-list">
            {draft.names.map((name, index) => (
              <div class="state-band-edit" key={index}>
                {index > 0 && (
                  <label>
                    cut
                    <input
                      value={draft.cuts[index - 1]}
                      inputMode="decimal"
                      onInput={(event) =>
                        setCut(
                          index - 1,
                          (event.target as HTMLInputElement).value,
                        )
                      }
                    />
                  </label>
                )}
                <label class="state-band-name">
                  name
                  <input
                    value={name}
                    onInput={(event) =>
                      setName(index, (event.target as HTMLInputElement).value)
                    }
                  />
                </label>
                <span class="state-band-range">
                  {rangeText(parsed, index)}
                </span>
                {index > 0 && (
                  <button
                    type="button"
                    class="mini"
                    title="Remove this cut (merge with the band above)"
                    onClick={() => removeCut(index - 1)}
                  >
                    −
                  </button>
                )}
              </div>
            ))}
          </div>
          <div class="state-rule-actions">
            <button type="button" onClick={addCut}>
              + Cut
            </button>
            <button type="button" onClick={onClear}>
              Default bands
            </button>
            <span class="spacer" />
            {error && <span class="sv-error">{error}</span>}
            <button
              type="button"
              class="primary"
              disabled={!!error}
              onClick={() => onApply(parsed, draft.names.map((n) => n.trim()))}
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
