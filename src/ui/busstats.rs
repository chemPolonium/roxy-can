use crate::app::App;
use imgui::{Condition, TableColumnFlags, TableColumnSetup, TableFlags, Ui};

/// Per-bus headline numbers in a window of their own: declared bitrates,
/// frame rate and wire load over the last second, error frames since start.
/// Load is a plain percentage -- the sparkline lived in the message
/// statistics window for one release and was judged too tall for its own
/// good; the bucket history stays collected for future views.
pub fn render(app: &mut App, ui: &Ui) {
    if !app.show_bus_stats {
        return;
    }
    let io = ui.io();
    let mut open = app.show_bus_stats;
    ui.window("Bus Statistics###busstats")
        .opened(&mut open)
        .position(
            [io.display_size[0] * 0.55, io.display_size[1] * 0.12],
            Condition::FirstUseEver,
        )
        .size([460.0, 150.0], Condition::FirstUseEver)
        .build(|| content(app, ui));
    app.show_bus_stats = open;
}

fn content(app: &mut App, ui: &Ui) {
    ui.text("Load and frame rate over the last second; errors since start");
    ui.separator();

    // NO_BORDERS_IN_BODY restricts column-resize dragging to the header row.
    let flags = TableFlags::BORDERS_INNER
        | TableFlags::ROW_BG
        | TableFlags::RESIZABLE
        | TableFlags::NO_BORDERS_IN_BODY
        | TableFlags::SCROLL_Y
        | TableFlags::SIZING_STRETCH_PROP;
    let Some(_table) = ui.begin_table_with_flags("bus_stats_table", 5, flags) else {
        return;
    };
    ui.table_setup_column_with(TableColumnSetup {
        flags: TableColumnFlags::WIDTH_STRETCH,
        init_width_or_weight: 1.0,
        ..TableColumnSetup::new("Bus")
    });
    ui.table_setup_column_with(TableColumnSetup {
        flags: TableColumnFlags::WIDTH_FIXED,
        init_width_or_weight: 110.0,
        ..TableColumnSetup::new("Rate")
    });
    ui.table_setup_column_with(TableColumnSetup {
        flags: TableColumnFlags::WIDTH_FIXED,
        init_width_or_weight: 70.0,
        ..TableColumnSetup::new("f/s")
    });
    ui.table_setup_column_with(TableColumnSetup {
        flags: TableColumnFlags::WIDTH_FIXED,
        init_width_or_weight: 70.0,
        ..TableColumnSetup::new("Load")
    });
    ui.table_setup_column_with(TableColumnSetup {
        flags: TableColumnFlags::WIDTH_FIXED,
        init_width_or_weight: 56.0,
        ..TableColumnSetup::new("Err")
    });
    ui.table_headers_row();

    for (i, ch) in app.channels.iter().enumerate() {
        let Some(load) = app.bus_loads.get(i) else {
            continue;
        };
        ui.table_next_row();
        if !ui.table_next_column() {
            continue;
        }
        ui.text(&ch.name);
        ui.table_next_column();
        if ch.bitrate_kbps != ch.fd_data_kbps {
            ui.text(format!("{}/{} k", ch.bitrate_kbps, ch.fd_data_kbps));
        } else {
            ui.text(format!("{} k", ch.bitrate_kbps));
        }
        ui.table_next_column();
        ui.text(format!("{:.0}", load.frame_rate()));
        ui.table_next_column();
        ui.text(format!("{:.1}%", load.load() * 100.0));
        ui.table_next_column();
        ui.text(format!("{}", load.errors));
    }
}
