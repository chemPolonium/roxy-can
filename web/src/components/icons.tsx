/// Small line icons for header/toolbar actions. Every glyph is drawn on the
/// same 16px grid with the stroke style used by the other chrome icons, so a
/// row of buttons reads as one set. Text labels stay in `title`/`aria-label`.
export type UiIconName =
  | "book"
  | "download"
  | "funnel"
  | "columns"
  | "clipboard"
  | "chevrons-down"
  | "chevrons-up"
  | "arrow-up"
  | "plus-signal"
  | "cursor"
  | "layers"
  | "file"
  | "play"
  | "stop"
  | "pause"
  | "trash"
  | "check"
  | "close";

export function UiIcon({ name }: { name: UiIconName }) {
  switch (name) {
    case "book":
      return (
        <svg viewBox="0 0 16 16" aria-hidden="true">
          <path d="M3 3.5h4a2 2 0 0 1 2 2V13a2 2 0 0 0-2-2H3z" />
          <path d="M13 3.5H9a2 2 0 0 0-2 2V13a2 2 0 0 1 2-2h4z" />
        </svg>
      );
    case "download":
      return (
        <svg viewBox="0 0 16 16" aria-hidden="true">
          <path d="M8 2.5v7M5 6.5 8 9.5l3-3M3 12.5h10" />
        </svg>
      );
    case "funnel":
      return (
        <svg viewBox="0 0 16 16" aria-hidden="true">
          <path d="M2.5 3h11l-4.3 5.2v3.6l-2.4 1.2V8.2z" />
        </svg>
      );
    case "columns":
      return (
        <svg viewBox="0 0 16 16" aria-hidden="true">
          <rect x="2.5" y="3" width="11" height="10" />
          <path d="M6 3v10M10 3v10" />
        </svg>
      );
    case "clipboard":
      return (
        <svg viewBox="0 0 16 16" aria-hidden="true">
          <rect x="3.5" y="3" width="9" height="10.5" />
          <path d="M6 3V1.8h4V3M6 6.5h4M6 9h4" />
        </svg>
      );
    case "chevrons-down":
      return (
        <svg viewBox="0 0 16 16" aria-hidden="true">
          <path d="M4 5l4 4 4-4M4 9l4 4 4-4" />
        </svg>
      );
    case "chevrons-up":
      return (
        <svg viewBox="0 0 16 16" aria-hidden="true">
          <path d="M4 11l4-4 4 4M4 7l4-4 4 4" />
        </svg>
      );
    case "arrow-up":
      return (
        <svg viewBox="0 0 16 16" aria-hidden="true">
          <path d="M8 13V3M4 7l4-4 4 4" />
        </svg>
      );
    case "plus-signal":
      return (
        <svg viewBox="0 0 16 16" aria-hidden="true">
          <path d="M2 8c1.3-3.5 2.7-3.5 4 0s2.7 3.5 4 0" />
          <path d="M12.5 5v5M10 7.5h5" />
        </svg>
      );
    case "cursor":
      return (
        <svg viewBox="0 0 16 16" aria-hidden="true">
          <path d="M5.5 13V3h7l-2 2.7 2 2.7h-7" />
        </svg>
      );
    case "layers":
      return (
        <svg viewBox="0 0 16 16" aria-hidden="true">
          <path d="M8 2.5 13.5 5.5 8 8.5 2.5 5.5z" />
          <path d="M2.5 8.7 8 11.7l5.5-3" />
        </svg>
      );
    case "file":
      return (
        <svg viewBox="0 0 16 16" aria-hidden="true">
          <path d="M4 2.5h5l3 3v8H4z" />
          <path d="M9 2.5v3h3" />
        </svg>
      );
    case "play":
      return (
        <svg viewBox="0 0 16 16" aria-hidden="true">
          <path d="M5.5 3.5 12 8l-6.5 4.5z" />
        </svg>
      );
    case "stop":
      return (
        <svg viewBox="0 0 16 16" aria-hidden="true">
          <rect x="4.5" y="4.5" width="7" height="7" />
        </svg>
      );
    case "pause":
      return (
        <svg viewBox="0 0 16 16" aria-hidden="true">
          <path d="M5.5 4v8M10.5 4v8" />
        </svg>
      );
    case "trash":
      return (
        <svg viewBox="0 0 16 16" aria-hidden="true">
          <path d="M3 4.5h10M6.5 2.5h3M5 4.5l.6 9h4.8l.6-9M7 7v4M9 7v4" />
        </svg>
      );
    case "check":
      return (
        <svg viewBox="0 0 16 16" aria-hidden="true">
          <path d="M3.5 8.4 6.6 11.5 12.5 4.5" />
        </svg>
      );
    case "close":
      return (
        <svg viewBox="0 0 16 16" aria-hidden="true">
          <path d="M4.5 4.5l7 7M11.5 4.5l-7 7" />
        </svg>
      );
  }
}
