import type { ComponentChildren } from "preact";

interface Props {
  title: ComponentChildren;
  onClose: () => void;
  closeLabel?: string;
  children?: ComponentChildren;
}

/// Shared modal header: title on the left, extra actions then Close.
export function ModalHeader({
  title,
  onClose,
  closeLabel = "Close",
  children,
}: Props) {
  return (
    <header class="modal-head">
      <h2>{title}</h2>
      {children && <div class="modal-head-actions">{children}</div>}
      <button type="button" onClick={onClose}>
        {closeLabel}
      </button>
    </header>
  );
}
