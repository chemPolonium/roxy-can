import { useEffect, useState } from "preact/hooks";
import type { Message } from "../types";

function hexId(message: Message): string {
  const width = message.extended ? 8 : 3;
  return `0x${message.id.toString(16).toUpperCase().padStart(width, "0")}`;
}

function countText(count: number): string {
  return count >= 10_000 ? `${(count / 1000).toFixed(1)}k` : count.toLocaleString();
}

/// Below ~1280px the payload column squeezes both itself and the name into
/// ellipses; the Trace still carries the bytes, so drop it there.
function useWideColumns(): boolean {
  const query = "(min-width: 1281px)";
  const [wide, setWide] = useState(() => window.matchMedia(query).matches);
  useEffect(() => {
    const media = window.matchMedia(query);
    const update = () => setWide(media.matches);
    media.addEventListener("change", update);
    return () => media.removeEventListener("change", update);
  }, []);
  return wide;
}

export function MessageTable({ rows }: { rows: Message[] }) {
  const wide = useWideColumns();
  if (rows.length === 0) {
    return (
      <div class="empty">
        <strong>No messages seen</strong>
        <span>Traffic will appear here once the bus runs.</span>
      </div>
    );
  }
  return (
    <div class="table-scroll">
      <table class="data-table message-table">
        <colgroup>
          <col style={{ width: "46px" }} />
          <col style={{ width: "72px" }} />
          <col style={{ width: wide ? "120px" : undefined }} />
          <col style={{ width: "52px" }} />
          <col style={{ width: "62px" }} />
          <col style={{ width: "30px" }} />
          {wide && <col />}
        </colgroup>
        <thead>
          <tr>
            <th>Bus</th>
            <th>ID</th>
            <th>Name</th>
            <th class="num">Count</th>
            <th class="num">Cycle</th>
            <th class="num">Len</th>
            {wide && <th>Data</th>}
          </tr>
        </thead>
        <tbody>
          {rows.map((message) => (
            <tr key={`${message.kind}-${message.bus}-${message.id}-${message.extended}`}>
              <td>{message.busName}</td>
              <td class="mono">{message.addr || hexId(message)}</td>
              <td class="name" title={message.name}>
                {message.name || <span class="dash">—</span>}
              </td>
              <td class="num mono">{countText(message.count)}</td>
              <td class="num mono">
                {message.cycleMs > 0 ? `${message.cycleMs.toFixed(1)} ms` : "event"}
              </td>
              <td class="num mono">{message.len}</td>
              {wide && (
                <td class="mono data" title={message.data}>
                  {message.data || <span class="dash">—</span>}
                </td>
              )}
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}
