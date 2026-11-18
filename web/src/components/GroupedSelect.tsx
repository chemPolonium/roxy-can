import { useEffect, useMemo, useRef, useState } from "preact/hooks";
import { filterGroupedOptions, type GroupedOption } from "../groupedOptions";

interface Props {
  options: GroupedOption[];
  valueKey: string;
  valueLabel: string;
  placeholder: string;
  searchPlaceholder?: string;
  ariaLabel: string;
  disabled?: boolean;
  className?: string;
  menuWidth?: number;
  noMatchText?: string;
  onPick: (option: GroupedOption) => void;
}

let selectSeq = 0;

function pickLabel(option: GroupedOption): string {
  return `${option.group} · ${option.label}`;
}

export function GroupedSelect({
  options,
  valueKey,
  valueLabel,
  placeholder,
  searchPlaceholder = "Search…",
  ariaLabel,
  disabled = false,
  className = "",
  menuWidth = 340,
  noMatchText = "No matching signals",
  onPick,
}: Props) {
  const input = useRef<HTMLInputElement>(null);
  const menu = useRef<HTMLDivElement>(null);
  const [menuId] = useState(() => `grouped-select-${++selectSeq}`);
  const [open, setOpen] = useState(false);
  const [query, setQuery] = useState("");
  const [active, setActive] = useState(-1);
  const [anchor, setAnchor] = useState<{
    left: number;
    top?: number;
    bottom?: number;
    width: number;
    maxHeight: number;
  } | null>(null);

  const groups = useMemo(
    () => filterGroupedOptions(options, open ? query : ""),
    [options, query, open],
  );
  const flat = useMemo(
    () => groups.flatMap((group) => group.options),
    [groups],
  );
  const indexes = useMemo(
    () => new Map(flat.map((option, index) => [option.key, index])),
    [flat],
  );

  const close = () => {
    setOpen(false);
    setQuery("");
    setActive(-1);
  };

  const show = () => {
    const box = input.current?.getBoundingClientRect();
    if (!box) return;
    const width = Math.min(
      Math.max(box.width, menuWidth),
      520,
      Math.max(220, window.innerWidth - 16),
    );
    const left = Math.max(8, Math.min(box.left, window.innerWidth - width - 8));
    const below = window.innerHeight - box.bottom;
    const above = box.top;
    const openUp = below < 180 && above > below;
    const maxHeight = Math.max(
      110,
      Math.min(300, (openUp ? above : below) - 10),
    );
    setAnchor(
      openUp
        ? {
            left,
            bottom: window.innerHeight - box.top + 2,
            width,
            maxHeight,
          }
        : { left, top: box.bottom + 2, width, maxHeight },
    );
    setOpen(true);
  };

  useEffect(() => {
    if (!open) return;
    const inside = (node: Node | null) =>
      !!node && (input.current?.contains(node) || menu.current?.contains(node));
    const closeOnPointer = (event: PointerEvent) => {
      if (!inside(event.target as Node)) close();
    };
    const closeOnScroll = (event: Event) => {
      if (!inside(event.target as Node)) close();
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

  useEffect(() => {
    if (!open || active < 0) return;
    const element = menu.current?.querySelector<HTMLElement>(
      `[data-index="${active}"]`,
    );
    element?.scrollIntoView({ block: "nearest" });
  }, [active, open]);

  const pick = (option: GroupedOption) => {
    onPick(option);
    close();
    input.current?.blur();
  };

  const openFromKeyboard = () => {
    setQuery("");
    setActive(-1);
    show();
  };

  return (
    <div class={`grouped-select ${className}`.trim()}>
      <input
        ref={input}
        class="grouped-select-input"
        role="combobox"
        aria-label={ariaLabel}
        aria-expanded={open}
        aria-controls={menuId}
        aria-autocomplete="list"
        aria-activedescendant={
          open && active >= 0 ? `${menuId}-${active}` : undefined
        }
        autoComplete="off"
        spellcheck={false}
        disabled={disabled}
        value={open ? query : valueLabel}
        placeholder={open ? searchPlaceholder : placeholder}
        title={valueLabel || placeholder}
        onFocus={() => {
          if (disabled) return;
          setQuery("");
          setActive(-1);
          show();
        }}
        onClick={() => {
          if (disabled || open) return;
          setQuery("");
          setActive(-1);
          show();
        }}
        onBlur={(event) => {
          const next = event.relatedTarget as Node | null;
          if (next && menu.current?.contains(next)) return;
          close();
        }}
        onInput={(event) => {
          setQuery((event.target as HTMLInputElement).value);
          setActive(-1);
          show();
        }}
        onKeyDown={(event) => {
          if (event.key === "ArrowDown" || event.key === "ArrowUp") {
            event.preventDefault();
            if (!open) {
              openFromKeyboard();
              return;
            }
            const step = event.key === "ArrowDown" ? 1 : -1;
            setActive((old) => {
              if (flat.length === 0) return -1;
              if (old < 0) return step > 0 ? 0 : flat.length - 1;
              const next = old + step;
              return next < 0 ? flat.length - 1 : next >= flat.length ? 0 : next;
            });
            return;
          }
          if (event.key === "Enter" && open) {
            event.preventDefault();
            const chosen =
              flat[active] ?? (flat.length === 1 ? flat[0] : undefined);
            if (chosen) pick(chosen);
            else close();
            return;
          }
          if (event.key === "Escape" && open) {
            event.preventDefault();
            event.stopPropagation();
            close();
            input.current?.blur();
          }
        }}
      />
      <span class="grouped-select-arrow" aria-hidden="true">
        ▾
      </span>
      {open && anchor && (
        <div
          ref={menu}
          id={menuId}
          class="grouped-select-menu"
          role="listbox"
          aria-label={ariaLabel}
          style={{
            left: `${anchor.left}px`,
            ...(anchor.top != null ? { top: `${anchor.top}px` } : {}),
            ...(anchor.bottom != null
              ? { bottom: `${anchor.bottom}px` }
              : {}),
            width: `${anchor.width}px`,
            maxHeight: `${anchor.maxHeight}px`,
          }}
        >
          {flat.length === 0 ? (
            <div class="grouped-select-empty">{noMatchText}</div>
          ) : (
            groups.map((group) => (
              <div class="grouped-select-group" key={group.key}>
                <div class="grouped-select-head">
                  <strong>{group.label}</strong>
                  {group.meta && <span>{group.meta}</span>}
                </div>
                {group.options.map((option) => {
                  const index = indexes.get(option.key) ?? -1;
                  const selected = option.key === valueKey;
                  return (
                    <button
                      type="button"
                      role="option"
                      tabIndex={-1}
                      id={`${menuId}-${index}`}
                      data-index={index}
                      aria-selected={selected}
                      class={`grouped-select-option ${
                        index === active ? "active" : ""
                      } ${selected ? "on" : ""}`}
                      key={option.key}
                      title={pickLabel(option)}
                      onMouseDown={(event) => event.preventDefault()}
                      onClick={() => pick(option)}
                    >
                      <span class="grouped-select-label">{option.label}</span>
                      {option.meta && (
                        <small class="grouped-select-meta">{option.meta}</small>
                      )}
                      {selected && (
                        <b class="grouped-select-check" aria-hidden="true">
                          ✓
                        </b>
                      )}
                    </button>
                  );
                })}
              </div>
            ))
          )}
        </div>
      )}
    </div>
  );
}
