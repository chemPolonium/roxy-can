import { useState } from "preact/hooks";

interface Props {
  initialKind: string;
  onExport: (kind: string, path: string) => void;
  onClose: () => void;
}

const KINDS: [string, string, string][] = [
  ["trace", "Trace (ASC / PCAP)", ".asc"],
  ["messages", "Messages (CSV)", ".csv"],
  ["stats", "Statistics (CSV)", ".csv"],
  ["data", "Data (CSV)", ".csv"],
  ["state", "State (CSV)", ".csv"],
  ["spec", "Specification (CSV)", ".csv"],
  ["write", "Write log (TXT)", ".txt"],
];

function withExt(path: string, ext: string): string {
  if (/\.[a-z0-9]+$/i.test(path)) {
    return path.replace(/\.[a-z0-9]+$/i, ext);
  }
  return `${path}${ext}`;
}

export function ExportModal({ initialKind, onExport, onClose }: Props) {
  const [kind, setKind] = useState(
    KINDS.some(([value]) => value === initialKind) ? initialKind : "trace",
  );
  const [path, setPath] = useState(
    `export${KINDS.find(([value]) => value === initialKind)?.[2] ?? ".asc"}`,
  );
  const current = KINDS.find(([value]) => value === kind) ?? KINDS[0];

  return (
    <div class="modal-backdrop" onMouseDown={onClose}>
      <div
        class="export-modal"
        role="dialog"
        aria-modal="true"
        aria-label="Export"
        onMouseDown={(event) => event.stopPropagation()}
      >
        <header class="modal-head">
          <h2>Export</h2>
          <button type="button" onClick={onClose}>
            Close
          </button>
        </header>
        <div class="export-body">
          <label>
            Content
            <select
              value={kind}
              onChange={(event) => {
                const next = (event.target as HTMLSelectElement).value;
                setKind(next);
                const ext = KINDS.find(([value]) => value === next)?.[2] ?? ".csv";
                setPath((current) => withExt(current, ext));
              }}
            >
              {KINDS.map(([value, label]) => (
                <option key={value} value={value}>
                  {label}
                </option>
              ))}
            </select>
          </label>
          <label class="export-path">
            Host path
            <input
              value={path}
              placeholder={`export${current[2]}`}
              onInput={(event) =>
                setPath((event.target as HTMLInputElement).value)
              }
              onKeyDown={(event) => {
                if (event.key === "Enter" && path.trim()) {
                  onExport(kind, path.trim());
                }
              }}
            />
          </label>
          <p class="hint">
            Relative paths resolve against the server's working directory; an
            absolute host path is used as typed. Trace writes ASC unless the
            path ends in <code>.pcap</code>. Exports use the first desktop
            window of that kind (All buses by default).
          </p>
          <div class="export-actions">
            <button
              type="button"
              class="primary"
              disabled={!path.trim()}
              onClick={() => onExport(kind, path.trim())}
            >
              Export
            </button>
            <button type="button" onClick={onClose}>
              Cancel
            </button>
          </div>
        </div>
      </div>
    </div>
  );
}
