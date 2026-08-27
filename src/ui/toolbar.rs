use crate::app::{App, Mode, REPLAY_SPEEDS, TOOLBAR_H};
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

/// Global shortcuts dispatched from winit key events (see `crate::CMD`):
/// F9 start/stop, Ctrl+R record, Ctrl+E export, Ctrl+O open DBC,
/// Space play/pause, -/+ replay speed, Home jump to the live edge,
/// Ctrl+S save, Ctrl+Shift+S save as, Ctrl+N new project,
/// Ctrl+Shift+O open project.
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
        5 => {
            if app.measuring {
                app.trace_paused = !app.trace_paused;
            } else {
                app.start_selected();
            }
        }
        6 => app.step_replay_speed(-1),
        7 => app.step_replay_speed(1),
        8 => app.jump_to_live(),
        9 => match app.project_path.clone() {
            Some(p) => {
                app.save_project(Some(p));
            }
            None => {
                app.save_project(None);
            }
        },
        10 => {
            app.save_project(None);
        }
        11 => app.guarded_action(crate::app::PendingAction::NewProject),
        12 => app.guarded_action(crate::app::PendingAction::OpenProject),
        _ => {}
    }
}

pub fn render(app: &mut App, ui: &Ui) {
    shortcuts(app, ui);
    let io = ui.io();
    // NO_DOCKING keeps observer windows from docking into the toolbar itself.
    let flags = WindowFlags::NO_TITLE_BAR
        | WindowFlags::MENU_BAR
        | WindowFlags::NO_RESIZE
        | WindowFlags::NO_MOVE
        | WindowFlags::NO_COLLAPSE
        | WindowFlags::NO_SCROLLBAR
        | WindowFlags::NO_DOCKING
        | WindowFlags::NO_SAVED_SETTINGS;
    ui.window("Toolbar")
        .flags(flags)
        .position([0.0, 0.0], Condition::Always)
        .size([io.display_size[0], TOOLBAR_H], Condition::Always)
        .build(|| {
            ui.menu_bar(|| {
                ui.menu("File", || {
                    if ui.menu_item_config("New Project").shortcut("Ctrl+N").build() {
                        app.guarded_action(crate::app::PendingAction::NewProject);
                    }
                    if ui
                        .menu_item_config("Open Project...")
                        .shortcut("Ctrl+Shift+O")
                        .build()
                    {
                        app.guarded_action(crate::app::PendingAction::OpenProject);
                    }
                    if !app.recent_projects.is_empty() {
                        ui.menu("Recent Projects", || {
                            let paths = app.recent_projects.clone();
                            for p in paths {
                                if ui.menu_item(&file_name(&p)) {
                                    let path = std::path::PathBuf::from(&p);
                                    if let Some(cur) = app.project_path.clone() {
                                        app.save_project(Some(cur));
                                        app.open_project_path(&path);
                                    } else if app.is_dirty() {
                                        app.pending_action =
                                            Some(crate::app::PendingAction::OpenPath(path));
                                    } else {
                                        app.open_project_path(&path);
                                    }
                                }
                            }
                        });
                    }
                    if ui.menu_item_config("Save Project").shortcut("Ctrl+S").build() {
                        match app.project_path.clone() {
                            Some(p) => {
                                app.save_project(Some(p));
                            }
                            None => {
                                app.save_project(None);
                            }
                        }
                    }
                    if ui
                        .menu_item_config("Save Project As...")
                        .shortcut("Ctrl+Shift+S")
                        .build()
                    {
                        app.save_project(None);
                    }
                    ui.separator();
                    if ui.menu_item_config("Open DBC...").shortcut("Ctrl+O").build() {
                        app.pick_dbc();
                    }
                    if ui.menu_item("Open ASC...") {
                        app.pick_asc();
                    }
                    if !app.recent_dbc.is_empty() {
                        ui.menu("Recent DBC", || {
                            let paths = app.recent_dbc.clone();
                            for p in paths {
                                if ui.menu_item(&file_name(&p)) {
                                    app.open_dbc_for(0, p);
                                }
                            }
                        });
                    }
                    if !app.recent_asc.is_empty() {
                        ui.menu("Recent ASC", || {
                            let paths = app.recent_asc.clone();
                            for p in paths {
                                if ui.menu_item(&file_name(&p)) {
                                    app.load_asc(&p);
                                }
                            }
                        });
                    }
                    ui.separator();
                    if ui
                        .menu_item_config("Export Trace ASC...")
                        .shortcut("Ctrl+E")
                        .build()
                    {
                        app.export_trace_dialog(0);
                    }
                    ui.separator();
                    if ui.menu_item("Exit") {
                        app.request_quit();
                    }
                });
                ui.menu("Measurement", || {
                    if ui
                        .menu_item_config("Start")
                        .shortcut("F9")
                        .enabled(!app.measuring)
                        .build()
                    {
                        app.start_selected();
                    }
                    if ui
                        .menu_item_config("Stop")
                        .shortcut("F9")
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
                ui.menu("Help", || {
                    if ui.menu_item("Shortcuts") {
                        app.show_shortcuts = true;
                    }
                    if ui.menu_item("About") {
                        app.show_about = true;
                    }
                });
            });

            let mut mode_pick = match app.run_mode {
                Mode::Virtual => 0,
                Mode::Replay => 1,
            };
            ui.set_next_item_width(110.0);
            if ui.combo_simple_string("##runmode", &mut mode_pick, &["Simulation", "Replay"]) {
                app.switch_run_mode(match mode_pick {
                    0 => Mode::Virtual,
                    _ => Mode::Replay,
                });
            }
            vsep(ui);
            // Player-style transport: slower | play/pause | faster | stop.
            let slower = ui.begin_disabled(!matches!(app.mode, Mode::Replay));
            if ui.button("<<") {
                app.step_replay_speed(-1);
            }
            slower.end();
            ui.same_line();
            // Fixed width so toggling Play/Pause never shifts the buttons
            // behind it.
            if !app.measuring {
                if ui.button_with_size("Play", [60.0, 0.0]) {
                    app.start_selected();
                }
            } else if app.trace_paused {
                if ui.button_with_size("Play", [60.0, 0.0]) {
                    app.trace_paused = false;
                }
            } else {
                if ui.button_with_size("Pause", [60.0, 0.0]) {
                    app.trace_paused = true;
                }
            }
            ui.same_line();
            let faster = ui.begin_disabled(!matches!(app.mode, Mode::Replay));
            if ui.button(">>") {
                app.step_replay_speed(1);
            }
            faster.end();
            ui.same_line();
            let stop = ui.begin_disabled(!app.measuring);
            if ui.button("Stop") {
                app.stop();
            }
            stop.end();
            ui.same_line();
            let labels = ["0.5x", "1x", "2x", "4x"];
            let mut pick = REPLAY_SPEEDS
                .iter()
                .position(|s| (*s - app.replay_speed).abs() < 1e-9)
                .unwrap_or(1);
            ui.set_next_item_width(64.0);
            if ui.combo_simple_string("##speed", &mut pick, &labels) {
                app.set_replay_speed(REPLAY_SPEEDS[pick]);
            }
            if matches!(app.run_mode, Mode::Virtual) {
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
            }
            if matches!(app.run_mode, Mode::Replay) {
                vsep(ui);
                if ui.button("Open ASC...") {
                    app.pick_asc();
                }
                if !app.asc_path.trim().is_empty() {
                    ui.same_line();
                    ui.align_text_to_frame_padding();
                    ui.text(file_name(&app.asc_path));
                }
            }
        });
}
