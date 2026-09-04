//! State Tracker: tracked signals rendered as horizontal state bands --
//! a logic-analyzer view over the same sampled histories the curve
//! windows draw. A *state* is a maximal stretch over which the signal's
//! value, rounded to six significant digits, stays put, and the band
//! carries the value as its label: a 0/1 signal shows its toggling cells,
//! while a smooth analog collapses into a few readable ranges instead of
//! one segment per sample.

use crate::app::{App, PALETTE, TrackedSignal};
use imgui::Ui;

const PANEL_W: f32 = 210.0;
const NAME_W: f32 = 120.0;
const ROW_H: f32 = 26.0;
const ROW_GAP: f32 = 3.0;
const RULER_H: f32 = 24.0;

/// Live trailing window presets -- a state band reads best wide, so the
/// choices start where the curve presets stop.
const TW_CHOICES: [(f64, &str); 5] = [
    (5.0, "5 s"),
    (20.0, "20 s"),
    (60.0, "1 min"),
    (300.0, "5 min"),
    (1800.0, "30 min"),
];

pub fn render(app: &mut App, ui: &Ui) {
    if !app.state_tracker.opened {
        return;
    }
    let mut open = app.state_tracker.opened;
    let disp_h = ui.io().display_size[1];
    ui.window("State Tracker###sttracker")
        .opened(&mut open)
        .position([16.0, disp_h * 0.08], imgui::Condition::FirstUseEver)
        .size([720.0, 340.0], imgui::Condition::FirstUseEver)
        .build(|| window_content(app, ui));
    app.state_tracker.opened = open;
}

fn window_content(app: &mut App, ui: &Ui) {
    for (val, label) in TW_CHOICES {
        let selected = (app.state_tracker.time_window_s - val).abs() < 1e-9;
        let text = if selected {
            format!("[{label}]##stw")
        } else {
            format!("{label}##stw")
        };
        if ui.small_button(text) {
            app.state_tracker.time_window_s = val;
        }
        ui.same_line();
    }
    ui.text_colored([0.6, 0.85, 1.0, 1.0], "live");
    ui.separator();
    let avail = ui.content_region_avail();
    ui.child_window("st_panel")
        .size([PANEL_W, avail[1]])
        .build(|| left_panel(app, ui));
    ui.same_line();
    ui.child_window("st_bands")
        .size([0.0, avail[1]])
        .build(|| bands_area(app, ui));
}

/// Left panel: pick a signal out of the databases and manage the tracked
/// rows. Adding goes straight through `subscribe` -- this window is not
/// in the Measurement Setup registry, so it carries its own picker.
fn left_panel(app: &mut App, ui: &Ui) {
    ui.text("Tracked signals");
    picker(app, ui);
    ui.separator();
    let mut rm = None;
    for j in 0..app.state_tracker.signals.len() {
        let key = app.state_tracker.signals[j].key.clone();
        let color = app
            .sub_view(&key)
            .map(|s| PALETTE[s.color % PALETTE.len()])
            .unwrap_or([0.5, 0.5, 0.5, 1.0]);
        let p = ui.cursor_screen_pos();
        let dl = ui.get_window_draw_list();
        dl.add_rect([p[0], p[1] + 4.0], [p[0] + 10.0, p[1] + 14.0], color)
            .filled(true)
            .build();
        ui.dummy([14.0, 0.0]);
        ui.same_line();
        let mut vis = app.state_tracker.signals[j].visible;
        if ui.checkbox(format!("##stvis{j}"), &mut vis) {
            app.state_tracker.signals[j].visible = vis;
        }
        ui.same_line();
        ui.text(&key.2);
        ui.same_line_with_pos(PANEL_W - 22.0);
        if ui.small_button(format!("x##strm{j}")) {
            rm = Some(j);
        }
    }
    if let Some(j) = rm {
        let key = app.state_tracker.signals.remove(j).key;
        app.prune_signal(&key);
    }
}

fn picker(app: &mut App, ui: &Ui) {
    let db = app
        .snap
        .channels
        .get(app.state_tracker.pick_bus)
        .and_then(|c| c.dbc.as_deref());
    let bus_names: Vec<&str> = app.snap.channels.iter().map(|c| c.name.as_str()).collect();
    let st = &mut app.state_tracker;
    ui.set_next_item_width(-1.0);
    if ui.combo_simple_string("##stbus", &mut st.pick_bus, &bus_names) {
        // A new bus needs a message its own database actually carries.
        st.pick_id = db.and_then(|d| d.order.first()).copied().unwrap_or(0);
        st.pick_signal = 0;
    }
    let add_key = 'pick: {
        let Some(db) = db else {
            ui.text_disabled("no database on this bus");
            break 'pick None;
        };
        let msgs: Vec<String> = db
            .order
            .iter()
            .map(|id| {
                let name = db.messages.get(id).map(|m| m.name.as_str()).unwrap_or("?");
                format!("0x{id:03X} {name}")
            })
            .collect();
        let refs: Vec<&str> = msgs.iter().map(|s| s.as_str()).collect();
        let mut pick = db
            .order
            .iter()
            .position(|id| *id == st.pick_id)
            .unwrap_or(0);
        ui.set_next_item_width(-1.0);
        if ui.combo_simple_string("##stmsg", &mut pick, &refs) {
            st.pick_id = db.order[pick];
            st.pick_signal = 0;
        }
        let sigs: Vec<String> = db
            .messages
            .get(&st.pick_id)
            .map(|m| m.signals.iter().map(|s| s.name.clone()).collect())
            .unwrap_or_default();
        if sigs.is_empty() {
            ui.text_disabled("message has no signals");
            break 'pick None;
        }
        let srefs: Vec<&str> = sigs.iter().map(|s| s.as_str()).collect();
        let mut spick = st.pick_signal.min(srefs.len() - 1);
        ui.set_next_item_width(-1.0);
        if ui.combo_simple_string("##stsig", &mut spick, &srefs) {
            st.pick_signal = spick;
        }
        let key = (
            st.pick_bus as u8,
            st.pick_id,
            sigs[st.pick_signal.min(sigs.len() - 1)].clone(),
        );
        if ui.button("Track##stadd") && !st.signals.iter().any(|s| s.key == key) {
            break 'pick Some(key);
        }
        None
    };
    if let Some(key) = add_key {
        // The draft borrow is gone by here, so the subscribe command and
        // the new row can go through the model together.
        app.subscribe(key.clone());
        app.state_tracker
            .signals
            .push(TrackedSignal { key, visible: true });
    }
}

/// Right area: the time ruler and one state band per tracked row. Names
/// and bands share one draw list, so a row's label and its band cannot
/// drift apart the way separately-laid-out widgets can.
fn bands_area(app: &mut App, ui: &Ui) {
    let avail = ui.content_region_avail();
    let w = avail[0].max(NAME_W + 60.0);
    let h = avail[1].max(RULER_H + ROW_H);
    let [x0, y0] = ui.cursor_screen_pos();
    let dl = ui.get_window_draw_list();

    let tw = app.state_tracker.time_window_s;
    // Always live: the right edge is the plot clock, which follows the
    // replay playhead in replay, so the bands track scrubbing too.
    let t_right = app.plot_now_s();
    let t_left = (t_right - tw).max(0.0);
    let lo_us = (t_left * 1e6) as u64;
    let hi_us = (t_right * 1e6) as u64;
    let bx0 = x0 + NAME_W;
    let bx1 = x0 + w;
    let span_s = (t_right - t_left).max(1e-6);

    // Ruler: pick the finest tick spacing that still leaves ~12 labels.
    let step = nice_step(span_s);
    let mut t = (t_left / step).ceil() * step;
    while t <= t_right {
        let x = bx0 + (((t - t_left) / span_s) as f32) * (bx1 - bx0);
        dl.add_line(
            [x, y0 + RULER_H - 5.0],
            [x, y0 + RULER_H],
            [0.55, 0.55, 0.6, 1.0],
        )
        .thickness(1.0)
        .build();
        let label = if step < 1.0 {
            format!("{t:.1}")
        } else {
            format!("{}", t as u64)
        };
        dl.add_text([x + 2.0, y0 + 2.0], [0.7, 0.7, 0.75, 1.0], label);
        t += step;
    }

    if app.state_tracker.signals.is_empty() {
        dl.add_text(
            [x0 + 8.0, y0 + RULER_H + 8.0],
            [0.6, 0.6, 0.65, 1.0],
            "Track a signal from the panel on the left.",
        );
    }
    // Rows that do not fit the window height are left out; the window can
    // be resized. Tracking dozens of signals wants a scrolling pane, not
    // this clip.
    let fit = (((h - RULER_H) / (ROW_H + ROW_GAP)).floor() as usize).max(1);
    for j in 0..app.state_tracker.signals.len().min(fit) {
        let key = app.state_tracker.signals[j].key.clone();
        let visible = app.state_tracker.signals[j].visible;
        let ry = y0 + RULER_H + j as f32 * (ROW_H + ROW_GAP);
        let bg = if j % 2 == 0 {
            [0.13, 0.13, 0.16, 1.0]
        } else {
            [0.10, 0.10, 0.125, 1.0]
        };
        dl.add_rect([x0, ry], [x0 + w, ry + ROW_H], bg)
            .filled(true)
            .build();
        let tsz = ui.calc_text_size(&key.2);
        let name_col = if visible {
            [0.9, 0.9, 0.95, 1.0]
        } else {
            [0.45, 0.45, 0.5, 1.0]
        };
        dl.add_text([x0 + 4.0, ry + (ROW_H - tsz[1]) * 0.5], name_col, &key.2);
        dl.add_rect([bx0, ry], [bx1, ry + ROW_H], [0.07, 0.07, 0.09, 1.0])
            .filled(true)
            .build();
        if !visible {
            continue;
        }
        let Some(sub) = app.sub_view(&key) else {
            continue;
        };
        let color = PALETTE[sub.color % PALETTE.len()];
        // The value carried into the window by the last sample before it:
        // a signal that updates once a second must still paint the whole
        // band, not just the instant of its sample.
        let held = sub.history.at(lo_us).map(quantize);
        let pts: Vec<(u64, f64)> = sub.history.range(lo_us, hi_us).copied().collect();
        for seg in state_segments(held, &pts, lo_us, hi_us) {
            let sx0 = x_of(bx0, bx1, t_left, span_s, seg.t0_us as f64 / 1e6).max(bx0);
            let sx1 = x_of(bx0, bx1, t_left, span_s, seg.t1_us as f64 / 1e6).min(bx1);
            if sx1 - sx0 < 1.0 {
                continue;
            }
            dl.add_rect(
                [sx0, ry + 2.0],
                [sx1, ry + ROW_H - 2.0],
                [color[0], color[1], color[2], 0.92],
            )
            .filled(true)
            .build();
            let tsz = ui.calc_text_size(&seg.label);
            if tsz[0] + 8.0 <= sx1 - sx0 {
                dl.add_text(
                    [(sx0 + sx1 - tsz[0]) * 0.5, ry + (ROW_H - tsz[1]) * 0.5],
                    [0.06, 0.06, 0.08, 1.0],
                    &seg.label,
                );
            }
        }
    }
}

fn x_of(bx0: f32, bx1: f32, t_left: f64, span_s: f64, t_s: f64) -> f32 {
    bx0 + (((t_s - t_left) / span_s) as f32) * (bx1 - bx0)
}

fn nice_step(span_s: f64) -> f64 {
    for s in [
        0.1, 0.2, 0.5, 1.0, 2.0, 5.0, 10.0, 15.0, 30.0, 60.0, 120.0, 300.0, 600.0,
    ] {
        if span_s / s <= 12.0 {
            return s;
        }
    }
    1200.0
}

/// One constant-value stretch of a tracked signal: `[t0_us, t1_us)` showing
/// `label`. A state opens either at the window's left edge (the held value)
/// or at the sample that changed the value, and ends at the next change or
/// the window's right edge.
#[derive(Debug)]
pub(crate) struct StateSeg {
    pub t0_us: u64,
    pub t1_us: u64,
    pub label: String,
}

/// Rounds to six significant digits -- the `%g` look that decides whether
/// two samples belong to the same state. The rounding is the whole trick:
/// raw floats never compare equal, so the raw values would give one
/// segment per sample.
pub(crate) fn quantize(v: f64) -> f64 {
    if v == 0.0 || !v.is_finite() {
        return v;
    }
    // log10 first, floor second: floor(0.25) is 0 and log10(0) is -inf.
    let mag = v.abs().log10().floor() as i32;
    let digits = (5 - mag).clamp(0, 9);
    let p = 10f64.powi(digits);
    (v * p).round() / p
}

/// Six significant digits, trailing zeros trimmed: the label a band shows.
pub(crate) fn fmt_val(v: f64) -> String {
    if v == 0.0 {
        return "0".to_string();
    }
    let mag = v.abs().log10().floor() as i32;
    let digits = (5 - mag).clamp(0, 9) as usize;
    let s = format!("{v:.digits$}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s == "-0" {
        "0".to_string()
    } else {
        s.to_string()
    }
}

/// Folds ascending samples into maximal constant-value stretches over
/// `[lo_us, hi_us]`. `held` is the value carried in by the last sample
/// before the window (piecewise-constant hold semantics -- a CAN signal
/// keeps its value between updates); `pts` are the in-window samples.
pub(crate) fn state_segments(
    held: Option<f64>,
    pts: &[(u64, f64)],
    lo_us: u64,
    hi_us: u64,
) -> Vec<StateSeg> {
    let mut cur = held.map(|v| {
        let q = quantize(v);
        (q, fmt_val(q), lo_us)
    });
    let mut out = Vec::new();
    for &(t, v) in pts {
        let q = quantize(v);
        match &mut cur {
            // Same state continues.
            Some((qv, _, _)) if q == *qv => {}
            Some((qv, label, t0)) => {
                out.push(StateSeg {
                    t0_us: *t0,
                    t1_us: t,
                    label: std::mem::take(label),
                });
                *qv = q;
                *label = fmt_val(q);
                *t0 = t;
            }
            None => cur = Some((q, fmt_val(q), t.max(lo_us))),
        }
    }
    if let Some((_q, label, t0)) = cur {
        out.push(StateSeg {
            t0_us: t0,
            t1_us: hi_us.max(t0),
            label,
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discrete_toggles_get_one_segment_per_cell() {
        let pts = [(10_000, 0.0), (20_000, 1.0), (30_000, 0.0), (40_000, 1.0)];
        let segs = state_segments(Some(1.0), &pts, 5_000, 45_000);
        assert_eq!(segs.len(), 5, "{segs:?}");
        assert_eq!(
            segs[0].t0_us, 5_000,
            "the held value paints from the left edge"
        );
        assert_eq!(segs[0].label, "1");
        assert_eq!(
            (segs[1].t0_us, segs[1].t1_us, segs[1].label.as_str()),
            (10_000, 20_000, "0")
        );
        assert_eq!(
            segs.last().map(|s| s.t1_us),
            Some(45_000),
            "the last state runs to the right edge"
        );
    }

    #[test]
    fn near_equal_values_collapse_into_one_state() {
        // Sub-factor jitter on a 0.25-resolution signal must not shred the
        // band into per-sample segments.
        let pts = [
            (10_000, 3399.9999),
            (20_000, 3400.0),
            (30_000, 3400.0001),
            (40_000, 3100.0),
        ];
        let segs = state_segments(None, &pts, 0, 50_000);
        assert_eq!(segs.len(), 2, "{segs:?}");
        assert_eq!((segs[0].t0_us, segs[0].label.as_str()), (10_000, "3400"));
        assert_eq!((segs[1].t0_us, segs[1].label.as_str()), (40_000, "3100"));
    }

    #[test]
    fn a_value_seen_before_the_window_holds_across_the_left_edge() {
        let segs = state_segments(Some(2.5), &[], 100_000, 200_000);
        assert_eq!(segs.len(), 1, "{segs:?}");
        assert_eq!((segs[0].t0_us, segs[0].t1_us), (100_000, 200_000));
        assert_eq!(segs[0].label, "2.5");
    }

    #[test]
    fn nothing_seen_means_nothing_drawn() {
        assert!(state_segments(None, &[], 0, 1000).is_empty());
    }

    #[test]
    fn labels_trim_like_percent_g() {
        assert_eq!(fmt_val(3400.0), "3400");
        assert_eq!(fmt_val(3100.0), "3100");
        assert_eq!(fmt_val(0.25), "0.25");
        assert_eq!(fmt_val(2.5), "2.5");
        assert_eq!(fmt_val(-3400.0), "-3400");
        assert_eq!(fmt_val(0.0), "0");
    }
}
