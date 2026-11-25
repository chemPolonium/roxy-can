import type { ComponentChildren } from "preact";
import { useEffect } from "preact/hooks";
import { ModalHeader } from "./ModalHeader";

interface Props {
  title: string;
  message: ComponentChildren;
  confirmLabel?: string;
  cancelLabel?: string;
  danger?: boolean;
  /// The X in the header duplicates Cancel; hide it when a dialog already
  /// has a Cancel button in the footer.
  showClose?: boolean;
  onConfirm: () => void;
  onClose: () => void;
}

/// In-app replacement for window.confirm: same backdrop, focus trap and
/// button language as the other dialogs.
export function ConfirmDialog({
  title,
  message,
  confirmLabel = "Remove",
  cancelLabel = "Cancel",
  danger = false,
  showClose = true,
  onConfirm,
  onClose,
}: Props) {
  useEffect(() => {
    const close = (event: KeyboardEvent) => {
      if (event.key !== "Escape") return;
      event.preventDefault();
      event.stopImmediatePropagation();
      onClose();
    };
    window.addEventListener("keydown", close, true);
    return () => window.removeEventListener("keydown", close, true);
  }, [onClose]);

  return (
    <div class="modal-backdrop" onMouseDown={onClose}>
      <div
        class="confirm-modal"
        role="dialog"
        aria-modal="true"
        aria-label={title}
        onMouseDown={(event) => event.stopPropagation()}
      >
        <ModalHeader title={title} onClose={onClose} showClose={showClose} />
        <div class="confirm-body">{message}</div>
        <footer class="confirm-actions">
          <button type="button" onClick={onClose}>
            {cancelLabel}
          </button>
          <button
            type="button"
            class={danger ? "danger" : "primary"}
            onClick={onConfirm}
          >
            {confirmLabel}
          </button>
        </footer>
      </div>
    </div>
  );
}
