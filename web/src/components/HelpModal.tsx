import { useEffect } from "preact/hooks";

interface Props {
  onClose: () => void;
}

const SHORTCUTS: [string, string][] = [
  ["Ctrl+K", "Command palette"],
  ["Ctrl+G", "Go to time in Trace, chart and replay"],
  ["Alt+1…4", "Switch Focus / Split / Main + side / Quad layout"],
  ["Alt+Shift+1…2", "Show Trace / Messages in the focused page"],
  ["Alt+Shift+P", "Switch Standard / Panel"],
  ["F9", "Start / stop the measurement"],
  ["F10", "Pause / resume the measurement"],
  ["F2", "Focus the current filter box"],
  ["Ctrl+S", "Save the open project; asks for a file when untitled"],
  ["Ctrl+Shift+S", "Save the project under a new name"],
  ["Ctrl+O", "Open a project"],
  ["Ctrl+E", "Export the visible Trace or Messages page"],
  ["?", "This help"],
  ["Esc", "Restore a maximized page / close the front dialog"],
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
          <h2>Roxy CAN</h2>
          <button type="button" onClick={onClose}>
            Close
          </button>
        </header>
        <div class="help-body">
          <p class="hint">
            Roxy CAN is a CAN / FlexRay / Ethernet simulation and analysis tool
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
