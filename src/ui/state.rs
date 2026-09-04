//! State Tracker: an observer window (Measurement Setup registry, like
//! Graphics/Data) rendering its tracked signals as horizontal state bands
//! -- a logic-analyzer view over the same sampled histories the curve
//! windows draw. A *state* is a maximal stretch over which the signal's
//! value, rounded to six significant digits, stays put, and the band
//! carries the value as its label: a 0/1 signal shows its toggling cells,
//! while a smooth analog collapses into a few readable ranges instead of
//! one segment per sample.

use crate::app::{App, PALETTE, StateRule};
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

/// Word-style default picks for custom bands: low-saturation so a row of
/// neighbours never turns into a shouting match on the dark background.
const BAND_PALETTE: [[f32; 3]; 12] = [
    [0.62, 0.68, 0.78], // 灰蓝
    [0.58, 0.70, 0.60], // 灰绿
    [0.76, 0.58, 0.54], // 灰红
    [0.80, 0.74, 0.58], // 米黄
    [0.65, 0.58, 0.74], // 灰紫
    [0.52, 0.68, 0.70], // 灰青
    [0.76, 0.66, 0.52], // 暖沙
    [0.74, 0.60, 0.66], // 灰粉
    [0.64, 0.67, 0.49], // 橄榄
    [0.50, 0.55, 0.68], // 靛灰
    [0.60, 0.60, 0.62], // 中灰
    [0.70, 0.62, 0.52], // 褐灰
];

/// Synthetic slot keys for band auto-colors, laid in the NaN bit range so
/// no real value's bits can ever collide with them.
fn band_slot_key(band: usize) -> f64 {
    f64::from_bits(u64::MAX - 1_000_000 + band as u64)
}

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
    rule_editor(app, ui);
}

/// CANoe's Value Definition editor for one tracked signal: ascending cut
/// values split the axis into bands (`< c0`, `c0 .. c1`, `>= cN`), and
/// every band gets its own name and fill color. Edits land directly in
/// the window's rule map -- rules are frontend state, so nothing feeds
/// back to fight the keystrokes.
fn rule_editor(app: &mut App, ui: &Ui) {
    let Some((wi, key)) = app.state_rule_edit.clone() else {
        return;
    };
    let title = format!("State bands \u{2014} {}###srules{wi}{}", key.2, key.1);
    let mut open = true;
    ui.window(title)
        .opened(&mut open)
        .size([410.0, 0.0], imgui::Condition::Appearing)
        .build(|| {
            let Some(w) = app.state_trackers.get_mut(wi) else {
                return;
            };
            let Some(rule) = w.rules.get_mut(&key) else {
                ui.text_disabled("No custom bands: values color and label themselves.");
                if ui.small_button("Define bands##srnew") {
                    w.rules.insert(
                        key.clone(),
                        StateRule {
                            cuts: vec![],
                            names: vec!["all".to_string()],
                            colors: vec![None],
                        },
                    );
                }
                return;
            };
            // Cut rows: the boundaries between bands.
            let mut rm_cut = None;
            for (ci, cut) in rule.cuts.iter_mut().enumerate() {
                let mut c = *cut as f32;
                if ui
                    .input_float(format!("cut##srcut{ci}"), &mut c)
                    .display_format("%g")
                    .build()
                {
                    *cut = c as f64;
                }
                ui.same_line();
                if ui.small_button(format!("x##srcutrm{ci}")) {
                    rm_cut = Some(ci);
                }
            }
            if let Some(ci) = rm_cut {
                rule.remove_cut(ci);
            }
            if ui.small_button("Add cut##sradd") {
                let next = rule.cuts.last().copied().unwrap_or(0.0) + 1.0;
                rule.add_cut(next);
            }
            ui.separator();
            // Band rows: range, color swatch, name. The swatch shows the
            // band's real color (an automatic band displays the palette
            // slot it resolves to, not a placeholder) and opens the
            // Word-style picker.
            for bi in 0..rule.names.len() {
                let lo = bi.checked_sub(1).and_then(|p| rule.cuts.get(p).copied());
                let hi = rule.cuts.get(bi).copied();
                let range = match (lo, hi) {
                    (None, Some(h)) => format!("< {h}"),
                    (Some(l), None) => format!(">= {l}"),
                    (Some(l), Some(h)) => format!("{l} .. {h}"),
                    (None, None) => "all".to_string(),
                };
                ui.text(&range);
                ui.same_line_with_pos(110.0);
                let swatch = match rule.colors[bi] {
                    Some(c) => c,
                    None => {
                        // Same slot lookup the band view runs, so the
                        // editor shows exactly what gets drawn.
                        let slots = w.color_slots.entry(key.clone()).or_default();
                        let p = PALETTE[slot_for(slots, band_slot_key(bi))];
                        [p[0], p[1], p[2]]
                    }
                };
                if ui.color_button(
                    format!("##srcol{bi}"),
                    [swatch[0], swatch[1], swatch[2], 1.0],
                ) {
                    ui.open_popup("##srpick");
                    app.state_rule_pick = Some((wi, key.clone(), bi));
                }
                if ui.is_item_hovered() && rule.colors[bi].is_none() {
                    ui.tooltip_text("automatic");
                }
                ui.same_line();
                let mut name = rule.names[bi].clone();
                ui.set_next_item_width(-1.0);
                if ui.input_text(format!("##srname{bi}"), &mut name).build() {
                    rule.names[bi] = name;
                }
            }
            ui.separator();
            if ui.small_button("Clear##srclear") {
                w.rules.remove(&key);
            }
            // Must render inside this window's ID scope: OpenPopup hashed
            // "##srpick" against it, and a popup begun outside would look
            // for a different ID and never show.
            color_picker_popup(app, ui);
        });
    if !open {
        app.state_rule_edit = None;
    }
}

/// The Word-style color picker for one custom band: 自动 plus a short row
/// of low-saturation defaults, with a full palette (and RGB inputs) at
/// the bottom for the rare case nothing default fits. Selecting applies
/// immediately and closes.
fn color_picker_popup(app: &mut App, ui: &Ui) {
    let Some((wi, key, band)) = app.state_rule_pick.clone() else {
        return;
    };
    ui.popup("##srpick", || {
        let Some(w) = app.state_trackers.get_mut(wi) else {
            return;
        };
        let Some(rule) = w.rules.get_mut(&key) else {
            return;
        };
        if band >= rule.colors.len() {
            return;
        }
        if ui.selectable_config("自动").build() {
            rule.colors[band] = None;
            ui.close_current_popup();
            app.state_rule_pick = None;
            return;
        }
        if ui.is_item_hovered() {
            ui.tooltip_text("配色跟随自动机制，与其他状态同样稳定");
        }
        ui.separator();
        ui.text_disabled("默认颜色");
        for (idx, c) in BAND_PALETTE.iter().enumerate() {
            if idx % 6 > 0 {
                ui.same_line();
            }
            if ui.color_button(format!("##srpal{idx}"), [c[0], c[1], c[2], 1.0]) {
                rule.colors[band] = Some(*c);
                ui.close_current_popup();
                app.state_rule_pick = None;
                return;
            }
        }
        ui.separator();
        ui.text_disabled("其他颜色");
        let mut col = rule.colors[band].unwrap_or([0.80, 0.80, 0.80]);
        if ui.color_picker3("##srccustom", &mut col) {
            rule.colors[band] = Some(col);
        }
    });
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
    let mut dl = ui.get_window_draw_list();

    let tw = app.state_trackers[i].time_window_s;
    // Always live: the right edge is the plot clock, which follows the
    // replay playhead in replay, so the bands track scrubbing too.
    let t_right = app.plot_now_s();
    let t_left = (t_right - tw).max(0.0);
    let lo_us = (t_left * 1e6) as u64;
    let hi_us = (t_right * 1e6) as u64;
    let geo = BandGeo {
        bx0: x0 + NAME_W,
        bx1: x0 + w,
        t_left,
        span_s: (t_right - t_left).max(1e-6),
        ry: 0.0,
    };

    // Ruler: minor ticks at a fifth of the label step first, then labeled
    // major ticks over them -- CANoe's axis reads absolute time at a
    // glance this way.
    let step = nice_step(geo.span_s);
    let minor = step / 5.0;
    let mut tm = (geo.t_left / minor).ceil() * minor;
    while tm <= t_right {
        let x = geo.x_of(tm);
        dl.add_line(
            [x, y0 + RULER_H - 3.0],
            [x, y0 + RULER_H],
            [0.32, 0.32, 0.38, 1.0],
        )
        .thickness(1.0)
        .build();
        tm += minor;
    }
    let mut t = (geo.t_left / step).ceil() * step;
    while t <= t_right {
        let x = geo.x_of(t);
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
        let geo = BandGeo { ry, ..geo };
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
        dl.add_rect(
            [geo.bx0, ry],
            [geo.bx1, ry + ROW_H],
            [0.07, 0.07, 0.09, 1.0],
        )
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
        // Custom state bands (CANoe's Value Definition) own both labels
        // and colors when present; otherwise states are quantized values
        // labeled from the DBC value table.
        let rule = app.state_trackers[i].rules.get(&key).cloned();
        let segs = if let Some(rule) = &rule {
            state_segments(held, &pts, lo_us, hi_us, |v| {
                let b = rule.band(v);
                (b as u64, rule.names[b].clone())
            })
        } else {
            state_segments(held, &pts, lo_us, hi_us, |v| {
                let q = quantize(v);
                let q = if q == 0.0 { 0.0 } else { q };
                (
                    q.to_bits(),
                    table_label(app, &key, q).unwrap_or_else(|| fmt_val(q)),
                )
            })
        };
        let mut states: Vec<f64> = segs.iter().map(|s| s.value).collect();
        states.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        states.dedup();

        // CANoe's Binary rendering: a signal whose whole visible world is
        // 0/1 draws as a square wave instead of filled bands -- but a
        // custom rule owns the palette, so it wins.
        if rule.is_none() && is_binary(&states) {
            draw_wave(ui, &mut dl, &segs, &geo, color);
            continue;
        }

        // State colors: with few distinct values visible, every state gets
        // its own palette slot -- gear 5 and gear 1 read apart at a glance.
        // Slots are remembered per value (see `slot_for`), so the same
        // value keeps its color across the whole run. A busy analog with
        // more visible states than the palette gets CANoe's plain neutral
        // band: the value label does the talking.
        let per_state = rule.is_none() && states.len() <= PALETTE.len();
        let win = &mut app.state_trackers[i];
        let slots = win.color_slots.entry(key.clone()).or_default();
        for seg in segs {
            let sx0 = geo.x_of(seg.t0_us as f64 / 1e6).max(geo.bx0);
            let sx1 = geo.x_of(seg.t1_us as f64 / 1e6).min(geo.bx1);
            if sx1 - sx0 < 1.0 {
                continue;
            }
            let fill = if let Some(rule) = &rule {
                let b = rule.band(seg.value);
                match rule.colors[b] {
                    Some(c) => [c[0], c[1], c[2], 0.92],
                    None => PALETTE[slot_for(slots, band_slot_key(b))],
                }
            } else if per_state {
                PALETTE[slot_for(slots, seg.value)]
            } else {
                NEUTRAL_FILL
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

/// Pixel geometry shared by the band drawing helpers: the track's screen
/// span, the visible time span, and the row's top edge. `x_of` maps a
/// sample time onto the track.
struct BandGeo {
    bx0: f32,
    bx1: f32,
    t_left: f64,
    span_s: f64,
    ry: f32,
}

impl BandGeo {
    fn x_of(&self, t_s: f64) -> f32 {
        self.bx0 + (((t_s - self.t_left) / self.span_s) as f32) * (self.bx1 - self.bx0)
    }
}

/// The plain band CANoe paints for signals without usable state coloring.
const NEUTRAL_FILL: [f32; 4] = [0.86, 0.86, 0.89, 0.92];

/// CANoe's Binary rendering applies to signals whose states are just 0
/// and 1 -- detected from the visible states, not configured.
pub(crate) fn is_binary(states: &[f64]) -> bool {
    states.iter().all(|&s| s == 0.0 || s == 1.0)
}

/// The Binary rendering itself: the high state runs along the top of the
/// row, the low state along the bottom, every transition drops a vertical
/// edge, and the label sits mid-cell -- the FlashLight row of the CANoe
/// screenshot.
fn draw_wave(
    ui: &Ui,
    dl: &mut imgui::DrawListMut,
    segs: &[StateSeg],
    geo: &BandGeo,
    color: [f32; 4],
) {
    let ry = geo.ry;
    let high_y = ry + 5.0;
    let low_y = ry + ROW_H - 6.0;
    let mut prev_y: Option<f32> = None;
    for seg in segs {
        let sx0 = geo.x_of(seg.t0_us as f64 / 1e6).max(geo.bx0);
        let sx1 = geo.x_of(seg.t1_us as f64 / 1e6).min(geo.bx1);
        if sx1 - sx0 < 1.0 {
            continue;
        }
        let y = if seg.value == 1.0 { high_y } else { low_y };
        if let Some(py) = prev_y {
            dl.add_line([sx0, py], [sx0, y], color)
                .thickness(1.5)
                .build();
        }
        dl.add_line([sx0, y], [sx1, y], color)
            .thickness(1.5)
            .build();
        prev_y = Some(y);
        let tsz = ui.calc_text_size(&seg.label);
        if tsz[0] + 8.0 <= sx1 - sx0 {
            dl.add_text(
                [(sx0 + sx1 - tsz[0]) * 0.5, ry + (ROW_H - tsz[1]) * 0.5],
                [0.88, 0.89, 0.92, 1.0],
                &seg.label,
            );
        }
    }
}

/// The DBC value-table label for physical state value `v` of signal
/// `key` -- CANoe shows `NM_STATE_NORMAL_OPERATION`, not the raw number.
/// Physical maps back to raw through the signal's own factor/offset;
/// None when there is no table or no entry for that raw value.
fn table_label(app: &App, key: &(u8, u32, String), v: f64) -> Option<String> {
    let db = app.snap.channels.get(key.0 as usize)?.dbc.as_deref()?;
    let sig = db
        .messages
        .get(&key.1)?
        .signals
        .iter()
        .find(|s| s.name == key.2)?;
    let raw = ((v - sig.offset) / sig.factor).round() as i64;
    let table = db.value_tables.get(&(key.1, key.2.clone()))?;
    table.get(&raw).cloned()
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

/// Folds ascending samples into maximal stretches of one state over
/// `[lo_us, hi_us]`. `held` is the value carried in by the last sample
/// before the window (piecewise-constant hold semantics -- a CAN signal
/// keeps its value between updates); `pts` are the in-window samples.
/// `classify` decides what "same state" means and names it: quantized
/// value + DBC label for auto coloring, or custom band index + band name
/// when a rule owns the signal.
pub(crate) fn state_segments(
    held: Option<f64>,
    pts: &[(u64, f64)],
    lo_us: u64,
    hi_us: u64,
    mut classify: impl FnMut(f64) -> (u64, String),
) -> Vec<StateSeg> {
    // The run carries the representative value it opened with: the sample
    // that closes a run belongs to the next one.
    let mut cur = held.map(|v| {
        let (k, label) = classify(v);
        (k, label, lo_us, v)
    });
    let mut out = Vec::new();
    for &(t, v) in pts {
        let (k, label) = classify(v);
        match &mut cur {
            // Same state continues.
            Some((ck, _, _, _)) if k == *ck => {}
            Some((ck, clabel, t0, cv)) => {
                out.push(StateSeg {
                    t0_us: *t0,
                    t1_us: t,
                    value: *cv,
                    label: std::mem::take(clabel),
                });
                *ck = k;
                *clabel = label;
                *t0 = t;
                *cv = v;
            }
            None => cur = Some((k, label, t.max(lo_us), v)),
        }
    }
    if let Some((_, label, t0, cv)) = cur {
        out.push(StateSeg {
            t0_us: t0,
            t1_us: hi_us.max(t0),
            value: cv,
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

    /// The unruled classification the bands draw with: quantize, then key
    /// by the value's bits and label with `%g`.
    fn auto(v: f64) -> (u64, String) {
        let q = quantize(v);
        let q = if q == 0.0 { 0.0 } else { q };
        (q.to_bits(), fmt_val(q))
    }

    #[test]
    fn discrete_toggles_get_one_segment_per_cell() {
        let pts = [(10_000, 0.0), (20_000, 1.0), (30_000, 0.0), (40_000, 1.0)];
        let segs = state_segments(Some(1.0), &pts, 5_000, 45_000, auto);
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
        let segs = state_segments(None, &pts, 0, 50_000, auto);
        assert_eq!(segs.len(), 2, "{segs:?}");
        assert_eq!((segs[0].t0_us, segs[0].label.as_str()), (10_000, "3400"));
        assert_eq!((segs[1].t0_us, segs[1].label.as_str()), (40_000, "3100"));
    }

    #[test]
    fn a_value_seen_before_the_window_holds_across_the_left_edge() {
        let segs = state_segments(Some(2.5), &[], 100_000, 200_000, auto);
        assert_eq!(segs.len(), 1, "{segs:?}");
        assert_eq!((segs[0].t0_us, segs[0].t1_us), (100_000, 200_000));
        assert_eq!(segs[0].label, "2.5");
    }

    #[test]
    fn nothing_seen_means_nothing_drawn() {
        assert!(state_segments(None, &[], 0, 1000, auto).is_empty());
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

    #[test]
    fn segment_labels_come_from_the_labeller() {
        let pts = [(10_000, 1.0), (20_000, 2.0)];
        let segs = state_segments(None, &pts, 0, 30_000, |v| {
            let k = v.to_bits();
            (k, format!("S{v}"))
        });
        assert_eq!(segs[0].label, "S1");
        assert_eq!(segs[1].label, "S2");
    }

    #[test]
    fn binary_rendering_applies_only_to_0_1_signals() {
        assert!(is_binary(&[0.0, 1.0]));
        assert!(is_binary(&[1.0]));
        assert!(!is_binary(&[0.0, 1.0, 2.0]), "a third state is an enum");
        assert!(!is_binary(&[-1.0, 1.0]), "signed values are not the pair");
    }

    const VAL_DBC: &str = r#"VERSION "roxy-can state val test"

NS_ :

BU_: ECU

BO_ 410 Enums: 8 ECU
 SG_ Gear : 0|8@1+ (1,0) [0|0] "" ECU
 SG_ Free : 16|8@1+ (1,0) [0|0] "" ECU

VAL_ 410 Gear 2 "Gear_2" 1 "Gear_1" 0 "Neutral";
"#;

    #[test]
    fn state_labels_prefer_the_dbc_value_table() {
        let mut app = App::headless();
        app.channels[0].dbc = Some(std::sync::Arc::new(
            crate::dbc::load_dbc_str(VAL_DBC).unwrap(),
        ));
        // table_label reads the snapshot's view of the channels.
        app.refresh_snapshot();
        let gear = (0u8, 410u32, "Gear".to_string());
        assert_eq!(table_label(&app, &gear, 2.0).as_deref(), Some("Gear_2"));
        assert_eq!(table_label(&app, &gear, 0.0).as_deref(), Some("Neutral"));
        assert_eq!(
            table_label(&app, &gear, 7.0),
            None,
            "a raw value with no entry stays numeric"
        );
        let free = (0u8, 410u32, "Free".to_string());
        assert_eq!(
            table_label(&app, &free, 1.0),
            None,
            "a signal with no table stays numeric"
        );
    }

    #[test]
    fn rule_cuts_split_and_merge_bands() {
        let mut r = StateRule {
            cuts: vec![],
            names: vec!["all".to_string()],
            colors: vec![None],
        };
        r.add_cut(1000.0);
        assert_eq!(r.band(999.0), 0);
        assert_eq!(
            r.band(1000.0),
            1,
            "the cut itself belongs to the upper band"
        );
        assert_eq!(r.names.len(), 2, "one cut means two bands");
        assert!(
            r.colors.iter().all(|c| c.is_none()),
            "new bands start automatic"
        );
        r.add_cut(500.0);
        assert_eq!(r.cuts, vec![500.0, 1000.0], "cuts stay sorted");
        assert_eq!(r.names.len(), 3);
        r.remove_cut(0);
        assert_eq!(r.cuts, vec![1000.0]);
        assert_eq!(r.names.len(), 2, "removing a cut merges its bands");
    }

    #[test]
    fn a_rule_groups_values_into_bands_with_its_own_names() {
        let rule = StateRule {
            cuts: vec![1000.0],
            names: vec!["low".to_string(), "high".to_string()],
            colors: vec![Some([0.1, 0.2, 0.9]), Some([0.9, 0.8, 0.1])],
        };
        let pts = [
            (10_000, 500.0),
            (20_000, 800.0),
            (30_000, 1200.0),
            (40_000, 900.0),
        ];
        let segs = state_segments(None, &pts, 0, 50_000, |v| {
            let b = rule.band(v);
            (b as u64, rule.names[b].clone())
        });
        assert_eq!(segs.len(), 3, "{segs:?}");
        assert_eq!(segs[0].label, "low", "500 and 800 share the low band");
        assert_eq!(segs[1].label, "high");
        assert_eq!(segs[2].label, "low", "900 drops back into the low band");
    }
}
