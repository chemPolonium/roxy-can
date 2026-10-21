use crate::app::{App, STATUSBAR_H, TABSTRIP_H};
use dear_imgui_rs::{Condition, MouseButton, StyleColor, StyleVar, Ui, WindowFlags};

/// The desktop strip's own state: the rename popup. The list and the active
/// index are workspace state on `App`; every change to them is a command.
#[derive(Default)]
pub struct DesktopsView {
    rename_target: Option<usize>,
    rename_buf: String,
}

pub enum DesktopsCmd {
    Switch(usize),
    Add,
    Delete(usize),
    Move { from: usize, to: usize },
    Rename { index: usize, name: String },
}

/// Desktop tab strip above the status bar: each tab is a named
/// workspace arrangement (open windows/panels + layout); clicking switches.
pub fn render(app: &mut App, ui: &Ui) {
    let mut view = std::mem::take(&mut app.desktops_view);
    let cmds = view.show(ui, app);
    app.desktops_view = view;
    app.apply_desktops_cmds(cmds);
}

impl DesktopsView {
    pub fn show(&mut self, ui: &Ui, app: &App) -> Vec<DesktopsCmd> {
        let mut cmds = Vec::new();
        let io = ui.io();
        let flags = WindowFlags::NO_TITLE_BAR
            | WindowFlags::NO_RESIZE
            | WindowFlags::NO_MOVE
            | WindowFlags::NO_COLLAPSE
            | WindowFlags::NO_SCROLLBAR
            | WindowFlags::NO_SAVED_SETTINGS
            | WindowFlags::NO_FOCUS_ON_APPEARING
            | WindowFlags::NO_NAV
            | WindowFlags::NO_DOCKING;
        // ImGui's default window_min_size (32) would inflate this 22px strip.
        let min = ui.push_style_var(StyleVar::WindowMinSize([0.0, 0.0]));
        let pad = ui.push_style_var(StyleVar::WindowPadding([4.0, 4.5]));
        ui.window("##desktops")
            .flags(flags)
            .position(
                [0.0, io.display_size()[1] - STATUSBAR_H - TABSTRIP_H],
                Condition::Always,
            )
            .size([io.display_size()[0], TABSTRIP_H], Condition::Always)
            .build(|| {
                let names: Vec<String> = app.desktops.iter().map(|d| d.name.clone()).collect();
                for (k, name) in names.iter().enumerate() {
                    let selected = k == app.active_desktop;
                    // Tokens must drop innermost-first: the tuple stores them
                    // in reverse push order.
                    let _colors = selected.then(|| {
                        let button =
                            ui.push_style_color(StyleColor::Button, [0.2, 0.45, 0.75, 1.0]);
                        let hovered =
                            ui.push_style_color(StyleColor::ButtonHovered, [0.25, 0.55, 0.85, 1.0]);
                        let active =
                            ui.push_style_color(StyleColor::ButtonActive, [0.15, 0.4, 0.7, 1.0]);
                        (active, hovered, button)
                    });
                    let label = if selected {
                        format!("[{name}]##desk{k}")
                    } else {
                        format!("{name}##desk{k}")
                    };
                    if ui.button(label) {
                        cmds.push(DesktopsCmd::Switch(k));
                    }
                    if ui.is_item_hovered() && ui.is_mouse_released(MouseButton::Right) {
                        ui.open_popup(format!("##deskctx{k}"));
                    }
                    ui.popup(format!("##deskctx{k}"), || {
                        if ui.menu_item("Rename") {
                            self.rename_target = Some(k);
                            self.rename_buf = app.desktops[k].name.clone();
                        }
                        if app.desktops.len() > 1 && ui.menu_item("Delete") {
                            cmds.push(DesktopsCmd::Delete(k));
                        }
                        if app.desktops.len() > 1 {
                            ui.separator();
                            ui.menu("Move to", || {
                                for i in 0..app.desktops.len() {
                                    let is_current = i == k;
                                    let label = format!("{}", i + 1);
                                    if ui.menu_item_enabled_selected(
                                        &label,
                                        None::<&str>,
                                        is_current,
                                        !is_current,
                                    ) {
                                        cmds.push(DesktopsCmd::Move { from: k, to: i });
                                    }
                                }
                            });
                        }
                    });
                    ui.same_line();
                }
                if ui.button("+##deskadd") {
                    cmds.push(DesktopsCmd::Add);
                }
            });
        pad.pop();
        min.pop();
        self.rename_modal(ui, &mut cmds);
        cmds
    }

    fn rename_modal(&mut self, ui: &Ui, cmds: &mut Vec<DesktopsCmd>) {
        let Some(idx) = self.rename_target else {
            return;
        };
        const ID: &str = "Rename Desktop##deskrename";
        if !ui.is_popup_open(ID) {
            ui.open_popup(ID);
        }
        let mut open = true;
        let mut done: Option<Option<String>> = None;
        let min = ui.push_style_var(StyleVar::WindowMinSize([340.0, 0.0]));
        ui.modal_popup_with_opened(ID, &mut open, || {
            ui.set_next_item_width(-1.0);
            ui.input_text("Name", &mut self.rename_buf).build();
            if ui.button("OK") {
                done = Some(Some(self.rename_buf.clone()));
            }
            ui.same_line();
            if ui.button("Cancel") {
                done = Some(None);
            }
        });
        min.pop();
        if !open && done.is_none() {
            done = Some(None);
        }
        if let Some(choice) = done {
            if let Some(name) = choice {
                cmds.push(DesktopsCmd::Rename { index: idx, name });
            }
            self.rename_target = None;
        }
    }
}

impl App {
    pub(crate) fn apply_desktops_cmds(&mut self, cmds: Vec<DesktopsCmd>) {
        for cmd in cmds {
            match cmd {
                DesktopsCmd::Switch(k) => self.switch_desktop(k),
                DesktopsCmd::Add => self.add_desktop(),
                DesktopsCmd::Delete(k) => self.delete_desktop(k),
                DesktopsCmd::Move { from, to } => self.move_desktop(from, to),
                DesktopsCmd::Rename { index, name } => self.rename_desktop(index, name),
            }
        }
    }
}
