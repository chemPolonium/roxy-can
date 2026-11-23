interface Props {
  colors: string[];
  value: number | null;
  onChange: (slot: number) => void;
  onFocus?: () => void;
  label?: string;
  compact?: boolean;
}

/// One palette slot selector, used by every cut-band/color editor.
export function PaletteSwatches({
  colors,
  value,
  onChange,
  onFocus,
  label = "Palette colour",
  compact = false,
}: Props) {
  return (
    <span
      class={`palette-swatches${compact ? " compact" : ""}`}
      role="group"
      aria-label={label}
    >
      {colors.map((color, slot) => (
        <button
          key={slot}
          type="button"
          class={`palette-swatch ${
            value != null && Math.abs(value) % colors.length === slot
              ? "on"
              : ""
          }`}
          style={{ background: color }}
          title={`Use palette colour ${slot + 1}`}
          aria-label={`${label} ${slot + 1}`}
          aria-pressed={
            value != null && Math.abs(value) % colors.length === slot
          }
          onClick={() => {
            onFocus?.();
            onChange(slot);
          }}
        />
      ))}
    </span>
  );
}
