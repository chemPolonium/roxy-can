export interface TabOption<T extends string> {
  value: T;
  label: string;
  disabled?: boolean;
}

interface Props<T extends string> {
  value: T;
  options: TabOption<T>[];
  onChange: (value: T) => void;
  label: string;
}

/// Underlined tab row shared by panel views and dialogs.
export function Tabs<T extends string>({
  value,
  options,
  onChange,
  label,
}: Props<T>) {
  return (
    <div class="ui-tabs" role="tablist" aria-label={label}>
      {options.map((option) => (
        <button
          key={option.value}
          type="button"
          role="tab"
          aria-selected={value === option.value}
          disabled={option.disabled}
          class={value === option.value ? "on" : ""}
          onClick={() => onChange(option.value)}
        >
          {option.label}
        </button>
      ))}
    </div>
  );
}
