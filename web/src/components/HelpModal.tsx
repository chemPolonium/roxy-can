import { useEffect } from "preact/hooks";

interface Props {
  onClose: () => void;
}

const SHORTCUTS: [string, string][] = [
  ["Ctrl+K", "Command palette"],
  ["F9", "Start / stop the measurement"],
  ["Ctrl+S", "Save the open project"],
  ["Ctrl+Shift+S", "Open Session (Save as / switch session)"],
  ["Ctrl+O", "Open Session"],
  ["Ctrl+E", "Export the trace"],
  ["?", "This help"],
  ["Esc", "Close the front dialog"],
];

export function HelpModal({ onClose }: Props) {
  useEffect(() => {
    const close = (event: KeyboardEvent) => {
      if (event.key === "Escape") onClose();
    };
    window.addEventListener("keydown", close);
    return () => window.removeEventListener("keydown", close);
  }, [onClose]);

  return (
    <div class="modal-backdrop" onMouseDown={onClose}>
      <div
        class="help-modal"
        role="dialog"
        aria-modal="true"
        aria-label="Help"
        onMouseDown={(event) => event.stopPropagation()}
      >
        <header class="modal-head">
          <h2>roxy-can web</h2>
          <button type="button" onClick={onClose}>
            Close
          </button>
        </header>
        <div class="help-body">
          <p class="hint">
            roxy-can is a CAN / FlexRay / Ethernet simulation and analysis tool
            running entirely in this local web UI. Files and hardware live on
            the host.
          </p>
          <table class="help-keys">
            <tbody>
              {SHORTCUTS.map(([key, action]) => (
                <tr key={key}>
                  <th>{key}</th>
                  <td>{action}</td>
                </tr>
              ))}
            </tbody>
          </table>
          <p class="hint">
            Shortcuts are ignored while typing in a field. Browser-reserved
            combinations other than the ones above are left to the browser.
          </p>
        </div>
      </div>
    </div>
  );
}
