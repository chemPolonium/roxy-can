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

pub(crate) fn file_name(p: &str) -> String {
    std::path::Path::new(p)
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| p.to_string())
}

/// True while a replay is running, i.e. while swapping the selected log would
/// leave the status bar and the scrub bar naming a file the live source is not
/// playing. `App::load_log` refuses too; this only stops the UI from offering
/// what it would reject.
fn log_switch_blocked(app: &App) -> bool {
    app.snap.measuring && matches!(app.snap.mode, Mode::Replay)
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
            if app.snap.measuring {
                app.stop();
            } else {
                app.play();
            }
        }
        2 => app.toggle_record(),
        3 => app.export_trace_dialog(0),
        4 => app.pick_dbc(),
        5 => app.toggle_play(),
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
                    if ui
                        .menu_item_config("New Project")
                        .shortcut("Ctrl+N")
                        .build()
                    {
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
                                if ui.menu_item(file_name(&p)) {
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
                    if ui
                        .menu_item_config("Save Project")
                        .shortcut("Ctrl+S")
                        .build()
                    {
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
                    if ui
                        .menu_item_config("Open DBC...")
                        .shortcut("Ctrl+O")
                        .build()
                    {
                        app.pick_dbc();
                    }
                    if ui
                        .menu_item_config("Open Log...")
                        .enabled(!log_switch_blocked(app))
                        .build()
                    {
                        app.pick_log();
                    }
                    if !app.recent_dbc.is_empty() {
                        ui.menu("Recent DBC", || {
                            let paths = app.recent_dbc.clone();
                            for p in paths {
                                if ui.menu_item(file_name(&p)) {
                                    app.open_dbc_for(0, p);
                                }
                            }
                        });
                    }
                    if !app.recent_log.is_empty() {
                        ui.menu("Recent Logs", || {
                            let blocked = log_switch_blocked(app);
                            let guard = ui.begin_disabled(blocked);
                            let paths = app.recent_log.clone();
                            for p in paths {
                                if ui.menu_item(file_name(&p)) {
                                    app.load_log(&p);
                                }
                            }
                            guard.end();
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
                        .enabled(!app.snap.measuring)
                        .build()
                    {
                        app.start_selected();
                    }
                    if ui
                        .menu_item_config("Stop")
                        .shortcut("F9")
                        .enabled(app.snap.measuring)
                        .build()
                    {
                        app.stop();
                    }
                    if ui
                        .menu_item_config("Pause Trace")
                        .selected(app.snap.trace_paused)
                        .build()
                    {
                        app.send(crate::bus::BusCommand::SetTracePaused(
                            !app.snap.trace_paused,
                        ));
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
                        .menu_item_config("Triggers")
                        .selected(app.show_triggers)
                        .build()
                    {
                        app.show_triggers = !app.show_triggers;
                    }
                    if ui
                        .menu_item_config("Bus Statistics")
                        .selected(app.show_bus_stats)
                        .build()
                    {
                        app.show_bus_stats = !app.show_bus_stats;
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
                        .menu_item_config("Specification")
                        .selected(app.show_spec)
                        .build()
                    {
                        app.show_spec = !app.show_spec;
                    }
                    if ui
                        .menu_item_config("Measurement Setup")
                        .selected(app.show_measurement)
                        .build()
                    {
                        app.show_measurement = !app.show_measurement;
                    }
                    if ui
                        .menu_item_config("Nodes")
                        .selected(app.show_nodes)
                        .build()
                    {
                        app.show_nodes = !app.show_nodes;
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

            let mut mode_pick = match app.snap.run_mode {
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
            let slower = ui.begin_disabled(!matches!(app.snap.mode, Mode::Replay));
            if ui.button("<<") {
                app.step_replay_speed(-1);
            }
            slower.end();
            ui.same_line();
            // Fixed width so toggling Play/Pause never shifts the buttons
            // behind it. App::toggle_play decides between resuming a scrubbed
            // replay and re-opening the log.
            let label = if app.snap.measuring && !app.snap.trace_paused {
                "Pause"
            } else {
                "Play"
            };
            if ui.button_with_size(label, [60.0, 0.0]) {
                app.toggle_play();
            }
            ui.same_line();
            let faster = ui.begin_disabled(!matches!(app.snap.mode, Mode::Replay));
            if ui.button(">>") {
                app.step_replay_speed(1);
            }
            faster.end();
            ui.same_line();
            let stop = ui.begin_disabled(!app.snap.measuring);
            if ui.button("Stop") {
                app.stop();
            }
            stop.end();
            // The speed ladder drives only the replay clock -- VirtualSource
            // ignores it -- so simulation mode hides it instead of offering
            // a control that does nothing.
            if matches!(app.snap.mode, Mode::Replay) {
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
            }
            ui.same_line();
            // How often number readouts (Data values, Statistics, Messages)
            // re-render; "full" follows the frame rate. Curves and bars
            // always draw at full speed -- a 60 fps stream of changing
            // digits is unreadable, which is what this throttles.
            let rates = [
                "Text: full",
                "Text: 20 Hz",
                "Text: 10 Hz",
                "Text: 5 Hz",
                "Text: 2 Hz",
            ];
            let vals = [0u32, 20, 10, 5, 2];
            let mut pick = vals
                .iter()
                .position(|&v| v == app.text_rate_hz)
                .unwrap_or(2);
            ui.set_next_item_width(104.0);
            if ui.combo_simple_string("##textrate", &mut pick, &rates) {
                app.text_rate_hz = vals[pick];
            }
            // Scrub bar. Live whenever a replay source with a known length
            // exists -- running, paused, or stopped after the log ran out.
            if matches!(app.snap.mode, Mode::Replay) {
                vsep(ui);
                let timeline = app.replay_position();
                let scrub = ui.begin_disabled(timeline.is_none());
                if let Some((pos_s, dur_s)) = timeline {
                    let mut t_s = pos_s.min(dur_s);
                    ui.set_next_item_width(240.0);
                    if ui
                        .slider_config("##scrub", 0.0, dur_s)
                        .display_format("%.2f")
                        .build(&mut t_s)
                    {
                        app.seek_replay_seconds(t_s);
                    }
                } else {
                    ui.set_next_item_width(240.0);
                    let mut unused = 0.0;
                    ui.slider_config("##scrub", 0.0, 1.0)
                        .display_format("log length unknown")
                        .build(&mut unused);
                }
                scrub.end();
            }
            if matches!(app.snap.run_mode, Mode::Virtual) {
                vsep(ui);
                let mut rec = app.snap.recording;
                if ui.checkbox("Record", &mut rec) {
                    app.toggle_record();
                }
                ui.same_line();
                // The baseline-aligned "to" leaves the cursor low; re-anchor
                // so the path input stays level with the Record checkbox.
                let row_top = ui.cursor_pos()[1];
                ui.align_text_to_frame_padding();
                ui.text("to");
                ui.same_line();
                let p = ui.cursor_pos();
                ui.set_cursor_pos([p[0], row_top]);
                ui.set_next_item_width(130.0);
                // The stem is a frontend draft; the recorder receives a
                // copy when Record is ticked (see `App::toggle_record`),
                // so a half-typed path never reaches the bus otherwise.
                ui.input_text("##record", &mut app.record_path_buf)
                    .hint("record")
                    .build();
                ui.same_line();
                ui.align_text_to_frame_padding();
                ui.text("_<date>.asc");
            }
            if matches!(app.snap.run_mode, Mode::Replay) {
                vsep(ui);
                let open = ui.begin_disabled(log_switch_blocked(app));
                if ui.button("Open Log...") {
                    app.pick_log();
                }
                open.end();
            }
        });
}
