import { useState } from "preact/hooks";
import {
  clearDownload,
  isDownloadName,
  triggerDownload,
  waitForDownload,
} from "../download";

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
  ["graphics", "Signals chart (CSV)", ".csv"],
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
  const [downloading, setDownloading] = useState(false);
  const [downloadError, setDownloadError] = useState("");
  const current = KINDS.find(([value]) => value === kind) ?? KINDS[0];
  const name = path.trim();
  const canDownload = isDownloadName(name);

  const exportAndDownload = async () => {
    setDownloading(true);
    setDownloadError("");
    await clearDownload(name);
    onExport(kind, name);
    const url = await waitForDownload(name);
    setDownloading(false);
    if (!url) {
      setDownloadError("The export did not produce a file in time.");
      return;
    }
    triggerDownload(url, name);
    onClose();
  };

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
            A bare file name exports into the host's download folder and
            <strong> Export &amp; download</strong> saves it through the
            browser. A path with a directory part is written there instead.
            Trace writes ASC unless the path ends in <code>.pcap</code>.
            Exports use the first observer window of that kind (All buses by
            default).
          </p>
          {downloadError && <p class="modal-error">{downloadError}</p>}
          <div class="export-actions">
            <button
              type="button"
              class="primary"
              disabled={!name || downloading}
              onClick={() => onExport(kind, name)}
            >
              Export
            </button>
            <button
              type="button"
              class="primary"
              disabled={!canDownload || downloading}
              onClick={() => void exportAndDownload()}
            >
              {downloading ? "Waiting…" : "Export & download"}
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
