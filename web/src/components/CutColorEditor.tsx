import { addCutBand, cutError, rangeText, removeCutBand } from "../bandCuts";
import { NumberField } from "./NumberField";
import { PaletteSwatches } from "./PaletteSwatches";

interface Props {
  label: string;
  cuts: number[];
  colors: number[];
  palette: string[];
  maxCuts?: number;
  strict?: boolean;
  onFocus?: () => void;
  onChange: (cuts: number[], colors: number[]) => void;
  onClear?: () => void;
}

/// Cut points and one palette slot per band, shared by Panel and Monitor.
export function CutColorEditor({
  label,
  cuts,
  colors,
  palette,
  maxCuts = 8,
  strict = true,
  onFocus,
  onChange,
  onClear,
}: Props) {
  const full = cuts.length >= maxCuts;
  const error = cutError(cuts);
  const colorAt = (index: number) => colors[index] ?? 0;

  const setCut = (index: number, value: number) => {
    const next = cuts.slice();
    next[index] = value;
    if (strict && next.some((cut, at) => at > 0 && cut <= next[at - 1])) {
      return;
    }
    onChange(next, colors);
  };

  const addCut = () => {
    const next = addCutBand(cuts, colors, (cuts.length + 1) % palette.length);
    onChange(next.cuts, next.colors);
  };

  const removeCut = (index: number) => {
    const next = removeCutBand(cuts, colors, index);
    onChange(next.cuts, next.colors);
  };

  return (
    <div class="cut-editor">
      <div class="cut-editor-head">
        <span>{label}</span>
        <span>
          {cuts.length} {cuts.length === 1 ? "cut" : "cuts"}
        </span>
      </div>
      {cuts.length === 0 && (
        <span class="cut-editor-empty">
          No cuts: the value keeps its normal colour.
        </span>
      )}
      {Array.from({ length: cuts.length + 1 }, (_, index) => (
        <div class="cut-editor-row" key={index}>
          <span class="cut-editor-range">{rangeText(cuts, index)}</span>
          <PaletteSwatches
            colors={palette}
            value={colorAt(index)}
            onFocus={onFocus}
            onChange={(slot) => {
              const next = Array.from({ length: cuts.length + 1 }, (_, at) =>
                at === index ? slot : colorAt(at),
              );
              onChange(cuts, next);
            }}
          />
          {index < cuts.length && (
            <>
              <span class="cut-editor-limit">
                ≤
                <NumberField
                  value={cuts[index]}
                  className="cut-editor-input"
                  onFocus={() => onFocus?.()}
                  onCommit={(value) => setCut(index, value)}
                />
              </span>
              <button
                type="button"
                class="mini cut-editor-remove"
                title="Remove cut"
                aria-label="Remove cut"
                onClick={() => {
                  onFocus?.();
                  removeCut(index);
                }}
              >
                ×
              </button>
            </>
          )}
        </div>
      ))}
      <div class="cut-editor-actions">
        <button
          type="button"
          class="mini"
          disabled={full}
          title="Add a value cut"
          onClick={() => {
            onFocus?.();
            addCut();
          }}
        >
          + Cut
        </button>
        {onClear && cuts.length > 0 && (
          <button
            type="button"
            class="mini"
            onClick={() => {
              onFocus?.();
              onClear();
            }}
          >
            Clear cuts
          </button>
        )}
      </div>
      {error && (
        <p class="cut-editor-error" role="alert">
          {error}
        </p>
      )}
    </div>
  );
}
