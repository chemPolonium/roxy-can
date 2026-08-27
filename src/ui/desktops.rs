use crate::app::{App, STATUSBAR_H, TABSTRIP_H};
use imgui::{Condition, Ui, WindowFlags};

/// CANoe-style desktop tab strip above the status bar: each tab is a named
/// workspace arrangement (open windows/panels + layout); clicking switches.
pub fn render(app: &mut App, ui: &Ui) {
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
    let pad = ui.push_style_var(imgui::StyleVar::WindowPadding([4.0, 3.0]));
    ui.window("##desktops")
        .flags(flags)
        .position(
            [0.0, io.display_size[1] - STATUSBAR_H - TABSTRIP_H],
            Condition::Always,
        )
        .size([io.display_size[0], TABSTRIP_H], Condition::Always)
        .build(|| {
            let names: Vec<String> = app.desktops.iter().map(|d| d.name.clone()).collect();
            for (k, name) in names.iter().enumerate() {
                let selected = k == app.active_desktop;
                let colors = selected.then(|| {
                    (
                        ui.push_style_color(imgui::StyleColor::Button, [0.2, 0.45, 0.75, 1.0]),
                        ui.push_style_color(
                            imgui::StyleColor::ButtonHovered,
                            [0.25, 0.55, 0.85, 1.0],
                        ),
                        ui.push_style_color(imgui::StyleColor::ButtonActive, [0.15, 0.4, 0.7, 1.0]),
                    )
                });
                let label = if selected {
                    format!("[{name}]##desk{k}")
                } else {
                    format!("{name}##desk{k}")
                };
                if ui.small_button(label) {
                    app.switch_desktop(k);
                }
                drop(colors);
                ui.same_line();
            }
            if ui.small_button("+##deskadd") {
                app.add_desktop();
            }
            if app.desktops.len() > 1 {
                ui.same_line();
                if ui.small_button("x##deskdel") {
                    app.delete_desktop(app.active_desktop);
                }
                ui.same_line();
            }
            if ui.small_button("Rename##deskren") {
                app.desktop_rename_target = Some(app.active_desktop);
                app.desktop_rename_buf = app.desktops[app.active_desktop].name.clone();
            }
        });
    pad.pop();
    rename_modal(app, ui);
}

fn rename_modal(app: &mut App, ui: &Ui) {
    let Some(idx) = app.desktop_rename_target else {
        return;
    };
    const ID: &str = "Rename Desktop##deskrename";
    let popup_open = unsafe {
        let id = std::ffi::CString::new(ID).unwrap();
        imgui::sys::igIsPopupOpen_Str(id.as_ptr(), imgui::sys::ImGuiPopupFlags_None as i32)
    };
    if !popup_open {
        ui.open_popup(ID);
    }
    let mut open = true;
    let mut done: Option<Option<String>> = None;
    ui.modal_popup_config(ID).opened(&mut open).build(|| {
        ui.input_text("Name", &mut app.desktop_rename_buf).build();
        if ui.button("OK") {
            done = Some(Some(app.desktop_rename_buf.clone()));
        }
        ui.same_line();
        if ui.button("Cancel") {
            done = Some(None);
        }
    });
    if !open && done.is_none() {
        done = Some(None);
    }
    if let Some(choice) = done {
        if let Some(name) = choice {
            app.rename_desktop(idx, name);
        }
        app.desktop_rename_target = None;
    }
}
