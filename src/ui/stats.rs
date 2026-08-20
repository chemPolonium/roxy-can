use crate::app::{App, PopupTarget};
use crate::ui::idfilter::scope_combo;
use imgui::{Condition, TableColumnFlags, TableColumnSetup, TableFlags, Ui};

fn ms(v: f64) -> String {
    format!("{:.2}", v / 1000.0)
}

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
            format!("Statistics {}", i + 1)
        } else {
            raw
        };
        let off = i as f32 * 30.0;
        if app.focus_title.as_deref() == Some(title.as_str()) {
            unsafe { imgui::sys::igSetNextWindowFocus() };
            app.focus_title = None;
        }
        ui.window(title)
            .opened(&mut open)
            .position(
                [
                    io.display_size[0] * 0.6 + off,
                    io.display_size[1] * 0.45 + off,
                ],
                Condition::FirstUseEver,
            )
            .size([640.0, io.display_size[1] * 0.42], Condition::FirstUseEver)
            .flags(imgui::WindowFlags::NO_SAVED_SETTINGS)
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
    if ui.is_item_hovered() {
        ui.tooltip_text("Export this view as CSV");
    }

    let w = app.stats_windows[i].clone();
    let mut keys: Vec<(u8, u32)> = app
        .aggs
        .keys()
        .copied()
        .filter(|&(ch, id)| App::scope_match(w.scope, &w.manual, ch, id))
        .collect();
    keys.sort_unstable();
    let total: u64 = keys
        .iter()
        .filter_map(|k| app.aggs.get(k))
        .map(|a| a.count)
        .sum();
    ui.same_line();
    ui.text(format!("{} messages, {} frames", keys.len(), total));
    ui.separator();

    let tbl_flags = TableFlags::BORDERS_INNER
        | TableFlags::ROW_BG
        | TableFlags::RESIZABLE
        | TableFlags::SCROLL_Y
        | TableFlags::SIZING_STRETCH_PROP;
    let Some(_table) = ui.begin_table_with_flags(format!("stats_table{i}"), 8, tbl_flags) else {
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
        init_width_or_weight: 70.0,
        ..TableColumnSetup::new("Count")
    });
    ui.table_setup_column_with(TableColumnSetup {
        flags: TableColumnFlags::WIDTH_FIXED,
        init_width_or_weight: 62.0,
        ..TableColumnSetup::new("Min(ms)")
    });
    ui.table_setup_column_with(TableColumnSetup {
        flags: TableColumnFlags::WIDTH_FIXED,
        init_width_or_weight: 62.0,
        ..TableColumnSetup::new("Avg(ms)")
    });
    ui.table_setup_column_with(TableColumnSetup {
        flags: TableColumnFlags::WIDTH_FIXED,
        init_width_or_weight: 62.0,
        ..TableColumnSetup::new("Max(ms)")
    });
    ui.table_setup_column_with(TableColumnSetup {
        flags: TableColumnFlags::WIDTH_FIXED,
        init_width_or_weight: 35.0,
        ..TableColumnSetup::new("DLC")
    });
    ui.table_setup_column_with(TableColumnSetup {
        flags: TableColumnFlags::WIDTH_FIXED,
        init_width_or_weight: 55.0,
        ..TableColumnSetup::new("Share")
    });
    ui.table_headers_row();

    for key in keys {
        let Some(agg) = app.aggs.get(&key) else {
            continue;
        };
        let id = agg.id;
        ui.table_next_row();
        if !ui.table_next_column() {
            continue;
        }
        let id_str = if agg.extended {
            format!("{:08X}x", id)
        } else {
            format!("{:03X}", id)
        };
        let name = app.message_name(agg.channel, id).unwrap_or("-");
        ui.text(format!("{id_str}  {name}"));
        ui.table_next_column();
        ui.text(app.channel_name(agg.channel));
        ui.table_next_column();
        ui.text(format!("{}", agg.count));
        ui.table_next_column();
        ui.text(if agg.count >= 2 {
            ms(agg.min_us)
        } else {
            "-".to_string()
        });
        ui.table_next_column();
        ui.text(if agg.count >= 2 {
            ms(agg.cycle_us)
        } else {
            "-".to_string()
        });
        ui.table_next_column();
        ui.text(if agg.count >= 2 {
            ms(agg.max_us)
        } else {
            "-".to_string()
        });
        ui.table_next_column();
        ui.text(format!("{}", agg.dlc));
        ui.table_next_column();
        let share = if total > 0 {
            agg.count as f64 / total as f64 * 100.0
        } else {
            0.0
        };
        ui.text(format!("{share:.1}%"));
    }
}
