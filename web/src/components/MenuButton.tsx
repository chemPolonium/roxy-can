import { useEffect, useRef, useState } from "preact/hooks";
import type { ComponentChildren } from "preact";

interface Props {
  label: ComponentChildren;
  children: (close: () => void) => ComponentChildren;
  /// Extra class on the positioning wrapper (e.g. `top-menu`).
  className?: string;
  /// Extra class on the menu panel (e.g. `mode-panel`).
  menuClass?: string;
  buttonClass?: string;
  ariaLabel?: string;
  title?: string;
  disabled?: boolean;
  menuLabel?: string;
}

/// One button-driven drop-down: open/close, outside click, Escape, arrow-key
/// navigation and the shared `.app-menu` panel all live here. Callers only
/// supply items; `close` is passed so an item can dismiss after acting.
export function MenuButton({
  label,
  children,
  className = "",
  menuClass = "",
  buttonClass,
  ariaLabel,
  title,
  disabled,
  menuLabel,
}: Props) {
  const [open, setOpen] = useState(false);
  const wrap = useRef<HTMLDivElement>(null);
  const trigger = useRef<HTMLButtonElement>(null);
  const panel = useRef<HTMLDivElement>(null);

  const close = () => setOpen(false);

  useEffect(() => {
    if (!open) return;
    panel.current
      ?.querySelector<HTMLButtonElement>("button:not(:disabled)")
      ?.focus();
    const away = (event: PointerEvent) => {
      if (
        event.target instanceof Node &&
        wrap.current?.contains(event.target)
      ) {
        return;
      }
      setOpen(false);
    };
    const escape = (event: KeyboardEvent) => {
      if (event.key !== "Escape") return;
      event.preventDefault();
      event.stopPropagation();
      setOpen(false);
      trigger.current?.focus();
    };
    window.addEventListener("pointerdown", away);
    window.addEventListener("keydown", escape, true);
    return () => {
      window.removeEventListener("pointerdown", away);
      window.removeEventListener("keydown", escape, true);
    };
  }, [open]);

  const onKeyDown = (event: KeyboardEvent) => {
    if (event.key === "Tab") {
      setOpen(false);
      return;
    }
    const items = [
      ...(panel.current?.querySelectorAll<HTMLButtonElement>(
        "button:not(:disabled)",
      ) ?? []),
    ];
    if (items.length === 0) return;
    const at = items.indexOf(document.activeElement as HTMLButtonElement);
    let next = -1;
    if (event.key === "ArrowDown") next = at < 0 ? 0 : (at + 1) % items.length;
    else if (event.key === "ArrowUp") {
      next = at <= 0 ? items.length - 1 : at - 1;
    } else if (event.key === "Home") next = 0;
    else if (event.key === "End") next = items.length - 1;
    else return;
    event.preventDefault();
    items[next].focus();
  };

  return (
    <div class={`app-menu-wrap ${className}`.trim()} ref={wrap}>
      <button
        type="button"
        ref={trigger}
        class={buttonClass}
        aria-label={ariaLabel}
        aria-haspopup="menu"
        aria-expanded={open}
        title={title}
        disabled={disabled}
        onClick={() => setOpen((value) => !value)}
      >
        {label}
      </button>
      {open && (
        <div
          class={`app-menu ${menuClass}`.trim()}
          role="menu"
          aria-label={menuLabel}
          ref={panel}
          onKeyDown={onKeyDown}
        >
          {children(close)}
        </div>
      )}
    </div>
  );
}
