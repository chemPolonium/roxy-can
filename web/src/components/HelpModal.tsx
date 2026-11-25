import { useEffect } from "preact/hooks";
import { ModalHeader } from "./ModalHeader";

interface Props {
  onClose: () => void;
}

interface ShortcutGroup {
  title: string;
  rows: [string, string][];
}

const SHORTCUT_GROUPS: ShortcutGroup[] = [
  {
    title: "Global",
    rows: [
      ["Ctrl+K", "Command palette"],
      ["Ctrl+O", "Open a project"],
      ["Ctrl+S", "Save the project"],
      ["Ctrl+Shift+S", "Save the project under a new name"],
      ["Ctrl+E", "Export the visible Trace or Messages page"],
      ["Ctrl+G", "Go to time in Trace, chart and replay"],
      ["Ctrl+Q", "Open the exit confirmation"],
      ["F9", "Start / stop the measurement"],
      ["F10", "Pause / resume the measurement"],
      ["F2", "Focus the current filter box; Esc clears it"],
      [
        "Alt+1…4",
        "Switch Focus / Split / Main + side / Quad layout (Standard)",
      ],
      ["Alt+Shift+1…2", "Show Trace / Messages in the focused page"],
      ["Alt+Shift+P", "Switch Standard / Panel"],
      ["← / →", "Previous / next page tab on the Panel page"],
      ["?", "This help"],
    ],
  },
  {
    title: "Script editor",
    rows: [
      ["F9", "Toggle a breakpoint at the caret; it does not start / stop the measurement"],
      ["Ctrl+S", "Save the linked .rxcan file; it does not save the project"],
      ["Tab", "Accept the open completion; otherwise indent"],
      ["Shift+Tab", "Outdent"],
      ["Alt+/", "Open the completion list"],
      ["Enter", "Accept the selected completion"],
      ["Ctrl+F", "Find / replace in the script"],
      ["Ctrl+/", "Comment or uncomment the current line"],
      ["Alt+↑ / Alt+↓", "Move the current line up / down"],
      ["Shift+Alt+↓", "Duplicate the current line"],
      ["Ctrl+Z", "Undo"],
      ["Ctrl+Shift+Z / Ctrl+Y", "Redo"],
      ["Esc", "Close search / completion first, then the editor"],
    ],
  },
  {
    title: "Dialogs and menus",
    rows: [
      ["Esc", "Close or cancel the front dialog / menu"],
      ["Enter", "Accept the selected menu or list item; commit a field"],
      ["↑ / ↓", "Move through a menu or list"],
      ["Tab / Shift+Tab", "Move focus"],
    ],
  },
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
        <ModalHeader title="Roxy CAN" onClose={onClose} />
        <div class="help-body">
          <p class="hint">
            Roxy CAN is a CAN / FlexRay / Ethernet simulation and analysis tool
            running entirely in this local web UI. Files and hardware live on
            the host.
          </p>
          {SHORTCUT_GROUPS.map((group) => (
            <section class="help-shortcut-group" key={group.title}>
              <h3>{group.title}</h3>
              <table class="help-keys">
                <tbody>
                  {group.rows.map(([key, action]) => (
                    <tr key={key}>
                      <th>{key}</th>
                      <td>{action}</td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </section>
          ))}
          <p class="hint">
            Global shortcuts pause while a dialog or menu is open, and while
            typing in a field. The script editor uses its own keys for F9 and
            Ctrl+S. Browser-reserved combinations are left to the browser.
          </p>
        </div>
      </div>
    </div>
  );
}
