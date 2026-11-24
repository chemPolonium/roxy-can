import { useEffect, useRef, useState } from "preact/hooks";
import { PaletteSwatches } from "./PaletteSwatches";
import { UiIcon } from "./icons";
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

/// One colour trigger per band: the current swatch is the button itself, and
/// the palette opens in a small popover with an explicit "No colour" entry.
/// Keeping the palette out of the row makes the selected colour obvious and
/// leaves a single trailing remove action.
function ColorInput({
  colors,
  value,
  onChange,
}: {
  colors: string[];
  value: number | null;
  onChange: (slot: number | null) => void;
}) {
  const trigger = useRef<HTMLButtonElement>(null);
  const menu = useRef<HTMLDivElement>(null);
  const [open, setOpen] = useState(false);
  const [anchor, setAnchor] = useState<{
    left: number;
    top: number;
    width: number;
  } | null>(null);
  const slot = value === null ? null : Math.abs(value) % colors.length;
  const color = slot === null ? null : colors[slot];

  const show = () => {
    const box = trigger.current?.getBoundingClientRect();
    if (!box) return;
    const width = Math.max(140, Math.min(colors.length * 21 + 10, 220));
    const left = Math.max(
      8,
      Math.min(box.left, window.innerWidth - width - 8),
    );
    const top =
      box.bottom + 66 > window.innerHeight
        ? Math.max(8, box.top - 62)
        : box.bottom + 2;
    setAnchor({ left, top, width });
    setOpen(true);
  };

  const close = () => {
    setOpen(false);
    setAnchor(null);
  };

  useEffect(() => {
    if (!open) return;
    menu.current
      ?.querySelector<HTMLElement>(".palette-swatch.on, .palette-swatch")
      ?.focus({ preventScroll: true });
    const inside = (node: Node | null) =>
      !!node && (trigger.current?.contains(node) || menu.current?.contains(node));
    const closeOnPointer = (event: PointerEvent) => {
      if (!inside(event.target as Node)) close();
    };
    const closeOnScroll = (event: Event) => {
      if (inside(event.target as Node)) return;
      close();
    };
    const closeOnResize = () => close();
    window.addEventListener("pointerdown", closeOnPointer);
    window.addEventListener("scroll", closeOnScroll, true);
    window.addEventListener("resize", closeOnResize);
    return () => {
      window.removeEventListener("pointerdown", closeOnPointer);
      window.removeEventListener("scroll", closeOnScroll, true);
      window.removeEventListener("resize", closeOnResize);
    };
  }, [open]);

  return (
    <div class="band-color-wrap">
      <button
        ref={trigger}
        type="button"
        class={`band-color${color ? "" : " none"}`}
        style={color ? { background: color } : undefined}
        title={color ? "Band colour — click to change" : "No colour"}
        aria-label={
          color
            ? `Band colour ${slot === null ? "" : slot + 1}`
            : "Band colour: none"
        }
        aria-haspopup="dialog"
        aria-expanded={open}
        disabled={colors.length === 0}
        onClick={() => (open ? close() : show())}
        onKeyDown={(event) => {
          if (event.key === "Escape" && open) {
            event.stopPropagation();
            close();
          }
        }}
      />
      {open && anchor && (
        <div
          ref={menu}
          class="band-color-menu"
          role="group"
          aria-label="Band colour"
          style={{
            left: `${anchor.left}px`,
            top: `${anchor.top}px`,
            width: `${anchor.width}px`,
          }}
          onKeyDown={(event) => {
            if (event.key !== "Escape") return;
            event.preventDefault();
            event.stopPropagation();
            close();
            trigger.current?.focus();
          }}
        >
          <PaletteSwatches
            colors={colors}
            value={value}
            label="Band colour"
            onChange={(next) => {
              onChange(next);
              close();
              trigger.current?.focus();
            }}
          />
          <button
            type="button"
            class={`band-color-none${color ? "" : " on"}`}
            aria-pressed={!color}
            onClick={() => {
              onChange(null);
              close();
              trigger.current?.focus();
            }}
          >
            No colour
          </button>
        </div>
      )}
    </div>
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
            <ColorInput
              colors={colors}
              value={band.color ?? null}
              onChange={(color) =>
                update(index, { color: color ?? undefined })
              }
            />
          )}
          <button
            type="button"
            class="mini band-remove"
            disabled={band.upper === null}
            title={band.upper === null ? "Open-ended band" : "Remove band"}
            aria-label={`Remove band ${index + 1}`}
            onClick={() => remove(index)}
          >
            <UiIcon name="trash" />
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
