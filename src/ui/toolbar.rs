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

fn tip(ui: &Ui, text: &str) {
    if ui.is_item_hovered() {
        ui.tooltip_text(text);
    }
}

/// Global shortcuts dispatched from winit key events (see `crate::CMD`):
/// F9 start/stop, Ctrl+R record, Ctrl+E export, Ctrl+O open DBC.
fn shortcuts(app: &mut App, ui: &Ui) {
    let cmd = crate::CMD.swap(0, std::sync::atomic::Ordering::Relaxed);
    if cmd == 0 {
        return;
    }
    if cmd != 1 && ui.io().want_text_input {
        return;
    }
    match cmd {
        1 => {
            if app.measuring {
                app.stop();
            } else {
                app.start_selected();
            }
        }
        2 => app.toggle_record(),
        3 => app.export_trace_dialog(0),
        4 => app.pick_dbc(),
        _ => {}
    }
}

pub fn render(app: &mut App, ui: &Ui) {
    shortcuts(app, ui);
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
                ui.menu("File", || {
                    if ui.menu_item("Open DBC...\tCtrl+O") {
                        app.pick_dbc();
                    }
                    if ui.menu_item("Open ASC...") {
                        app.pick_asc();
                    }
                    ui.separator();
                    if ui.menu_item("Export Trace ASC...\tCtrl+E") {
                        app.export_trace_dialog(0);
                    }
                    ui.separator();
                    if ui.menu_item("Exit") {
                        app.quit = true;
                    }
                });
                ui.menu("Measurement", || {
                    if ui
                        .menu_item_config("Start\tF9")
                        .enabled(!app.measuring)
                        .build()
                    {
                        app.start_selected();
                    }
                    if ui
                        .menu_item_config("Stop\tF9")
                        .enabled(app.measuring)
                        .build()
                    {
                        app.stop();
                    }
                    if ui
                        .menu_item_config("Pause Trace")
                        .selected(app.trace_paused)
                        .build()
                    {
                        app.trace_paused = !app.trace_paused;
                    }
                });
                ui.menu("View", || {
                    if ui
                        .menu_item_config("Buses")
                        .selected(app.show_buses)
                        .build()
                    {
                        app.show_buses = !app.show_buses;
                    }
                    if ui
                        .menu_item_config("Interactive Generator")
                        .selected(app.show_tx)
                        .build()
                    {
                        app.show_tx = !app.show_tx;
                    }
                    if ui
                        .menu_item_config("Network")
                        .selected(app.show_network)
                        .build()
                    {
                        app.show_network = !app.show_network;
                    }
                    if ui
                        .menu_item_config("Measurement Setup")
                        .selected(app.show_measurement)
                        .build()
                    {
                        app.show_measurement = !app.show_measurement;
                    }
                });
            });

            ui.radio_button("Simulation", &mut app.run_mode, Mode::Virtual);
            tip(
                ui,
                "Start runs the virtual buses driven by the Interactive Generator",
            );
            ui.same_line();
            ui.radio_button("Replay", &mut app.run_mode, Mode::Replay);
            tip(
                ui,
                "Start replays the loaded ASC log (opens a picker if none is loaded)",
            );
            vsep(ui);
            if app.measuring {
                if ui.button("Stop") {
                    app.stop();
                }
                tip(ui, "Stop measurement (F9)");
            } else if ui.button("Start") {
                app.start_selected();
            }
            if !app.measuring {
                let what = match app.run_mode {
                    Mode::Virtual => "simulation",
                    Mode::Replay => "replay",
                };
                tip(ui, &format!("Start {what} (F9)"));
            }
            ui.same_line();
            ui.checkbox("Pause", &mut app.trace_paused);
            tip(
                ui,
                "Freeze the trace; replay playback stops and resumes in place",
            );
            vsep(ui);
            let mut rec = app.recording;
            if ui.checkbox("Record", &mut rec) {
                app.toggle_record();
            }
            tip(ui, "Record ASC (Ctrl+R)");
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
            if app.dbc_pick >= app.channels.len() {
                app.dbc_pick = 0;
            }
            let ch_names: Vec<String> = (0..app.channels.len())
                .map(|i| app.channel_name(i as u8))
                .collect();
            ui.set_next_item_width(64.0);
            ui.combo_simple_string("##dbcch", &mut app.dbc_pick, &ch_names);
            tip(ui, "Bus to load the next DBC into");
            ui.same_line();
            if ui.button("Open DBC...") {
                app.pick_dbc();
            }
            tip(ui, "Open a DBC for the selected bus (Ctrl+O)");
            let path = app
                .channels
                .get(app.dbc_pick)
                .map(|c| c.dbc_path.clone())
                .unwrap_or_default();
            if !path.trim().is_empty() {
                ui.same_line();
                ui.align_text_to_frame_padding();
                ui.text(file_name(&path));
            }
            vsep(ui);
            if ui.button("Open ASC...") {
                app.pick_asc();
            }
            tip(
                ui,
                "Load an ASC log for replay (Start in Replay mode plays it)",
            );
            if !app.asc_path.trim().is_empty() {
                ui.same_line();
                ui.align_text_to_frame_padding();
                ui.text(file_name(&app.asc_path));
            }
            vsep(ui);
            ui.align_text_to_frame_padding();
            ui.text("Speed");
            ui.same_line();
            const SPEEDS: [f64; 4] = [0.5, 1.0, 2.0, 4.0];
            let labels = ["0.5x", "1x", "2x", "4x"];
            let mut pick = SPEEDS
                .iter()
                .position(|s| (*s - app.replay_speed).abs() < 1e-9)
                .unwrap_or(1);
            ui.set_next_item_width(64.0);
            if ui.combo_simple_string("##speed", &mut pick, &labels) {
                app.set_replay_speed(SPEEDS[pick]);
            }
            tip(ui, "Replay playback speed (applies immediately)");
            vsep(ui);
            ui.align_text_to_frame_padding();
            let mode = match app.mode {
                Mode::Virtual => "Virtual",
                Mode::Replay => "Replay",
            };
            ui.text(format!("Mode: {mode}"));
        });
}
