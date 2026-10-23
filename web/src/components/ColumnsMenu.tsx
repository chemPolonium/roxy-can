import { useEffect, useRef, useState } from "preact/hooks";
import {
  TRACE_COLUMNS,
  type TraceColumnPrefs,
} from "../traceLayout";

export function ColumnsMenu({
  columns,
  onChange,
}: {
  columns: TraceColumnPrefs;
  onChange: (next: TraceColumnPrefs) => void;
}) {
  const [open, setOpen] = useState(false);
  const [pos, setPos] = useState<{ left: number; top: number } | null>(null);
  const ref = useRef<HTMLDivElement>(null);
  const hideable = TRACE_COLUMNS.filter(
    (column) => !column.locked && !column.flex,
  );

  useEffect(() => {
    if (!open) return;
    const down = (event: PointerEvent) => {
      if (ref.current && !ref.current.contains(event.target as Node)) {
        setOpen(false);
      }
    };
    const escape = (event: KeyboardEvent) => {
      if (event.key === "Escape") setOpen(false);
    };
    window.addEventListener("pointerdown", down);
    window.addEventListener("keydown", escape);
    return () => {
      window.removeEventListener("pointerdown", down);
      window.removeEventListener("keydown", escape);
    };
  }, [open]);

  return (
    <div class="columns-menu" ref={ref}>
      <button
        type="button"
        class={`mini ${columns.hidden.length > 0 ? "on" : ""}`}
        title="Show or hide Trace columns"
        aria-expanded={open}
        onClick={(event) => {
          if (open) {
            setOpen(false);
            return;
          }
          const rect = event.currentTarget.getBoundingClientRect();
          setPos({
            left: Math.min(rect.left, window.innerWidth - 144),
            top: rect.bottom + 2,
          });
          setOpen(true);
        }}
      >
        Columns <span aria-hidden="true">▾</span>
      </button>
      {open && pos && (
        <div
          class="columns-menu-panel"
          role="menu"
          style={{ left: `${pos.left}px`, top: `${pos.top}px` }}
        >
          {hideable.map((column) => {
            const hidden = columns.hidden.includes(column.id);
            return (
              <label key={column.id}>
                <input
                  type="checkbox"
                  checked={!hidden}
                  onChange={() =>
                    onChange({
                      ...columns,
                      hidden: hidden
                        ? columns.hidden.filter((id) => id !== column.id)
                        : [...columns.hidden, column.id],
                    })
                  }
                />
                {column.label}
              </label>
            );
          })}
        </div>
      )}
    </div>
  );
}
