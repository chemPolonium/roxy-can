import type { ComponentChildren } from "preact";
import { useEffect } from "preact/hooks";
import { ModalHeader } from "./ModalHeader";

interface Props {
  title: string;
  message: ComponentChildren;
  confirmLabel?: string;
  cancelLabel?: string;
  danger?: boolean;
  /// False for confirmations with only one safe outcome (e.g. Exit): no X,
  /// no Cancel, and no backdrop/Escape dismissal.
  dismissable?: boolean;
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
  dismissable = true,
  onConfirm,
  onClose,
}: Props) {
  useEffect(() => {
    const close = (event: KeyboardEvent) => {
      if (event.key !== "Escape") return;
      event.preventDefault();
      event.stopImmediatePropagation();
      if (dismissable) onClose();
    };
    window.addEventListener("keydown", close, true);
    return () => window.removeEventListener("keydown", close, true);
  }, [dismissable, onClose]);

  return (
    <div class="modal-backdrop" onMouseDown={dismissable ? onClose : undefined}>
      <div
        class="confirm-modal"
        role="dialog"
        aria-modal="true"
        aria-label={title}
        onMouseDown={(event) => event.stopPropagation()}
      >
        <ModalHeader title={title} onClose={onClose} showClose={dismissable} />
        <div class="confirm-body">{message}</div>
        <footer class="confirm-actions">
          {dismissable && (
            <button type="button" onClick={onClose}>
              {cancelLabel}
            </button>
          )}
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
