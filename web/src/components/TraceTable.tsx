import {
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
} from "preact/hooks";
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
  selected,
  cursorA,
  cursorB,
  onSelect,
  onMenu,
}: {
  frame: Frame;
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
export function TraceTable({
  rows,
  onQuery,
}: {
  rows: Frame[];
  onQuery?: (text: string) => void;
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
              <th class="num">{sortButton("Time", "time")}</th>
              <th>{sortButton("Bus", "bus")}</th>
              <th>{sortButton("ID", "id")}</th>
              <th>{sortButton("Name", "name")}</th>
              <th>{sortButton("Dir", "dir")}</th>
              <th class="num">{sortButton("Len", "len")}</th>
              <th>Data</th>
            </tr>
          </thead>
          <tbody>
            {topHeight > 0 && (
              <tr class="spacer" aria-hidden="true">
                <td colSpan={COLUMNS} style={{ height: `${topHeight}px` }} />
              </tr>
            )}
            {sortedRows.slice(start, end).map((frame, index) => {
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
                <td colSpan={COLUMNS} style={{ height: `${bottomHeight}px` }} />
              </tr>
            )}
          </tbody>
        </table>
      </div>
      {menu && (
        <div
          class="trace-menu"
          style={{
            left: `${Math.min(menu.x, window.innerWidth - 190)}px`,
            top: `${Math.min(menu.y, window.innerHeight - 150)}px`,
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
              setMenu(null);
            }}
          >
            Set cursor A
          </button>
          <button
            type="button"
            onClick={() => {
              setCursorB(mark(menu.frame));
              setMenu(null);
            }}
          >
            Set cursor B
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
