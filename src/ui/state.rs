//! State Tracker: an observer window (Measurement Setup registry, like
//! Graphics/Data) rendering its tracked signals as horizontal state bands
//! -- a logic-analyzer view over the same sampled histories the curve
//! windows draw. A *state* is a maximal stretch over which the signal's
//! value, rounded to six significant digits, stays put, and the band
//! carries the value as its label: a 0/1 signal shows its toggling cells,
//! while a smooth analog collapses into a few readable ranges instead of
//! one segment per sample.

use crate::app::{App, PALETTE};
use imgui::Ui;
use std::collections::HashMap;

const PANEL_W: f32 = 190.0;
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
    let n = app.state_trackers.len();
    let disp_h = ui.io().display_size[1];
    for i in 0..n {
        let mut open = app.state_trackers[i].opened;
        if !open {
            continue;
        }
        let raw = app.state_trackers[i].name.clone();
        let name = if raw.trim().is_empty() {
            format!("State Tracker {}", i + 1)
        } else {
            raw
        };
        if app.focus_title.as_deref() == Some(name.as_str()) {
            unsafe { imgui::sys::igSetNextWindowFocus() };
            app.focus_title = None;
        }
        ui.window(format!("{name}###st{i}"))
            .opened(&mut open)
            .position(
                [16.0 + i as f32 * 30.0, disp_h * 0.08],
                imgui::Condition::FirstUseEver,
            )
            .size([720.0, 340.0], imgui::Condition::FirstUseEver)
            .build(|| {
                window_content(app, ui, i);
            });
        app.state_trackers[i].opened = open;
    }
}

fn window_content(app: &mut App, ui: &Ui, i: usize) {
    for (val, label) in TW_CHOICES {
        let selected = (app.state_trackers[i].time_window_s - val).abs() < 1e-9;
        let text = if selected {
            format!("[{label}]##stw{i}")
        } else {
            format!("{label}##stw{i}")
        };
        if ui.small_button(text) {
            app.state_trackers[i].time_window_s = val;
        }
        ui.same_line();
    }
    ui.text_colored([0.6, 0.85, 1.0, 1.0], "live");
    ui.separator();
    let avail = ui.content_region_avail();
    ui.child_window(format!("st_panel{i}"))
        .size([PANEL_W, avail[1]])
        .build(|| {
            ui.text("Tracked signals");
            crate::ui::siglist::draw(app, ui, crate::ui::siglist::ListKind::State(i));
        });
    ui.same_line();
    ui.child_window(format!("st_bands{i}"))
        .size([0.0, avail[1]])
        .build(|| bands_area(app, ui, i));
}

/// Right area: the time ruler and one state band per tracked row. Names
/// and bands share one draw list, so a row's label and its band cannot
/// drift apart the way separately-laid-out widgets can.
fn bands_area(app: &mut App, ui: &Ui, i: usize) {
    let avail = ui.content_region_avail();
    let w = avail[0].max(NAME_W + 60.0);
    let h = avail[1].max(RULER_H + ROW_H);
    let [x0, y0] = ui.cursor_screen_pos();
    let dl = ui.get_window_draw_list();

    let tw = app.state_trackers[i].time_window_s;
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

    if app.state_trackers[i].signals.is_empty() {
        dl.add_text(
            [x0 + 8.0, y0 + RULER_H + 8.0],
            [0.6, 0.6, 0.65, 1.0],
            "Select signals for this window in Measurement Setup.",
        );
    }
    // Rows that do not fit the window height are left out; the window can
    // be resized. Tracking dozens of signals wants a scrolling pane, not
    // this clip.
    let fit = (((h - RULER_H) / (ROW_H + ROW_GAP)).floor() as usize).max(1);
    for j in 0..app.state_trackers[i].signals.len().min(fit) {
        let key = app.state_trackers[i].signals[j].key.clone();
        let visible = app.state_trackers[i].signals[j].visible;
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
        let segs = state_segments(held, &pts, lo_us, hi_us);
        // State colors: with few distinct values visible, every state gets
        // its own palette slot -- gear 5 and gear 1 read apart at a glance.
        // Slots are remembered per value (see `slot_for`), so the same
        // value keeps its color across the whole run. A busy analog with
        // more visible states than the palette keeps the signal's own
        // color for the whole band.
        let mut states: Vec<f64> = segs.iter().map(|s| s.value).collect();
        states.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        states.dedup();
        let per_state = states.len() <= PALETTE.len();
        let win = &mut app.state_trackers[i];
        let slots = win.color_slots.entry(key.clone()).or_default();
        for seg in segs {
            let sx0 = x_of(bx0, bx1, t_left, span_s, seg.t0_us as f64 / 1e6).max(bx0);
            let sx1 = x_of(bx0, bx1, t_left, span_s, seg.t1_us as f64 / 1e6).min(bx1);
            if sx1 - sx0 < 1.0 {
                continue;
            }
            let fill = if per_state {
                PALETTE[slot_for(slots, seg.value)]
            } else {
                color
            };
            // Per-state mode needs no outlines: neighbouring segments
            // always differ in value and color, so the boundary reads by
            // itself -- an outline anti-aliased against the bright fills
            // just grows a fuzzy halo. In the uniform fallback, same-
            // colored neighbours are separated by a 1px gap instead.
            let (fx0, fx1) = if per_state {
                (sx0, sx1)
            } else {
                ((sx0 + 1.0).min(sx1), (sx1 - 1.0).max(sx0))
            };
            dl.add_rect(
                [fx0, ry + 2.0],
                [fx1, ry + ROW_H - 2.0],
                [fill[0], fill[1], fill[2], 0.92],
            )
            .filled(true)
            .build();
            let tsz = ui.calc_text_size(&seg.label);
            if tsz[0] + 8.0 <= fx1 - fx0 {
                dl.add_text(
                    [(fx0 + fx1 - tsz[0]) * 0.5, ry + (ROW_H - tsz[1]) * 0.5],
                    text_on(fill),
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
    /// The quantized value itself, for state-color assignment.
    pub value: f64,
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
                    value: *qv,
                    label: std::mem::take(label),
                });
                *qv = q;
                *label = fmt_val(q);
                *t0 = t;
            }
            None => cur = Some((q, fmt_val(q), t.max(lo_us))),
        }
    }
    if let Some((q, label, t0)) = cur {
        out.push(StateSeg {
            t0_us: t0,
            t1_us: hi_us.max(t0),
            value: q,
            label,
        });
    }
    out
}

/// The palette slot of one state value, stable for the whole session: the
/// first time a value is seen it takes the lowest free slot, and it keeps
/// that slot even after leaving the view, so colors never reshuffle as
/// the run goes on. Once every slot is spoken for, a brand-new value
/// hashes into the palette and accepts a possible collision -- the
/// alternative, reassigning slots, is what made colors drift.
pub(crate) fn slot_for(slots: &mut HashMap<u64, usize>, v: f64) -> usize {
    // -0.0 and 0.0 print the same label; they share a slot.
    let bits = if v == 0.0 {
        0.0f64.to_bits()
    } else {
        v.to_bits()
    };
    if let Some(&s) = slots.get(&bits) {
        return s;
    }
    let mut used: Vec<usize> = slots.values().copied().collect();
    used.sort_unstable();
    used.dedup();
    let slot = (0..PALETTE.len())
        .find(|s| !used.contains(s))
        .unwrap_or_else(|| {
            let mut h = bits;
            h = (h ^ (h >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
            h = (h ^ (h >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
            h ^= h >> 31;
            (h % PALETTE.len() as u64) as usize
        });
    slots.insert(bits, slot);
    slot
}

/// Dark or light label text, by the fill's relative luminance.
pub(crate) fn text_on(fill: [f32; 4]) -> [f32; 4] {
    let lum = 0.299 * fill[0] + 0.587 * fill[1] + 0.114 * fill[2];
    if lum > 0.55 {
        [0.06, 0.06, 0.08, 1.0]
    } else {
        [0.92, 0.93, 0.95, 1.0]
    }
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

    #[test]
    fn a_value_keeps_its_color_slot_as_others_come_and_go() {
        let mut slots = HashMap::new();
        let gear5 = slot_for(&mut slots, 5.0);
        for v in [1.0, 3.0, 6.0, 0.0, 2.0, 4.0] {
            slot_for(&mut slots, v);
        }
        assert_eq!(slot_for(&mut slots, 5.0), gear5, "the first slot sticks");
        assert_eq!(
            slot_for(&mut slots, 3.0),
            slot_for(&mut slots, 3.0),
            "repeated lookups are free and stable"
        );
        let distinct: std::collections::HashSet<usize> = slots.values().copied().collect();
        assert_eq!(distinct.len(), 7, "each of the 7 states owns a slot");
    }

    #[test]
    fn zero_and_negative_zero_share_a_slot() {
        let mut slots = HashMap::new();
        let a = slot_for(&mut slots, 0.0);
        let b = slot_for(&mut slots, -0.0);
        assert_eq!(a, b);
    }

    #[test]
    fn an_overflowing_value_still_lands_in_the_palette() {
        let mut slots = HashMap::new();
        for v in 0..12 {
            slot_for(&mut slots, f64::from(v));
        }
        assert!(slot_for(&mut slots, 99.0) < PALETTE.len());
    }

    #[test]
    fn label_text_contrasts_with_the_fill() {
        assert_eq!(text_on([0.95, 0.95, 0.2, 1.0])[0], 0.06, "dark on bright");
        assert_eq!(text_on([0.1, 0.2, 0.8, 1.0])[0], 0.92, "light on dark");
    }
}
