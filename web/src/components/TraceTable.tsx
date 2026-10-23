import {
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
} from "preact/hooks";
import type { Frame } from "../types";
import {
  columnWidth,
  TRACE_COLUMNS,
  type TraceColumnId,
  type TraceColumnPrefs,
} from "../traceLayout";

const ROW_H = 21;
const OVERSCAN = 8;
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

type SortKey = "time" | "bus" | "id" | "name" | "dir" | "len";
type SortDir = "asc" | "desc";

function compareFrames(a: Frame, b: Frame, key: SortKey): number {
  switch (key) {
    case "time":
      return a.tUs - b.tUs;
    case "bus":
      return a.busName.localeCompare(b.busName);
    case "id":
      return a.kind === "eth" || b.kind === "eth"
        ? a.addr.localeCompare(b.addr)
        : a.id - b.id;
    case "name":
      return a.name.localeCompare(b.name);
    case "dir":
      return a.dir.localeCompare(b.dir);
    case "len":
      return a.len - b.len;
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
  visible,
  selected,
  cursorA,
  cursorB,
  onSelect,
  onMenu,
}: {
  frame: Frame;
  visible: TraceColumnId[];
  selected: boolean;
  cursorA: boolean;
  cursorB: boolean;
  onSelect: () => void;
  onMenu: (event: MouseEvent) => void;
}) {
  const classes = [
    selected ? "selected" : "",
    cursorA ? "cursor-a" : "",
    cursorB ? "cursor-b" : "",
  ]
    .filter(Boolean)
    .join(" ");
  return (
    <tr class={classes} onClick={onSelect} onContextMenu={onMenu}>
      {visible.map((id) => {
        switch (id) {
          case "time":
            return (
              <td key={id} class="num mono">
                {(frame.tUs / 1e6).toFixed(6)}
              </td>
            );
          case "bus":
            return <td key={id}>{frame.busName}</td>;
          case "id":
            return (
              <td key={id} class="mono">
                {frame.addr || hexId(frame)}
                {frame.fd && <span class="badge">FD</span>}
              </td>
            );
          case "name":
            return (
              <td key={id} class="name" title={frame.name}>
                {frame.name || <span class="dash">—</span>}
              </td>
            );
          case "dir":
            return (
              <td key={id} class={frame.dir === "Tx" ? "tx" : "rx"}>
                {frame.dir}
              </td>
            );
          case "len":
            return (
              <td key={id} class="num mono">
                {frame.len}
              </td>
            );
          case "data":
            return (
              <td key={id} class="mono data" title={frame.data}>
                {frame.data || <span class="dash">—</span>}
              </td>
            );
        }
      })}
    </tr>
  );
}

/// Fixed-row-height virtualization: the table keeps its column layout and
/// sticky header, only the tbody window is in the DOM, and two spacer rows
/// carry the unrendered height. Five thousand rows cost about thirty nodes.
export interface TraceJump {
  tUs: number;
  nonce: number;
}

export function TraceTable({
  rows,
  columns,
  onColumnsChange,
  jump,
  onJumpDone,
  onQuery,
  onChartCursor,
  onSetChartCursor,
  onAddToChart,
  onWatch,
  onShowTime,
}: {
  rows: Frame[];
  columns: TraceColumnPrefs;
  onColumnsChange: (next: TraceColumnPrefs) => void;
  jump?: TraceJump | null;
  onJumpDone?: () => void;
  onQuery?: (text: string) => void;
  onChartCursor?: (tUs: number) => void;
  onSetChartCursor?: (slot: "A" | "B", tUs: number) => void;
  onAddToChart?: (frame: Frame) => void;
  onWatch?: (frame: Frame) => void;
  onShowTime?: (tUs: number) => void;
}) {
  const scrollRef = useRef<HTMLDivElement>(null);
  const scrollTopRef = useRef(0);
  const previousFirst = useRef<string | null>(null);
  const [scrollTop, setScrollTop] = useState(0);
  const [viewport, setViewport] = useState(480);
  const [selectedKey, setSelectedKey] = useState<string | null>(null);
  const [cursorA, setCursorA] = useState<Mark | null>(null);
  const [cursorB, setCursorB] = useState<Mark | null>(null);
  const [menu, setMenu] = useState<{
    x: number;
    y: number;
    frame: Frame;
  } | null>(null);
  const [sortKey, setSortKey] = useState<SortKey | null>(null);
  const [sortDir, setSortDir] = useState<SortDir>("asc");
  const sortedRows = useMemo(() => {
    if (!sortKey) return rows;
    const sign = sortDir === "asc" ? 1 : -1;
    return [...rows].sort((a, b) => sign * compareFrames(a, b, sortKey));
  }, [rows, sortKey, sortDir]);
  const total = sortedRows.length;
  const visible = TRACE_COLUMNS.filter(
    (column) => !columns.hidden.includes(column.id),
  );

  // A one-off jump from the go-to-time box or a palette command: park the
  // nearest frame about a third of the way down instead of at the very top.
  useEffect(() => {
    const element = scrollRef.current;
    if (!jump || !element || sortedRows.length === 0) return;
    let best = 0;
    let bestDelta = Number.POSITIVE_INFINITY;
    for (let i = 0; i < sortedRows.length; i += 1) {
      const delta = Math.abs(sortedRows[i].tUs - jump.tUs);
      if (delta < bestDelta) {
        bestDelta = delta;
        best = i;
      }
    }
    const header = element.querySelector("thead")?.clientHeight ?? 0;
    const top = Math.max(
      0,
      best * ROW_H - header - (element.clientHeight - header) / 3,
    );
    element.scrollTop = top;
    scrollTopRef.current = top;
    setScrollTop(top);
    setSelectedKey(frameKey(sortedRows[best]));
    scrollRef.current?.focus();
    // Consume the jump so a later filter change (which remounts the table)
    // does not scroll back to the old target.
    onJumpDone?.();
    // sortedRows is replaced on every poll; the jump itself drives this.
  }, [jump]);

  useEffect(() => {
    if (!menu) return;
    const close = () => setMenu(null);
    const escape = (event: KeyboardEvent) => {
      if (event.key === "Escape") setMenu(null);
    };
    window.addEventListener("pointerdown", close);
    window.addEventListener("keydown", escape);
    return () => {
      window.removeEventListener("pointerdown", close);
      window.removeEventListener("keydown", escape);
    };
  }, [menu]);

  // A new sort starts at the top; keeping a raw pixel offset would land in
  // the middle of an unrelated order.
  useLayoutEffect(() => {
    const element = scrollRef.current;
    previousFirst.current = null;
    if (!element) return;
    element.scrollTop = 0;
    scrollTopRef.current = 0;
    setScrollTop(0);
  }, [sortKey, sortDir]);

  // New rows are prepended. A reader who scrolled down to inspect an older
  // frame must not be pushed away by that insertion: keep the same frame at
  // the same viewport offset, while a reader at the live edge stays at 0.
  // Sorted views are exempt: their order changes with every arrival.
  useLayoutEffect(() => {
    const element = scrollRef.current;
    const first = sortedRows.length > 0 ? frameKey(sortedRows[0]) : null;
    const previous = previousFirst.current;
    previousFirst.current = first;
    if (sortKey || !element || !previous || first === previous) return;
    const limit = Math.min(sortedRows.length, ANCHOR_SCAN);
    let prepended = 0;
    for (let i = 1; i < limit; i += 1) {
      if (frameKey(sortedRows[i]) === previous) {
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
  }, [sortedRows, sortKey]);

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
      : sortedRows.findIndex((frame) => frameKey(frame) === selectedKey);
  const selectedFrame = selectedIndex >= 0 ? sortedRows[selectedIndex] : null;

  const moveSelection = (delta: number) => {
    if (sortedRows.length === 0) return;
    let next: number;
    if (selectedIndex < 0) {
      next = delta > 0 ? 0 : sortedRows.length - 1;
    } else {
      next = Math.min(
        sortedRows.length - 1,
        Math.max(0, selectedIndex + delta),
      );
    }
    const frame = sortedRows[next];
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
  const cycleSort = (key: SortKey) => {
    if (sortKey !== key) {
      setSortKey(key);
      setSortDir("asc");
    } else if (sortDir === "asc") {
      setSortDir("desc");
    } else {
      setSortKey(null);
    }
  };
  const sortButton = (label: string, key: SortKey) => (
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

  const startResize = (event: PointerEvent, id: TraceColumnId) => {
    event.preventDefault();
    event.stopPropagation();
    const def = TRACE_COLUMNS.find((column) => column.id === id);
    const startX = event.clientX;
    const startWidth = columnWidth(columns, id);
    const move = (moveEvent: PointerEvent) => {
      const width = Math.min(
        def?.max ?? 420,
        Math.max(def?.min ?? 40, startWidth + moveEvent.clientX - startX),
      );
      onColumnsChange({
        ...columns,
        widths: { ...columns.widths, [id]: width },
      });
    };
    const up = () => {
      document.body.classList.remove("col-resizing");
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", up);
    };
    document.body.classList.add("col-resizing");
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", up);
  };

  const headerFor = (id: TraceColumnId) => {
    switch (id) {
      case "time":
        return sortButton("Time", "time");
      case "bus":
        return sortButton("Bus", "bus");
      case "id":
        return sortButton("ID", "id");
      case "name":
        return sortButton("Name", "name");
      case "dir":
        return sortButton("Dir", "dir");
      case "len":
        return sortButton("Len", "len");
      case "data":
        return "Data";
    }
  };

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
            {visible.map((column) =>
              column.flex ? (
                <col key={column.id} />
              ) : (
                <col
                  key={column.id}
                  style={{ width: `${columnWidth(columns, column.id)}px` }}
                />
              ),
            )}
          </colgroup>
          <thead>
            <tr>
              {visible.map((column) => (
                <th
                  key={column.id}
                  class={
                    column.id === "time" || column.id === "len" ? "num" : ""
                  }
                >
                  {headerFor(column.id)}
                  {!column.flex && (
                    <span
                      class="col-resize"
                      onPointerDown={(event) => startResize(event, column.id)}
                    />
                  )}
                </th>
              ))}
            </tr>
          </thead>
          <tbody>
            {topHeight > 0 && (
              <tr class="spacer" aria-hidden="true">
                <td
                  colSpan={visible.length}
                  style={{ height: `${topHeight}px` }}
                />
              </tr>
            )}
            {sortedRows.slice(start, end).map((frame, index) => {
              const key = frameKey(frame);
              return (
                <TraceRow
                  key={`${key}:${start + index}`}
                  frame={frame}
                  visible={visible.map((column) => column.id)}
                  selected={key === selectedKey}
                  cursorA={key === cursorA?.key}
                  cursorB={key === cursorB?.key}
                  onSelect={() => {
                    setSelectedKey(key);
                    scrollRef.current?.focus();
                  }}
                  onMenu={(event) => {
                    event.preventDefault();
                    setSelectedKey(key);
                    scrollRef.current?.focus();
                    setMenu({
                      x: event.clientX,
                      y: event.clientY,
                      frame,
                    });
                  }}
                />
              );
            })}
            {bottomHeight > 0 && (
              <tr class="spacer" aria-hidden="true">
                <td
                  colSpan={visible.length}
                  style={{ height: `${bottomHeight}px` }}
                />
              </tr>
            )}
          </tbody>
        </table>
      </div>
      {menu && (
        <div
          class="trace-menu"
          style={{
            left: `${Math.min(menu.x, window.innerWidth - 210)}px`,
            top: `${Math.min(menu.y, window.innerHeight - 250)}px`,
          }}
          onPointerDown={(event) => event.stopPropagation()}
        >
          <button
            type="button"
            disabled={!onQuery}
            onClick={() => {
              onQuery?.(menu.frame.addr || hexId(menu.frame));
              setMenu(null);
            }}
          >
            Filter this ID
          </button>
          <button
            type="button"
            onClick={() => {
              setCursorA(mark(menu.frame));
              onSetChartCursor?.("A", menu.frame.tUs);
              setMenu(null);
            }}
          >
            Set cursor A
          </button>
          <button
            type="button"
            onClick={() => {
              setCursorB(mark(menu.frame));
              onSetChartCursor?.("B", menu.frame.tUs);
              setMenu(null);
            }}
          >
            Set cursor B
          </button>
          <button
            type="button"
            disabled={!onChartCursor}
            onClick={() => {
              onChartCursor?.(menu.frame.tUs);
              setMenu(null);
            }}
          >
            Place chart cursor
          </button>
          <button
            type="button"
            disabled={!onShowTime}
            onClick={() => {
              onShowTime?.(menu.frame.tUs);
              setMenu(null);
            }}
          >
            Show at this time
          </button>
          <button
            type="button"
            disabled={!onAddToChart}
            onClick={() => {
              onAddToChart?.(menu.frame);
              setMenu(null);
            }}
          >
            Add signals to chart
          </button>
          <button
            type="button"
            disabled={!onWatch}
            onClick={() => {
              onWatch?.(menu.frame);
              setMenu(null);
            }}
          >
            Watch in Monitor
          </button>
          <button
            type="button"
            onClick={() => {
              copyText(rowText(menu.frame));
              setMenu(null);
            }}
          >
            Copy row
          </button>
        </div>
      )}
    </div>
  );
}
