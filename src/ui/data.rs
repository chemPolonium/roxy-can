use crate::app::{App, TOOLBAR_H};
use imgui::{Condition, StyleColor, TableColumnFlags, TableColumnSetup, TableFlags, Ui};

const PANEL_W: f32 = 190.0;

pub fn render(app: &mut App, ui: &Ui) {
    let io = ui.io();
    let n = app.data_windows.len();
    for i in 0..n {
        let mut open = app.data_windows[i].opened;
        if !open {
            continue;
        }
        let raw = app.data_windows[i].name.clone();
        let name = if raw.trim().is_empty() {
            format!("Data {}", i + 1)
        } else {
            raw
        };
        if app.focus_title.as_deref() == Some(name.as_str()) {
            unsafe { imgui::sys::igSetNextWindowFocus() };
            app.focus_title = None;
        }
        ui.window(format!("{name}###data{i}"))
            .opened(&mut open)
            .position(
                [io.display_size[0] - 480.0, TOOLBAR_H + i as f32 * 28.0],
                Condition::FirstUseEver,
            )
            .size([480.0, 360.0], Condition::FirstUseEver)
            .build(|| {
                window_content(app, ui, i);
            });
        app.data_windows[i].opened = open;
    }
}

fn window_content(app: &mut App, ui: &Ui, i: usize) {
    let avail = ui.content_region_avail();

    ui.child_window("sig_panel")
        .size([PANEL_W, avail[1]])
        .build(|| left_panel(app, ui, i));

    ui.same_line();

    ui.child_window("values_area")
        .size([0.0, avail[1]])
        .build(|| values_area(app, ui, i));
}

/// Left panel: the window's selected signal list; each signal can be
/// toggled for display. Bus, node identity, and signal selection live in
/// Measurement Setup.
fn left_panel(app: &mut App, ui: &Ui, i: usize) {
    ui.text("Signals");
    crate::ui::siglist::draw(app, ui, crate::ui::siglist::ListKind::Data(i));
}

/// Right area: value table for the visible signals, the reference Data
/// window's column set -- physical value, unit, and raw wire value in
/// their own columns, then a bar. The bar draws the latest value's place
/// in the database's declared min..max; a signal without a declared range
/// falls back to its observed one.
///
/// The value/unit/raw columns draw a throttled snapshot (`sync_data_text`)
/// so digits hold still long enough to read, while the bar -- drawn as a
/// plain filled rect, no text inside -- animates at full frame rate.
fn values_area(app: &mut App, ui: &Ui, i: usize) {
    app.sync_data_text(i);
    let (keys, cache) = {
        let w = &app.data_windows[i];
        (w.text_keys.clone(), w.text_cache.clone())
    };
    if keys.is_empty() {
        ui.text("add signals via Measurement Setup (…)");
        return;
    }
    let tbl_flags = TableFlags::BORDERS_INNER | TableFlags::ROW_BG | TableFlags::SCROLL_Y;
    if let Some(_table) = ui.begin_table_with_flags("data_table", 5, tbl_flags) {
        // Fixed widths for the text columns; the Bar column stretches and
        // takes whatever is left.
        ui.table_setup_column_with(TableColumnSetup {
            flags: TableColumnFlags::WIDTH_FIXED,
            init_width_or_weight: 150.0,
            ..TableColumnSetup::new("Name")
        });
        ui.table_setup_column_with(TableColumnSetup {
            flags: TableColumnFlags::WIDTH_FIXED,
            init_width_or_weight: 70.0,
            ..TableColumnSetup::new("Value")
        });
        ui.table_setup_column_with(TableColumnSetup {
            flags: TableColumnFlags::WIDTH_FIXED,
            init_width_or_weight: 60.0,
            ..TableColumnSetup::new("Unit")
        });
        ui.table_setup_column_with(TableColumnSetup {
            flags: TableColumnFlags::WIDTH_FIXED,
            init_width_or_weight: 85.0,
            ..TableColumnSetup::new("Raw Value")
        });
        ui.table_setup_column_with(TableColumnSetup {
            flags: TableColumnFlags::WIDTH_STRETCH,
            init_width_or_weight: 1.0,
            ..TableColumnSetup::new("Bar")
        });
        ui.table_headers_row();
        let dl = ui.get_window_draw_list();
        let bar_bg = ui.style_color(StyleColor::FrameBg);
        let bar_fill = ui.style_color(StyleColor::PlotHistogram);
        for (key, text) in keys.iter().zip(cache.iter()) {
            let Some(sub) = app.sub_view(key) else {
                continue;
            };
            ui.table_next_row();
            if !ui.table_next_column() {
                continue;
            }
            ui.text(&key.2);
            ui.table_next_column();
            ui.text(&text[0]);
            ui.table_next_column();
            ui.text(&text[1]);
            ui.table_next_column();
            ui.text(&text[2]);
            ui.table_next_column();
            let frac = match app.declared_range(key) {
                Some((lo, hi)) => ((sub.latest - lo) / (hi - lo)).clamp(0.0, 1.0),
                None if sub.max > sub.min => {
                    ((sub.latest - sub.min) / (sub.max - sub.min)).clamp(0.0, 1.0)
                }
                None => 0.0,
            };
            // A plain filled rect instead of ProgressBar: the widget paints
            // a fill percentage inside the bar, and text is exactly what
            // this bar must not have -- the value column next to it is
            // throttled, the bar itself is not. The bar fills the cell's
            // whole content height, so it lines up with the text columns
            // instead of floating between margins.
            let p = ui.cursor_screen_pos();
            let avail = ui.content_region_avail();
            let w = avail[0];
            let h = avail[1].max(6.0);
            dl.add_rect([p[0], p[1]], [p[0] + w, p[1] + h], bar_bg)
                .filled(true)
                .build();
            dl.add_rect([p[0], p[1]], [p[0] + w * frac as f32, p[1] + h], bar_fill)
                .filled(true)
                .build();
            ui.dummy([w, h]);
        }
    }
}
