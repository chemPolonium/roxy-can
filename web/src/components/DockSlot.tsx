import { useEffect, useRef, useState } from "preact/hooks";
import type { ComponentChildren, JSX } from "preact";
import { MenuButton } from "./MenuButton";
import {
  PANELS,
  layoutInfo,
  panelLabel,
  type DockState,
  type PanelId,
} from "../dockLayout";
import type { Theme } from "../theme";

function CheckIcon() {
  return (
    <svg viewBox="0 0 16 16" aria-hidden="true">
      <path d="M3.5 8.4 6.6 11.5 12.5 4.5" />
    </svg>
  );
}

function MaximizeIcon({ restore }: { restore: boolean }) {
  return (
    <svg viewBox="0 0 16 16" aria-hidden="true">
      {restore ? (
        <path d="M6 2v4H2M10 2v4h4M6 14v-4H2M10 14v-4h4" />
      ) : (
        <path d="M2 6V2h4M14 6V2h-4M2 10v4h4M14 10v4h-4" />
      )}
    </svg>
  );
}

function ThemeIcon({ theme }: { theme: Theme }) {
  return (
    <svg viewBox="0 0 16 16" aria-hidden="true">
      {theme === "dark" ? (
        <path d="M8 1.5v2M8 12.5v2M1.5 8h2M12.5 8h2M3.4 3.4l1.4 1.4M11.2 11.2l1.4 1.4M12.6 3.4l-1.4 1.4M4.8 11.2l-1.4 1.4M8 5a3 3 0 1 0 3 3" />
      ) : (
        <path d="M13 10.2A5.6 5.6 0 0 1 5.8 3a5.5 5.5 0 1 0 7.2 7.2z" />
      )}
    </svg>
  );
}

/// One glyph per page. The header keeps only this icon so the panel chooser
/// costs about 25 px instead of a page name; the name stays in the trigger
/// tooltip, its accessible label and the chooser menu.
function PanelIcon({ panel }: { panel: PanelId }) {
  switch (panel) {
    case "trace":
      return (
        <svg viewBox="0 0 16 16" aria-hidden="true">
          <path d="M2 4h9M2 8h12M2 12h7" />
        </svg>
      );
    case "messages":
      return (
        <svg viewBox="0 0 16 16" aria-hidden="true">
          <path d="M3 3.5h10v6.5H8l-3 2.5v-2.5H3z" />
          <path d="M5.5 6h5M5.5 8h3" />
        </svg>
      );
    case "rate":
      return (
        <svg viewBox="0 0 16 16" aria-hidden="true">
          <path d="M2.5 13.5V2.5M2.5 13.5h11" />
          <path d="M4.5 11 7 7.5l2 2L13 4" />
        </svg>
      );
    case "signals":
      return (
        <svg viewBox="0 0 16 16" aria-hidden="true">
          <path d="M2 8c1.5-5 4.5-5 6 0s4.5 5 6 0" />
        </svg>
      );
    case "monitor":
      return (
        <svg viewBox="0 0 16 16" aria-hidden="true">
          <path d="M2.5 12a5.5 5.5 0 1 1 11 0" />
          <path d="M8 12l3-4" />
        </svg>
      );
    case "data":
      return (
        <svg viewBox="0 0 16 16" aria-hidden="true">
          <rect x="2.5" y="3.5" width="11" height="9" />
          <path d="M2.5 6.5h11M6.5 6.5v6" />
        </svg>
      );
    case "state":
      return (
        <svg viewBox="0 0 16 16" aria-hidden="true">
          <rect x="2.5" y="3.5" width="11" height="3.5" />
          <rect x="2.5" y="9" width="11" height="3.5" />
        </svg>
      );
    case "statistics":
      return (
        <svg viewBox="0 0 16 16" aria-hidden="true">
          <path d="M3 13.5V9M6.5 13.5V5M10 13.5V10.5M13.5 13.5V2.5" />
        </svg>
      );
    case "bus":
      return (
        <svg viewBox="0 0 16 16" aria-hidden="true">
          <path d="M2 5h12M2 11h12M5 5v6M11 5v6" />
        </svg>
      );
    case "spec":
      return (
        <svg viewBox="0 0 16 16" aria-hidden="true">
          <rect x="2.5" y="2.5" width="11" height="11" />
          <path d="M5 8.5l2 2 4-4.5" />
        </svg>
      );
    case "someip":
      return (
        <svg viewBox="0 0 16 16" aria-hidden="true">
          <circle cx="4" cy="4" r="1.5" />
          <circle cx="12" cy="4" r="1.5" />
          <circle cx="8" cy="12" r="1.5" />
          <path d="M5.2 5.2 7 10.4M10.8 5.2 9 10.4" />
        </svg>
      );
    case "write":
      return (
        <svg viewBox="0 0 16 16" aria-hidden="true">
          <rect x="2.5" y="3" width="11" height="10" />
          <path d="M5 7l2 2-2 2M8.5 11h3" />
        </svg>
      );
    default:
      return null;
  }
}

interface DockSlotSurfaceProps {
  panel: PanelId;
  slot: number;
  maximized: boolean;
  style?: JSX.CSSProperties;
  onFocus: () => void;
  onDragOver?: (event: DragEvent) => void;
  onDrop?: (event: DragEvent) => void;
  children: ComponentChildren;
}

/// One page container. Panels own their content height, but a narrow page
/// needs its own wrapping decisions (a Trace toolbar does not fit a quad
/// slot even on a 1280 screen), so the surface measures itself instead of
/// relying on the viewport width.
export function DockSlotSurface({
  panel,
  slot,
  maximized,
  style,
  onFocus,
  onDragOver,
  onDrop,
  children,
}: DockSlotSurfaceProps) {
  const ref = useRef<HTMLElement>(null);
  const [narrow, setNarrow] = useState(false);
  useEffect(() => {
    const element = ref.current;
    if (!element) return;
    const update = () => setNarrow(element.clientWidth < 720);
    update();
    const observer = new ResizeObserver(update);
    observer.observe(element);
    return () => observer.disconnect();
  }, []);
  return (
    <section
      ref={ref}
      class={`panel dock-slot ${
        panel === "trace" ? "trace-panel trace" : ""
      } ${maximized ? "dock-maximized" : ""} ${narrow ? "dock-narrow" : ""}`}
      data-panel={panel}
      data-slot={slot}
      style={style}
      onPointerDownCapture={onFocus}
      onDragOver={onDragOver}
      onDrop={onDrop}
    >
      {children}
    </section>
  );
}

interface SlotMenuProps {
  dock: DockState;
  slot: number;
  panel: PanelId;
  onPick: (panel: PanelId) => void;
  onMove: (to: number) => void;
}

export function SlotMenu({ dock, slot, panel, onPick, onMove }: SlotMenuProps) {
  const info = layoutInfo(dock.layout);
  const slots = dock.panels[dock.layout];
  return (
    <MenuButton
      className="slot-menu-wrap"
      buttonClass="slot-panel-trigger"
      menuClass="slot-panel-menu"
      fixed
      ariaLabel={`Panel for ${info.slotLabels[slot] ?? `slot ${slot + 1}`}: ${panelLabel(panel)}`}
      title={`${panelLabel(panel)} — choose this page's panel, or move it to another slot`}
      menuLabel="Panel and slot"
      label={
        <>
          <span class="panel-glyph">
            <PanelIcon panel={panel} />
          </span>
          <span class="top-menu-caret" aria-hidden="true">
            ▾
          </span>
        </>
      }
    >
      {(close) => (
        <>
          {(["Stream", "Live", "Analysis"] as const).map((group) => (
            <div key={group}>
              <div class="app-menu-group">{group}</div>
              {PANELS.filter((entry) => entry.group === group).map((entry) => {
                const usedAt = slots.indexOf(entry.id);
                const active = entry.id === panel;
                return (
                  <button
                    type="button"
                    key={entry.id}
                    class={active ? "on" : ""}
                    aria-pressed={active}
                    title={entry.hint}
                    onClick={() => {
                      onPick(entry.id);
                      close();
                    }}
                  >
                    <span class="slot-menu-check">
                      {active ? <CheckIcon /> : null}
                    </span>
                    <span class="slot-menu-icon" aria-hidden="true">
                      <PanelIcon panel={entry.id} />
                    </span>
                    <span class="slot-menu-label">{entry.label}</span>
                    {usedAt >= 0 && !active && (
                      <em>{info.slotLabels[usedAt]}</em>
                    )}
                  </button>
                );
              })}
            </div>
          ))}
          {slots.length > 1 && (
            <>
              <div class="app-menu-sep" />
              <div class="app-menu-group">Move to</div>
              {slots.map((other, index) =>
                index === slot ? null : (
                  <button
                    type="button"
                    key={index}
                    onClick={() => {
                      onMove(index);
                      close();
                    }}
                  >
                    <span class="slot-menu-check" />
                    <span>{info.slotLabels[index]}</span>
                    <em>{panelLabel(other)}</em>
                  </button>
                ),
              )}
            </>
          )}
        </>
      )}
    </MenuButton>
  );
}

export function SlotMaximizeButton({
  maximized,
  onClick,
  theme,
  onTheme,
}: {
  maximized: boolean;
  onClick: () => void;
  theme?: Theme;
  onTheme?: () => void;
}) {
  return (
    <>
      <button
        type="button"
        class="chart-focus dock-maximize"
        aria-label={maximized ? "Restore this page" : "Maximize this page"}
        title={maximized ? "Restore this page" : "Maximize this page"}
        onClick={onClick}
      >
        <MaximizeIcon restore={maximized} />
      </button>
      {maximized && theme && onTheme && (
        <button
          type="button"
          class="chart-theme"
          aria-label={
            theme === "dark" ? "Switch to light theme" : "Switch to dark theme"
          }
          title={
            theme === "dark" ? "Switch to light theme" : "Switch to dark theme"
          }
          onClick={onTheme}
        >
          <ThemeIcon theme={theme} />
        </button>
      )}
    </>
  );
}

interface SlotHeaderProps {
  dock: DockState;
  slot: number;
  panel: PanelId;
  onPick: (panel: PanelId) => void;
  onMove: (to: number) => void;
  onMaximize: () => void;
  maximized: boolean;
  theme: Theme;
  onTheme: () => void;
  leading?: ComponentChildren;
  meta?: ComponentChildren;
  actions?: ComponentChildren;
}

/// One row shared by every page: panel chooser on the left, page-specific
/// actions on the right, and a single maximize affordance that replaces the
/// old chart-only fullscreen.
export function SlotHeader({
  dock,
  slot,
  panel,
  onPick,
  onMove,
  onMaximize,
  maximized,
  theme,
  onTheme,
  leading,
  meta,
  actions,
}: SlotHeaderProps) {
  return (
    <div class="panel-head dock-head">
      <SlotMenu
        dock={dock}
        slot={slot}
        panel={panel}
        onPick={onPick}
        onMove={onMove}
      />
      {leading}
      <span class="dock-head-spacer" />
      {meta && <span class="meta">{meta}</span>}
      {actions && <div class="dock-head-actions">{actions}</div>}
      <SlotMaximizeButton
        maximized={maximized}
        onClick={onMaximize}
        theme={theme}
        onTheme={onTheme}
      />
    </div>
  );
}
