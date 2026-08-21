use crate::app::App;
use imgui::{Condition, TableColumnFlags, TableColumnSetup, TableFlags, Ui, WindowFlags};

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
        .flags(WindowFlags::NO_SAVED_SETTINGS)
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
    if ui.is_item_hovered() {
        ui.tooltip_text("Add a new CAN bus (loads the sample DBC)");
    }
    ui.same_line();
    ui.text(format!("{} bus(es)", app.channels.len()));
    ui.separator();

    // NO_BORDERS_IN_BODY restricts column-resize dragging to the header row.
    let flags = TableFlags::BORDERS_INNER
        | TableFlags::ROW_BG
        | TableFlags::RESIZABLE
        | TableFlags::NO_BORDERS_IN_BODY
        | TableFlags::SCROLL_Y
        | TableFlags::SIZING_STRETCH_PROP;
    let mut remove: Option<usize> = None;
    let n = app.channels.len();
    {
        let Some(_table) = ui.begin_table_with_flags("bus_table", 3, flags) else {
            return;
        };
        ui.table_setup_column_with(TableColumnSetup {
            flags: TableColumnFlags::WIDTH_STRETCH,
            init_width_or_weight: 1.0,
            ..TableColumnSetup::new("Name")
        });
        ui.table_setup_column_with(TableColumnSetup {
            flags: TableColumnFlags::WIDTH_STRETCH,
            init_width_or_weight: 1.8,
            ..TableColumnSetup::new("DBC")
        });
        ui.table_setup_column_with(TableColumnSetup {
            flags: TableColumnFlags::WIDTH_FIXED,
            init_width_or_weight: 26.0,
            ..TableColumnSetup::new("")
        });
        ui.table_headers_row();

        for i in 0..n {
            ui.table_next_row();
            if !ui.table_next_column() {
                continue;
            }
            ui.set_next_item_width(-1.0);
            ui.input_text(format!("##busname{i}"), &mut app.channels[i].name)
                .build();
            ui.table_next_column();
            let path = app.channels[i].dbc_path.clone();
            ui.align_text_to_frame_padding();
            ui.text(if path.trim().is_empty() {
                "(none)".to_string()
            } else {
                file_name(&path)
            });
            ui.same_line();
            if ui.small_button(format!("Open...##busdbc{i}")) {
                app.pick_dbc_for(i);
            }
            ui.table_next_column();
            if ui.small_button(format!("x##busrm{i}")) {
                remove = Some(i);
            }
            if ui.is_item_hovered() {
                ui.tooltip_text("Remove this bus (the last one cannot be removed)");
            }
        }
    }
    if let Some(i) = remove {
        app.remove_channel(i);
    }
}
