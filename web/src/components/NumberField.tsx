import { useEffect, useRef, useState } from "preact/hooks";

interface Props {
  value: number;
  onCommit: (value: number) => void;
  min?: number;
  max?: number;
  round?: boolean;
  step?: number | "any";
  className?: string;
  ariaLabel?: string;
  placeholder?: string;
  disabled?: boolean;
  onFocus?: () => void;
}

/// Draft numeric input: edits locally, commits on blur/Enter, restores on Esc.
export function NumberField({
  value,
  onCommit,
  min,
  max,
  round = false,
  step = "any",
  className,
  ariaLabel,
  placeholder,
  disabled = false,
  onFocus,
}: Props) {
  const [draft, setDraft] = useState(() => String(value));
  const [editing, setEditing] = useState(false);
  const cancelled = useRef(false);

  useEffect(() => {
    if (!editing) setDraft(String(value));
  }, [value, editing]);

  const commit = () => {
    setEditing(false);
    if (cancelled.current) {
      cancelled.current = false;
      setDraft(String(value));
      return;
    }
    const parsed = Number(draft);
    if (draft.trim() === "" || !Number.isFinite(parsed)) {
      setDraft(String(value));
      return;
    }
    let next = round ? Math.round(parsed) : parsed;
    if (min !== undefined) next = Math.max(min, next);
    if (max !== undefined) next = Math.min(max, next);
    onCommit(next);
  };

  return (
    <input
      class={className}
      type="number"
      step={step}
      value={draft}
      disabled={disabled}
      placeholder={placeholder}
      aria-label={ariaLabel}
      onFocus={() => {
        setEditing(true);
        onFocus?.();
      }}
      onInput={(event) => setDraft((event.target as HTMLInputElement).value)}
      onBlur={commit}
      onKeyDown={(event) => {
        if (event.key === "Enter") {
          (event.target as HTMLInputElement).blur();
        } else if (event.key === "Escape") {
          cancelled.current = true;
          setDraft(String(value));
          (event.target as HTMLInputElement).blur();
        }
      }}
    />
  );
}
