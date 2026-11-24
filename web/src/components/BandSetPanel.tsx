import { useState } from "preact/hooks";
import type { BandSet } from "../types";
import {
  bandSetMatchesQuery,
  normaliseBands,
  validateBands,
  type Band,
} from "../bands";
import { palette, type Theme } from "../theme";
import { BandEditor } from "./BandEditor";
import { ConfirmDialog } from "./ConfirmDialog";

interface Props {
  sets: BandSet[];
  theme: Theme;
  onSave: (set: BandSet, oldId: string | null) => void;
  onDelete: (id: string) => void;
}

interface Draft {
  id: string;
  name: string;
  bands: Band[];
  editing: boolean;
}

export function BandSetPanel({ sets, theme, onSave, onDelete }: Props) {
  const colors = palette(theme).monitors;
  const [draft, setDraft] = useState<Draft | null>(null);
  const [filter, setFilter] = useState("");
  const [pendingDelete, setPendingDelete] = useState<string | null>(null);
  const edit = (set: BandSet) =>
    setDraft({ ...set, bands: set.bands.map((band) => ({ ...band })), editing: true });
  const visible = sets.filter((set) =>
    bandSetMatchesQuery(set.name, set.id, filter),
  );
  const pendingSet = sets.find((set) => set.id === pendingDelete);
  const error = draft ? validateBands(normaliseBands(draft.bands)) : "";
  const canApply = !!draft && !!draft.name.trim() && !error;

  const apply = () => {
    if (!draft || !canApply) return;
    onSave(
      {
        id: draft.id,
        name: draft.name.trim(),
        bands: normaliseBands(draft.bands),
      },
      draft.editing ? draft.id : null,
    );
    setDraft(null);
    setFilter("");
  };

  return (
    <>
      <div class="sv-tools">
        <button
          type="button"
          onClick={() => {
            let n = sets.length + 1;
            const ids = new Set(sets.map((set) => set.id));
            while (ids.has(`bands${n}`)) n += 1;
            setDraft({
              id: `bands${n}`,
              name: `Bands ${n}`,
              bands: [{ upper: null, inclusive: false, text: "all" }],
              editing: false,
            });
          }}
        >
          + Band set
        </button>
        <input
          class="sv-filter"
          value={filter}
          placeholder="Filter name or id…"
          aria-label="Filter band sets"
          spellcheck={false}
          onInput={(event) =>
            setFilter((event.target as HTMLInputElement).value)
          }
        />
        <span class="sv-hint">
          Reusable by SysVars, Monitor, State, Derived and Panel controls.
        </span>
      </div>
      {draft && (
        <form
          class="sv-form"
          onSubmit={(event) => {
            event.preventDefault();
            apply();
          }}
        >
          <div class="sv-form-grid">
            <label class="wide">
              Name
              <input
                value={draft.name}
                maxLength={40}
                onInput={(event) =>
                  setDraft({
                    ...draft,
                    name: (event.target as HTMLInputElement).value,
                  })
                }
              />
            </label>
          </div>
          <BandEditor
            bands={draft.bands}
            colors={colors}
            onChange={(bands) => setDraft({ ...draft, bands })}
          />
          <div class="sv-form-foot">
            {error ? (
              <span class="sv-error">{error}</span>
            ) : (
              <span class="sv-hint">
                {draft.editing ? `Editing ${draft.id}` : "New band set"}
              </span>
            )}
            <span class="spacer" />
            <button type="button" onClick={() => setDraft(null)}>
              Cancel
            </button>
            <button type="submit" class="primary" disabled={!canApply}>
              Apply
            </button>
          </div>
        </form>
      )}
      <div class="sv-table">
        <div class="sv-row sv-head">
          <span>Band set</span>
          <span>Bands</span>
          <span />
          <span />
          <span />
        </div>
        {visible.map((set) => (
          <div class="sv-row band-set-row" key={set.id}>
            <span class="sv-name">
              <strong>{set.name}</strong>
              <small>{set.id}</small>
            </span>
            <span class="sv-meta">{set.bands.length} bands</span>
            <span class="sv-actions">
              <button type="button" onClick={() => edit(set)}>
                edit
              </button>
              <button
                type="button"
                class="danger"
                onClick={() => setPendingDelete(set.id)}
              >
                ×
              </button>
            </span>
          </div>
        ))}
        {sets.length === 0 && (
          <div class="sv-empty-row">
            no band sets -- add one to reuse display rules
          </div>
        )}
        {sets.length > 0 && visible.length === 0 && (
          <div class="sv-empty-row">
            no band sets match "{filter.trim()}"
          </div>
        )}
      </div>
      {pendingSet && (
        <ConfirmDialog
          title="Delete band set"
          message={`Delete band set ${pendingSet.name}? This cannot be undone.`}
          confirmLabel="Delete"
          danger
          onConfirm={() => {
            onDelete(pendingSet.id);
            setPendingDelete(null);
          }}
          onClose={() => setPendingDelete(null)}
        />
      )}
    </>
  );
}
