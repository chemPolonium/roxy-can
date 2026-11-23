import { useState } from "preact/hooks";
import type { BandSet } from "../types";
import type { Band } from "../bands";
import { palette, type Theme } from "../theme";
import { BandEditor } from "./BandEditor";

interface Props {
  sets: BandSet[];
  theme: Theme;
  onSave: (set: BandSet, oldId: string | null) => void;
  onDelete: (id: string) => void;
}

export function BandSetPanel({ sets, theme, onSave, onDelete }: Props) {
  const colors = palette(theme).monitors;
  const [draft, setDraft] = useState<{ id: string; name: string; bands: Band[] } | null>(null);
  const edit = (set: BandSet) => setDraft({ ...set });
  return (
    <div class="sv-table">
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
            });
          }}
        >
          + Band set
        </button>
        <span class="sv-hint">
          Reusable by SysVars, Monitor, State, Derived and Panel controls.
        </span>
      </div>
      {draft && (
        <div class="sv-form">
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
            <span class="spacer" />
            <button
              type="button"
              class="primary"
              disabled={!draft.name.trim()}
              onClick={() => {
                onSave(
                  {
                    id: draft.id,
                    name: draft.name.trim(),
                    bands: draft.bands,
                  },
                  sets.some((set) => set.id === draft.id) ? draft.id : null,
                );
                setDraft(null);
              }}
            >
              Apply
            </button>
            <button type="button" onClick={() => setDraft(null)}>
              Cancel
            </button>
          </div>
        </div>
      )}
      <div class="sv-table">
        {sets.map((set) => (
          <div class="sv-row" key={set.id}>
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
                onClick={() => {
                  if (window.confirm(`Delete band set ${set.name}?`)) {
                    onDelete(set.id);
                  }
                }}
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
      </div>
    </div>
  );
}
