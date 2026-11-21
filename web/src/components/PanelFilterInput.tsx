import { UiIcon } from "./icons";

interface Props {
  value: string;
  placeholder: string;
  label: string;
  onInput: (value: string) => void;
}

/// Search field shared by the hand-built panel toolbars (Write, SOME/IP).
/// Keeps the same clear button and Esc behaviour as the FilterBar query.
export function PanelFilterInput({
  value,
  placeholder,
  label,
  onInput,
}: Props) {
  return (
    <>
      <input
        class="panel-filter"
        type="search"
        value={value}
        placeholder={placeholder}
        aria-label={label}
        onInput={(event) => onInput((event.target as HTMLInputElement).value)}
        onKeyDown={(event) => {
          if (event.key === "Escape" && value) {
            event.preventDefault();
            event.stopPropagation();
            onInput("");
            event.currentTarget.blur();
          }
        }}
      />
      {value && (
        <button
          type="button"
          class="mini icon-btn"
          title="Clear the filter"
          aria-label="Clear filter"
          onClick={() => onInput("")}
        >
          <UiIcon name="close" />
        </button>
      )}
    </>
  );
}
