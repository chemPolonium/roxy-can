import type { ReplayMap, ReplayMapRow } from "./types";

/// One channel-map choice. `"same"` means "use the log's number as the bus
/// number" and stores nothing, so the project stays portable; a number names
/// the bus index the channel lands on.
export type ReplayChoice = "same" | "drop" | number;

export interface ReplayRowView {
  raw: number;
  count: number;
  /// The bus the core actually uses; null = these frames are dropped.
  target: number | null;
  choice: ReplayChoice;
  /// As-is, but the project has no bus at that number.
  unmatched: boolean;
  /// Saved for the project, but this recording carries none of it.
  absent: boolean;
}

/// The stored choice, not the effective one: a row that was never configured
/// stays "same" even when no bus answers to its number.
export function replayChoice(row: ReplayMapRow): ReplayChoice {
  if (row.configured !== true) return "same";
  return row.target == null ? "drop" : row.target;
}

export function replayRowView(row: ReplayMapRow): ReplayRowView {
  const choice = replayChoice(row);
  return {
    raw: row.raw,
    count: row.count,
    target: row.target,
    choice,
    unmatched: choice === "same" && row.target == null,
    absent: row.count === 0,
  };
}

export function replaySelectValue(choice: ReplayChoice): string {
  if (choice === "same") return "same";
  if (choice === "drop") return "drop";
  return `b${choice}`;
}

export function replaySelectChoice(value: string): ReplayChoice {
  if (value === "same") return "same";
  if (value === "drop") return "drop";
  const bus = Number(value.slice(1));
  return Number.isFinite(bus) ? bus : "same";
}

export interface ReplayMapTotals {
  channels: number;
  routed: number;
  /// Channels with frames that land nowhere.
  dropped: number;
  droppedFrames: number;
  /// Rows saved for the project that this recording does not carry.
  absent: number;
}

export function replayMapTotals(map: ReplayMap): ReplayMapTotals {
  const rows = [...map.can, ...map.fr, ...map.eth];
  let routed = 0;
  let dropped = 0;
  let droppedFrames = 0;
  let absent = 0;
  for (const row of rows) {
    const view = replayRowView(row);
    if (view.target != null) routed += 1;
    if (view.absent) absent += 1;
    if (view.target == null) {
      droppedFrames += view.count;
      if (view.count > 0) dropped += 1;
    }
  }
  return { channels: rows.length, routed, dropped, droppedFrames, absent };
}
