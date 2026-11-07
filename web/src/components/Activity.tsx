import type { WriteLine } from "../types";

export function Activity({
  rows,
  clock,
  filtered = false,
  onFind,
}: {
  rows: WriteLine[];
  clock: (wallUs: number) => string;
  filtered?: boolean;
  onFind?: (query: string) => void;
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
        <li
          key={`${line.wallUs}-${index}`}
          class={`${line.kind} ${
            onFind && /0x[0-9a-f]+/i.test(line.text) ? "linked" : ""
          }`}
          title={
            onFind && /0x[0-9a-f]+/i.test(line.text)
              ? "Click to find this ID in Trace"
              : undefined
          }
          tabIndex={onFind && /0x[0-9a-f]+/i.test(line.text) ? 0 : undefined}
          onClick={() => {
            const id = line.text.match(/0x[0-9a-f]+/i)?.[0];
            if (id) onFind?.(`id:${id}`);
          }}
          onKeyDown={(event) => {
            if (event.key !== "Enter") return;
            const id = line.text.match(/0x[0-9a-f]+/i)?.[0];
            if (id) {
              event.preventDefault();
              onFind?.(`id:${id}`);
            }
          }}
        >
          <span class="clock">{clock(line.wallUs)}</span>
          <span class="text">{line.text}</span>
        </li>
      ))}
    </ol>
  );
}
