import type { Bus, ReplayMap, ReplayMapRow } from "../types";
import {
  replayMapTotals,
  replayRowView,
  replaySelectChoice,
  replaySelectValue,
  type ReplayChoice,
  type ReplayRowView,
} from "../replayTargets";

interface Props {
  map: ReplayMap;
  buses: Bus[];
  logName: string | null;
  onSet: (kind: string, raw: number, target: ReplayChoice) => void;
}

interface Group {
  key: "can" | "fr" | "eth";
  title: string;
  busKind: Bus["kind"];
  channel: (raw: number) => string;
}

const GROUPS: Group[] = [
  {
    key: "can",
    title: "CAN",
    busKind: "can",
    channel: (raw) => `Channel ${raw + 1}`,
  },
  {
    key: "fr",
    title: "FlexRay",
    busKind: "flexray",
    channel: (raw) => `#${raw}`,
  },
  {
    key: "eth",
    title: "Ethernet",
    busKind: "ethernet",
    channel: (raw) => `#${raw}`,
  },
];

function frameText(count: number): string {
  return `${count.toLocaleString()} frame${count === 1 ? "" : "s"}`;
}

function slotName(kind: Bus["kind"], index: number): string {
  return kind === "can"
    ? `CAN${index + 1}`
    : kind === "flexray"
      ? `FR${index}`
      : `ETH${index}`;
}

function landing(row: ReplayRowView, targets: Bus[], kind: Bus["kind"]) {
  if (row.target == null) return { text: "not replayed", missing: true };
  const bus = targets.find((entry) => entry.index === row.target);
  return { text: bus?.name ?? slotName(kind, row.target), missing: false };
}

function rowTitle(row: ReplayRowView): string {
  if (row.absent) {
    return "Saved for this project, but this recording carries no frames on this channel";
  }
  if (row.unmatched) {
    return "The recording carries this channel, but the project has no bus at that number";
  }
  return "Which project bus replays this log channel";
}

function rowsOf(map: ReplayMap, key: Group["key"]): ReplayMapRow[] {
  return map[key];
}

export function ReplayChannelMap({ map, buses, logName, onSet }: Props) {
  const totals = replayMapTotals(map);
  const groups = GROUPS.map((group) => ({
    ...group,
    targets: buses.filter((bus) => bus.kind === group.busKind),
    rows: rowsOf(map, group.key).map(replayRowView),
  })).filter((group) => group.rows.length > 0);

  return (
    <section class="session-task channel-map" id="replay-channel-map">
      <h3>
        Channel map
        <span class="channel-map-meta">
          {totals.channels === 0
            ? "no channels yet"
            : `${totals.routed} of ${totals.channels} channel${
                totals.channels === 1 ? "" : "s"
              } routed`}
        </span>
      </h3>

      {totals.channels === 0 ? (
        <p class="hint">
          {logName
            ? "This recording reports no channel numbers."
            : "Pick a recording above; the channel numbers it carries are listed here."}
        </p>
      ) : (
        groups.map((group) => (
          <div class="replay-group" key={group.key}>
            <span class="replay-group-title">
              {group.title} · {group.rows.length} channel
              {group.rows.length === 1 ? "" : "s"}
            </span>
            {group.rows.map((row) => {
              const land = landing(row, group.targets, group.busKind);
              const classes = ["replay-row"];
              if (row.absent) classes.push("absent");
              if (row.unmatched) classes.push("unmatched");
              return (
                <div
                  class={classes.join(" ")}
                  key={row.raw}
                  title={rowTitle(row)}
                >
                  <span class="replay-channel">{group.channel(row.raw)}</span>
                  <em class="replay-frames">{frameText(row.count)}</em>
                  <span
                    class={`replay-lands${land.missing ? " none" : ""}`}
                  >
                    {land.text}
                  </span>
                  <select
                    value={replaySelectValue(row.choice)}
                    aria-label={`${group.channel(row.raw)} replays on`}
                    title="Choose the project bus that replays this log channel"
                    onChange={(event) => {
                      const choice = replaySelectChoice(
                        (event.target as HTMLSelectElement).value,
                      );
                      onSet(group.key, row.raw, choice);
                    }}
                  >
                    <option value="same">As-is</option>
                    {group.targets.map((target) => (
                      <option key={target.index} value={`b${target.index}`}>
                        {target.name}
                      </option>
                    ))}
                    <option value="drop">No replay</option>
                  </select>
                </div>
              );
            })}
          </div>
        ))
      )}

      <span class="task-state">
        Saved with the project and applied to every Offline replay. As-is
        keeps the log's channel number as the bus number.
      </span>
      {totals.droppedFrames > 0 && (
        <span class="task-state warn">
          {frameText(totals.droppedFrames)} not replayed.
        </span>
      )}
      {totals.absent > 0 && (
        <span class="task-state">
          {totals.absent} saved row{totals.absent === 1 ? "" : "s"} not in this
          recording.
        </span>
      )}
    </section>
  );
}
