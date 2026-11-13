import { useLayoutEffect, useMemo, useRef, useState } from "preact/hooks";
import type { StatsRow } from "../types";
import {
  ColumnResizeHandle,
  useColumnWidths,
  type ColumnWidthDef,
} from "../tableColumns";

/// The message name absorbs the remaining width; the numeric columns keep
/// their distance and can be dragged, like Trace. Bus/Min/Max appear only when
/// this page (not the whole window) has room for them.
const STATS_COLUMNS: ColumnWidthDef[] = [
  { id: "message", width: 0, min: 0, max: 0, flex: true },
  { id: "bus", width: 50, min: 36, max: 120 },
  { id: "count", width: 60, min: 50, max: 110 },
  { id: "avg", width: 58, min: 48, max: 110 },
  { id: "min", width: 55, min: 45, max: 110 },
  { id: "max", width: 55, min: 45, max: 110 },
  { id: "jitter", width: 60, min: 50, max: 110 },
  { id: "share", width: 50, min: 44, max: 110 },
];

const STATS_COLUMNS_KEY = "roxy-can-web.stats-columns";

type StatsSortKey =
  | "message"
  | "bus"
  | "count"
  | "avg"
  | "min"
  | "max"
  | "jitter"
  | "share";

function statsNumber(value: string): number {
  return Number.parseFloat(value) || 0;
}

function compareStats(a: StatsRow, b: StatsRow, key: StatsSortKey): number {
  switch (key) {
    case "message":
      return a.label.localeCompare(b.label);
    case "bus":
      return a.bus.localeCompare(b.bus);
    case "count":
      return a.count - b.count;
    case "avg":
      return statsNumber(a.avgMs) - statsNumber(b.avgMs);
    case "min":
      return statsNumber(a.minMs) - statsNumber(b.minMs);
    case "max":
      return statsNumber(a.maxMs) - statsNumber(b.maxMs);
    case "jitter":
      return statsNumber(a.jitterMs) - statsNumber(b.jitterMs);
    case "share":
      return statsNumber(a.share) - statsNumber(b.share);
  }
}

export function StatsTable({
  rows,
  filtered = false,
}: {
  rows: StatsRow[];
  filtered?: boolean;
}) {
  const wrapRef = useRef<HTMLDivElement>(null);
  const [available, setAvailable] = useState(0);
  const { widths, startResize } = useColumnWidths(
    STATS_COLUMNS_KEY,
    STATS_COLUMNS,
  );
  const [sortKey, setSortKey] = useState<StatsSortKey | null>(null);
  const [sortDir, setSortDir] = useState<"asc" | "desc">("asc");
  const sortedRows = useMemo(() => {
    if (!sortKey) return rows;
    const sign = sortDir === "asc" ? 1 : -1;
    return [...rows].sort((a, b) => sign * compareStats(a, b, sortKey));
  }, [rows, sortKey, sortDir]);
  const cycleSort = (key: StatsSortKey) => {
    if (sortKey !== key) {
      setSortKey(key);
      setSortDir("asc");
    } else if (sortDir === "asc") {
      setSortDir("desc");
    } else {
      setSortKey(null);
    }
  };
  const sortButton = (label: string, key: StatsSortKey) => (
    <button
      type="button"
      class={`th-sort ${sortKey === key ? "on" : ""}`}
      onClick={() => cycleSort(key)}
      title={`Sort by ${label.toLowerCase()}`}
    >
      {label}
      {sortKey === key && (sortDir === "asc" ? " ▲" : " ▼")}
    </button>
  );
  useLayoutEffect(() => {
    const element = wrapRef.current;
    if (!element) return;
    const update = () => setAvailable(element.clientWidth);
    update();
    const observer = new ResizeObserver(update);
    observer.observe(element);
    return () => observer.disconnect();
  }, [rows.length]);
  const showBus = available >= 480;
  const showRange = available >= 700;
  if (rows.length === 0) {
    return (
      <div class="empty">
        <strong>
          {filtered ? "No matching statistics" : "No traffic statistics"}
        </strong>
        <span>
          {filtered
            ? "Try a different bus or message name."
            : "Counts, cycle times and jitter appear once frames arrive."}
        </span>
      </div>
    );
  }
  return (
    <div class="table-scroll" ref={wrapRef}>
      <table class="data-table stats-table">
        <colgroup>
          <col />
          {showBus && <col style={{ width: `${widths.bus}px` }} />}
          <col style={{ width: `${widths.count}px` }} />
          <col style={{ width: `${widths.avg}px` }} />
          {showRange && <col style={{ width: `${widths.min}px` }} />}
          {showRange && <col style={{ width: `${widths.max}px` }} />}
          <col style={{ width: `${widths.jitter}px` }} />
          <col style={{ width: `${widths.share}px` }} />
        </colgroup>
        <thead>
          <tr>
            <th>{sortButton("Message", "message")}</th>
            {showBus && (
              <th>
                {sortButton("Bus", "bus")}
                <ColumnResizeHandle
                  onResize={(event) => startResize(event, "bus")}
                />
              </th>
            )}
            <th class="num">
              {sortButton("Count", "count")}
              <ColumnResizeHandle
                onResize={(event) => startResize(event, "count")}
              />
            </th>
            <th class="num">
              {sortButton("Avg", "avg")}
              <ColumnResizeHandle
                onResize={(event) => startResize(event, "avg")}
              />
            </th>
            {showRange && (
              <th class="num">
                {sortButton("Min", "min")}
                <ColumnResizeHandle
                  onResize={(event) => startResize(event, "min")}
                />
              </th>
            )}
            {showRange && (
              <th class="num">
                {sortButton("Max", "max")}
                <ColumnResizeHandle
                  onResize={(event) => startResize(event, "max")}
                />
              </th>
            )}
            <th class="num">
              {sortButton("Jitter", "jitter")}
              <ColumnResizeHandle
                onResize={(event) => startResize(event, "jitter")}
              />
            </th>
            <th class="num">
              {sortButton("Share", "share")}
              <ColumnResizeHandle
                onResize={(event) => startResize(event, "share")}
              />
            </th>
          </tr>
        </thead>
        <tbody>
          {sortedRows.map((row, index) => (
            <tr
              key={`${row.bus}-${row.label}-${index}`}
              title={`${row.label} · ${row.bus} · min ${row.minMs} ms · avg ${row.avgMs} ms · max ${row.maxMs} ms · jitter ${row.jitterMs} ms · len ${row.len}`}
            >
              <td class="name" title={row.label}>
                {row.label}
              </td>
              {showBus && <td>{row.bus}</td>}
              <td class="num mono">{row.count.toLocaleString()}</td>
              <td class="num mono">{row.avgMs}</td>
              {showRange && <td class="num mono">{row.minMs}</td>}
              {showRange && <td class="num mono">{row.maxMs}</td>}
              <td class="num mono">{row.jitterMs}</td>
              <td class="num mono">{row.share}</td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}
