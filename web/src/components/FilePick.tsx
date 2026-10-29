import { useRef, useState } from "preact/hooks";
import { uploadFile } from "../api";

interface Props {
  label: string;
  accept: string;
  onPick?: (path: string) => void | Promise<void>;
  /// Text consumers (the script editor) read the file in the browser
  /// instead of uploading it.
  onText?: (text: string, name: string) => void;
  onError?: (message: string) => void;
  class?: string;
  /// Render as a menu item inside a custom drop-down.
  menuItem?: boolean;
}

/// Opens the browser's file picker, uploads the chosen file to the host's
/// temp folder and hands the returned path to the caller. The input is reset
/// after every pick so choosing the same file twice still fires a change.
export function FilePick({
  label,
  accept,
  onPick,
  onText,
  onError,
  class: cls,
  menuItem,
}: Props) {
  const input = useRef<HTMLInputElement>(null);
  const [busy, setBusy] = useState(false);
  return (
    <>
      <button
        type="button"
        role={menuItem ? "menuitem" : undefined}
        class={cls ?? "mini"}
        disabled={busy}
        onClick={() => input.current?.click()}
      >
        {busy ? "Uploading…" : label}
      </button>
      <input
        ref={input}
        type="file"
        accept={accept}
        hidden
        onChange={async (event) => {
          const element = event.target as HTMLInputElement;
          const file = element.files?.[0];
          element.value = "";
          if (!file) return;
          setBusy(true);
          try {
            if (onText) {
              onText(await file.text(), file.name);
            } else if (onPick) {
              const path = await uploadFile(file);
              await onPick(path);
            }
          } catch (error) {
            onError?.(error instanceof Error ? error.message : String(error));
          } finally {
            setBusy(false);
          }
        }}
      />
    </>
  );
}
