import { useEffect, useRef, useState } from "preact/hooks";
import type { Frame } from "../types";

const ROW_H = 21;
const OVERSCAN = 8;
const COLUMNS = 7;

function hexId(frame: Frame): string {
  const width = frame.extended ? 8 : 3;
  return `0x${frame.id.toString(16).toUpperCase().padStart(width, "0")}`;
}

function TraceRow({ frame }: { frame: Frame }) {
  return (
    <tr>
      <td class="num mono">{(frame.tUs / 1e6).toFixed(6)}</td>
      <td>{frame.busName}</td>
      <td class="mono">
        {frame.addr || hexId(frame)}
        {frame.fd && <span class="badge">FD</span>}
      </td>
      <td class="name" title={frame.name}>
        {frame.name || <span class="dash">—</span>}
      </td>
      <td class={frame.dir === "Tx" ? "tx" : "rx"}>{frame.dir}</td>
      <td class="num mono">{frame.len}</td>
      <td class="mono data" title={frame.data}>
        {frame.data || <span class="dash">—</span>}
      </td>
    </tr>
  );
}

/// Fixed-row-height virtualization: the table keeps its column layout and
/// sticky header, only the tbody window is in the DOM, and two spacer rows
/// carry the unrendered height. Five thousand rows cost about thirty nodes.
export function TraceTable({ rows }: { rows: Frame[] }) {
  const scrollRef = useRef<HTMLDivElement>(null);
  const [scrollTop, setScrollTop] = useState(0);
  const [viewport, setViewport] = useState(480);
  const total = rows.length;

  useEffect(() => {
    const element = scrollRef.current;
    if (!element) return;
    const measure = () => setViewport(element.clientHeight);
    measure();
    const observer = new ResizeObserver(measure);
    observer.observe(element);
    return () => observer.disconnect();
  }, []);

  // A clear, a tighter filter or a shorter buffer can leave the scroll
  // position past the end; pull it back instead of showing a blank body.
  useEffect(() => {
    const element = scrollRef.current;
    if (!element) return;
    const max = Math.max(0, total * ROW_H - element.clientHeight);
    if (element.scrollTop > max) {
      element.scrollTop = max;
      setScrollTop(max);
    }
  }, [total]);

  if (total === 0) {
    return (
      <div class="empty">
        <strong>No frames yet</strong>
        <span>Start the simulation or clear the filter.</span>
      </div>
    );
  }

  const start = Math.max(0, Math.floor(scrollTop / ROW_H) - OVERSCAN);
  const end = Math.min(
    total,
    start + Math.ceil(viewport / ROW_H) + OVERSCAN * 2,
  );
  const topHeight = start * ROW_H;
  const bottomHeight = Math.max(0, (total - end) * ROW_H);

  return (
    <div
      class="table-scroll"
      ref={scrollRef}
      onScroll={(event) =>
        setScrollTop((event.currentTarget as HTMLDivElement).scrollTop)
      }
    >
      <table class="data-table trace-table">
        <colgroup>
          <col style={{ width: "92px" }} />
          <col style={{ width: "54px" }} />
          <col style={{ width: "80px" }} />
          <col style={{ width: "22%" }} />
          <col style={{ width: "40px" }} />
          <col style={{ width: "46px" }} />
          <col />
        </colgroup>
        <thead>
          <tr>
            <th class="num">Time</th>
            <th>Bus</th>
            <th>ID</th>
            <th>Name</th>
            <th>Dir</th>
            <th class="num">Len</th>
            <th>Data</th>
          </tr>
        </thead>
        <tbody>
          {topHeight > 0 && (
            <tr class="spacer" aria-hidden="true">
              <td colSpan={COLUMNS} style={{ height: `${topHeight}px` }} />
            </tr>
          )}
          {rows.slice(start, end).map((frame, index) => (
            <TraceRow
              key={`${frame.tUs}-${frame.bus}-${frame.id}-${start + index}`}
              frame={frame}
            />
          ))}
          {bottomHeight > 0 && (
            <tr class="spacer" aria-hidden="true">
              <td colSpan={COLUMNS} style={{ height: `${bottomHeight}px` }} />
            </tr>
          )}
        </tbody>
      </table>
    </div>
  );
}
