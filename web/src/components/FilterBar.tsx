import { useEffect, useRef, useState } from "preact/hooks";
import { UiIcon } from "./icons";
import {
  FILTER_OP_LABELS,
  emptyFilterGroup,
  filterRuleCount,
  newFilterGroup,
  newFilterRule,
  pruneFilter,
  type FilterFieldSpec,
  type FilterGroup,
  type FilterNode,
  type FilterRule,
} from "../filterTree";

interface Props {
  query: string;
  onQuery: (value: string) => void;
  filter: FilterNode;
  onFilter: (filter: FilterNode) => void;
  fields: FilterFieldSpec[];
  label: string;
  placeholder: string;
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
  return (
    <div class="filter-rule-row">
      <select
        class="filter-field"
        aria-label="Filter field"
        value={rule.field}
        onChange={(event) =>
          onChange({
            ...rule,
            field: (event.target as HTMLSelectElement).value,
          })
        }
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
        {Object.entries(FILTER_OP_LABELS).map(([op, label]) => (
          <option key={op} value={op}>
            {label}
          </option>
        ))}
      </select>
      <input
        class="filter-value"
        value={rule.value}
        placeholder="value"
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
              children: [...group.children, newFilterRule(fields[0]?.key ?? "")],
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
}: Props) {
  const [open, setOpen] = useState(false);
  const [draft, setDraft] = useState<FilterNode>(filter);
  const [pos, setPos] = useState({ x: 8, y: 8 });
  const trigger = useRef<HTMLButtonElement>(null);
  const popover = useRef<HTMLDivElement>(null);
  const active = filterRuleCount(filter);

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
    setPos({ x: left, y: top });
    setOpen(true);
  };

  const apply = () => {
    onFilter(pruneFilter(draft) ?? emptyFilterGroup());
    setOpen(false);
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
            ? `Advanced filter: ${active} condition${active === 1 ? "" : "s"}`
            : "Advanced AND/OR filter"
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

