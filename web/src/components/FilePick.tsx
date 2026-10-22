import { useRef, useState } from "preact/hooks";
import { uploadFile } from "../api";

interface Props {
  label: string;
  accept: string;
  onPick: (path: string) => void | Promise<void>;
  onError?: (message: string) => void;
  class?: string;
}

/// Opens the browser's file picker, uploads the chosen file to the host's
/// temp folder and hands the returned path to the caller. The input is reset
/// after every pick so choosing the same file twice still fires a change.
export function FilePick({ label, accept, onPick, onError, class: cls }: Props) {
  const input = useRef<HTMLInputElement>(null);
  const [busy, setBusy] = useState(false);
  return (
    <>
      <button
        type="button"
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
            const path = await uploadFile(file);
            await onPick(path);
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
