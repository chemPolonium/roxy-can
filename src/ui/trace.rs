use crate::app::{App, TOOLBAR_H};
use crate::can::frame::Direction;
use imgui::{Condition, TableColumnFlags, TableColumnSetup, TableFlags, Ui};

const MAX_VISIBLE: usize = 1_000;

pub fn render(app: &mut App, ui: &Ui) {
    let io = ui.io();
    let mut open = app.show_trace;
    if open {
        ui.window("Trace")
            .opened(&mut open)
            .position([0.0, TOOLBAR_H], Condition::FirstUseEver)
            .size(
                [io.display_size[0] * 0.55, io.display_size[1] * 0.5 - TOOLBAR_H],
                Condition::FirstUseEver,
            )
            .flags(imgui::WindowFlags::NO_SAVED_SETTINGS)
            .build(|| {
            ui.text(format!(
                "{} frames (showing newest {})",
                app.trace.len(),
                MAX_VISIBLE.min(app.trace.len())
            ));
            ui.separator();
            let tbl_flags = TableFlags::BORDERS_INNER
                | TableFlags::ROW_BG
                | TableFlags::RESIZABLE
                | TableFlags::SCROLL_Y
                | TableFlags::SIZING_STRETCH_PROP;
            if let Some(_table) = ui.begin_table_with_flags("trace_table", 7, tbl_flags) {
                ui.table_setup_column_with(TableColumnSetup {
                    flags: TableColumnFlags::WIDTH_FIXED,
                    init_width_or_weight: 80.0,
                    ..TableColumnSetup::new("Time")
                });
                ui.table_setup_column_with(TableColumnSetup {
                    flags: TableColumnFlags::WIDTH_FIXED,
                    init_width_or_weight: 30.0,
                    ..TableColumnSetup::new("Ch")
                });
                ui.table_setup_column_with(TableColumnSetup {
                    flags: TableColumnFlags::WIDTH_FIXED,
                    init_width_or_weight: 60.0,
                    ..TableColumnSetup::new("ID")
                });
                ui.table_setup_column_with(TableColumnSetup {
                    flags: TableColumnFlags::WIDTH_STRETCH,
                    init_width_or_weight: 1.6,
                    ..TableColumnSetup::new("Name")
                });
                ui.table_setup_column_with(TableColumnSetup {
                    flags: TableColumnFlags::WIDTH_FIXED,
                    init_width_or_weight: 35.0,
                    ..TableColumnSetup::new("DLC")
                });
                ui.table_setup_column_with(TableColumnSetup {
                    flags: TableColumnFlags::WIDTH_STRETCH,
                    init_width_or_weight: 2.0,
                    ..TableColumnSetup::new("Data")
                });
                ui.table_setup_column_with(TableColumnSetup {
                    flags: TableColumnFlags::WIDTH_FIXED,
                    init_width_or_weight: 34.0,
                    ..TableColumnSetup::new("Dir")
                });
                ui.table_headers_row();

                for f in app.trace.iter().rev().take(MAX_VISIBLE) {
                    ui.table_next_row();
                    if !ui.table_next_column() {
                        continue;
                    }
                    ui.text(format!("{:.6}", f.t_us as f64 / 1e6));
                    ui.table_next_column();
                    ui.text(format!("{}", f.channel + 1));
                    ui.table_next_column();
                    if f.extended {
                        ui.text(format!("{:08X}x", f.id));
                    } else {
                        ui.text(format!("{:03X}", f.id));
                    }
                    ui.table_next_column();
                    match app.dbc.as_ref().and_then(|db| db.message_name(f.id)) {
                        Some(name) => ui.text(name),
                        None => ui.text("-"),
                    }
                    ui.table_next_column();
                    ui.text(format!("{}", f.dlc));
                    ui.table_next_column();
                    let data_str: String = f.data[..f.dlc.min(8) as usize]
                        .iter()
                        .map(|b| format!("{b:02X} "))
                        .collect();
                    ui.text(data_str);
                    ui.table_next_column();
                    ui.text(match f.dir {
                        Direction::Rx => "Rx",
                        Direction::Tx => "Tx",
                    });
                }
            }
        });
    }
    app.show_trace = open;
}
