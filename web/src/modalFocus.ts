import { useEffect } from "preact/hooks";

const FOCUSABLE =
  'button:not(:disabled), input:not(:disabled), select:not(:disabled), ' +
  'textarea:not(:disabled), a[href], [tabindex]:not([tabindex="-1"])';

function focusables(root: HTMLElement): HTMLElement[] {
  return [...root.querySelectorAll<HTMLElement>(FOCUSABLE)].filter(
    (element) => element.offsetParent !== null,
  );
}

/// Keeps Tab inside the topmost open dialog and restores the focus that was
/// active before it opened. Every modal renders `.modal-backdrop` around a
/// `[role="dialog"]`, so one observer covers the whole app; nested dialogs use
/// the last backdrop in the DOM.
export function useModalFocus(): void {
  useEffect(() => {
    let dialog: HTMLElement | null = null;
    let previous: HTMLElement | null = null;

    const sync = () => {
      const backdrops = document.querySelectorAll<HTMLElement>(".modal-backdrop");
      const backdrop = backdrops[backdrops.length - 1];
      const next =
        backdrop?.querySelector<HTMLElement>('[role="dialog"]') ?? null;
      if (next === dialog) return;
      if (dialog) {
        previous?.focus?.();
        previous = null;
      }
      dialog = next;
      if (!dialog) return;
      previous = document.activeElement as HTMLElement | null;
      if (dialog.contains(document.activeElement)) return;
      const items = focusables(dialog);
      (dialog.querySelector<HTMLElement>("[autofocus]") ?? items[0])?.focus();
    };

    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key !== "Tab" || !dialog) return;
      const items = focusables(dialog);
      if (items.length === 0) return;
      const first = items[0];
      const last = items[items.length - 1];
      if (!dialog.contains(document.activeElement)) {
        event.preventDefault();
        first.focus();
      } else if (event.shiftKey && document.activeElement === first) {
        event.preventDefault();
        last.focus();
      } else if (!event.shiftKey && document.activeElement === last) {
        event.preventDefault();
        first.focus();
      }
    };

    // Mouse or script focus must not escape the dialog either.
    const onFocusIn = (event: FocusEvent) => {
      if (!dialog || dialog.contains(event.target as Node)) return;
      focusables(dialog)[0]?.focus();
    };

    const observer = new MutationObserver(sync);
    observer.observe(document.body, { childList: true, subtree: true });
    window.addEventListener("keydown", onKeyDown, true);
    document.addEventListener("focusin", onFocusIn, true);
    sync();
    return () => {
      observer.disconnect();
      window.removeEventListener("keydown", onKeyDown, true);
      document.removeEventListener("focusin", onFocusIn, true);
    };
  }, []);
}
