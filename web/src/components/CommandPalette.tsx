import { useEffect, useMemo, useRef, useState } from "preact/hooks";

export interface PaletteCommand {
  id: string;
  label: string;
  group: string;
  hint?: string;
  run: () => void;
}

export function CommandPalette({
  commands,
  onClose,
}: {
  commands: PaletteCommand[];
  onClose: () => void;
}) {
  const [query, setQuery] = useState("");
  const [active, setActive] = useState(0);
  const inputRef = useRef<HTMLInputElement>(null);

  const filtered = useMemo(() => {
    const needle = query.trim().toLowerCase();
    if (!needle) return commands;
    return commands.filter((command) =>
      `${command.label} ${command.group} ${command.hint ?? ""}`
        .toLowerCase()
        .includes(needle),
    );
  }, [commands, query]);

  useEffect(() => {
    inputRef.current?.focus();
  }, []);

  useEffect(() => {
    setActive(0);
  }, [query]);

  useEffect(() => {
    const close = (event: KeyboardEvent) => {
      if (event.key === "Escape") onClose();
    };
    window.addEventListener("keydown", close);
    return () => window.removeEventListener("keydown", close);
  }, [onClose]);

  const run = (command: PaletteCommand | undefined) => {
    if (!command) return;
    onClose();
    command.run();
  };

  return (
    <div class="modal-backdrop" onMouseDown={onClose}>
      <div
        class="palette"
        role="dialog"
        aria-modal="true"
        aria-label="Command palette"
        onMouseDown={(event) => event.stopPropagation()}
      >
        <input
          ref={inputRef}
          type="search"
          value={query}
          placeholder="Type a command…"
          aria-label="Command search"
          onInput={(event) => setQuery((event.target as HTMLInputElement).value)}
          onKeyDown={(event) => {
            if (event.key === "ArrowDown") {
              event.preventDefault();
              setActive((old) => Math.min(filtered.length - 1, old + 1));
            } else if (event.key === "ArrowUp") {
              event.preventDefault();
              setActive((old) => Math.max(0, old - 1));
            } else if (event.key === "Enter") {
              event.preventDefault();
              run(filtered[active]);
            }
          }}
        />
        <div class="palette-list" role="listbox">
          {filtered.map((command, index) => (
            <button
              type="button"
              role="option"
              aria-selected={index === active}
              class={`palette-item ${index === active ? "on" : ""}`}
              key={command.id}
              onMouseEnter={() => setActive(index)}
              onClick={() => run(command)}
            >
              <span>
                <strong>{command.label}</strong>
                {command.hint && <small>{command.hint}</small>}
              </span>
              <em>{command.group}</em>
            </button>
          ))}
          {filtered.length === 0 && (
            <div class="palette-empty">No commands match.</div>
          )}
        </div>
        <p class="palette-hint">↑↓ select · Enter run · Esc close</p>
      </div>
    </div>
  );
}
