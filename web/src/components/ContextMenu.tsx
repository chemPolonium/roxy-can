import type { ComponentChildren } from "preact";
import { useEffect, useRef } from "preact/hooks";

export interface ContextMenuItem {
  label: string;
  onSelect: () => void;
  danger?: boolean;
  disabled?: boolean;
  separator?: boolean;
}

interface Props {
  x: number;
  y: number;
  label: string;
  items?: ContextMenuItem[];
  children?: ComponentChildren;
  className?: string;
  focusKey?: string | number;
  onClose: () => void;
}

/// One fixed-position row menu for every right-click surface. Keeps focus on
/// the first enabled item, walks with arrows and closes on Esc, outside click
/// or scroll.
export function ContextMenu({
  x,
  y,
  label,
  items = [],
  children,
  className = "",
  focusKey,
  onClose,
}: Props) {
  const menuRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    const enabled = () =>
      [...(menuRef.current?.querySelectorAll<HTMLButtonElement>(
        "button:not(:disabled)",
      ) ?? [])];
    enabled()[0]?.focus();
    const close = (event: MouseEvent) => {
      if (!menuRef.current?.contains(event.target as Node)) onClose();
    };
    const key = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        event.preventDefault();
        onClose();
        return;
      }
      const list = enabled();
      if (list.length === 0) return;
      if (
        event.key !== "ArrowDown" &&
        event.key !== "ArrowUp" &&
        event.key !== "Home" &&
        event.key !== "End"
      ) {
        return;
      }
      event.preventDefault();
      const active = list.indexOf(document.activeElement as HTMLButtonElement);
      const index =
        event.key === "Home"
          ? 0
          : event.key === "End"
            ? list.length - 1
            : active < 0
              ? event.key === "ArrowUp"
                ? list.length - 1
                : 0
              : event.key === "ArrowDown"
                ? (active + 1) % list.length
                : (active - 1 + list.length) % list.length;
      list[index]?.focus();
    };
    const scroll = () => onClose();
    document.addEventListener("mousedown", close);
    document.addEventListener("keydown", key);
    window.addEventListener("scroll", scroll, true);
    return () => {
      document.removeEventListener("mousedown", close);
      document.removeEventListener("keydown", key);
      window.removeEventListener("scroll", scroll, true);
    };
  }, [onClose, focusKey]);

  return (
    <div
      ref={menuRef}
      class={`row-menu ${className}`.trim()}
      role="menu"
      aria-label={label}
      style={{ top: `${y}px`, left: `${x}px` }}
      onPointerDown={(event) => event.stopPropagation()}
    >
      {children ??
        items.map((item) =>
          item.separator ? (
            <div key={item.label} class="page-menu-sep" role="separator" />
          ) : (
            <button
              key={item.label}
              type="button"
              role="menuitem"
              class={item.danger ? "danger" : undefined}
              disabled={item.disabled}
              onClick={() => {
                item.onSelect();
                onClose();
              }}
            >
              {item.label}
            </button>
          ),
        )}
    </div>
  );
}
