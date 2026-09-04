use crate::app::App;
use imgui::{Condition, TableColumnFlags, TableColumnSetup, TableFlags, Ui};

/// Bus management: rename buses, load a DBC per bus, add/remove buses.
pub fn render(app: &mut App, ui: &Ui) {
    if !app.show_buses {
        return;
    }
    let io = ui.io();
    let mut open = app.show_buses;
    ui.window("Buses")
        .opened(&mut open)
        .position(
            [io.display_size[0] * 0.38, io.display_size[1] * 0.25],
            Condition::FirstUseEver,
        )
        .size([480.0, 240.0], Condition::FirstUseEver)
        .build(|| content(app, ui));
    app.show_buses = open;
}

fn file_name(p: &str) -> String {
    std::path::Path::new(p)
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| p.to_string())
}

fn content(app: &mut App, ui: &Ui) {
    if ui.small_button("+ Add bus") {
        app.add_channel();
    }
    ui.same_line();
    ui.text(format!("{} bus(es)", app.snap.channel_count));
    ui.separator();

    // NO_BORDERS_IN_BODY restricts column-resize dragging to the header row.
    let flags = TableFlags::BORDERS_INNER
        | TableFlags::ROW_BG
        | TableFlags::RESIZABLE
        | TableFlags::NO_BORDERS_IN_BODY
        | TableFlags::SCROLL_Y
        | TableFlags::SIZING_STRETCH_PROP;
    let mut remove: Option<usize> = None;
    {
        let Some(_table) = ui.begin_table_with_flags("bus_table", 4, flags) else {
            return;
        };
        ui.table_setup_column_with(TableColumnSetup {
            flags: TableColumnFlags::WIDTH_STRETCH,
            init_width_or_weight: 1.0,
            ..TableColumnSetup::new("Name")
        });
        ui.table_setup_column_with(TableColumnSetup {
            flags: TableColumnFlags::WIDTH_STRETCH,
            init_width_or_weight: 1.6,
            ..TableColumnSetup::new("DBC")
        });
        ui.table_setup_column_with(TableColumnSetup {
            flags: TableColumnFlags::WIDTH_FIXED,
            init_width_or_weight: 130.0,
            ..TableColumnSetup::new("kbit/s (arb / FD data)")
        });
        ui.table_setup_column_with(TableColumnSetup {
            flags: TableColumnFlags::WIDTH_FIXED,
            init_width_or_weight: 26.0,
            ..TableColumnSetup::new("")
        });
        ui.table_headers_row();

        // The rows render from the snapshot; edits are frontend drafts
        // that commit as commands.
        let views: Vec<(String, String, u32, u32)> = app
            .snap
            .channels
            .iter()
            .map(|c| {
                (
                    c.name.clone(),
                    c.dbc_path.clone(),
                    c.bitrate_kbps,
                    c.fd_data_kbps,
                )
            })
            .collect();
        for (i, (name, path, arb_kbps, data_kbps)) in views.into_iter().enumerate() {
            ui.table_next_row();
            if !ui.table_next_column() {
                continue;
            }
            ui.set_next_item_width(-1.0);
            // The rename draft lives in `bus_name_edit` while the box has
            // focus; the bus sees the new name when the edit commits.
            let editing = matches!(&app.bus_name_edit, Some((r, _)) if *r == i);
            let mut name_buf = match &app.bus_name_edit {
                Some((r, s)) if *r == i => s.clone(),
                _ => name,
            };
            ui.input_text(format!("##busname{i}"), &mut name_buf)
                .build();
            if ui.is_item_active() {
                app.bus_name_edit = Some((i, name_buf.clone()));
            }
            if ui.is_item_deactivated_after_edit() {
                app.bus_name_edit = None;
                app.send(crate::bus::BusCommand::SetChannelConfig {
                    ch: i as u8,
                    name: Some(name_buf),
                    dbc_path: None,
                    bitrate_kbps: None,
                    fd_data_kbps: None,
                    sim_nodes: None,
                });
            } else if editing && !ui.is_item_active() {
                app.bus_name_edit = None;
            }
            ui.table_next_column();
            // Aligning the label to the frame baseline leaves the cursor a
            // few pixels low; without re-anchoring, the Open button after
            // `same_line` draws visibly below the row's input widgets.
            let cell_top = ui.cursor_pos()[1];
            ui.align_text_to_frame_padding();
            ui.text(if path.trim().is_empty() {
                "(none)".to_string()
            } else {
                file_name(&path)
            });
            ui.same_line();
            let p = ui.cursor_pos();
            ui.set_cursor_pos([p[0], cell_top]);
            if ui.small_button(format!("Open...##busdbc{i}")) {
                app.pick_dbc_for(i);
            }
            ui.table_next_column();
            // The load view divides wire bits by these; there is no hardware
            // behind the simulation, so the values are declarations about the
            // bus being analysed, not device settings. Each accepted step is
            // its own command -- there is no draft state worth the trouble
            // for two integers.
            ui.set_next_item_width(56.0);
            let mut arb = arb_kbps as i32;
            if ui
                .input_int(format!("##busarb{i}"), &mut arb)
                .step(50)
                .step_fast(500)
                .build()
            {
                app.send(crate::bus::BusCommand::SetChannelConfig {
                    ch: i as u8,
                    name: None,
                    dbc_path: None,
                    bitrate_kbps: Some(arb.max(1) as u32),
                    fd_data_kbps: None,
                    sim_nodes: None,
                });
            }
            ui.same_line();
            ui.set_next_item_width(56.0);
            let mut data = data_kbps as i32;
            if ui
                .input_int(format!("##busdata{i}"), &mut data)
                .step(100)
                .step_fast(1000)
                .build()
            {
                app.send(crate::bus::BusCommand::SetChannelConfig {
                    ch: i as u8,
                    name: None,
                    dbc_path: None,
                    bitrate_kbps: None,
                    fd_data_kbps: Some(data.max(1) as u32),
                    sim_nodes: None,
                });
            }
            ui.table_next_column();
            if ui.small_button(format!("x##busrm{i}")) {
                remove = Some(i);
            }
        }
    }
    if let Some(i) = remove {
        app.remove_channel(i);
    }
}
