import { useLayoutEffect, useRef, useState } from "preact/hooks";
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
            <th>Message</th>
            {showBus && (
              <th>
                Bus
                <ColumnResizeHandle
                  onResize={(event) => startResize(event, "bus")}
                />
              </th>
            )}
            <th class="num">
              Count
              <ColumnResizeHandle
                onResize={(event) => startResize(event, "count")}
              />
            </th>
            <th class="num">
              Avg
              <ColumnResizeHandle
                onResize={(event) => startResize(event, "avg")}
              />
            </th>
            {showRange && (
              <th class="num">
                Min
                <ColumnResizeHandle
                  onResize={(event) => startResize(event, "min")}
                />
              </th>
            )}
            {showRange && (
              <th class="num">
                Max
                <ColumnResizeHandle
                  onResize={(event) => startResize(event, "max")}
                />
              </th>
            )}
            <th class="num">
              Jitter
              <ColumnResizeHandle
                onResize={(event) => startResize(event, "jitter")}
              />
            </th>
            <th class="num">
              Share
              <ColumnResizeHandle
                onResize={(event) => startResize(event, "share")}
              />
            </th>
          </tr>
        </thead>
        <tbody>
          {rows.map((row, index) => (
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
