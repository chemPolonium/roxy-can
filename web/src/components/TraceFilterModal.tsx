import { useState } from "preact/hooks";

export interface TraceFilter {
  dir: "all" | "Rx" | "Tx";
  dbcOnly: boolean;
  payload: string;
  from: string;
  to: string;
}

export const EMPTY_TRACE_FILTER: TraceFilter = {
  dir: "all",
  dbcOnly: false,
  payload: "",
  from: "",
  to: "",
};

export function traceFilterActive(filter: TraceFilter): boolean {
  return (
    filter.dir !== "all" ||
    filter.dbcOnly ||
    filter.payload.trim() !== "" ||
    filter.from.trim() !== "" ||
    filter.to.trim() !== ""
  );
}

interface Props {
  filter: TraceFilter;
  onApply: (filter: TraceFilter) => void;
  onClose: () => void;
}

export function TraceFilterModal({ filter, onApply, onClose }: Props) {
  const [draft, setDraft] = useState(filter);
  const set = (patch: Partial<TraceFilter>) =>
    setDraft((current) => ({ ...current, ...patch }));
  const invalidTime =
    (draft.from.trim() !== "" && !Number.isFinite(Number(draft.from))) ||
    (draft.to.trim() !== "" && !Number.isFinite(Number(draft.to)));

  return (
    <div class="modal-backdrop" onMouseDown={onClose}>
      <div
        class="filter-modal"
        role="dialog"
        aria-modal="true"
        aria-label="Trace filters"
        onMouseDown={(event) => event.stopPropagation()}
      >
        <header class="modal-head">
          <h2>Trace Filters</h2>
          <button type="button" onClick={onClose}>
            Close
          </button>
        </header>
        <div class="filter-body">
          <label>
            Direction
            <select
              value={draft.dir}
              onChange={(event) =>
                set({
                  dir: (event.target as HTMLSelectElement)
                    .value as TraceFilter["dir"],
                })
              }
            >
              <option value="all">All directions</option>
              <option value="Rx">Rx only</option>
              <option value="Tx">Tx only</option>
            </select>
          </label>
          <label class="filter-check">
            <input
              type="checkbox"
              checked={draft.dbcOnly}
              onChange={(event) =>
                set({ dbcOnly: (event.target as HTMLInputElement).checked })
              }
            />
            Only frames decoded by a database
          </label>
          <label>
            Payload contains (hex)
            <input
              value={draft.payload}
              placeholder="e.g. 17 34"
              spellcheck={false}
              onInput={(event) =>
                set({ payload: (event.target as HTMLInputElement).value })
              }
            />
          </label>
          <div class="filter-time">
            <label>
              From (s)
              <input
                value={draft.from}
                inputMode="decimal"
                placeholder="start"
                onInput={(event) =>
                  set({ from: (event.target as HTMLInputElement).value })
                }
              />
            </label>
            <label>
              To (s)
              <input
                value={draft.to}
                inputMode="decimal"
                placeholder="end"
                onInput={(event) =>
                  set({ to: (event.target as HTMLInputElement).value })
                }
              />
            </label>
          </div>
          <p class="hint">
            Times are frame timestamps in seconds. Text and bus filters in the
            header still apply on top of these.
          </p>
          <div class="filter-actions">
            <button
              type="button"
              onClick={() => {
                onApply(EMPTY_TRACE_FILTER);
                onClose();
              }}
            >
              Reset
            </button>
            <span class="spacer" />
            <button
              type="button"
              class="primary"
              disabled={invalidTime}
              onClick={() => {
                onApply(draft);
                onClose();
              }}
            >
              Apply
            </button>
          </div>
        </div>
      </div>
    </div>
  );
}
