import { useEffect, useRef, useState } from "preact/hooks";
import { HostPathPick, fileBrowserOpen } from "./FileBrowser";

export interface ReferenceColumns {
  [label: string]: [number, number][];
}

export interface ReferenceData {
  name: string;
  offsetS: number;
  columns: ReferenceColumns;
}

export function parseReferenceCsv(text: string): ReferenceColumns {
  const lines = text.split(/\r?\n/).filter((line) => line.trim() !== "");
  if (lines.length < 2) return {};
  const header = lines[0].split(",");
  const labels = header.slice(1);
  const columns: ReferenceColumns = Object.fromEntries(
    labels.map((label) => [label.trim(), [] as [number, number][]]),
  );
  for (const line of lines.slice(1, 200_001)) {
    const fields = line.split(",");
    const time = Number(fields[0]);
    if (!Number.isFinite(time)) continue;
    for (let i = 0; i < labels.length; i += 1) {
      const raw = fields[i + 1]?.trim();
      if (!raw) continue;
      const value = Number(raw);
      if (Number.isFinite(value)) {
        columns[labels[i].trim()].push([time, value]);
      }
    }
  }
  return columns;
}

export function ReferenceModal({
  current,
  onApply,
  onClear,
  onLoadLog,
  onClose,
}: {
  current: ReferenceData | null;
  onApply: (data: ReferenceData) => void;
  onClear: () => void;
  onLoadLog?: (path: string) => void;
  onClose: () => void;
}) {
  const [file, setFile] = useState<File | null>(null);
  const [columns, setColumns] = useState<ReferenceColumns | null>(
    current?.columns ?? null,
  );
  const [name, setName] = useState(current?.name ?? "");
  const [offset, setOffset] = useState(String(current?.offsetS ?? 0));
  const [logPath, setLogPath] = useState("");
  const [logError, setLogError] = useState("");
  const [error, setError] = useState("");
  const [reading, setReading] = useState(false);
  const inputRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    const escape = (event: KeyboardEvent) => {
      if (event.key === "Escape" && !fileBrowserOpen()) onClose();
    };
    window.addEventListener("keydown", escape);
    return () => window.removeEventListener("keydown", escape);
  }, [onClose]);

  const choose = async (picked: File | null) => {
    if (!picked) return;
    setReading(true);
    setError("");
    try {
      const columns = parseReferenceCsv(await picked.text());
      const count = Object.keys(columns).length;
      if (count === 0) {
        setError("No signal columns found. Use time_s as the first header.");
        setColumns(null);
      } else {
        setColumns(columns);
        setName(picked.name);
        setFile(picked);
      }
    } catch (e) {
      setError(String(e));
    } finally {
      setReading(false);
    }
  };

  const parsed = Number(offset);
  const canApply = Boolean(columns) && Number.isFinite(parsed);

  return (
    <div class="modal-backdrop" onMouseDown={onClose}>
      <div
        class="reference-modal"
        role="dialog"
        aria-modal="true"
        aria-label="Reference run"
        onMouseDown={(event) => event.stopPropagation()}
      >
        <header class="modal-head">
          <h2>Reference run</h2>
          <button type="button" onClick={onClose}>
            Close
          </button>
        </header>
        <div class="reference-body">
          <p class="hint">
            Overlay a previous run on the Signals chart. The file is the
            aligned CSV exported from this tool: header{" "}
            <code>time_s,CAN1:EngineSpeed,…</code>. Reference time plus offset
            is matched to the live time.
          </p>
          <div class="reference-file">
            <button
              type="button"
              onClick={() => inputRef.current?.click()}
              disabled={reading}
            >
              {reading ? "Reading…" : "Choose aligned CSV…"}
            </button>
            <span>{file?.name || name || "no file"}</span>
            <input
              ref={inputRef}
              type="file"
              accept=".csv,text/csv"
              onChange={(event) =>
                void choose((event.target as HTMLInputElement).files?.[0] ?? null)
              }
            />
          </div>
          {columns && (
            <p class="reference-summary">
              {Object.keys(columns).length} signals ·{" "}
              {Math.max(
                0,
                ...Object.values(columns).map((points) => points.length),
              ).toLocaleString()}{" "}
              rows
            </p>
          )}
          {onLoadLog && (
            <div class="reference-log">
              <div class="reference-log-row">
                <input
                  value={logPath}
                  placeholder="Host path to a reference .asc or .blf"
                  onInput={(event) => {
                    setLogPath((event.target as HTMLInputElement).value);
                    setLogError("");
                  }}
                  onKeyDown={(event) => {
                    if (event.key === "Enter" && logPath.trim()) {
                      onLoadLog(logPath.trim());
                    }
                  }}
                />
                <button
                  type="button"
                  disabled={!logPath.trim()}
                  onClick={() => onLoadLog(logPath.trim())}
                >
                  Load log
                </button>
                <HostPathPick
                  label="Choose…"
                  accept=".asc,.blf"
                  onError={setLogError}
                  onPick={(path) => {
                    setLogPath(path);
                    onLoadLog(path);
                  }}
                />
              </div>
              <p class="hint">
                ASC/BLF is decoded on the host with the current DBC, FIBEX and
                ARXML descriptions; matching chart signals are compared.
              </p>
              {logError && <p class="modal-error">{logError}</p>}
            </div>
          )}
          <label class="reference-offset">
            Reference offset
            <input
              type="number"
              step="0.001"
              value={offset}
              onInput={(event) =>
                setOffset((event.target as HTMLInputElement).value)
              }
            />
            <span>s</span>
          </label>
          {error && <p class="modal-error">{error}</p>}
          <div class="reference-actions">
            {current && (
              <button type="button" class="danger" onClick={onClear}>
                Remove reference
              </button>
            )}
            <span class="spacer" />
            <button type="button" onClick={onClose}>
              Cancel
            </button>
            <button
              type="button"
              class="primary"
              disabled={!canApply}
              onClick={() =>
                columns &&
                onApply({
                  name: name || "reference.csv",
                  offsetS: Number.isFinite(parsed) ? parsed : 0,
                  columns,
                })
              }
            >
              Apply
            </button>
          </div>
        </div>
      </div>
    </div>
  );
}
