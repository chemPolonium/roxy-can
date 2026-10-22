import { useEffect, useLayoutEffect, useRef, useState } from "preact/hooks";
import type { Frame } from "../types";

const ROW_H = 21;
const OVERSCAN = 8;
const COLUMNS = 7;
/// New rows arrive at the head every 100 ms. Only a live feed prepends this
/// many, so the anchor search does not need to scan the whole 50k buffer.
const ANCHOR_SCAN = 10_000;

interface Mark {
  key: string;
  tUs: number;
}

function hexId(frame: Frame): string {
  const width = frame.extended ? 8 : 3;
  return `0x${frame.id.toString(16).toUpperCase().padStart(width, "0")}`;
}

function frameKey(frame: Frame): string {
  return `${frame.tUs}:${frame.bus}:${frame.kind}:${frame.id}:${frame.data}`;
}

function rowText(frame: Frame): string {
  return [
    (frame.tUs / 1e6).toFixed(6),
    frame.busName,
    frame.addr || hexId(frame),
    frame.name,
    frame.dir,
    String(frame.len),
    frame.data,
  ].join("\t");
}

function copyText(text: string): void {
  const fallback = () => {
    const area = document.createElement("textarea");
    area.value = text;
    area.style.position = "fixed";
    area.style.opacity = "0";
    document.body.appendChild(area);
    area.select();
    document.execCommand("copy");
    area.remove();
  };
  if (navigator.clipboard?.writeText) {
    void navigator.clipboard.writeText(text).catch(fallback);
  } else {
    fallback();
  }
}

function deltaText(a: number, b: number): string {
  const us = Math.abs(a - b);
  if (us < 1_000) return `${us} µs`;
  if (us < 1_000_000) return `${(us / 1_000).toFixed(3)} ms`;
  return `${(us / 1e6).toFixed(6)} s`;
}

function TraceRow({
  frame,
  selected,
  cursorA,
  cursorB,
  onSelect,
}: {
  frame: Frame;
  selected: boolean;
  cursorA: boolean;
  cursorB: boolean;
  onSelect: () => void;
}) {
  const classes = [
    selected ? "selected" : "",
    cursorA ? "cursor-a" : "",
    cursorB ? "cursor-b" : "",
  ]
    .filter(Boolean)
    .join(" ");
  return (
    <tr class={classes} onClick={onSelect}>
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
  const scrollTopRef = useRef(0);
  const previousFirst = useRef<string | null>(null);
  const [scrollTop, setScrollTop] = useState(0);
  const [viewport, setViewport] = useState(480);
  const [selectedKey, setSelectedKey] = useState<string | null>(null);
  const [cursorA, setCursorA] = useState<Mark | null>(null);
  const [cursorB, setCursorB] = useState<Mark | null>(null);
  const total = rows.length;

  // New rows are prepended. A reader who scrolled down to inspect an older
  // frame must not be pushed away by that insertion: keep the same frame at
  // the same viewport offset, while a reader at the live edge stays at 0.
  useLayoutEffect(() => {
    const element = scrollRef.current;
    const first = rows.length > 0 ? frameKey(rows[0]) : null;
    const previous = previousFirst.current;
    previousFirst.current = first;
    if (!element || !previous || first === previous) return;
    const limit = Math.min(rows.length, ANCHOR_SCAN);
    let prepended = 0;
    for (let i = 1; i < limit; i += 1) {
      if (frameKey(rows[i]) === previous) {
        prepended = i;
        break;
      }
    }
    if (prepended === 0) return;
    if (scrollTopRef.current <= 2) {
      element.scrollTop = 0;
      scrollTopRef.current = 0;
      setScrollTop(0);
      return;
    }
    const next = scrollTopRef.current + prepended * ROW_H;
    element.scrollTop = next;
    scrollTopRef.current = next;
    setScrollTop(next);
  }, [rows]);

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
      scrollTopRef.current = max;
      setScrollTop(max);
    }
  }, [total]);

  const selectedIndex =
    selectedKey == null
      ? -1
      : rows.findIndex((frame) => frameKey(frame) === selectedKey);
  const selectedFrame = selectedIndex >= 0 ? rows[selectedIndex] : null;

  const moveSelection = (delta: number) => {
    if (rows.length === 0) return;
    let next: number;
    if (selectedIndex < 0) {
      next = delta > 0 ? 0 : rows.length - 1;
    } else {
      next = Math.min(rows.length - 1, Math.max(0, selectedIndex + delta));
    }
    const frame = rows[next];
    if (!frame) return;
    setSelectedKey(frameKey(frame));
    const element = scrollRef.current;
    if (!element) return;
    const top = next * ROW_H;
    const header = element.querySelector("thead")?.clientHeight ?? 0;
    if (top < element.scrollTop + header) {
      element.scrollTop = Math.max(0, top - header);
    } else if (top + ROW_H > element.scrollTop + element.clientHeight) {
      element.scrollTop = top + ROW_H - element.clientHeight + header;
    }
    scrollTopRef.current = element.scrollTop;
    setScrollTop(element.scrollTop);
  };

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
  const mark = (frame: Frame): Mark => ({ key: frameKey(frame), tUs: frame.tUs });

  return (
    <div class="trace-body">
      <div class="trace-cursors">
        <span class="trace-selection">
          {selectedFrame
            ? `Row ${(selectedFrame.tUs / 1e6).toFixed(6)} · ↑/↓ · Ctrl+C`
            : selectedKey
              ? "Selected row left the buffer"
              : "Click a row · ↑/↓ moves · Ctrl+C copies"}
        </span>
        {(cursorA || cursorB) && (
          <span class="trace-cursor-times">
            {cursorA && `A ${(cursorA.tUs / 1e6).toFixed(6)}`}
            {cursorA && cursorB && " · "}
            {cursorB && `B ${(cursorB.tUs / 1e6).toFixed(6)}`}
            {cursorA && cursorB && ` · Δt ${deltaText(cursorA.tUs, cursorB.tUs)}`}
          </span>
        )}
        <span class="spacer" />
        <button
          type="button"
          disabled={!selectedFrame}
          title="Set cursor A on the selected row"
          onClick={() => selectedFrame && setCursorA(mark(selectedFrame))}
        >
          Set A
        </button>
        <button
          type="button"
          disabled={!selectedFrame}
          title="Set cursor B on the selected row"
          onClick={() => selectedFrame && setCursorB(mark(selectedFrame))}
        >
          Set B
        </button>
        <button
          type="button"
          disabled={!selectedFrame}
          title="Copy the selected row"
          onClick={() => selectedFrame && copyText(rowText(selectedFrame))}
        >
          Copy
        </button>
        {(cursorA || cursorB) && (
          <button
            type="button"
            title="Clear both cursors"
            onClick={() => {
              setCursorA(null);
              setCursorB(null);
            }}
          >
            Clear
          </button>
        )}
      </div>
      <div
        class="table-scroll"
        ref={scrollRef}
        tabIndex={0}
        onKeyDown={(event) => {
          if (event.key === "ArrowDown" || event.key === "ArrowUp") {
            event.preventDefault();
            moveSelection(event.key === "ArrowDown" ? 1 : -1);
            return;
          }
          if (
            (event.ctrlKey || event.metaKey) &&
            event.key.toLowerCase() === "c" &&
            selectedFrame
          ) {
            event.preventDefault();
            copyText(rowText(selectedFrame));
          }
        }}
        onScroll={(event) => {
          const top = (event.currentTarget as HTMLDivElement).scrollTop;
          scrollTopRef.current = top;
          setScrollTop(top);
        }}
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
            {rows.slice(start, end).map((frame, index) => {
              const key = frameKey(frame);
              return (
                <TraceRow
                  key={`${key}:${start + index}`}
                  frame={frame}
                  selected={key === selectedKey}
                  cursorA={key === cursorA?.key}
                  cursorB={key === cursorB?.key}
                  onSelect={() => {
                    setSelectedKey(key);
                    scrollRef.current?.focus();
                  }}
                />
              );
            })}
            {bottomHeight > 0 && (
              <tr class="spacer" aria-hidden="true">
                <td colSpan={COLUMNS} style={{ height: `${bottomHeight}px` }} />
              </tr>
            )}
          </tbody>
        </table>
      </div>
    </div>
  );
}
