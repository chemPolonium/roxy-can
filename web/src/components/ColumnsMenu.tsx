import {
  TRACE_COLUMNS,
  type TraceColumnPrefs,
} from "../traceLayout";
import { MenuButton } from "./MenuButton";
import { UiIcon } from "./icons";

export function ColumnsMenu({
  columns,
  onChange,
}: {
  columns: TraceColumnPrefs;
  onChange: (next: TraceColumnPrefs) => void;
}) {
  const hideable = TRACE_COLUMNS.filter(
    (column) => !column.locked && !column.flex,
  );

  return (
    <MenuButton
      className="columns-menu"
      buttonClass={`mini icon-btn ${columns.hidden.length > 0 ? "on" : ""}`}
      ariaLabel="Trace columns"
      title="Show or hide Trace columns"
      menuLabel="Trace columns"
      fixed
      fixedWidth={150}
      label={
        <>
          <UiIcon name="columns" /> <span aria-hidden="true">▾</span>
        </>
      }
    >
      {() =>
        hideable.map((column) => {
          const hidden = columns.hidden.includes(column.id);
          return (
            <button
              type="button"
              role="menuitemcheckbox"
              aria-checked={!hidden}
              key={column.id}
              onClick={() =>
                onChange({
                  ...columns,
                  hidden: hidden
                    ? columns.hidden.filter((id) => id !== column.id)
                    : [...columns.hidden, column.id],
                })
              }
            >
              <span class="menu-check" aria-hidden="true">
                {!hidden ? "✓" : ""}
              </span>
              <span>{column.label}</span>
            </button>
          );
        })
      }
    </MenuButton>
  );
}
