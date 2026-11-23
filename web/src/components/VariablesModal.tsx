import { useEffect, useState } from "preact/hooks";
import { ModalHeader } from "./ModalHeader";
import { Tabs } from "./Tabs";
import type { SysVar, SysVarLabel } from "../types";
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
  labels: SysVarLabel[];
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
  onClose: () => void;
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
  labels: { value: string; label: string }[];
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
    labels: [],
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
    labels: (v.labels ?? []).map((entry) => ({
      value: String(entry.value),
      label: entry.label,
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
  const labels: SysVarLabel[] = [];
  for (const [index, entry] of draft.labels.entries()) {
    if (!entry.value.trim() && !entry.label.trim()) continue;
    const value = Number(entry.value);
    const label = entry.label.trim();
    if (!entry.value.trim() || !Number.isFinite(value)) {
      return { error: `state label ${index + 1}: value must be a number` };
    }
    if (!label) {
      return { error: `state label ${index + 1}: text must not be empty` };
    }
    if (labels.some((known) => known.value === value)) {
      return { error: `state label ${index + 1}: value ${value} is duplicated` };
    }
    labels.push({ value, label });
  }
  if (labels.length > 64) {
    return { error: "at most 64 state labels are allowed" };
  }
  labels.sort((a, b) => a.value - b.value);
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
      labels,
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
  return v.labels?.find((entry) => entry.value === v.value)?.label ?? null;
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
  onClose,
}: Props) {
  const [draft, setDraft] = useState<Draft | null>(null);
  const [tab, setTab] = useState<"values" | "computed">(initialTab);
  const checked = draft ? validate(draft) : null;

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
            </div>
            <div class="sv-labels">
              <div class="sv-labels-head">
                <span>State labels</span>
                <span>{draft.labels.length}/64</span>
              </div>
              {draft.labels.length === 0 && (
                <span class="sv-hint">
                  Optional: map numeric values to Panel text, e.g. 0 = Normal,
                  1 = Fault.
                </span>
              )}
              {draft.labels.map((entry, index) => (
                <div class="sv-label-row" key={index}>
                  <input
                    class="sv-label-value"
                    value={entry.value}
                    inputMode="decimal"
                    placeholder="value"
                    aria-label={`State label ${index + 1} value`}
                    onInput={(event) => {
                      const value = (event.target as HTMLInputElement).value;
                      set({
                        labels: draft.labels.map((candidate, at) =>
                          at === index ? { ...candidate, value } : candidate,
                        ),
                      });
                    }}
                  />
                  <input
                    class="sv-label-text"
                    value={entry.label}
                    maxLength={80}
                    placeholder="Normal"
                    aria-label={`State label ${index + 1} text`}
                    onInput={(event) => {
                      const label = (event.target as HTMLInputElement).value;
                      set({
                        labels: draft.labels.map((candidate, at) =>
                          at === index ? { ...candidate, label } : candidate,
                        ),
                      });
                    }}
                  />
                  <button
                    type="button"
                    class="mini"
                    title="Remove state label"
                    aria-label="Remove state label"
                    onClick={() =>
                      set({
                        labels: draft.labels.filter((_, at) => at !== index),
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
                  disabled={draft.labels.length >= 64}
                  onClick={() =>
                    set({ labels: [...draft.labels, { value: "", label: "" }] })
                  }
                >
                  + Add label
                </button>
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
