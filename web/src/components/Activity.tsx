import type { WriteLine } from "../types";

export function Activity({
  rows,
  clock,
  filtered = false,
}: {
  rows: WriteLine[];
  clock: (wallUs: number) => string;
  filtered?: boolean;
}) {
  if (rows.length === 0) {
    return (
      <div class="empty">
        <strong>
          {filtered ? "No matching lines" : "Nothing reported yet"}
        </strong>
        <span>
          {filtered
            ? "Try a different category or text."
            : "Command results and bus events land here."}
        </span>
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
