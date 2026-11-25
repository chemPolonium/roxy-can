import type { ComponentChildren } from "preact";
import { UiIcon } from "./icons";

interface Props {
  title: ComponentChildren;
  onClose: () => void;
  closeLabel?: string;
  showClose?: boolean;
  children?: ComponentChildren;
}

/// Shared modal header: title on the left, extra actions then an icon-only close.
export function ModalHeader({
  title,
  onClose,
  closeLabel = "Close",
  showClose = true,
  children,
}: Props) {
  return (
    <header class="modal-head">
      <h2>{title}</h2>
      {children && <div class="modal-head-actions">{children}</div>}
      {showClose && (
        <button
          type="button"
          class="mini icon-btn"
          aria-label={closeLabel}
          title={closeLabel}
          onClick={onClose}
        >
          <UiIcon name="close" />
        </button>
      )}
    </header>
  );
}
