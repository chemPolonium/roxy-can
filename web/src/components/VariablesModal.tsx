import { useEffect, useState } from "preact/hooks";
import { ModalHeader } from "./ModalHeader";
import { Tabs } from "./Tabs";
import type {
  SysVar,
  SysVarLabelEntry,
  SysVarLabelSet,
} from "../types";
import { palette, type Theme } from "../theme";
import { PaletteSwatches } from "./PaletteSwatches";
import type { DerivedDef, DerivedReference } from "../derived";
import { DerivedPanel } from "./DerivedPanel";

export interface SysVarEdit {
  namespace: string;
  name: string;
  init: number;
  min: number | null;
  max: number | null;
  unit: string;
  comment: string;
  labelSets: SysVarLabelSet[];
  oldNamespace: string | null;
  oldName: string | null;
}

interface Props {
  vars: SysVar[];
  onSetValue: (namespace: string, name: string, value: number) => void;
  onDefine: (edit: SysVarEdit) => void;
  onDelete: (namespace: string, name: string) => void;
  derived: DerivedDef[];
  derivedRefs: DerivedReference[];
  onAddDerived: (definition: DerivedDef) => void;
  onRemoveDerived: (id: string) => void;
  onRuleDerived?: (definition: DerivedDef) => void;
  initialTab?: "values" | "computed";
  theme: Theme;
  onClose: () => void;
}

interface DraftLabelEntry {
  value: string;
  text: string;
  color: number | null;
}

interface DraftLabelSet {
  id: string;
  name: string;
  isDefault: boolean;
  entries: DraftLabelEntry[];
}

interface Draft {
  editing: { namespace: string; name: string } | null;
  namespace: string;
  name: string;
  init: string;
  min: string;
  max: string;
  unit: string;
  comment: string;
  labelSets: DraftLabelSet[];
}

function newDraft(): Draft {
  return {
    editing: null,
    namespace: "SysVar",
    name: "",
    init: "0",
    min: "",
    max: "",
    unit: "",
    comment: "",
    labelSets: [],
  };
}

function editDraft(v: SysVar): Draft {
  return {
    editing: { namespace: v.namespace, name: v.name },
    namespace: v.namespace,
    name: v.name,
    init: String(v.init),
    min: v.min == null ? "" : String(v.min),
    max: v.max == null ? "" : String(v.max),
    unit: v.unit,
    comment: v.comment,
    labelSets: (v.labelSets ?? []).map((labelSet) => ({
      id: labelSet.id,
      name: labelSet.name,
      isDefault: labelSet.isDefault,
      entries: labelSet.entries.map((entry) => ({
        value: String(entry.value),
        text: entry.text ?? "",
        color: entry.color ?? null,
      })),
    })),
  };
}

function optionalNumber(text: string): number | null | undefined {
  const value = text.trim();
  if (!value) return null;
  const parsed = Number(value);
  return Number.isFinite(parsed) ? parsed : undefined;
}

function validate(draft: Draft): { edit?: SysVarEdit; error: string } {
  const namespace = draft.namespace.trim();
  const name = draft.name.trim();
  if (!namespace || !name) {
    return { error: "namespace and name must not be empty" };
  }
  if (namespace.includes("::") || name.includes("::")) {
    return { error: 'namespace and name must not contain "::"' };
  }
  const init = Number(draft.init);
  if (!draft.init.trim() || !Number.isFinite(init)) {
    return { error: "initial value must be a number" };
  }
  const min = optionalNumber(draft.min);
  const max = optionalNumber(draft.max);
  if (min === undefined || max === undefined) {
    return { error: "bounds must be numbers or blank" };
  }
  if (min != null && max != null && min > max) {
    return { error: `lower bound ${min} is greater than upper bound ${max}` };
  }
  if (draft.labelSets.length > 8) {
    return { error: "at most 8 label sets are allowed" };
  }
  const labelSets: SysVarLabelSet[] = [];
  const ids = new Set<string>();
  const names = new Set<string>();
  let defaults = 0;
  for (const [labelSetIndex, labelSet] of draft.labelSets.entries()) {
    const id = labelSet.id.trim();
    const setName = labelSet.name.trim();
    if (!id) return { error: `label set ${labelSetIndex + 1}: id is empty` };
    if (!setName) return { error: `label set ${labelSetIndex + 1}: name is empty` };
    if (ids.has(id)) return { error: `label set id "${id}" is duplicated` };
    if (names.has(setName)) {
      return { error: `label set name "${setName}" is duplicated` };
    }
    ids.add(id);
    names.add(setName);
    const entries: SysVarLabelEntry[] = [];
    for (const [entryIndex, entry] of labelSet.entries.entries()) {
      const text = entry.text.trim();
      if (!entry.value.trim() && !text && entry.color == null) continue;
      const value = Number(entry.value);
      if (!entry.value.trim() || !Number.isFinite(value)) {
        return {
          error: `${setName}: label ${entryIndex + 1} value must be a number`,
        };
      }
      if (text.length > 80) {
        return { error: `${setName}: text is limited to 80 characters` };
      }
      if (entry.color != null && (!Number.isInteger(entry.color) || entry.color < 0 || entry.color > 63)) {
        return { error: `${setName}: color slot must be 0–63` };
      }
      if (!text && entry.color == null) {
        return {
          error: `${setName}: label ${entryIndex + 1} needs text or color`,
        };
      }
      if (entries.some((known) => known.value === value)) {
        return {
          error: `${setName}: value ${value} is duplicated`,
        };
      }
      entries.push({
        value,
        ...(text ? { text } : {}),
        ...(entry.color != null ? { color: entry.color } : {}),
      });
    }
    if (entries.length === 0) {
      return { error: `${setName}: add at least one label` };
    }
    if (entries.length > 64) {
      return { error: `${setName}: at most 64 labels are allowed` };
    }
    entries.sort((a, b) => a.value - b.value);
    if (labelSet.isDefault) defaults += 1;
    labelSets.push({ id, name: setName, isDefault: labelSet.isDefault, entries });
  }
  if (defaults > 1) {
    return { error: "only one label set can be the default" };
  }
  if (labelSets.length > 0 && defaults === 0) {
    labelSets[0].isDefault = true;
  }
  return {
    error: "",
    edit: {
      namespace,
      name,
      init,
      min,
      max,
      unit: draft.unit.trim(),
      comment: draft.comment.trim(),
      labelSets,
      oldNamespace: draft.editing?.namespace ?? null,
      oldName: draft.editing?.name ?? null,
    },
  };
}

function rangeText(v: SysVar): string {
  if (v.min == null && v.max == null) return "unbounded";
  const lo = v.min == null ? "-" : String(v.min);
  const hi = v.max == null ? "+" : String(v.max);
  return `${lo} .. ${hi}`;
}

function currentLabel(v: SysVar): string | null {
  const set =
    v.labelSets?.find((entry) => entry.isDefault) ?? v.labelSets?.[0];
  const entry = set?.entries.find((candidate) => candidate.value === v.value);
  if (!entry) return null;
  return entry.text || (entry.color != null ? `Color ${entry.color + 1}` : null);
}

/** Editable live value; polling state refreshes it only while not typing. */
function ValueInput({
  value,
  onCommit,
}: {
  value: number;
  onCommit: (value: number) => void;
}) {
  const [text, setText] = useState(String(value));
  const [editing, setEditing] = useState(false);
  useEffect(() => {
    if (!editing) setText(String(value));
  }, [value, editing]);
  const commit = () => {
    setEditing(false);
    const parsed = Number(text);
    if (Number.isFinite(parsed)) {
      if (parsed !== value) onCommit(parsed);
    } else {
      setText(String(value));
    }
  };
  return (
    <input
      class="sv-value"
      value={text}
      inputMode="decimal"
      onFocus={() => setEditing(true)}
      onInput={(event) => setText((event.target as HTMLInputElement).value)}
      onBlur={commit}
      onKeyDown={(event) => {
        if (event.key === "Enter") (event.target as HTMLInputElement).blur();
        if (event.key === "Escape") {
          setText(String(value));
          (event.target as HTMLInputElement).blur();
        }
      }}
    />
  );
}

export function VariablesModal({
  vars,
  onSetValue,
  onDefine,
  onDelete,
  derived,
  derivedRefs,
  onAddDerived,
  onRemoveDerived,
  onRuleDerived,
  initialTab = "values",
  theme,
  onClose,
}: Props) {
  const [draft, setDraft] = useState<Draft | null>(null);
  const [tab, setTab] = useState<"values" | "computed">(initialTab);
  const checked = draft ? validate(draft) : null;
  const monitorColors = palette(theme).monitors;

  useEffect(() => {
    const close = (event: KeyboardEvent) => {
      if (event.key === "Escape") onClose();
    };
    window.addEventListener("keydown", close);
    return () => window.removeEventListener("keydown", close);
  }, [onClose]);

  const set = (patch: Partial<Draft>) =>
    setDraft((current) => (current ? { ...current, ...patch } : current));

  const submit = () => {
    if (!draft) return;
    const result = validate(draft);
    if (!result.edit) return;
    onDefine(result.edit);
    setDraft(null);
  };

  return (
    <div class="modal-backdrop" onMouseDown={onClose}>
      <div
        class="sysvars-modal"
        role="dialog"
        aria-modal="true"
        aria-label="Variables"
        onMouseDown={(event) => event.stopPropagation()}
      >
        <ModalHeader title="Variables" onClose={onClose}>
          <span>{vars.length} values · {derived.length} computed</span>
        </ModalHeader>

        <div class="variables-tabs">
          <Tabs
            value={tab}
            onChange={setTab}
            label="Variables view"
            options={[
              { value: "values", label: "System variables" },
              { value: "computed", label: "Computed channels" },
            ]}
          />
        </div>

        {tab === "computed" ? (
          <DerivedPanel
            defs={derived}
            references={derivedRefs}
            onSave={onAddDerived}
            onRemove={onRemoveDerived}
            onRule={onRuleDerived}
          />
        ) : (
          <>
        <div class="sv-tools">
          <button type="button" onClick={() => setDraft(newDraft())}>
            + Variable
          </button>
          <span class="sv-hint">
            Values reset to Init when measurement starts · scripts:{" "}
            <code>sys_get("ns::name")</code>,{" "}
            <code>sys_set("ns::name", value)</code>
          </span>
        </div>

        {draft && (
          <form
            class="sv-form"
            onSubmit={(event) => {
              event.preventDefault();
              submit();
            }}
          >
            <div class="sv-form-grid">
              <label>
                Namespace
                <input
                  value={draft.namespace}
                  onInput={(event) =>
                    set({ namespace: (event.target as HTMLInputElement).value })
                  }
                />
              </label>
              <label>
                Name
                <input
                  value={draft.name}
                  onInput={(event) =>
                    set({ name: (event.target as HTMLInputElement).value })
                  }
                />
              </label>
              <label>
                Init
                <input
                  value={draft.init}
                  inputMode="decimal"
                  onInput={(event) =>
                    set({ init: (event.target as HTMLInputElement).value })
                  }
                />
              </label>
              <label>
                Min
                <input
                  value={draft.min}
                  placeholder="no limit"
                  inputMode="decimal"
                  onInput={(event) =>
                    set({ min: (event.target as HTMLInputElement).value })
                  }
                />
              </label>
              <label>
                Max
                <input
                  value={draft.max}
                  placeholder="no limit"
                  inputMode="decimal"
                  onInput={(event) =>
                    set({ max: (event.target as HTMLInputElement).value })
                  }
                />
              </label>
              <label>
                Unit
                <input
                  value={draft.unit}
                  onInput={(event) =>
                    set({ unit: (event.target as HTMLInputElement).value })
                  }
                />
              </label>
              <label class="wide">
                Comment
                <input
                  value={draft.comment}
                  onInput={(event) =>
                    set({ comment: (event.target as HTMLInputElement).value })
                  }
                />
              </label>
            <div class="sv-labels">
              <div class="sv-labels-head">
                <span>Label sets</span>
                <span>{draft.labelSets.length}/8</span>
              </div>
              {draft.labelSets.length === 0 && (
                <span class="sv-hint">
                  Optional: add named text/color sets. A widget chooses one set
                  from its Labels property.
                </span>
              )}
              {draft.labelSets.map((labelSet, labelSetIndex) => (
                <div class="sv-label-set" key={labelSet.id}>
                  <div class="sv-label-set-head">
                    <input
                      class="sv-label-set-name"
                      value={labelSet.name}
                      maxLength={40}
                      placeholder="Status"
                      aria-label={`Label set ${labelSetIndex + 1} name`}
                      onInput={(event) => {
                        const name = (event.target as HTMLInputElement).value;
                        set({
                          labelSets: draft.labelSets.map((candidate, at) =>
                            at === labelSetIndex ? { ...candidate, name } : candidate,
                          ),
                        });
                      }}
                    />
                    <label class="sv-label-default">
                      <input
                        type="radio"
                        name="sv-label-default"
                        checked={labelSet.isDefault}
                        onChange={() =>
                          set({
                            labelSets: draft.labelSets.map((candidate, at) => ({
                              ...candidate,
                              isDefault: at === labelSetIndex,
                            })),
                          })
                        }
                      />
                      default
                    </label>
                    <button
                      type="button"
                      class="mini"
                      title="Remove label set"
                      aria-label={`Remove label set ${labelSet.name || labelSetIndex + 1}`}
                      onClick={() =>
                        set({
                          labelSets: draft.labelSets.filter(
                            (_, at) => at !== labelSetIndex,
                          ),
                        })
                      }
                    >
                      ×
                    </button>
                  </div>
                  {labelSet.entries.map((entry, entryIndex) => (
                    <div class="sv-label-row" key={entryIndex}>
                      <input
                        class="sv-label-value"
                        value={entry.value}
                        inputMode="decimal"
                        placeholder="value"
                        aria-label={`${labelSet.name || "Set"} label ${entryIndex + 1} value`}
                        onInput={(event) => {
                          const value = (event.target as HTMLInputElement).value;
                          set({
                            labelSets: draft.labelSets.map((candidate, at) =>
                              at === labelSetIndex
                                ? {
                                    ...candidate,
                                    entries: candidate.entries.map(
                                      (item, itemIndex) =>
                                        itemIndex === entryIndex
                                          ? { ...item, value }
                                          : item,
                                    ),
                                  }
                                : candidate,
                            ),
                          });
                        }}
                      />
                      <input
                        class="sv-label-text"
                        value={entry.text}
                        maxLength={80}
                        placeholder="Normal"
                        aria-label={`${labelSet.name || "Set"} label ${entryIndex + 1} text`}
                        onInput={(event) => {
                          const text = (event.target as HTMLInputElement).value;
                          set({
                            labelSets: draft.labelSets.map((candidate, at) =>
                              at === labelSetIndex
                                ? {
                                    ...candidate,
                                    entries: candidate.entries.map(
                                      (item, itemIndex) =>
                                        itemIndex === entryIndex
                                          ? { ...item, text }
                                          : item,
                                    ),
                                  }
                                : candidate,
                            ),
                          });
                        }}
                      />
                      <PaletteSwatches
                        colors={monitorColors}
                        value={entry.color ?? 0}
                        compact
                        onChange={(color) =>
                          set({
                            labelSets: draft.labelSets.map((candidate, at) =>
                              at === labelSetIndex
                                ? {
                                    ...candidate,
                                    entries: candidate.entries.map(
                                      (item, itemIndex) =>
                                        itemIndex === entryIndex
                                          ? { ...item, color }
                                          : item,
                                    ),
                                  }
                                : candidate,
                            ),
                          })
                        }
                      />
                      {entry.color != null && (
                        <button
                          type="button"
                          class="mini sv-label-clear"
                          title="Remove colour"
                          aria-label="Remove colour"
                          onClick={() =>
                            set({
                              labelSets: draft.labelSets.map((candidate, at) =>
                                at === labelSetIndex
                                  ? {
                                      ...candidate,
                                      entries: candidate.entries.map(
                                        (item, itemIndex) =>
                                          itemIndex === entryIndex
                                            ? { ...item, color: null }
                                            : item,
                                      ),
                                    }
                                  : candidate,
                              ),
                            })
                          }
                        >
                          ×
                        </button>
                      )}
                      <button
                        type="button"
                        class="mini"
                        title="Remove label"
                        aria-label="Remove label"
                        onClick={() =>
                          set({
                            labelSets: draft.labelSets.map((candidate, at) =>
                              at === labelSetIndex
                                ? {
                                    ...candidate,
                                    entries: candidate.entries.filter(
                                      (_, itemIndex) => itemIndex !== entryIndex,
                                    ),
                                  }
                                : candidate,
                            ),
                          })
                        }
                      >
                        ×
                      </button>
                    </div>
                  ))}
                  <div>
                    <button
                      type="button"
                      class="mini"
                      disabled={labelSet.entries.length >= 64}
                      onClick={() =>
                        set({
                          labelSets: draft.labelSets.map((candidate, at) =>
                            at === labelSetIndex
                              ? {
                                  ...candidate,
                                  entries: [
                                    ...candidate.entries,
                                    { value: "", text: "", color: null },
                                  ],
                                }
                              : candidate,
                          ),
                        })
                      }
                    >
                      + Label
                    </button>
                  </div>
                </div>
              ))}
              <div>
                <button
                  type="button"
                  class="mini"
                  disabled={draft.labelSets.length >= 8}
                  onClick={() => {
                    let n = draft.labelSets.length + 1;
                    const ids = new Set(draft.labelSets.map((labelSet) => labelSet.id));
                    while (ids.has(`labels${n}`)) n += 1;
                    set({
                      labelSets: [
                        ...draft.labelSets,
                        {
                          id: `labels${n}`,
                          name: `Labels ${n}`,
                          isDefault: draft.labelSets.length === 0,
                          entries: [{ value: "", text: "", color: null }],
                        },
                      ],
                    });
                  }}
                >
                  + Label set
                </button>
              </div>
            </div>
            </div>
            <div class="sv-form-foot">
              {checked?.error ? (
                <span class="sv-error">{checked.error}</span>
              ) : (
                <span class="sv-hint">
                  {draft.editing
                    ? `Editing ${draft.editing.namespace}::${draft.editing.name}`
                    : "New variable"}
                </span>
              )}
              <span class="spacer" />
              <button type="submit" class="primary" disabled={!!checked?.error}>
                Apply
              </button>
              <button type="button" onClick={() => setDraft(null)}>
                Cancel
              </button>
            </div>
          </form>
        )}

        <div class="sv-table">
          <div class="sv-row sv-head">
            <span>Variable</span>
            <span>Value</span>
            <span>Range</span>
            <span>Unit / comment</span>
            <span />
          </div>
          {vars.map((v) => {
            const key = `${v.namespace}::${v.name}`;
            return (
              <div class="sv-row" key={key}>
                <span class="sv-name" title={key}>
                  <strong>{v.name}</strong>
                  <small>{v.namespace}</small>
                </span>
                <div class="sv-value-cell">
                  <ValueInput
                    value={v.value}
                    onCommit={(value) => onSetValue(v.namespace, v.name, value)}
                  />
                  {currentLabel(v) && (
                    <small class="sv-label-preview">{currentLabel(v)}</small>
                  )}
                </div>
                <span class="sv-range" title="writes are clamped to these bounds">
                  {rangeText(v)}
                </span>
                <span class="sv-meta">
                  {v.unit && <em>{v.unit}</em>}
                  {v.comment}
                  {!v.unit && !v.comment && <span class="sv-empty">-</span>}
                </span>
                <span class="sv-actions">
                  <button type="button" onClick={() => setDraft(editDraft(v))}>
                    edit
                  </button>
                  <button
                    type="button"
                    class="danger"
                    title={`Delete ${key}`}
                    onClick={() => onDelete(v.namespace, v.name)}
                  >
                    ×
                  </button>
                </span>
              </div>
            );
          })}
          {vars.length === 0 && (
            <div class="sv-empty-row">
              no variables -- add one to expose it to scripts and triggers
            </div>
          )}
        </div>
          </>
        )}
      </div>
    </div>
  );
}
