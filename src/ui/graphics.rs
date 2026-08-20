use crate::app::{App, PALETTE};
use imgui::{Condition, Ui};

const TIME_PRESETS: [f64; 4] = [5.0, 10.0, 30.0, 60.0];
const PANEL_W: f32 = 190.0;

pub fn render(app: &mut App, ui: &Ui) {
    let n = app.graphics.len();
    let disp_h = ui.io().display_size[1];
    for i in 0..n {
        let mut open = app.graphics[i].opened;
        if !open {
            continue;
        }
        let raw = app.graphics[i].name.clone();
        let name = if raw.trim().is_empty() {
            format!("Graphics {}", i + 1)
        } else {
            raw
        };
        if app.focus_title.as_deref() == Some(name.as_str()) {
            unsafe { imgui::sys::igSetNextWindowFocus() };
            app.focus_title = None;
        }
        ui.window(format!("{name}###gfx{i}"))
            .opened(&mut open)
            .position(
                [16.0 + i as f32 * 36.0, disp_h * 0.55 + i as f32 * 28.0],
                Condition::FirstUseEver,
            )
            .size([760.0, (disp_h * 0.40).max(240.0)], Condition::FirstUseEver)
            .flags(imgui::WindowFlags::NO_SAVED_SETTINGS)
            .build(|| {
                window_content(app, ui, i);
            });
        app.graphics[i].opened = open;
    }
}

fn window_content(app: &mut App, ui: &Ui, i: usize) {
    let tw = app.graphics[i].time_window_s;
    for &preset in &TIME_PRESETS {
        let label = if (tw - preset).abs() < 1e-9 {
            format!("[{}s]", preset as i32)
        } else {
            format!("{}s", preset as i32)
        };
        if ui.button(&label) {
            app.graphics[i].time_window_s = preset;
        }
        ui.same_line();
    }
    let mut stacked = app.graphics[i].stacked;
    ui.radio_button("Overlay", &mut stacked, false);
    ui.same_line();
    ui.radio_button("One plot per signal", &mut stacked, true);
    app.graphics[i].stacked = stacked;

    ui.separator();
    let avail = ui.content_region_avail();

    ui.child_window("sig_panel")
        .size([PANEL_W, avail[1]])
        .build(|| left_panel(app, ui, i));

    ui.same_line();

    ui.child_window("plot_area")
        .size([0.0, avail[1]])
        .build(|| plot_area(app, ui, i));
}

/// Left panel: the window's selected signal list; each signal can be
/// toggled for drawing. Bus, node identity, and signal selection live in
/// Measurement Setup.
fn left_panel(app: &mut App, ui: &Ui, i: usize) {
    ui.text("Curves");
    crate::ui::siglist::draw(app, ui, crate::ui::siglist::ListKind::Graphics(i));
}

/// Right area: draws plots directly on the draw list and reserves exactly
/// the available space, so no scrollbar appears.
fn plot_area(app: &App, ui: &Ui, i: usize) {
    let avail = ui.content_region_avail();
    let w = avail[0].max(40.0);
    let h = avail[1].max(40.0);
    let p0 = ui.cursor_screen_pos();
    let [x0, y0] = p0;
    let dl = ui.get_window_draw_list();

    let stacked = app.graphics[i].stacked;
    let tw = app.graphics[i].time_window_s;
    let t_now = app.last_tick_us as f64 / 1e6;
    let keys: Vec<(u8, u32, String)> = app.graphics[i]
        .signals
        .iter()
        .filter(|s| s.visible)
        .map(|s| s.key.clone())
        .collect();

    if keys.is_empty() {
        draw_plot_frame(&dl, x0, y0, w, h);
        dl.add_text(
            [x0 + 8.0, y0 + 8.0],
            [0.5, 0.5, 0.6, 1.0],
            "add signals via Measurement Setup (…)".to_string(),
        );
    } else if stacked {
        let ph = h / keys.len() as f32;
        for (k, key) in keys.iter().enumerate() {
            draw_plot(
                &dl,
                app,
                x0,
                y0 + k as f32 * ph,
                w,
                ph,
                &[key.clone()],
                t_now,
                tw,
            );
        }
    } else {
        draw_plot(&dl, app, x0, y0, w, h, &keys, t_now, tw);
    }

    ui.dummy([w, h]);
}

fn draw_plot_frame(dl: &imgui::DrawListMut<'_>, x0: f32, y0: f32, w: f32, h: f32) {
    dl.add_rect([x0, y0], [x0 + w, y0 + h], [0.08, 0.08, 0.10, 1.0])
        .filled(true)
        .build();
    dl.add_rect([x0, y0], [x0 + w, y0 + h], [0.20, 0.20, 0.25, 1.0])
        .build();
}

fn fmt_val(v: f64) -> String {
    let a = v.abs();
    if a >= 1000.0 {
        format!("{:.0}", v)
    } else if a >= 10.0 {
        format!("{:.1}", v)
    } else {
        format!("{:.2}", v)
    }
}

fn draw_plot(
    dl: &imgui::DrawListMut<'_>,
    app: &App,
    x0: f32,
    y0: f32,
    w: f32,
    h: f32,
    keys: &[(u8, u32, String)],
    t_now: f64,
    tw: f64,
) {
    draw_plot_frame(dl, x0, y0, w, h);
    let t_min = t_now - tw;

    let mut vmin = f64::INFINITY;
    let mut vmax = f64::NEG_INFINITY;
    for key in keys {
        let Some(sub) = app.subs.get(key) else {
            continue;
        };
        for &(t, v) in &sub.history {
            if (t as f64 / 1e6) < t_min {
                continue;
            }
            if v < vmin {
                vmin = v;
            }
            if v > vmax {
                vmax = v;
            }
        }
    }
    if !vmin.is_finite() {
        vmin = 0.0;
        vmax = 1.0;
    }
    if vmax - vmin < 1e-9 {
        vmax = vmin + 1.0;
    }

    for g in 0..=10 {
        let x = x0 + w * g as f32 / 10.0;
        dl.add_line([x, y0], [x, y0 + h], [0.18, 0.18, 0.22, 1.0])
            .build();
        if g % 2 == 0 && g < 10 && h > 20.0 {
            let t = t_min + tw * g as f64 / 10.0;
            dl.add_text(
                [x + 3.0, y0 + h - 13.0],
                [0.55, 0.55, 0.65, 1.0],
                format!("{:.1}s", t),
            );
        }
    }
    for g in 0..=4 {
        let y = y0 + h * g as f32 / 4.0;
        dl.add_line([x0, y], [x0 + w, y], [0.18, 0.18, 0.22, 1.0])
            .build();
        let v = vmax - (vmax - vmin) * g as f64 / 4.0;
        let ly = if g == 0 { y + 2.0 } else { y - 13.0 };
        dl.add_text([x0 + 3.0, ly], [0.55, 0.55, 0.65, 1.0], fmt_val(v));
    }

    for key in keys {
        let Some(sub) = app.subs.get(key) else {
            continue;
        };
        let mut pts = Vec::new();
        for &(t, v) in &sub.history {
            let tf = t as f64 / 1e6;
            if tf < t_min {
                continue;
            }
            let x = x0 + w * ((tf - t_min) / tw).clamp(0.0, 1.0) as f32;
            let y = y0 + h * (1.0 - ((v - vmin) / (vmax - vmin)).clamp(0.0, 1.0)) as f32;
            pts.push([x, y]);
        }
        if pts.len() >= 2 {
            dl.add_polyline(pts, PALETTE[sub.color % PALETTE.len()])
                .thickness(1.5)
                .build();
        }
    }

    let entries: Vec<([f32; 4], String)> = keys
        .iter()
        .filter_map(|key| {
            app.subs.get(key).map(|sub| {
                (
                    PALETTE[sub.color % PALETTE.len()],
                    format!("{} = {:.3} {}", key.2, sub.latest, sub.unit),
                )
            })
        })
        .collect();
    if !entries.is_empty() {
        let max_w = entries
            .iter()
            .map(|(_, text)| text.len() as f32 * 7.0)
            .fold(0.0f32, f32::max);
        let box_h = entries.len() as f32 * 14.0 + 8.0;
        dl.add_rect(
            [x0 + 2.0, y0 + 2.0],
            [x0 + 40.0 + max_w, y0 + 2.0 + box_h],
            [0.10, 0.10, 0.14, 0.85],
        )
        .filled(true)
        .build();
        for (n, (color, text)) in entries.iter().enumerate() {
            let ly = y0 + 6.0 + n as f32 * 14.0;
            dl.add_line([x0 + 8.0, ly + 6.0], [x0 + 24.0, ly + 6.0], *color)
                .thickness(2.0)
                .build();
            dl.add_text([x0 + 30.0, ly], [0.9, 0.9, 0.95, 1.0], text.clone());
        }
    }

    if h > 34.0 {
        dl.add_text(
            [x0 + w - 90.0, y0 + 4.0],
            [0.5, 0.5, 0.6, 1.0],
            format!("{} s window", tw),
        );
    }
}
