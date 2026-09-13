use crate::app::{App, TOOLBAR_H};
use dear_imgui_rs::{Condition, TableColumnFlags, TableFlags, Ui};

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
        let focus = app.focus_title.as_deref() == Some(name.as_str());
        if focus {
            app.focus_title = None;
        }
        let mut window = ui
            .window(format!("{name}###data{i}"))
            .opened(&mut open)
            .position(
                [io.display_size()[0] - 480.0, TOOLBAR_H + i as f32 * 28.0],
                Condition::FirstUseEver,
            )
            .size([480.0, 360.0], Condition::FirstUseEver);
        if focus {
            window = window.focused(true);
        }
        window.build(|| {
            window_content(app, ui, i);
        });
        app.data_windows[i].opened = open;
    }
}

fn window_content(app: &mut App, ui: &Ui, i: usize) {
    let avail = ui.content_region_avail();

    ui.child_window("sig_panel")
        .size([PANEL_W, avail[1]])
        .build(ui, || left_panel(app, ui, i));

    ui.same_line();

    ui.child_window("values_area")
        .size([0.0, avail[1]])
        .build(ui, || values_area(app, ui, i));
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
    if let Some(_table) = ui.begin_table_with_flags("data_table", 8, tbl_flags) {
        // Fixed widths for the text columns; the Bar column stretches and
        // takes whatever is left.
        ui.table_setup_column_fixed_width("Name", TableColumnFlags::NONE, 130.0);
        ui.table_setup_column_fixed_width("Value", TableColumnFlags::NONE, 70.0);
        ui.table_setup_column_fixed_width("Unit", TableColumnFlags::NONE, 56.0);
        ui.table_setup_column_fixed_width("Raw", TableColumnFlags::NONE, 76.0);
        ui.table_setup_column_fixed_width("Min", TableColumnFlags::NONE, 70.0);
        ui.table_setup_column_fixed_width("Avg", TableColumnFlags::NONE, 70.0);
        ui.table_setup_column_fixed_width("Max", TableColumnFlags::NONE, 70.0);
        ui.table_setup_column_stretch_weight("Bar", TableColumnFlags::NONE, 1.0);
        ui.table_setup_scroll_freeze(0, 1);
        ui.table_headers_row();
        for (key, text) in keys.iter().zip(cache.iter()) {
            let Some(sub) = app.sub_view(key) else {
                continue;
            };
            ui.table_next_row();
            if !ui.table_next_column() {
                continue;
            }
            ui.text(&key.3);
            ui.table_next_column();
            ui.text(&text[0]);
            ui.table_next_column();
            ui.text(&text[1]);
            ui.table_next_column();
            ui.text(&text[2]);
            ui.table_next_column();
            ui.text(&text[3]);
            ui.table_next_column();
            ui.text(&text[4]);
            ui.table_next_column();
            ui.text(&text[5]);
            ui.table_next_column();
            let frac = match app.declared_range(key) {
                Some((lo, hi)) => ((sub.latest - lo) / (hi - lo)).clamp(0.0, 1.0),
                None if sub.max > sub.min => {
                    ((sub.latest - sub.min) / (sub.max - sub.min)).clamp(0.0, 1.0)
                }
                None => 0.0,
            };
            // The widget's default size is what this cell wants: the width
            // fills the column and the height is the standard frame height.
            // The overlay percentage is silenced -- the value column next
            // to it is throttled, so a live number on the bar would read
            // as truth while the text column lags behind.
            ui.progress_bar(frac as f32)
                .size([-1.0, 13.0])
                .overlay_text("")
                .build();
        }
    }
}
