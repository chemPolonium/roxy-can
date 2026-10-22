import type { WriteLine } from "../types";

export function Activity({
  rows,
  clock,
}: {
  rows: WriteLine[];
  clock: (wallUs: number) => string;
}) {
  if (rows.length === 0) {
    return (
      <div class="empty">
        <strong>Nothing reported yet</strong>
        <span>Command results and bus events land here.</span>
      </div>
    );
  }
  return (
    <ol class="activity">
      {rows.map((line, index) => (
        <li key={`${line.wallUs}-${index}`} class={line.kind}>
          <span class="clock">{clock(line.wallUs)}</span>
          <span class="text">{line.text}</span>
        </li>
      ))}
    </ol>
  );
}
