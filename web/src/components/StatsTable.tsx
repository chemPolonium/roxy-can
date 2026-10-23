import { useEffect, useState } from "preact/hooks";
import type { StatsRow } from "../types";

function useMinWidth(query: string): boolean {
  const [match, setMatch] = useState(() => window.matchMedia(query).matches);
  useEffect(() => {
    const media = window.matchMedia(query);
    const update = () => setMatch(media.matches);
    media.addEventListener("change", update);
    return () => media.removeEventListener("change", update);
  }, [query]);
  return match;
}

export function StatsTable({
  rows,
  filtered = false,
}: {
  rows: StatsRow[];
  filtered?: boolean;
}) {
  const medium = useMinWidth("(min-width: 1200px)");
  const wide = useMinWidth("(min-width: 1500px)");
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
    <div class="table-scroll">
      <table class="data-table stats-table">
        <colgroup>
          <col />
          {medium && <col style={{ width: "50px" }} />}
          <col style={{ width: "60px" }} />
          <col style={{ width: "58px" }} />
          {wide && <col style={{ width: "55px" }} />}
          {wide && <col style={{ width: "55px" }} />}
          <col style={{ width: "60px" }} />
          <col style={{ width: "50px" }} />
        </colgroup>
        <thead>
          <tr>
            <th>Message</th>
            {medium && <th>Bus</th>}
            <th class="num">Count</th>
            <th class="num">Avg</th>
            {wide && <th class="num">Min</th>}
            {wide && <th class="num">Max</th>}
            <th class="num">Jitter</th>
            <th class="num">Share</th>
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
              {medium && <td>{row.bus}</td>}
              <td class="num mono">{row.count.toLocaleString()}</td>
              <td class="num mono">{row.avgMs}</td>
              {wide && <td class="num mono">{row.minMs}</td>}
              {wide && <td class="num mono">{row.maxMs}</td>}
              <td class="num mono">{row.jitterMs}</td>
              <td class="num mono">{row.share}</td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}
