import { useEffect, useRef, useState } from "preact/hooks";

/// Accepts `12.345`, `12.345s` and `1:02.5` so log positions can be typed
/// straight from a timestamp column.
export function parseTime(text: string): number | null {
  const value = text.trim().toLowerCase().replace(/s$/, "");
  if (!value) return null;
  const parts = value.split(":");
  if (parts.length > 2) return null;
  let seconds = 0;
  for (const part of parts) {
    if (part.trim() === "" || !/^\d*\.?\d*$/.test(part.trim())) return null;
    seconds = seconds * 60 + Number(part);
  }
  return Number.isFinite(seconds) && seconds >= 0 ? seconds : null;
}

export function TimeJump({
  maxS,
  onGo,
  onClose,
}: {
  maxS: number | null;
  onGo: (seconds: number) => void;
  onClose: () => void;
}) {
  const [text, setText] = useState("");
  const inputRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    inputRef.current?.focus();
    const escape = (event: KeyboardEvent) => {
      if (event.key === "Escape") onClose();
    };
    window.addEventListener("keydown", escape);
    return () => window.removeEventListener("keydown", escape);
  }, [onClose]);

  const seconds = parseTime(text);
  const submit = () => {
    if (seconds === null) return;
    onGo(seconds);
  };

  return (
    <div class="modal-backdrop" onMouseDown={onClose}>
      <div
        class="time-jump"
        role="dialog"
        aria-modal="true"
        aria-label="Go to time"
        onMouseDown={(event) => event.stopPropagation()}
      >
        <h2>Go to time</h2>
        <input
          ref={inputRef}
          type="text"
          value={text}
          placeholder="e.g. 12.345 or 1:02.5"
          aria-label="Time in seconds"
          onInput={(event) => setText((event.target as HTMLInputElement).value)}
          onKeyDown={(event) => {
            if (event.key === "Enter") {
              event.preventDefault();
              submit();
            }
          }}
        />
        <p class="hint">
          {maxS !== null
            ? `Session time · latest ${maxS.toFixed(3)} s`
            : "Session time in seconds"}
        </p>
        <div class="time-jump-actions">
          <button type="button" onClick={onClose}>
            Cancel
          </button>
          <button
            type="button"
            class="primary"
            disabled={seconds === null}
            onClick={submit}
          >
            Go
          </button>
        </div>
      </div>
    </div>
  );
}
