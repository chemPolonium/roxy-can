import { useEffect, useRef, useState } from "preact/hooks";
import {
  clampPanelNumber,
  filterPanelOptions,
  panelGaugeArc,
  panelMeterRatio,
} from "../panelBinding";
import {
  formatPanelValue,
  type PanelGauge,
  type PanelSetpoint,
} from "../panelPage";
export function SysVarSetpointControl({
  widget,
  value,
  min,
  max,
  unit,
  state,
  disabled,
  reason,
  title,
  onCommit,
}: {
  widget: PanelSetpoint;
  value: number | null;
  min: number | null;
  max: number | null;
  unit: string;
  state?: string | null;
  disabled: boolean;
  reason: string;
  title?: string;
  onCommit: (value: number) => void;
}) {
  const [draft, setDraft] = useState("");
  const [editing, setEditing] = useState(false);
  useEffect(() => {
    if (!editing) {
      setDraft(value === null ? "" : value.toFixed(widget.digits));
    }
  }, [value, editing, widget.digits]);
  const commit = () => {
    setEditing(false);
    const parsed = draft.trim() === "" ? Number.NaN : Number(draft);
    if (!Number.isFinite(parsed)) {
      setDraft(value === null ? "" : value.toFixed(widget.digits));
      return;
    }
    let next = clampPanelNumber(parsed, min, max);
    onCommit(next);
  };
  return (
    <div class="page-setpoint" title={disabled ? reason : title ?? widget.sysvar}>
      {widget.label && <span class="page-setpoint-label">{widget.label}</span>}
      <span class="page-setpoint-row">
        <input
          type="number"
          value={draft}
          step={widget.step}
          disabled={disabled}
          onFocus={() => setEditing(true)}
          onInput={(event) => setDraft((event.target as HTMLInputElement).value)}
          onBlur={commit}
          onKeyDown={(event) => {
            if (event.key === "Enter") {
              (event.target as HTMLInputElement).blur();
            } else if (event.key === "Escape") {
              setDraft(value === null ? "" : value.toFixed(widget.digits));
              (event.target as HTMLInputElement).blur();
            }
          }}
        />
        {state ? (
          <em class="page-setpoint-state" title={unit || undefined}>
            {state}
          </em>
        ) : (
          unit && <em>{unit}</em>
        )}
      </span>
    </div>
  );
}

export function SetpointControl({
  widget,
  value,
  min,
  max,
  unit,
  disabled,
  reason,
  onWrite,
}: {
  widget: PanelSetpoint;
  /// Live value read back from the bus; falls back to the last write.
  value: number | null;
  min: number | null;
  max: number | null;
  unit: string;
  disabled: boolean;
  reason: string;
  onWrite: (value: number) => void;
}) {
  const [sent, setSent] = useState<number | null>(null);
  useEffect(() => {
    setSent(null);
  }, [widget.source]);
  const shape: PanelSetpoint = {
    id: widget.id,
    kind: "setpoint",
    label: widget.label,
    target: "signal",
    sysvar: "",
    source: widget.source,
    digits: widget.digits,
    step: widget.step,
    x: widget.x,
    y: widget.y,
    w: widget.w,
    h: widget.h,
  };
  const frSource =
    widget.source.startsWith("fr:") || widget.source.startsWith("sig:fr:");
  const ethSource =
    widget.source.startsWith("eth:") || widget.source.startsWith("sig:eth:");
  const title = [
    widget.source,
    value !== null ? `value ${formatPanelValue(value, widget.digits)}` : "",
    sent !== null ? `written ${formatPanelValue(sent, widget.digits)}` : "",
    frSource
      ? "writes the FR port when live"
      : ethSource
        ? "session only"
        : "",
  ]
    .filter(Boolean)
    .join(" · ");
  return (
    <SysVarSetpointControl
      widget={shape}
      value={value ?? sent}
      min={min}
      max={max}
      unit={unit}
      disabled={disabled}
      reason={reason}
      title={title}
      onCommit={(next) => {
        setSent(next);
        onWrite(next);
      }}
    />
  );
}

export function GaugeView({
  widget,
  value,
  unit,
  state,
  color,
}: {
  widget: PanelGauge;
  value: number | null;
  unit: string;
  state?: string | null;
  color: string;
}) {
  const ratio = panelMeterRatio(value, widget.min, widget.max);
  return (
    <svg
      class="page-gauge-svg"
      viewBox="0 0 100 64"
      preserveAspectRatio="xMidYMid meet"
      aria-hidden="true"
    >
      <path class="page-gauge-track" d="M6 50 A44 44 0 0 1 94 50" />
      {ratio !== null && ratio > 0 && (
        <path class="page-gauge-fill" style={{ stroke: color }} d={panelGaugeArc(ratio)} />
      )}
      <text class="page-gauge-value" x="50" y="45">
        {state ?? formatPanelValue(value, widget.digits)}
      </text>
      {unit && (
        <text class="page-gauge-unit" x="50" y="57">
          {unit}
        </text>
      )}
      <text class="page-gauge-range" x="6" y="62">
        {formatPanelValue(widget.min, 0)}
      </text>
      <text class="page-gauge-range end" x="94" y="62">
        {formatPanelValue(widget.max, 0)}
      </text>
    </svg>
  );
}

export function SuggestionInput({
  value,
  options,
  placeholder,
  maxLength,
  onFocus,
  onCommit,
}: {
  value: string;
  options: string[];
  placeholder?: string;
  maxLength?: number;
  onFocus: () => void;
  onCommit: (value: string) => void;
}) {
  const input = useRef<HTMLInputElement>(null);
  const menu = useRef<HTMLDivElement>(null);
  const [open, setOpen] = useState(false);
  const [query, setQuery] = useState<string | null>(null);
  const [active, setActive] = useState(-1);
  const [anchor, setAnchor] = useState<{
    left: number;
    top: number;
    width: number;
    maxHeight: number;
  } | null>(null);
  // Focusing an already bound field lists everything; typing narrows it.
  const list = filterPanelOptions(options, query ?? "");

  const show = () => {
    const box = input.current?.getBoundingClientRect();
    if (!box) return;
    setAnchor({
      left: box.left,
      top: box.bottom + 2,
      width: box.width,
      maxHeight: Math.max(60, window.innerHeight - box.bottom - 12),
    });
    setOpen(true);
  };

  useEffect(() => {
    if (!open) return;
    const inside = (node: Node | null) =>
      !!node && (input.current?.contains(node) || menu.current?.contains(node));
    const closeOnPointer = (event: PointerEvent) => {
      if (!inside(event.target as Node)) setOpen(false);
    };
    const closeOnScroll = (event: Event) => {
      if (inside(event.target as Node)) return;
      setOpen(false);
    };
    const closeOnResize = () => setOpen(false);
    window.addEventListener("pointerdown", closeOnPointer);
    window.addEventListener("scroll", closeOnScroll, true);
    window.addEventListener("resize", closeOnResize);
    return () => {
      window.removeEventListener("pointerdown", closeOnPointer);
      window.removeEventListener("scroll", closeOnScroll, true);
      window.removeEventListener("resize", closeOnResize);
    };
  }, [open]);

  const pick = (option: string) => {
    onCommit(option);
    setQuery(null);
    setActive(-1);
    setOpen(false);
  };

  return (
    <div class="page-suggest">
      <input
        ref={input}
        value={value}
        placeholder={placeholder}
        maxLength={maxLength}
        spellcheck={false}
        onFocus={() => {
          onFocus();
          setQuery(null);
          setActive(-1);
          show();
        }}
        onBlur={(event) => {
          const next = event.relatedTarget as Node | null;
          if (next && menu.current?.contains(next)) return;
          setOpen(false);
        }}
        onInput={(event) => {
          const next = (event.target as HTMLInputElement).value;
          setQuery(next);
          setActive(-1);
          show();
          onCommit(next);
        }}
        onKeyDown={(event) => {
          if (event.key === "ArrowDown" || event.key === "ArrowUp") {
            event.preventDefault();
            if (!open) {
              setQuery(null);
              show();
              return;
            }
            const step = event.key === "ArrowDown" ? 1 : -1;
            setActive((old) => {
              if (list.length === 0) return -1;
              const next = old + step;
              return next < 0 ? list.length - 1 : next >= list.length ? 0 : next;
            });
            return;
          }
          if (event.key === "Enter") {
            event.preventDefault();
            if (open && active >= 0 && list[active]) {
              pick(list[active]);
            } else {
              setOpen(false);
            }
            return;
          }
          if (event.key === "Escape" && open) {
            event.preventDefault();
            // Keep the editor's own Esc handler from clearing the selection.
            event.stopPropagation();
            setOpen(false);
            setActive(-1);
          }
        }}
      />
      {open && anchor && list.length > 0 && (
        <div
          ref={menu}
          class="page-suggest-menu"
          role="listbox"
          style={{
            left: `${anchor.left}px`,
            top: `${anchor.top}px`,
            width: `${anchor.width}px`,
            maxHeight: `${Math.min(184, anchor.maxHeight)}px`,
          }}
        >
          {list.map((option, index) => (
            <button
              type="button"
              role="option"
              aria-selected={index === active}
              class={index === active ? "active" : ""}
              key={option}
              title={option}
              onMouseDown={(event) => event.preventDefault()}
              onClick={() => pick(option)}
            >
              {option}
            </button>
          ))}
        </div>
      )}
    </div>
  );
}


