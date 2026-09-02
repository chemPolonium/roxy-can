use crate::app::{App, PALETTE};
use crate::observe::YMode;
use imgui::{Condition, Ui};

/// Zoom ladder: round, analysis-friendly window lengths from a close-up up
/// to one hour; wheel zoom and the preset button row share this ladder.
const TIME_STEPS: [f64; 14] = [
    0.1, 0.2, 0.5, 1.0, 5.0, 10.0, 20.0, 30.0, 60.0, 120.0, 300.0, 600.0, 1800.0, 3600.0,
];
pub(crate) const PANEL_W: f32 = 190.0;

/// Widest window the ladder offers. Signal history has to be able to back it,
/// or the curve's head silently vanishes once the retention cap is reached.
/// Only read by the invariant test in `crate::app`.
#[cfg(test)]
pub(crate) const MAX_TIME_WINDOW_S: f64 = TIME_STEPS[TIME_STEPS.len() - 1];

/// Radius of the dot drawn on each sample while `Dots` is enabled.
const MARKER_RADIUS_PX: f32 = 2.2;

/// Vertex budget for a single curve. ImGui indexes a window's draw list with
/// 16-bit indices, so one window cannot exceed 65 536 vertices -- and several
/// curves, their markers and the axis labels all share it. An hour retained at
/// the 50 ms stride is 72 000 points, so anything past this is folded rather
/// than submitted whole.
const MAX_CURVE_POINTS: usize = 2_048;

/// Vertices any one Graphics window may submit. Comfortably under the 65 536
/// ceiling: the frame, grid, axis labels and legend live in the same list, and
/// stacked mode draws every curve into it.
const WINDOW_VERTEX_BUDGET: usize = 40_000;

/// ImGui tessellates a filled circle from twelve segments.
const CIRCLE_VERTS: usize = 13;

/// Least pixels two adjacent sample dots may keep apart. This must be at least
/// the dot's diameter: closer than that, consecutive filled circles overlap and
/// merge into a band about three times the line weight, which reads as the curve
/// having inexplicably become thick rather than as points on a line.
const DOT_MIN_SPACING_PX: f32 = MARKER_RADIUS_PX * 2.0;

/// What one curve may submit this frame, after the window's budget has been
/// shared out between all the curves drawn into the same draw list.
#[derive(Clone, Copy, Debug)]
struct CurveBudget {
    points: usize,
    /// `None` when markers are switched off.
    dots: Option<usize>,
}

impl CurveBudget {
    fn split(curves: usize, width_px: f32, dots_enabled: bool) -> Self {
        let share = WINDOW_VERTEX_BUDGET / curves.max(1);
        // Markers dominate the cost, so when they are on they get the larger
        // half and the polyline gives up the rest.
        let (points, dots) = if dots_enabled {
            (share * 2 / 5, Some(share * 3 / 5 / CIRCLE_VERTS))
        } else {
            (share, None)
        };
        let columns = (width_px / DOT_MIN_SPACING_PX) as usize;
        Self {
            points: points.clamp(64, MAX_CURVE_POINTS),
            dots: dots.map(|d| d.clamp(1, columns.max(1))),
        }
    }

    /// Worst-case vertices this curve can submit. Read only by the budget test.
    #[cfg(test)]
    fn vertices(&self) -> usize {
        self.points + self.dots.unwrap_or(0) * CIRCLE_VERTS
    }
}

/// Width held at the left of a plot for its value labels, and height held along
/// the bottom for the time labels. Fixed rather than measured from the current
/// values so the pointer maths and the drawing can share one function without
/// needing the label text first.
const AXIS_GUTTER_W: f32 = 50.0;
const AXIS_LABEL_H: f32 = 16.0;

/// The rect the curves actually occupy, inset from the panel so axis labels sit
/// outside the data instead of overprinting it.
fn axis_inset(x0: f32, y0: f32, w: f32, h: f32) -> (f32, f32, f32, f32) {
    (
        x0 + AXIS_GUTTER_W,
        y0,
        (w - AXIS_GUTTER_W).max(20.0),
        (h - AXIS_LABEL_H).max(20.0),
    )
}

/// Approximate rendered width of an axis label. Matches the estimate the legend
/// uses; imgui text metrics are not reachable from the draw list.
fn label_width(text: &str) -> f32 {
    text.chars().count() as f32 * 7.0
}

/// A curve ready for submission: its colour plus points already folded to the
/// vertex budget.
type Curve = ([f32; 4], Vec<(u64, f64)>);

/// Collapses an ascending slice to at most `cap` points, keeping each bucket's
/// minimum *and* maximum in timestamp order. Stride decimation would alias away
/// narrow spikes, which is the whole reason a bus trace is worth plotting.
fn bucket_extremes(pts: &[(u64, f64)], cap: usize) -> Vec<(u64, f64)> {
    if pts.len() <= cap {
        return pts.to_vec();
    }
    let buckets = (cap / 2).max(1);
    let per = pts.len().div_ceil(buckets);
    let mut out = Vec::with_capacity(buckets * 2);
    for chunk in pts.chunks(per) {
        let lo = chunk
            .iter()
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .copied()
            .unwrap();
        let hi = chunk
            .iter()
            .max_by(|a, b| a.1.total_cmp(&b.1))
            .copied()
            .unwrap();
        if lo.0 == hi.0 {
            out.push(lo);
        } else if lo.0 < hi.0 {
            out.extend_from_slice(&[lo, hi]);
        } else {
            out.extend_from_slice(&[hi, lo]);
        }
    }
    out
}

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

/// The pan offset a wheel zoom leaves behind, given the offset before it.
///
/// A view riding the live edge (`t_offset_s == 0`) stays there: the right
/// edge keeps sitting on "now" while the window just grows or shrinks, the
/// way a running measurement is expected to behave. Only a view already
/// panned back anchors on the time point under the mouse, so whatever
/// spike brought you here stays put under the cursor.
pub(crate) fn zoom_offset(
    t_offset_s: f64,
    t_now: f64,
    old_tw: f64,
    new_tw: f64,
    frac: f64,
    max_off: f64,
) -> f64 {
    if t_offset_s <= 0.0 {
        return 0.0;
    }
    let t_mouse = t_now - t_offset_s - old_tw * frac;
    let new_right = t_mouse + new_tw * frac;
    (t_now - new_right).clamp(0.0, max_off)
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
    wrap_same_line(ui, "Zoom");
    ui.checkbox("Zoom", &mut app.graphics[i].zoom_enabled);
    wrap_same_line(ui, "Dots");
    ui.checkbox("Dots", &mut app.graphics[i].show_markers);
    wrap_same_line(ui, "Live");
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

/// Chains the next toolbar item onto the current row when an item labelled
/// `text` still fits, otherwise leaves the cursor where it is so the item
/// starts a new row. The toolbar outgrew one row when the Y combo arrived,
/// and a plain `same_line` chain never wraps: whatever runs past the window
/// edge is silently clipped -- which is how the Live button vanished on
/// narrow windows.
///
/// The size comes from the live font and style, not hardcoded pixels, and
/// is padded generously: a needlessly short row costs nothing, while an
/// underestimate is what clipped the button in the first place.
fn wrap_same_line(ui: &Ui, text: &str) {
    let item_w = ui.calc_text_size(text)[0] + ui.frame_height() * 1.6;
    if ui.content_region_avail()[0] >= item_w {
        ui.same_line();
    }
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

    // The legend readouts are throttled text; refresh the snapshot once
    // before the panes pick out their slices.
    app.sync_gfx_legend(i);

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
    // Curves are inset so the axis labels sit outside them; the pointer maths
    // has to use the same rect draw_plot works in.
    let (ix0, iy0, iw, _ih) = axis_inset(x0, y0, w, h);
    let hover = mx >= ix0 && mx <= ix0 + iw && my >= iy0 && my <= iy0 + h;
    if hover && app.graphics[i].zoom_enabled {
        if io.mouse_down[0] && io.mouse_delta[0] != 0.0 {
            let dt = tw as f32 * io.mouse_delta[0] / iw;
            let off = &mut app.graphics[i].t_offset_s;
            *off = (*off + dt as f64).clamp(0.0, max_off);
        }
        if io.mouse_wheel != 0.0 {
            let new_tw = zoom_step(tw, io.mouse_wheel);
            if (new_tw - tw).abs() > 1e-9 {
                let frac = ((mx - ix0) / iw) as f64;
                app.graphics[i].time_window_s = new_tw;
                app.graphics[i].t_offset_s =
                    zoom_offset(app.graphics[i].t_offset_s, t_now, tw, new_tw, frac, max_off);
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
        let frac = ((mx - ix0) / iw) as f64;
        Some((mx, t_right - tw * (1.0 - frac)))
    } else {
        None
    };

    // Every curve in this window writes the same draw list, so the budget is
    // shared out here rather than assumed per call.
    let budget = CurveBudget::split(keys.len(), iw, app.graphics[i].show_markers);
    // The value range is resolved per pane out here, where the Y mode can
    // read and freeze window state; the drawing only consumes it.
    let vis_lo = ((t_right - tw).max(0.0) * 1e6) as u64;
    let vis_hi = (t_right.max(0.0) * 1e6) as u64;
    if keys.is_empty() {
        draw_plot_frame(&dl, x0, y0, w, h);
        dl.add_text(
            [x0 + 8.0, y0 + 8.0],
            [0.5, 0.5, 0.6, 1.0],
            "add signals via Measurement Setup (…)",
        );
    } else if stacked {
        let ph = h / keys.len() as f32;
        let last = keys.len() - 1;
        for (k, key) in keys.iter().enumerate() {
            let legend = app.graphics[i].legend_for(std::slice::from_ref(key));
            let y_range = resolve_y_range(app, i, std::slice::from_ref(key), vis_lo, vis_hi);
            draw_plot(
                &dl,
                app,
                PlotPane {
                    x0,
                    y0: y0 + k as f32 * ph,
                    w,
                    h: ph,
                    keys: std::slice::from_ref(key),
                    legend,
                    t_right,
                    tw,
                    y_range,
                    budget,
                    // The time axis is shared, so labelling it once at the bottom
                    // is enough -- per pane it collided with the next pane's top
                    // value label.
                    time_labels: k == last,
                    cursor,
                },
            );
        }
    } else {
        let y_range = resolve_y_range(app, i, &keys, vis_lo, vis_hi);
        draw_plot(
            &dl,
            app,
            PlotPane {
                x0,
                y0,
                w,
                h,
                keys: &keys,
                legend: app.graphics[i].legend_for(&keys),
                t_right,
                tw,
                y_range,
                budget,
                time_labels: true,
                cursor,
            },
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

/// One draw_plot invocation: where the pane sits and what it plots. A plain
/// argument list ran to twelve parameters and tripped the lint.
struct PlotPane<'a> {
    x0: f32,
    y0: f32,
    w: f32,
    h: f32,
    keys: &'a [(u8, u32, String)],
    /// Throttled legend strings, one per key (see
    /// `App::sync_gfx_legend`); the readout holds still while the curve
    /// animates at full frame rate.
    legend: Vec<String>,
    t_right: f64,
    tw: f64,
    /// The value range this pane draws against, resolved by the window's
    /// [`YMode`] before the call.
    y_range: (f64, f64),
    budget: CurveBudget,
    time_labels: bool,
    cursor: Option<(f32, f64)>,
}

/// The value range one pane draws against.
///
/// Each signal carries its own [`YMode`], so in one-plot-per-signal mode
/// every pane scales by its own signal's policy. The shared overlay axis is
/// the union of the visible signals' individual ranges: a locked signal pins
/// its floor or ceiling into the axis while an Auto neighbour keeps
/// breathing around it.
pub(crate) fn resolve_y_range(
    app: &mut App,
    gi: usize,
    keys: &[(u8, u32, String)],
    lo_us: u64,
    hi_us: u64,
) -> (f64, f64) {
    let (mut lo, mut hi) = (f64::INFINITY, f64::NEG_INFINITY);
    for key in keys {
        let (a, b) = resolve_signal_range(app, gi, key, lo_us, hi_us);
        lo = lo.min(a);
        hi = hi.max(b);
    }
    if !lo.is_finite() || !hi.is_finite() || hi - lo < 1e-9 {
        (0.0, 1.0)
    } else {
        (lo, hi)
    }
}

/// One signal's value range under its own Y policy.
///
/// Locking captures the auto range the first frame it is needed, so a
/// project restored straight into Lock mode waits for data instead of
/// freezing the empty 0..1 placeholder.
fn resolve_signal_range(
    app: &mut App,
    gi: usize,
    key: &(u8, u32, String),
    lo_us: u64,
    hi_us: u64,
) -> (f64, f64) {
    let mode = app.graphics[gi]
        .signals
        .iter()
        .find(|s| &s.key == key)
        .map(|s| s.y_mode)
        .unwrap_or(YMode::Auto);
    let lock_key = format!("{key:?}");

    /// What the visible slice spans right now -- the behaviour of every
    /// Graphics tool before the Y modes existed.
    fn auto(app: &App, key: &(u8, u32, String), lo_us: u64, hi_us: u64) -> (f64, f64) {
        let mut vmin = f64::INFINITY;
        let mut vmax = f64::NEG_INFINITY;
        if let Some(sub) = app.subs.get(key) {
            for &(_, v) in sub.history.range(lo_us, hi_us) {
                if v < vmin {
                    vmin = v;
                }
                if v > vmax {
                    vmax = v;
                }
            }
        }
        if !vmin.is_finite() || !vmax.is_finite() {
            (0.0, 1.0)
        } else if vmax - vmin < 1e-9 {
            (vmin, vmin + 1.0)
        } else {
            (vmin, vmax)
        }
    }

    match mode {
        YMode::Auto => auto(app, key, lo_us, hi_us),
        YMode::Lock => {
            if let Some(&r) = app.graphics[gi].y_locks.get(&lock_key) {
                r
            } else {
                let r = auto(app, key, lo_us, hi_us);
                app.graphics[gi].y_locks.insert(lock_key, r);
                r
            }
        }
        YMode::FitAll => {
            // The cumulative sampler stats are the run's whole envelope: they
            // only ever widen, which is the point of this mode.
            match app.subs.get(key) {
                Some(sub) if sub.min.is_finite() && sub.max.is_finite() => {
                    if sub.max - sub.min < 1e-9 {
                        (sub.min, sub.max + 1.0)
                    } else {
                        (sub.min, sub.max)
                    }
                }
                _ => (0.0, 1.0),
            }
        }
        YMode::Dbc => {
            // The database's word wins; an undeclared signal falls back to
            // its observed extremes once anything has been sampled.
            let declared = app.declared_range(key).or_else(|| {
                app.subs
                    .get(key)
                    .map(|s| (s.min, s.max))
                    .filter(|(a, b)| a.is_finite() && b.is_finite() && b - a >= 1e-9)
            });
            declared.unwrap_or((0.0, 1.0))
        }
    }
}

fn draw_plot(dl: &imgui::DrawListMut<'_>, app: &App, pane: PlotPane<'_>) {
    let PlotPane {
        x0,
        y0,
        w,
        h,
        keys,
        legend,
        t_right,
        tw,
        y_range,
        budget,
        time_labels,
        cursor,
    } = pane;
    // Everything below works in the inset rect, leaving the gutter and bottom
    // strip free for the axis labels.
    let (x0, y0, w, h) = axis_inset(x0, y0, w, h);
    draw_plot_frame(dl, x0, y0, w, h);
    let t_min = t_right - tw;
    // Slice the cache to the window: with an hour of retained samples, a view
    // showing seconds must not walk every point each frame.
    let lo_us = (t_min.max(0.0) * 1e6) as u64;
    let hi_us = (t_right.max(0.0) * 1e6) as u64;

    // One pass per curve, folded to the vertex budget: imgui caps a window's
    // draw list at 65 536 vertices, and an hour retained at the 50 ms stride is
    // 72 000 points for a single signal.
    let curves: Vec<Curve> = keys
        .iter()
        .filter_map(|key| {
            let sub = app.subs.get(key)?;
            let color = PALETTE[sub.color % PALETTE.len()];
            Some((
                color,
                bucket_extremes(sub.history.range(lo_us, hi_us), budget.points),
            ))
        })
        .collect();

    // The pane's range was resolved by the window's Y mode before the call;
    // the guard only keeps a degenerate value from wrecking the mapping.
    let (mut vmin, mut vmax) = y_range;
    if !vmin.is_finite() || !vmax.is_finite() || vmax - vmin < 1e-9 {
        vmin = 0.0;
        vmax = 1.0;
    }

    // Labels live outside the curve rect: time labels centred under their
    // gridline in the bottom strip, value labels right-aligned in the left
    // gutter. They used to be drawn inside the data, where the bottom value and
    // the leftmost time label landed on the same pixel.
    let y_labels: Vec<String> = (0..=4)
        .map(|g| fmt_val(vmax - (vmax - vmin) * g as f64 / 4.0))
        .collect();

    for g in 0..=10 {
        let x = x0 + w * g as f32 / 10.0;
        dl.add_line([x, y0], [x, y0 + h], [0.18, 0.18, 0.22, 1.0])
            .build();
        if time_labels && g % 2 == 0 && g < 10 && h > 20.0 {
            let t = t_min + tw * g as f64 / 10.0;
            let text = format!("{:.1}s", t);
            let lx = (x - label_width(&text) * 0.5).max(x0 - AXIS_GUTTER_W + 2.0);
            dl.add_text([lx, y0 + h + 3.0], [0.55, 0.55, 0.65, 1.0], text);
        }
    }
    for (g, label) in y_labels.iter().enumerate() {
        let y = y0 + h * g as f32 / 4.0;
        dl.add_line([x0, y], [x0 + w, y], [0.18, 0.18, 0.22, 1.0])
            .build();
        let ly = (y - 6.5).max(y0 - 6.0);
        dl.add_text(
            [x0 - 4.0 - label_width(label), ly],
            [0.55, 0.55, 0.65, 1.0],
            label.clone(),
        );
    }

    for entry in &curves {
        let color = entry.0;
        let samples = &entry.1;
        let pts: Vec<[f32; 2]> = samples
            .iter()
            .map(|&(t, v)| {
                let tf = t as f64 / 1e6;
                let x = x0 + w * ((tf - t_min) / tw).clamp(0.0, 1.0) as f32;
                let y = y0 + h * (1.0 - ((v - vmin) / (vmax - vmin)).clamp(0.0, 1.0)) as f32;
                [x, y]
            })
            .collect();
        if let Some(max_dots) = budget.dots {
            // One circle costs about thirteen vertices, so markers are the
            // expensive half of the budget. Points closer together than a pixel
            // column share that column anyway: the stride drops duplicates, not
            // information.
            let stride = (pts.len() / max_dots.max(1)) + 1;
            for &p in pts.iter().step_by(stride) {
                dl.add_circle(p, MARKER_RADIUS_PX, color)
                    .filled(true)
                    .build();
            }
        }
        if pts.len() >= 2 {
            dl.add_polyline(pts, color).thickness(1.5).build();
        }
    }

    // The value strings are the throttled legend snapshot, aligned with
    // `keys` by position; only the color comes from the live subscription.
    let entries: Vec<([f32; 4], String)> = keys
        .iter()
        .enumerate()
        .filter_map(|(n, key)| {
            let sub = app.subs.get(key)?;
            Some((PALETTE[sub.color % PALETTE.len()], legend[n].clone()))
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
    use super::{
        AXIS_GUTTER_W, AXIS_LABEL_H, CurveBudget, MARKER_RADIUS_PX, MAX_CURVE_POINTS, axis_inset,
        bucket_extremes, zoom_offset, zoom_step,
    };

    #[test]
    fn axis_inset_reserves_room_outside_the_data() {
        let (ix, iy, iw, ih) = axis_inset(100.0, 50.0, 600.0, 300.0);
        assert_eq!((ix, iy), (150.0, 50.0), "left gutter for value labels");
        assert_eq!(iw, 600.0 - AXIS_GUTTER_W);
        assert_eq!(ih, 300.0 - AXIS_LABEL_H, "bottom strip for time labels");
        let (_, _, tiny_w, tiny_h) = axis_inset(0.0, 0.0, 10.0, 5.0);
        assert_eq!(
            (tiny_w, tiny_h),
            (20.0, 20.0),
            "a pane too small to pay both margins keeps a usable rect"
        );
    }

    #[test]
    fn a_window_never_exceeds_the_16bit_vertex_ceiling() {
        // The assert that killed replay: imgui indexes a window's draw list with
        // 16-bit indices, and every curve, marker and label in one Graphics
        // window shares that one list.
        for curves in [1usize, 4, 12, 32, 64] {
            for width in [400.0f32, 1920.0, 3840.0] {
                let b = CurveBudget::split(curves, width, true);
                let total = curves * b.vertices();
                assert!(
                    total < 65_536,
                    "{curves} curves at {width}px submit {total} vertices"
                );
                let plain = CurveBudget::split(curves, width, false);
                assert!(curves * plain.vertices() < 65_536, "markers off");
            }
        }
    }

    #[test]
    fn many_curves_share_the_budget_instead_of_each_taking_a_full_one() {
        let few = CurveBudget::split(1, 1920.0, true);
        let many = CurveBudget::split(16, 1920.0, true);
        assert!(
            many.points < few.points && many.dots < few.dots,
            "16 curves must get a smaller share each: {many:?} vs {few:?}"
        );
        assert!(few.points == MAX_CURVE_POINTS, "a lone curve takes the cap");
    }

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

    #[test]
    fn wheel_zoom_at_live_stays_live() {
        // The view rides the newest data; zooming may grow or shrink the
        // window but must not drop the right edge behind "now" -- however
        // far from the right edge the mouse sits.
        for frac in [0.0, 0.25, 0.5, 0.9, 1.0] {
            assert_eq!(
                zoom_offset(0.0, 100.0, 1.0, 0.5, frac, 500.0),
                0.0,
                "live stays live at frac {frac}"
            );
        }
    }

    #[test]
    fn wheel_zoom_when_panned_anchors_the_mouse_point() {
        // Panned 2 s behind a playhead at 10 s, window 1 s -> 2 s with the
        // mouse mid-plot: the point under the mouse (7.5 s) stays under it.
        let off = zoom_offset(2.0, 10.0, 1.0, 2.0, 0.5, 500.0);
        assert!((off - 1.5).abs() < 1e-9, "offset {off}");
        let right = 10.0 - off;
        let under_mouse = right - 2.0 * 0.5;
        assert!((under_mouse - 7.5).abs() < 1e-9);
    }

    #[test]
    fn a_panned_zoom_out_cannot_reach_past_live_or_the_history_head() {
        // Zooming out from near-live clamps at the live edge instead of
        // panning into the future...
        assert_eq!(zoom_offset(0.2, 100.0, 1.0, 5.0, 0.5, 500.0), 0.0);
        // ...and zooming out far back clamps at the oldest sample.
        assert_eq!(zoom_offset(50.0, 100.0, 1.0, 5.0, 0.5, 30.0), 30.0);
    }

    fn ramp(n: usize) -> Vec<(u64, f64)> {
        (0..n as u64).map(|i| (i * 50_000, i as f64)).collect()
    }

    #[test]
    fn dots_cannot_outnumber_the_columns_that_fit_their_own_diameter() {
        // Markers closer together than their diameter overlap and read as a
        // thick line rather than points, so the budget must stay under
        // width / (2 * radius) whatever the curve count.
        for curves in [1usize, 4, 16] {
            for width in [400.0f32, 1920.0, 3840.0] {
                let b = CurveBudget::split(curves, width, true);
                let room = (width / (2.0 * MARKER_RADIUS_PX)) as usize;
                assert!(
                    b.dots.unwrap() <= room.max(1),
                    "{curves} curves at {width}px allow {} dots into {room} columns",
                    b.dots.unwrap()
                );
            }
        }
    }

    #[test]
    fn folding_never_exceeds_the_vertex_budget() {
        // This is the imgui guard: one window cannot submit more than 65 536
        // vertices, and several curves plus their markers share it.
        let pts = ramp(72_000);
        let folded = bucket_extremes(&pts, MAX_CURVE_POINTS);
        assert!(
            folded.len() <= MAX_CURVE_POINTS,
            "folded to {} points, over the {MAX_CURVE_POINTS} cap",
            folded.len()
        );
    }

    #[test]
    fn folding_keeps_a_single_narrow_spike() {
        // Stride decimation would step straight over this and the plot would
        // silently lose its most interesting feature.
        let mut pts: Vec<(u64, f64)> = (0..72_000u64).map(|i| (i * 50_000, 0.0)).collect();
        pts[50_000].1 = 12_345.0;
        pts[1234].1 = -9_876.0;
        let folded = bucket_extremes(&pts, MAX_CURVE_POINTS);
        let lo = folded.iter().map(|(_, v)| *v).fold(f64::INFINITY, f64::min);
        let hi = folded
            .iter()
            .map(|(_, v)| *v)
            .fold(f64::NEG_INFINITY, f64::max);
        assert_eq!(hi, 12_345.0, "the spike must survive");
        assert_eq!(lo, -9_876.0, "the dip must survive");
    }

    #[test]
    fn folding_preserves_time_order() {
        let folded = bucket_extremes(&ramp(9_000), MAX_CURVE_POINTS);
        assert!(
            folded
                .iter()
                .zip(folded.iter().skip(1))
                .all(|(a, b)| a.0 <= b.0),
            "add_polyline walks the points in order"
        );
    }

    #[test]
    fn short_curves_are_left_alone() {
        let pts = ramp(10);
        assert_eq!(bucket_extremes(&pts, MAX_CURVE_POINTS), pts);
    }
}
