use crate::app::App;
use dear_imgui_rs::{StyleVar, TableColumnSetup, Ui};

/// The two help popups' open flags. No commands: opening/closing them is
/// UI-only.
#[derive(Default)]
pub struct HelpView {
    pub shortcuts: bool,
    pub about: bool,
}

pub(crate) fn popup_is_open(ui: &Ui, id: &str) -> bool {
    ui.is_popup_open(id)
}

pub fn render(app: &mut App, ui: &Ui) {
    let mut view = std::mem::take(&mut app.help_view);
    view.show(ui);
    app.help_view = view;
}

impl HelpView {
    pub fn show(&mut self, ui: &Ui) {
        if self.shortcuts {
            const ID: &str = "Shortcuts##help_shortcuts";
            if !popup_is_open(ui, ID) {
                ui.open_popup(ID);
            }
            let mut open = true;
            let min = ui.push_style_var(StyleVar::WindowMinSize([380.0, 0.0]));
            ui.modal_popup_with_opened(ID, &mut open, || {
                ui.table("##shortcut_cols")
                    .add_column(TableColumnSetup::new("Key"))
                    .add_column(TableColumnSetup::new("Action"))
                    .build(|ui| {
                        for (key, desc) in SHORTCUTS {
                            ui.table_next_row();
                            ui.table_next_column();
                            ui.text(key);
                            ui.table_next_column();
                            ui.text(desc);
                        }
                    });
            });
            min.pop();
            self.shortcuts = open;
        }
        if self.about {
            const ID: &str = "About##help_about";
            if !popup_is_open(ui, ID) {
                ui.open_popup(ID);
            }
            let mut open = true;
            let mut close = false;
            ui.modal_popup_with_opened(ID, &mut open, || {
                ui.text(format!("roxy-can {}", env!("CARGO_PKG_VERSION")));
                ui.text("CAN / FlexRay / Ethernet simulation and analysis tool");
                ui.text("Virtual simulation and ASC/BLF replay · DBC and ARXML decoding · multi-window observation · multiple desktops");
                ui.separator();
                if ui.button("Close") {
                    ui.close_current_popup();
                    close = true;
                }
            });
            self.about = open && !close;
        }
    }
}

const SHORTCUTS: [(&str, &str); 11] = [
    ("F9", "Start / stop measurement"),
    ("Space", "Play / pause"),
    ("- / +", "Replay one step slower / faster"),
    ("Home", "Graphics window: jump to live edge"),
    ("Ctrl+N", "New project"),
    ("Ctrl+O", "Open DBC"),
    ("Ctrl+Shift+O", "Open project"),
    ("Ctrl+R", "Toggle log recording (ASC / BLF)"),
    ("Ctrl+E", "Export Trace (ASC / PCAP)"),
    ("Ctrl+S", "Save project"),
    ("Ctrl+Shift+S", "Save project as"),
];
