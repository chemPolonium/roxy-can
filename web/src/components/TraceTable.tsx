import type { Frame } from "../types";

function hexId(frame: Frame): string {
  const width = frame.extended ? 8 : 3;
  return `0x${frame.id.toString(16).toUpperCase().padStart(width, "0")}`;
}

export function TraceTable({ rows }: { rows: Frame[] }) {
  if (rows.length === 0) {
    return (
      <div class="empty">
        <strong>No frames yet</strong>
        <span>Start the simulation or clear the filter.</span>
      </div>
    );
  }
  return (
    <div class="table-scroll">
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
          {rows.map((frame, index) => (
            <tr key={`${frame.tUs}-${frame.bus}-${frame.id}-${index}`}>
              <td class="num mono">{(frame.tUs / 1e6).toFixed(6)}</td>
              <td>{frame.busName}</td>
              <td class="mono">
                {hexId(frame)}
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
          ))}
        </tbody>
      </table>
    </div>
  );
}
