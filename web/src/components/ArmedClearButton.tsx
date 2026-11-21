import { useEffect, useState } from "preact/hooks";
import { UiIcon } from "./icons";

interface Props {
  title: string;
  confirmTitle: string;
  onClear: () => void;
  disabled?: boolean;
  label?: string;
  confirmLabel?: string;
  className?: string;
}

/// Trash action with an in-place two-step arm: first click asks, second click
/// clears, three seconds of silence disarms. Used by every list-clear icon.
export function ArmedClearButton({
  title,
  confirmTitle,
  onClear,
  disabled = false,
  label = "Clear",
  confirmLabel = "Confirm clear",
  className = "",
}: Props) {
  const [armed, setArmed] = useState(false);

  useEffect(() => {
    if (!armed) return;
    const timer = window.setTimeout(() => setArmed(false), 3000);
    return () => window.clearTimeout(timer);
  }, [armed]);

  return (
    <button
      type="button"
      class={`mini icon-btn ${armed ? "danger" : ""} ${className}`.trim()}
      disabled={disabled}
      title={armed ? confirmTitle : title}
      aria-label={armed ? confirmLabel : label}
      onClick={() => {
        if (!armed) {
          setArmed(true);
          return;
        }
        setArmed(false);
        onClear();
      }}
    >
      <UiIcon name={armed ? "check" : "trash"} />
    </button>
  );
}
