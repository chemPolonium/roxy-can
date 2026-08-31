use crate::app::{App, PopupTarget};
use crate::ui::flags_color;
use crate::ui::idfilter::scope_combo;
use imgui::{Condition, TableColumnFlags, TableColumnSetup, TableFlags, Ui};

fn ms(v: f64) -> String {
    format!("{:.2}", v / 1000.0)
}

/// Per-bus headline numbers above the per-message table: wire load and frame
/// rate over the last second, error frames since start, and the last minute
/// of load as a sparkline. Load is bit-time weighted -- a 64-byte BRS frame
/// clocks its payload at the FD data rate, so the same traffic reads very
/// differently at 500 kbit/s and at a 2 Mbit/s data phase.
fn bus_summary(app: &mut App, ui: &Ui) {
    let flags = TableFlags::BORDERS_INNER | TableFlags::ROW_BG | TableFlags::SIZING_STRETCH_PROP;
    let Some(_table) = ui.begin_table_with_flags("bus_load_table", 6, flags) else {
        return;
    };
    ui.table_setup_column_with(TableColumnSetup {
        flags: TableColumnFlags::WIDTH_STRETCH,
        init_width_or_weight: 1.0,
        ..TableColumnSetup::new("Bus")
    });
    ui.table_setup_column_with(TableColumnSetup {
        flags: TableColumnFlags::WIDTH_FIXED,
        init_width_or_weight: 96.0,
        ..TableColumnSetup::new("Rate")
    });
    ui.table_setup_column_with(TableColumnSetup {
        flags: TableColumnFlags::WIDTH_FIXED,
        init_width_or_weight: 60.0,
        ..TableColumnSetup::new("f/s")
    });
    ui.table_setup_column_with(TableColumnSetup {
        flags: TableColumnFlags::WIDTH_FIXED,
        init_width_or_weight: 60.0,
        ..TableColumnSetup::new("Load")
    });
    ui.table_setup_column_with(TableColumnSetup {
        flags: TableColumnFlags::WIDTH_FIXED,
        init_width_or_weight: 48.0,
        ..TableColumnSetup::new("Err")
    });
    ui.table_setup_column_with(TableColumnSetup {
        flags: TableColumnFlags::WIDTH_FIXED,
        init_width_or_weight: 140.0,
        ..TableColumnSetup::new("Load (60 s)")
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
        ui.table_next_column();
        let pos = ui.cursor_screen_pos();
        let w = ui.content_region_avail()[0];
        let h = ui.frame_height();
        draw_load_sparkline(ui, pos, w, h, load);
    }
}

/// Last minute of bus load as a filled silhouette; 100 % is the full height.
/// The newest bucket hugs the right edge; the left side stays empty until a
/// minute of history has accumulated.
fn draw_load_sparkline(ui: &Ui, pos: [f32; 2], w: f32, h: f32, load: &crate::load::BusLoad) {
    let dl = ui.get_window_draw_list();
    dl.add_rect(pos, [pos[0] + w, pos[1] + h], [0.16, 0.16, 0.20, 1.0])
        .filled(true)
        .rounding(2.0)
        .build();
    let dx = w / crate::load::HISTORY_BUCKETS as f32;
    let points: Vec<[f32; 2]> = load
        .history()
        .enumerate()
        .map(|(j, (_, v))| {
            [
                pos[0] + (j + 1) as f32 * dx,
                pos[1] + h - (v.clamp(0.0, 1.0) as f32) * (h - 2.0) - 1.0,
            ]
        })
        .collect();
    if points.len() >= 2 {
        dl.add_polyline(points, [0.30, 0.62, 0.98, 1.0])
            .thickness(1.2)
            .build();
    }
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

    bus_summary(app, ui);

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
    ui.text(format!(
        "{} messages, {} frames since start",
        keys.len(),
        total
    ));
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
        ui.text(format!("{}", agg.len));
        ui.table_next_column();
        ui.text_colored(flags_color(agg.flags), agg.flags.tag());
        ui.table_next_column();
        let share = if total > 0 {
            agg.count as f64 / total as f64 * 100.0
        } else {
            0.0
        };
        ui.text(format!("{share:.1}%"));
    }
}
