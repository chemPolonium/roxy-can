use crate::app::{App, PopupTarget};
use crate::ui::flags_color;
use crate::ui::idfilter::scope_combo;
use imgui::{Condition, TableColumnFlags, TableColumnSetup, TableFlags, Ui};

pub fn render(app: &mut App, ui: &Ui) {
    let io = ui.io();
    let n = app.stats_windows.len();
    for i in 0..n {
        if !app.stats_windows[i].opened {
            continue;
        }
        let mut open = true;
        let raw = app.stats_windows[i].name.clone();
        let title = if raw.trim().is_empty() {
            format!("Message Statistics {}", i + 1)
        } else {
            raw
        };
        let off = i as f32 * 30.0;
        if app.focus_title.as_deref() == Some(title.as_str()) {
            unsafe { imgui::sys::igSetNextWindowFocus() };
            app.focus_title = None;
        }
        ui.window(format!("{title}###stats{i}"))
            .opened(&mut open)
            .position(
                [
                    io.display_size[0] * 0.6 + off,
                    io.display_size[1] * 0.45 + off,
                ],
                Condition::FirstUseEver,
            )
            .size([640.0, io.display_size[1] * 0.42], Condition::FirstUseEver)
            .build(|| window_content(app, ui, i));
        app.stats_windows[i].opened = open;
    }
}

fn window_content(app: &mut App, ui: &Ui, i: usize) {
    let new_scope = scope_combo(
        app,
        ui,
        &format!("##sscope{i}"),
        app.stats_windows[i].scope,
        PopupTarget::Stats(i),
    );
    app.stats_windows[i].scope = new_scope;
    ui.same_line();
    if ui.small_button(format!("Export##sx{i}")) {
        app.export_stats_dialog(i);
    }
    ui.separator();

    // The rows are throttled text: refreshed on the text gate so the
    // counters hold still long enough to read, not on every frame.
    app.sync_stats_text(i);
    ui.text(&app.stats_windows[i].text_header);
    ui.separator();

    // NO_BORDERS_IN_BODY restricts column-resize dragging to the header row.
    let tbl_flags = TableFlags::BORDERS_INNER
        | TableFlags::ROW_BG
        | TableFlags::RESIZABLE
        | TableFlags::NO_BORDERS_IN_BODY
        | TableFlags::SCROLL_Y
        | TableFlags::SIZING_STRETCH_PROP;
    let Some(_table) = ui.begin_table_with_flags(format!("stats_table{i}"), 9, tbl_flags) else {
        return;
    };
    ui.table_setup_column_with(TableColumnSetup {
        flags: TableColumnFlags::WIDTH_STRETCH,
        init_width_or_weight: 2.0,
        ..TableColumnSetup::new("Message")
    });
    ui.table_setup_column_with(TableColumnSetup {
        flags: TableColumnFlags::WIDTH_FIXED,
        init_width_or_weight: 60.0,
        ..TableColumnSetup::new("Bus")
    });
    ui.table_setup_column_with(TableColumnSetup {
        flags: TableColumnFlags::WIDTH_FIXED,
        init_width_or_weight: 55.0,
        ..TableColumnSetup::new("Count")
    });
    // "{:.2}" milliseconds: ~7 chars
    for label in ["Min(ms)", "Avg(ms)", "Max(ms)"] {
        ui.table_setup_column_with(TableColumnSetup {
            flags: TableColumnFlags::WIDTH_FIXED,
            init_width_or_weight: 62.0,
            ..TableColumnSetup::new(label)
        });
    }
    ui.table_setup_column_with(TableColumnSetup {
        flags: TableColumnFlags::WIDTH_FIXED,
        init_width_or_weight: 36.0,
        ..TableColumnSetup::new("Len")
    });
    ui.table_setup_column_with(TableColumnSetup {
        flags: TableColumnFlags::WIDTH_FIXED,
        init_width_or_weight: 44.0,
        ..TableColumnSetup::new("Flags")
    });
    ui.table_setup_column_with(TableColumnSetup {
        flags: TableColumnFlags::WIDTH_FIXED,
        init_width_or_weight: 52.0,
        ..TableColumnSetup::new("Share")
    });
    ui.table_headers_row();

    for row in &app.stats_windows[i].text_rows {
        ui.table_next_row();
        if !ui.table_next_column() {
            continue;
        }
        ui.text(&row.label);
        ui.table_next_column();
        ui.text(&row.bus);
        ui.table_next_column();
        ui.text(&row.count);
        ui.table_next_column();
        ui.text(&row.min);
        ui.table_next_column();
        ui.text(&row.avg);
        ui.table_next_column();
        ui.text(&row.max);
        ui.table_next_column();
        ui.text(&row.len);
        ui.table_next_column();
        ui.text_colored(flags_color(row.flags), row.flags.tag());
        ui.table_next_column();
        ui.text(&row.share);
    }
}
