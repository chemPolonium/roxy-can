use crate::app::{App, TOOLBAR_H};
use imgui::{Condition, ProgressBar, TableFlags, Ui};

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
fn values_area(app: &mut App, ui: &Ui, i: usize) {
    let keys: Vec<(u8, u32, String)> = app.data_windows[i]
        .signals
        .iter()
        .filter(|s| s.visible)
        .map(|s| s.key.clone())
        .collect();
    if keys.is_empty() {
        ui.text("add signals via Measurement Setup (…)");
        return;
    }
    let tbl_flags = TableFlags::BORDERS_INNER | TableFlags::ROW_BG | TableFlags::SCROLL_Y;
    if let Some(_table) = ui.begin_table_with_flags("data_table", 5, tbl_flags) {
        ui.table_setup_column("Name");
        ui.table_setup_column("Value");
        ui.table_setup_column("Unit");
        ui.table_setup_column("Raw Value");
        ui.table_setup_column("Bar");
        ui.table_headers_row();
        for key in keys.iter() {
            let Some(sub) = app.subs.get(key) else {
                continue;
            };
            ui.table_next_row();
            if !ui.table_next_column() {
                continue;
            }
            ui.text(&key.2);
            ui.table_next_column();
            // An enum-labelled signal shows the label as its value ("On"),
            // the way the reference window does.
            let value = sub
                .label
                .clone()
                .unwrap_or_else(|| crate::dbc::fmt_decoded(&sub.type_tag, sub.latest));
            ui.text(value);
            ui.table_next_column();
            ui.text(&sub.unit);
            ui.table_next_column();
            ui.text(sub.last_raw.to_string());
            ui.table_next_column();
            let frac = match app.declared_range(key) {
                Some((lo, hi)) => ((sub.latest - lo) / (hi - lo)).clamp(0.0, 1.0),
                None if sub.max > sub.min => {
                    ((sub.latest - sub.min) / (sub.max - sub.min)).clamp(0.0, 1.0)
                }
                None => 0.0,
            };
            // A slim bar, not a full widget-height one: anything taller
            // than the text propped every row open.
            let font_h = unsafe { imgui::sys::igGetFontSize() };
            ProgressBar::new(frac as f32)
                .size([ui.content_region_avail()[0], (font_h * 0.6).max(6.0)])
                .build(ui);
        }
    }
}
