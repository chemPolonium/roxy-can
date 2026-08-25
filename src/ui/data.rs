use crate::app::{App, PALETTE, Subscription, TOOLBAR_H};
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

/// Right area: value table for the visible signals.
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
    let viz_bar = app.data_windows[i].viz_bar;
    let mut new_mode: Option<bool> = None;
    let tbl_flags = TableFlags::BORDERS_INNER | TableFlags::ROW_BG | TableFlags::SCROLL_Y;
    if let Some(_table) = ui.begin_table_with_flags("data_table", 6, tbl_flags) {
        ui.table_setup_column("Signal");
        ui.table_setup_column("Value");
        ui.table_setup_column("Min");
        ui.table_setup_column("Avg");
        ui.table_setup_column("Max");
        ui.table_setup_column("Viz");
        ui.table_headers_row();
        for (row, key) in keys.iter().enumerate() {
            let Some(sub) = app.subs.get(key) else {
                continue;
            };
            ui.table_next_row();
            if !ui.table_next_column() {
                continue;
            }
            ui.text(&key.2);
            ui.table_next_column();
            ui.text(format!("{:.3} {}", sub.latest, sub.unit));
            ui.table_next_column();
            ui.text(fmt_stat(sub.min));
            ui.table_next_column();
            ui.text(fmt_stat(sub.avg));
            ui.table_next_column();
            ui.text(fmt_stat(sub.max));
            ui.table_next_column();
            let frac = if sub.max > sub.min {
                ((sub.latest - sub.min) / (sub.max - sub.min)).clamp(0.0, 1.0)
            } else {
                0.0
            };
            if viz_bar {
                ProgressBar::new(frac as f32).build(ui);
            } else {
                let w = ui.content_region_avail()[0].max(40.0);
                let h = ui.frame_height();
                let pos = ui.cursor_screen_pos();
                ui.invisible_button(format!("##viz{row}"), [w, h]);
                draw_sparkline(ui, pos, w, h, sub);
            }
            if ui.is_item_clicked() {
                new_mode = Some(!viz_bar);
            }
            if ui.is_item_hovered() {
                ui.tooltip_text("Click to switch between bar and sparkline");
            }
        }
    }
    if let Some(mode) = new_mode {
        app.data_windows[i].viz_bar = mode;
    }
}

/// "-" until the first sample arrives (min/max start infinite).
fn fmt_stat(v: f64) -> String {
    if v.is_finite() {
        format!("{:.3}", v)
    } else {
        "-".to_string()
    }
}

/// Recent history as a polyline, newest at the right edge, scaled to the
/// signal's observed min..max range.
fn draw_sparkline(ui: &Ui, pos: [f32; 2], w: f32, h: f32, sub: &Subscription) {
    let dl = ui.get_window_draw_list();
    dl.add_rect(pos, [pos[0] + w, pos[1] + h], [0.16, 0.16, 0.20, 1.0])
        .filled(true)
        .rounding(2.0)
        .build();
    let max_pts = ((w / 2.0) as usize).max(2);
    let skip = sub.history.len().saturating_sub(max_pts);
    let vals: Vec<f64> = sub.history.iter().skip(skip).map(|(_, v)| *v).collect();
    if vals.is_empty() {
        return;
    }
    let (lo, hi) = if sub.min.is_finite() && sub.max.is_finite() && sub.max > sub.min {
        (sub.min, sub.max)
    } else {
        let v = *vals.last().unwrap();
        (v - 1.0, v + 1.0)
    };
    let dx = w / max_pts as f32;
    let points: Vec<[f32; 2]> = vals
        .iter()
        .enumerate()
        .map(|(j, v)| {
            let x = pos[0] + w - (vals.len() - 1 - j) as f32 * dx;
            let t = ((v - lo) / (hi - lo)).clamp(0.0, 1.0) as f32;
            [x, pos[1] + h - 1.0 - t * (h - 2.0)]
        })
        .collect();
    let color = PALETTE[sub.color % PALETTE.len()];
    dl.add_polyline(points, color).thickness(1.2).build();
}
