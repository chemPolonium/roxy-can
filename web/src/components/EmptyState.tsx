import type { ComponentChildren } from "preact";

interface Props {
  title: string;
  hint?: string;
  actions?: ComponentChildren;
  children?: ComponentChildren;
  workspace?: boolean;
  className?: string;
}

/// Centered panel empty state: title, one next-step line and optional actions.
export function EmptyState({
  title,
  hint,
  actions,
  children,
  workspace = false,
  className = "",
}: Props) {
  return (
    <div
      class={`empty ${workspace ? "empty-workspace" : ""} ${className}`.trim()}
    >
      <strong>{title}</strong>
      {hint && <span>{hint}</span>}
      {actions && <div class="empty-actions">{actions}</div>}
      {children}
    </div>
  );
}
