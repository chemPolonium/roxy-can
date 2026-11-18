import type { MessageSignal } from "./types";

/// Large frames get a filter row; shorter frames stay quiet.
export const MESSAGE_SIGNAL_FILTER_MIN = 16;

/// Rendering a 500-signal frame at once makes expansion crawl. The filter and
/// "Show all" row keep the first paint cheap.
export const MESSAGE_SIGNAL_INITIAL_ROWS = 50;

function matches(signal: MessageSignal, needle: string): boolean {
  return (
    signal.name.toLowerCase().includes(needle) ||
    (signal.label ?? "").toLowerCase().includes(needle) ||
    signal.unit.toLowerCase().includes(needle)
  );
}

export function messageSignalRows(
  signals: MessageSignal[],
  query: string,
  showAll: boolean,
): {
  visible: MessageSignal[];
  matches: number;
  total: number;
  hidden: number;
} {
  const needle = query.trim().toLowerCase();
  const matched = needle
    ? signals.filter((signal) => matches(signal, needle))
    : signals;
  const visible = showAll
    ? matched
    : matched.slice(0, MESSAGE_SIGNAL_INITIAL_ROWS);
  return {
    visible,
    matches: matched.length,
    total: signals.length,
    hidden: matched.length - visible.length,
  };
}
