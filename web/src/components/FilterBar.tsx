import { useEffect, useRef, useState } from "preact/hooks";
import { UiIcon } from "./icons";
import {
  FILTER_NUMBER_OPS,
  FILTER_OP_LABELS,
  FILTER_TEXT_OPS,
  emptyFilterGroup,
  filterRuleCount,
  filterSummary,
  newFilterGroup,
  newFilterRule,
  pruneFilter,
  type FilterFieldSpec,
  type FilterGroup,
  type FilterNode,
  type FilterRule,
} from "../filterTree";
import {
  FILTER_SCOPE_LABELS,
  cloneFilter,
  savedFilterId,
  type FilterScope,
  type SavedFilter,
} from "../filterLibrary";

interface Props {
  query: string;
  onQuery: (value: string) => void;
  filter: FilterNode;
  onFilter: (filter: FilterNode) => void;
  fields: FilterFieldSpec[];
  label: string;
  placeholder: string;
  scope: FilterScope;
  savedFilters: SavedFilter[];
  onSavedFilters: (filters: SavedFilter[]) => void;
}

const MAX_GROUP_DEPTH = 3;

function RuleEditor({
  rule,
  fields,
  onChange,
  onRemove,
}: {
  rule: FilterRule;
  fields: FilterFieldSpec[];
  onChange: (next: FilterRule) => void;
  onRemove: () => void;
}) {
  const spec = fields.find((field) => field.key === rule.field);
  const ops = spec?.numeric ? FILTER_NUMBER_OPS : FILTER_TEXT_OPS;
  return (
    <div class="filter-rule-row">
      <select
        class="filter-field"
        aria-label="Filter field"
        value={rule.field}
        onChange={(event) => {
          const next = event.target as HTMLSelectElement;
          const nextSpec = fields.find((field) => field.key === next.value);
          const allowed = nextSpec?.numeric
            ? FILTER_NUMBER_OPS
            : FILTER_TEXT_OPS;
          onChange({
            ...rule,
            field: next.value,
            op: allowed.includes(rule.op) ? rule.op : allowed[0],
            numeric: nextSpec?.numeric || undefined,
          });
        }}
      >
        {fields.map((field) => (
          <option key={field.key} value={field.key}>
            {field.label}
          </option>
        ))}
      </select>
      <select
        class="filter-op"
        aria-label="Filter operator"
        value={rule.op}
        onChange={(event) =>
          onChange({
            ...rule,
            op: (event.target as HTMLSelectElement)
              .value as FilterRule["op"],
          })
        }
      >
        {ops.map((op) => (
          <option key={op} value={op}>
            {FILTER_OP_LABELS[op]}
          </option>
        ))}
      </select>
      <input
        class="filter-value"
        type={spec?.numeric ? "number" : "text"}
        step={spec?.numeric ? "any" : undefined}
        value={rule.value}
        placeholder={spec?.numeric ? "number" : "value"}
        spellcheck={false}
        onInput={(event) =>
          onChange({
            ...rule,
            value: (event.target as HTMLInputElement).value,
          })
        }
      />
      <button
        type="button"
        class="mini"
        title="Remove this condition"
        aria-label="Remove condition"
        onClick={onRemove}
      >
        ×
      </button>
    </div>
  );
}

function GroupEditor({
  group,
  fields,
  depth,
  onChange,
  onRemove,
}: {
  group: FilterGroup;
  fields: FilterFieldSpec[];
  depth: number;
  onChange: (next: FilterGroup) => void;
  onRemove?: () => void;
}) {
  const setChild = (index: number, next: FilterNode) =>
    onChange({
      ...group,
      children: group.children.map((child, at) => (at === index ? next : child)),
    });
  const removeChild = (index: number) =>
    onChange({
      ...group,
      children: group.children.filter((_, at) => at !== index),
    });

  return (
    <div class={`filter-group ${depth === 0 ? "root" : ""}`}>
      <div class="filter-group-head">
        <select
          class="filter-op"
          aria-label="Combine conditions"
          value={group.op}
          onChange={(event) =>
            onChange({
              ...group,
              op: (event.target as HTMLSelectElement).value as "and" | "or",
            })
          }
        >
          <option value="and">AND</option>
          <option value="or">OR</option>
        </select>
        <button
          type="button"
          class={`mini ${group.negate ? "on" : ""}`}
          title="Invert this group"
          aria-pressed={!!group.negate}
          onClick={() => onChange({ ...group, negate: !group.negate })}
        >
          NOT
        </button>
        <span class="spacer" />
        <button
          type="button"
          class="mini"
          onClick={() =>
            onChange({
              ...group,
              children: [
                ...group.children,
                newFilterRule(
                  fields[0]?.key ?? "",
                  fields[0]?.numeric ?? false,
                ),
              ],
            })
          }
        >
          + Rule
        </button>
        <button
          type="button"
          class="mini"
          disabled={depth >= MAX_GROUP_DEPTH}
          title={
            depth >= MAX_GROUP_DEPTH
              ? "Groups can nest up to three levels"
              : "Add a nested group"
          }
          onClick={() =>
            onChange({
              ...group,
              children: [...group.children, newFilterGroup()],
            })
          }
        >
          + Group
        </button>
        {onRemove && (
          <button
            type="button"
            class="mini"
            title="Remove this group and its conditions"
            aria-label="Remove group"
            onClick={onRemove}
          >
            ×
          </button>
        )}
      </div>
      {group.children.length === 0 && (
        <div class="filter-empty">No conditions</div>
      )}
      {group.children.map((child, index) => (
        <div class="filter-child" key={index}>
          {child.kind === "rule" ? (
            <RuleEditor
              rule={child}
              fields={fields}
              onChange={(next) => setChild(index, next)}
              onRemove={() => removeChild(index)}
            />
          ) : (
            <GroupEditor
              group={child}
              fields={fields}
              depth={depth + 1}
              onChange={(next) => setChild(index, next)}
              onRemove={() => removeChild(index)}
            />
          )}
        </div>
      ))}
    </div>
  );
}

/// Header search + the shared AND/OR/NOT signal filter popover.
export function FilterBar({
  query,
  onQuery,
  filter,
  onFilter,
  fields,
  label,
  placeholder,
  scope,
  savedFilters,
  onSavedFilters,
}: Props) {
  const [open, setOpen] = useState(false);
  const [draft, setDraft] = useState<FilterNode>(filter);
  const [pos, setPos] = useState({ x: 8, y: 8 });
  const [selectedId, setSelectedId] = useState("");
  const [saveOpen, setSaveOpen] = useState(false);
  const [saveName, setSaveName] = useState("");
  const [saveError, setSaveError] = useState("");
  const trigger = useRef<HTMLButtonElement>(null);
  const popover = useRef<HTMLDivElement>(null);
  const active = filterRuleCount(filter);
  const scoped = savedFilters.filter((entry) => entry.scope === scope);
  const draftCount = filterRuleCount(draft);
  const summary = filterSummary(filter, fields);

  useEffect(() => {
    if (!open) return;
    popover.current?.querySelector<HTMLInputElement>("input")?.focus();
    const close = (event: PointerEvent) => {
      if (
        !(event.target instanceof Node) ||
        (!popover.current?.contains(event.target) &&
          !trigger.current?.contains(event.target))
      ) {
        setOpen(false);
      }
    };
    const key = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        event.preventDefault();
        event.stopPropagation();
        setOpen(false);
      }
    };
    const scroll = () => setOpen(false);
    window.addEventListener("pointerdown", close);
    window.addEventListener("keydown", key);
    window.addEventListener("scroll", scroll, true);
    return () => {
      window.removeEventListener("pointerdown", close);
      window.removeEventListener("keydown", key);
      window.removeEventListener("scroll", scroll, true);
    };
  }, [open]);

  const openEditor = () => {
    const rect = trigger.current?.getBoundingClientRect();
    const width = Math.min(480, window.innerWidth - 16);
    const left = Math.max(
      8,
      Math.min((rect?.right ?? width) - width, window.innerWidth - width - 8),
    );
    const top = Math.max(
      8,
      Math.min((rect?.bottom ?? 40) + 4, window.innerHeight - 368),
    );
    setDraft(filter);
    setSaveOpen(false);
    setSaveError("");
    if (selectedId && !scoped.some((entry) => entry.id === selectedId)) {
      setSelectedId("");
    }
    setPos({ x: left, y: top });
    setOpen(true);
  };

  const apply = () => {
    onFilter(pruneFilter(draft) ?? emptyFilterGroup());
    setOpen(false);
  };

  const loadSaved = (id: string) => {
    setSelectedId(id);
    setSaveError("");
    const entry = scoped.find((candidate) => candidate.id === id);
    if (entry) setDraft(cloneFilter(entry.filter));
  };

  const updateSaved = () => {
    const entry = scoped.find((candidate) => candidate.id === selectedId);
    const pruned = pruneFilter(draft);
    if (!entry || !pruned || pruned.kind !== "group") return;
    onSavedFilters(
      savedFilters.map((candidate) =>
        candidate.id === entry.id
          ? { ...candidate, filter: cloneFilter(pruned) }
          : candidate,
      ),
    );
  };

  const beginSave = () => {
    if (!draftCount) return;
    if (selectedId && scoped.some((entry) => entry.id === selectedId)) {
      updateSaved();
      return;
    }
    setSaveError("");
    setSaveName("");
    setSaveOpen(true);
  };

  const saveAsNew = () => {
    const name = saveName.trim().slice(0, 40);
    if (!name) {
      setSaveError("Give this filter a name");
      return;
    }
    if (
      scoped.some(
        (entry) => entry.name.toLowerCase() === name.toLowerCase(),
      )
    ) {
      setSaveError("That name is already used");
      return;
    }
    const pruned = pruneFilter(draft);
    if (!pruned || pruned.kind !== "group") {
      setSaveError("Add at least one condition first");
      return;
    }
    const entry: SavedFilter = {
      id: savedFilterId(),
      name,
      scope,
      filter: cloneFilter(pruned),
    };
    onSavedFilters([...savedFilters, entry]);
    setSelectedId(entry.id);
    setSaveOpen(false);
    setSaveError("");
  };

  const deleteSaved = () => {
    if (!selectedId) return;
    onSavedFilters(savedFilters.filter((entry) => entry.id !== selectedId));
    setSelectedId("");
  };

  return (
    <div class="filter-bar">
      <input
        class="panel-filter"
        type="search"
        value={query}
        placeholder={placeholder}
        title="Quick text filter; the funnel adds AND/OR conditions"
        aria-label={label}
        onInput={(event) => onQuery((event.target as HTMLInputElement).value)}
        onKeyDown={(event) => {
          if (event.key !== "Escape") return;
          event.preventDefault();
          event.stopPropagation();
          onQuery("");
          (event.target as HTMLInputElement).blur();
        }}
      />
      <button
        ref={trigger}
        type="button"
        class={`mini icon-btn filter-open ${active > 0 ? "on" : ""}`}
        title={
          active > 0
            ? `Advanced filter: ${active} condition${active === 1 ? "" : "s"} — ${summary}. Click to edit; conditions persist with the project.`
            : "Advanced AND/OR filter. Blank conditions are ignored; conditions persist with the project."
        }
        aria-label="Advanced filter"
        aria-expanded={open}
        onClick={() => (open ? setOpen(false) : openEditor())}
      >
        <UiIcon name="funnel" />
        {active > 0 && <span class="filter-count">{active}</span>}
      </button>
      {open && (
        <div
          ref={popover}
          class="filter-popover"
          role="dialog"
          aria-label="Advanced filter"
          style={{ left: `${pos.x}px`, top: `${pos.y}px` }}
          onPointerDown={(event) => event.stopPropagation()}
        >
          <header class="filter-popover-head">
            <strong>Filter</strong>
            <span class="meta">blank conditions are ignored</span>
            <button
              type="button"
              class="mini"
              aria-label="Close filter"
              onClick={() => setOpen(false)}
            >
              ×
            </button>
          </header>
          <div class="filter-popover-body">
            <div class="filter-saved-row">
              <select
                class="filter-saved"
                aria-label="Saved filters"
                title={`Saved ${FILTER_SCOPE_LABELS[scope]} filters. Choosing one replaces the draft; Apply uses it on this panel.`}
                value={selectedId}
                onChange={(event) =>
                  loadSaved((event.target as HTMLSelectElement).value)
                }
              >
                <option value="">Saved filters…</option>
                {scoped.map((entry) => (
                  <option key={entry.id} value={entry.id}>
                    {entry.name}
                  </option>
                ))}
              </select>
              <button
                type="button"
                class="mini"
                disabled={draftCount === 0}
                title={
                  draftCount === 0
                    ? "Add at least one condition first"
                    : selectedId
                      ? "Update this saved filter"
                      : "Save this filter under a name"
                }
                onClick={beginSave}
              >
                {selectedId ? "Save" : "Save as…"}
              </button>
              <button
                type="button"
                class="mini"
                disabled={!selectedId}
                title="Delete the selected saved filter"
                onClick={deleteSaved}
              >
                Delete
              </button>
            </div>
            {saveOpen && (
              <div class="filter-save-row">
                <input
                  class="filter-save-name"
                  autoFocus
                  value={saveName}
                  maxLength={40}
                  placeholder="Filter name"
                  onInput={(event) =>
                    setSaveName((event.target as HTMLInputElement).value)
                  }
                  onKeyDown={(event) => {
                    if (event.key === "Enter") {
                      event.preventDefault();
                      saveAsNew();
                    } else if (event.key === "Escape") {
                      event.preventDefault();
                      setSaveOpen(false);
                    }
                  }}
                />
                <button type="button" class="mini" onClick={saveAsNew}>
                  Save
                </button>
                <button
                  type="button"
                  class="mini"
                  onClick={() => setSaveOpen(false)}
                >
                  Cancel
                </button>
              </div>
            )}
            {saveError && <div class="filter-save-error">{saveError}</div>}
            <GroupEditor
              group={draft as FilterGroup}
              fields={fields}
              depth={0}
              onChange={setDraft}
            />
          </div>
          <footer class="filter-popover-actions">
            <button type="button" onClick={() => setDraft(emptyFilterGroup())}>
              Clear
            </button>
            <span class="spacer" />
            <button type="button" onClick={() => setOpen(false)}>
              Cancel
            </button>
            <button type="button" class="primary" onClick={apply}>
              Apply
            </button>
          </footer>
        </div>
      )}
    </div>
  );
}

