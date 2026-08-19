use crate::app::{App, Mode, TOOLBAR_H};
use imgui::{Condition, Ui, WindowFlags};

fn vsep(ui: &Ui) {
    ui.same_line();
    let p = ui.cursor_screen_pos();
    let h = ui.frame_height();
    ui.get_window_draw_list()
        .add_line([p[0], p[1]], [p[0], p[1] + h], [0.4, 0.4, 0.45, 1.0])
        .build();
    ui.dummy([3.0, 0.0]);
    ui.same_line();
}

fn file_name(p: &str) -> String {
    std::path::Path::new(p)
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| p.to_string())
}

pub fn render(app: &mut App, ui: &Ui) {
    let io = ui.io();
    let flags = WindowFlags::NO_TITLE_BAR
        | WindowFlags::MENU_BAR
        | WindowFlags::NO_RESIZE
        | WindowFlags::NO_MOVE
        | WindowFlags::NO_COLLAPSE
        | WindowFlags::NO_SCROLLBAR
        | WindowFlags::NO_SAVED_SETTINGS;
    ui.window("Toolbar")
        .flags(flags)
        .position([0.0, 0.0], Condition::Always)
        .size([io.display_size[0], TOOLBAR_H], Condition::Always)
        .build(|| {
            ui.menu_bar(|| {
                ui.menu("View", || {
                    if ui.menu_item_config("Trace").selected(app.show_trace).build() {
                        app.show_trace = !app.show_trace;
                    }
                    if ui.menu_item_config("Messages").selected(app.show_messages).build() {
                        app.show_messages = !app.show_messages;
                    }
                    ui.menu("Data", || {
                        if ui.menu_item("New Data Window") {
                            app.new_data_window();
                        }
                        ui.separator();
                        for i in 0..app.data_windows.len() {
                            let name = app.data_windows[i].name.clone();
                            let sel = app.data_windows[i].opened;
                            if ui.menu_item_config(&name).selected(sel).build() {
                                app.data_windows[i].opened = !app.data_windows[i].opened;
                            }
                        }
                    });
                    ui.menu("Graphics", || {
                        if ui.menu_item("New Graphics Window") {
                            app.new_graphics_window();
                        }
                        ui.separator();
                        for i in 0..app.graphics.len() {
                            let name = app.graphics[i].name.clone();
                            let sel = app.graphics[i].opened;
                            if ui.menu_item_config(&name).selected(sel).build() {
                                app.graphics[i].opened = !app.graphics[i].opened;
                            }
                        }
                    });
                });
            });

            if app.measuring {
                if ui.button("Stop") {
                    app.stop();
                }
            } else if ui.button("Start") {
                app.start_virtual();
            }
            ui.same_line();
            ui.checkbox("Pause", &mut app.trace_paused);
            vsep(ui);
            let mut rec = app.recording;
            if ui.checkbox("Record", &mut rec) {
                app.toggle_record();
            }
            ui.same_line();
            ui.align_text_to_frame_padding();
            ui.text("to");
            ui.same_line();
            ui.set_next_item_width(130.0);
            ui.input_text("##record", &mut app.record_path)
                .hint("record")
                .build();
            ui.same_line();
            ui.align_text_to_frame_padding();
            ui.text("_<date>.asc");
            vsep(ui);
            if ui.button("Open DBC...") {
                app.pick_dbc();
            }
            if !app.dbc_path.trim().is_empty() {
                ui.same_line();
                ui.align_text_to_frame_padding();
                ui.text(file_name(&app.dbc_path));
            }
            vsep(ui);
            if ui.button("Open ASC...") {
                app.pick_asc();
            }
            if !app.asc_path.trim().is_empty() {
                ui.same_line();
                ui.align_text_to_frame_padding();
                ui.text(file_name(&app.asc_path));
            }
            vsep(ui);
            ui.align_text_to_frame_padding();
            let mode = match app.mode {
                Mode::Virtual => "Virtual",
                Mode::Replay => "Replay",
            };
            ui.text(format!("Mode: {mode}"));
        });
}
