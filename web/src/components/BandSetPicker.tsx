import { useEffect, useRef, useState } from "preact/hooks";
import { bandSetMatchesQuery } from "../bands";

export interface BandSetChoice {
  value: string;
  label: string;
  hint?: string;
}

interface Props {
  value: string;
  choices: BandSetChoice[];
  ariaLabel: string;
  disabled?: boolean;
  onChange: (value: string) => void;
  /// Called when the list opens; PagePanel uses it to record undo history.
  onOpen?: () => void;
}

/// Dropdown for the project BandSets. The list is fixed-positioned so the
/// modal/inspector scroll area cannot clip it, and the search box filters by
/// name or id for projects with many sets.
export function BandSetPicker({
  value,
  choices,
  ariaLabel,
  disabled = false,
  onChange,
  onOpen,
}: Props) {
  const trigger = useRef<HTMLButtonElement>(null);
  const menu = useRef<HTMLDivElement>(null);
  const search = useRef<HTMLInputElement>(null);
  const [open, setOpen] = useState(false);
  const [query, setQuery] = useState("");
  const [active, setActive] = useState(-1);
  const [anchor, setAnchor] = useState<{
    left: number;
    top: number;
    width: number;
    maxHeight: number;
  } | null>(null);

  const selected = choices.find((choice) => choice.value === value);
  const list = choices.filter((choice) =>
    bandSetMatchesQuery(choice.label, choice.hint ?? "", query),
  );

  const close = () => {
    setOpen(false);
    setQuery("");
    setActive(-1);
  };

  const show = () => {
    const box = trigger.current?.getBoundingClientRect();
    if (!box) return;
    const width = Math.min(Math.max(box.width, 240), window.innerWidth - 16);
    const left = Math.max(
      8,
      Math.min(box.left, window.innerWidth - width - 8),
    );
    setAnchor({
      left,
      top: box.bottom + 2,
      width,
      maxHeight: Math.max(80, window.innerHeight - box.bottom - 12),
    });
    setQuery("");
    setActive(-1);
    setOpen(true);
    onOpen?.();
  };

  useEffect(() => {
    if (open) search.current?.focus({ preventScroll: true });
  }, [open]);

  const pick = (choice: BandSetChoice) => {
    onChange(choice.value);
    close();
    trigger.current?.focus();
  };

  useEffect(() => {
    if (!open) return;
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
    <div class="bandset-picker">
      <button
        ref={trigger}
        type="button"
        class="bandset-picker-trigger"
        aria-label={ariaLabel}
        aria-haspopup="listbox"
        aria-expanded={open}
        disabled={disabled}
        onClick={() => (open ? close() : show())}
      >
        <span class="name">{selected?.label ?? "—"}</span>
        {selected?.hint && <small>{selected.hint}</small>}
        <span class="caret" aria-hidden="true">
          ▾
        </span>
      </button>
      {open && anchor && (
        <div
          ref={menu}
          class="page-suggest-menu bandset-picker-menu"
          role="listbox"
          aria-label={ariaLabel}
          style={{
            left: `${anchor.left}px`,
            top: `${anchor.top}px`,
            width: `${anchor.width}px`,
            maxHeight: `${Math.min(320, anchor.maxHeight)}px`,
          }}
        >
          <input
            ref={search}
            class="bandset-picker-search"
            value={query}
            placeholder="Filter band sets…"
            aria-label="Filter band sets"
            spellcheck={false}
            onInput={(event) => {
              setQuery((event.target as HTMLInputElement).value);
              setActive(-1);
            }}
            onKeyDown={(event) => {
              if (event.key === "ArrowDown" || event.key === "ArrowUp") {
                event.preventDefault();
                const step = event.key === "ArrowDown" ? 1 : -1;
                setActive((old) => {
                  if (list.length === 0) return -1;
                  const next = old + step;
                  return next < 0
                    ? list.length - 1
                    : next >= list.length
                      ? 0
                      : next;
                });
                return;
              }
              if (event.key === "Enter") {
                event.preventDefault();
                if (active >= 0 && list[active]) pick(list[active]);
                return;
              }
              if (event.key === "Escape") {
                event.preventDefault();
                // Keep the modal's Esc handler from closing the whole dialog.
                event.stopPropagation();
                close();
                trigger.current?.focus();
                return;
              }
              if (event.key === "Tab") close();
            }}
          />
          <div class="bandset-picker-list">
            {list.length === 0 && (
              <span class="bandset-picker-empty">No matching band sets</span>
            )}
            {list.map((choice, index) => (
              <button
                key={choice.value || "__none"}
                type="button"
                role="option"
                aria-selected={choice.value === value}
                class={index === active ? "active" : ""}
                title={
                  choice.hint ? `${choice.label} · ${choice.hint}` : choice.label
                }
                onMouseDown={(event) => event.preventDefault()}
                onClick={() => pick(choice)}
              >
                <span class="name">{choice.label}</span>
                {choice.hint && <small>{choice.hint}</small>}
              </button>
            ))}
          </div>
        </div>
      )}
    </div>
  );
}
