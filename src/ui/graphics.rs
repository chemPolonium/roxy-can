use crate::app::{App, PALETTE};
use imgui::{Condition, Ui};

/// Zoom ladder: round, analysis-friendly window lengths from a close-up up
/// to one hour; wheel zoom and the preset button row share this ladder.
const TIME_STEPS: [f64; 14] = [
    0.1, 0.2, 0.5, 1.0, 5.0, 10.0, 20.0, 30.0, 60.0, 120.0, 300.0, 600.0, 1800.0, 3600.0,
];
const PANEL_W: f32 = 190.0;

/// Widest window the ladder offers. Signal history has to be able to back it,
/// or the curve's head silently vanishes once the retention cap is reached.
/// Only read by the invariant test in `crate::app`.
#[cfg(test)]
pub(crate) const MAX_TIME_WINDOW_S: f64 = TIME_STEPS[TIME_STEPS.len() - 1];

/// Radius of the dot drawn on each sample while `Dots` is enabled.
const MARKER_RADIUS_PX: f32 = 2.2;

/// Direct-select window lengths: the whole TIME_STEPS ladder as a button
/// row in each Graphics window.
const TW_PRESETS: [(f64, &str); 14] = [
    (0.1, "0.1s"),
    (0.2, "0.2s"),
    (0.5, "0.5s"),
    (1.0, "1s"),
    (5.0, "5s"),
    (10.0, "10s"),
    (20.0, "20s"),
    (30.0, "30s"),
    (60.0, "1m"),
    (120.0, "2m"),
    (300.0, "5m"),
    (600.0, "10m"),
    (1800.0, "30m"),
    (3600.0, "1h"),
];

/// Moves the current window along TIME_STEPS: wheel up zooms in (smaller
/// window), wheel down zooms out. Snaps to the nearest step first when
/// `tw` sits between steps.
pub(crate) fn zoom_step(tw: f64, notches: f32) -> f64 {
    let mut idx = 0;
    let mut best = f64::INFINITY;
    for (n, s) in TIME_STEPS.iter().enumerate() {
        let d = (*s - tw).abs();
        if d < best {
            best = d;
            idx = n;
        }
    }
    let delta = -(notches.round() as i32);
    let next = (idx as i32 + delta).clamp(0, TIME_STEPS.len() as i32 - 1);
    TIME_STEPS[next as usize]
}

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
            .build(|| {
                window_content(app, ui, i);
            });
        app.graphics[i].opened = open;
    }
}

fn window_content(app: &mut App, ui: &Ui, i: usize) {
    for (k, (val, label)) in TW_PRESETS.iter().enumerate() {
        let selected = (app.graphics[i].time_window_s - val).abs() < 1e-9;
        let colors = selected.then(|| {
            (
                ui.push_style_color(imgui::StyleColor::Button, [0.2, 0.45, 0.75, 1.0]),
                ui.push_style_color(imgui::StyleColor::ButtonHovered, [0.25, 0.55, 0.85, 1.0]),
                ui.push_style_color(imgui::StyleColor::ButtonActive, [0.15, 0.4, 0.7, 1.0]),
            )
        });
        let text = if selected {
            format!("[{label}]##tw{i}")
        } else {
            format!("{label}##tw{i}")
        };
        if ui.small_button(text) {
            app.graphics[i].time_window_s = *val;
        }
        drop(colors);
        if k + 1 < TW_PRESETS.len() || app.graphics[i].t_offset_s > 0.0 {
            ui.same_line();
        }
    }
    if app.graphics[i].t_offset_s > 0.0 {
        ui.text_colored(
            [1.0, 0.8, 0.4, 1.0],
            format!("{:.1}s behind live", app.graphics[i].t_offset_s),
        );
    }
    let mut stacked = app.graphics[i].stacked;
    ui.radio_button("Overlay", &mut stacked, false);
    ui.same_line();
    ui.radio_button("One plot per signal", &mut stacked, true);
    app.graphics[i].stacked = stacked;
    ui.same_line();
    ui.checkbox("Cursor", &mut app.graphics[i].show_cursor);
    ui.same_line();
    ui.checkbox("Zoom", &mut app.graphics[i].zoom_enabled);
    ui.same_line();
    ui.checkbox("Dots", &mut app.graphics[i].show_markers);
    ui.same_line();
    if ui.button("Live") {
        app.graphics[i].t_offset_s = 0.0;
    }

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
/// the available space, so no scrollbar appears. Also handles mouse-wheel
/// zoom, left-drag pan, and the measurement cursor.
fn plot_area(app: &mut App, ui: &Ui, i: usize) {
    let avail = ui.content_region_avail();
    let w = avail[0].max(40.0);
    let h = avail[1].max(40.0);
    let p0 = ui.cursor_screen_pos();
    let [x0, y0] = p0;
    let dl = ui.get_window_draw_list();

    let stacked = app.graphics[i].stacked;
    let tw = app.graphics[i].time_window_s;
    // Follows the replay playhead, not the wall clock, so the axis moves with
    // the scrub bar and the curve stays in view at any playback speed.
    let t_now = app.plot_now_s();
    let keys: Vec<(u8, u32, String)> = app.graphics[i]
        .signals
        .iter()
        .filter(|s| s.visible)
        .map(|s| s.key.clone())
        .collect();

    // Never pan or zoom further back than the oldest stored sample.
    let mut oldest = f64::INFINITY;
    for key in &keys {
        if let Some(sub) = app.subs.get(key)
            && let Some((t, _)) = sub.history.first()
        {
            oldest = oldest.min(t as f64 / 1e6);
        }
    }
    let max_off = if oldest.is_finite() {
        (t_now - oldest).max(0.0)
    } else {
        0.0
    };
    // Re-clamp unconditionally: the pan and zoom handlers were not the only
    // thing that can make an existing offset unreachable. Scrubbing moves the
    // playhead and rewinds history out from under it, and without this the
    // window sits over ground with no samples and the curve looks gone.
    if app.graphics[i].t_offset_s > max_off {
        app.graphics[i].t_offset_s = max_off;
    }

    let io = ui.io();
    let mx = io.mouse_pos[0];
    let my = io.mouse_pos[1];
    let hover = mx >= x0 && mx <= x0 + w && my >= y0 && my <= y0 + h;
    if hover && app.graphics[i].zoom_enabled {
        if io.mouse_down[0] && io.mouse_delta[0] != 0.0 {
            let dt = tw as f32 * io.mouse_delta[0] / w;
            let off = &mut app.graphics[i].t_offset_s;
            *off = (*off + dt as f64).clamp(0.0, max_off);
        }
        if io.mouse_wheel != 0.0 {
            let new_tw = zoom_step(tw, io.mouse_wheel);
            if (new_tw - tw).abs() > 1e-9 {
                let frac = ((mx - x0) / w) as f64;
                let right = t_now - app.graphics[i].t_offset_s;
                let t_mouse = right - tw * frac;
                app.graphics[i].time_window_s = new_tw;
                let new_right = t_mouse + new_tw * frac;
                app.graphics[i].t_offset_s = (t_now - new_right).clamp(0.0, max_off);
            }
        }
    }

    let t_right = t_now - app.graphics[i].t_offset_s;
    // The view asks for its own data rather than waiting for playback to walk
    // past it, so a scrubbed or panned-to window is complete immediately. The
    // extra window-width ahead of the right edge keeps ordinary playback from
    // triggering a scan every frame.
    let need_lo = ((t_right - tw).max(0.0) * 1e6) as u64;
    let need_hi = ((t_right + tw).max(0.0) * 1e6) as u64;
    app.ensure_samples_in(need_lo, need_hi);
    let cursor = if hover && app.graphics[i].show_cursor {
        let frac = ((mx - x0) / w) as f64;
        Some((mx, t_right - tw * (1.0 - frac)))
    } else {
        None
    };

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
                t_right,
                tw,
                app.graphics[i].show_markers,
                cursor,
            );
        }
    } else {
        draw_plot(
            &dl,
            app,
            x0,
            y0,
            w,
            h,
            &keys,
            t_right,
            tw,
            app.graphics[i].show_markers,
            cursor,
        );
    }

    ui.dummy([w, h]);
}

pub(crate) fn draw_plot_frame(dl: &imgui::DrawListMut<'_>, x0: f32, y0: f32, w: f32, h: f32) {
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
    t_right: f64,
    tw: f64,
    show_dots: bool,
    cursor: Option<(f32, f64)>,
) {
    draw_plot_frame(dl, x0, y0, w, h);
    let t_min = t_right - tw;
    // Slice the cache to the window: with an hour of retained samples, a view
    // showing seconds must not walk every point each frame.
    let lo_us = (t_min.max(0.0) * 1e6) as u64;
    let hi_us = (t_right.max(0.0) * 1e6) as u64;

    let mut vmin = f64::INFINITY;
    let mut vmax = f64::NEG_INFINITY;
    for key in keys {
        let Some(sub) = app.subs.get(key) else {
            continue;
        };
        for &(_, v) in sub.history.range(lo_us, hi_us) {
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
        let color = PALETTE[sub.color % PALETTE.len()];
        let pts: Vec<[f32; 2]> = sub
            .history
            .range(lo_us, hi_us)
            .iter()
            .map(|&(t, v)| {
                let tf = t as f64 / 1e6;
                let x = x0 + w * ((tf - t_min) / tw).clamp(0.0, 1.0) as f32;
                let y = y0 + h * (1.0 - ((v - vmin) / (vmax - vmin)).clamp(0.0, 1.0)) as f32;
                [x, y]
            })
            .collect();
        if show_dots {
            for &p in &pts {
                dl.add_circle(p, MARKER_RADIUS_PX, color)
                    .filled(true)
                    .build();
            }
        }
        if pts.len() >= 2 {
            dl.add_polyline(pts, color).thickness(1.5).build();
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

    if let Some((cx, ct)) = cursor {
        dl.add_line([cx, y0], [cx, y0 + h], [0.95, 0.85, 0.4, 0.9])
            .build();
        let left_side = cx > x0 + w - 120.0;
        let time_txt = format!("{:.3}s", ct);
        let tx = if left_side {
            cx - 8.0 - time_txt.len() as f32 * 6.5
        } else {
            cx + 6.0
        };
        dl.add_text([tx, y0 + h - 13.0], [0.95, 0.85, 0.4, 1.0], time_txt);
        let t_us = ct * 1e6;
        let mut row = 0;
        for key in keys {
            let Some(sub) = app.subs.get(key) else {
                continue;
            };
            let txt = match value_at(&sub.history, t_us) {
                Some(v) => format!("{} = {}", key.2, fmt_val(v)),
                None => format!("{} = -", key.2),
            };
            let lx = if left_side {
                cx - 8.0 - txt.len() as f32 * 6.5
            } else {
                cx + 6.0
            };
            dl.add_text(
                [lx, y0 + 4.0 + row as f32 * 12.0],
                PALETTE[sub.color % PALETTE.len()],
                txt,
            );
            row += 1;
        }
    }
}

/// Last sample at or before the given time (step-signal semantics).
fn value_at(history: &crate::app::SampleCache, t_us: f64) -> Option<f64> {
    if !t_us.is_finite() || t_us < 0.0 {
        return None;
    }
    history.at(t_us as u64)
}

#[cfg(test)]
mod tests {
    use super::zoom_step;

    #[test]
    fn zoom_walks_the_step_ladder() {
        assert_eq!(zoom_step(10.0, 1.0), 5.0, "wheel up zooms in");
        assert_eq!(zoom_step(10.0, -1.0), 20.0, "wheel down zooms out");
        assert_eq!(zoom_step(0.2, 1.0), 0.1, "zooms down to the 0.1s step");
        assert_eq!(zoom_step(0.1, 1.0), 0.1, "clamped at the close-up end");
        assert_eq!(zoom_step(3600.0, -1.0), 3600.0, "clamped at 1 h");
        assert_eq!(zoom_step(12.0, -1.0), 20.0, "snaps to nearest step first");
        assert_eq!(zoom_step(10.0, 0.3), 10.0, "sub-notch deltas are ignored");
    }
}
