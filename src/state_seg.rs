//! State Tracker band segmentation. It lives in core so CSV exports keep the
//! same bands the web chart draws.

use crate::app::App;
use crate::observe::{SigKey, StateWin};

fn bits_of(v: f64) -> u64 {
    let q = quantize(v);
    if q == 0.0 { 0.0 } else { q }.to_bits()
}

fn table_label(app: &App, key: &crate::observe::SigKey, v: f64) -> Option<String> {
    // A FlexRay signal's labels are its own declaration (the description's
    // VALUE table for the I-SIGNAL), so the state row reads the same as the
    // Trace cell for that very signal -- both show `2 (完成)`, neither a bare 2.
    if let crate::observe::SigKey::Fr {
        bus, slot, name, ..
    } = key
    {
        let sig = app.fr_db(*bus)?.slot_signal(*slot, name)?;
        let raw = ((v - sig.offset) / sig.factor).round() as i64;
        return sig
            .value_descriptions
            .iter()
            .find(|(rv, _)| rv == &raw)
            .map(|(_, t)| t.clone());
    }
    // A `VAL_` value table is a DBC concept, so only a CAN key can have one.
    let crate::observe::SigKey::Can {
        ch, id, ext, name, ..
    } = key
    else {
        return None;
    };
    let db = app.snap.channels.get(*ch as usize)?.dbc.as_deref()?;
    let sig = db
        .messages
        .get(&(*id, *ext))?
        .signals
        .iter()
        .find(|s| &s.name == name)?
        .clone();
    let raw = ((v - sig.offset) / sig.factor).round() as i64;
    let table = db.value_tables.get(&((*id, *ext), name.clone()))?;
    table.get(&raw).cloned()
}
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
    if let Some((_, label, t0, _cv)) = cur {
        out.push(StateSeg {
            t0_us: t0,
            t1_us: hi_us.max(t0),
            label,
        });
    }
    out
}

/// The classified, min-duration-filtered state bands for one tracked
/// signal over `[lo_us, hi_us]`: exactly what the bands draw, shared with
/// the CSV export so the file shows the same picture as the screen.
/// `classify` is custom rules when present, else quantized values labeled
/// from the DBC value table; short bands are absorbed per the window's
/// minimum display duration.
pub(crate) fn compute_segs(
    app: &App,
    wi: usize,
    key: &SigKey,
    held: Option<f64>,
    pts: &[(u64, f64)],
    lo_us: u64,
    hi_us: u64,
) -> Vec<StateSeg> {
    compute_segs_win(&app.state_trackers[wi], app, key, held, pts, lo_us, hi_us)
}

fn compute_segs_win(
    w: &StateWin,
    app: &App,
    key: &SigKey,
    held: Option<f64>,
    pts: &[(u64, f64)],
    lo_us: u64,
    hi_us: u64,
) -> Vec<StateSeg> {
    let rule = w.rules.get(key);
    let mut segs = if let Some(rule) = rule {
        state_segments(held, pts, lo_us, hi_us, |v| {
            let b = rule.band(v);
            (b as u64, rule.names[b].clone())
        })
    } else {
        state_segments(held, pts, lo_us, hi_us, |v| {
            (
                bits_of(v),
                table_label(app, key, v).unwrap_or_else(|| fmt_val(v)),
            )
        })
    };
    if w.min_shown_ms > 0 {
        segs = merge_short_segments(segs, w.min_shown_ms * 1_000);
    }
    segs
}

/// Absorbs state bands shorter than `min_us` so a jittery signal stays
/// readable: a short band merges into the band **before** it (that state
/// simply held longer), except the very first, which merges into the one
/// after. Absorbing can cascade -- a merged band may itself become short
/// relative to its neighbours and disappears the same way.
pub(crate) fn merge_short_segments(mut segs: Vec<StateSeg>, min_us: u64) -> Vec<StateSeg> {
    // Each pass absorbs one short band into a neighbour; absorbing can
    // cascade (a merged band may itself fall under the minimum), so the
    // sweep repeats until every band is long enough -- or one is left.
    while segs.len() > 1 {
        let Some(i) = segs
            .iter()
            .position(|s| s.t1_us.saturating_sub(s.t0_us) < min_us)
        else {
            break;
        };
        if i == 0 {
            // The first band has no predecessor: the successor absorbs it
            // and starts where the absorbed band started.
            segs[1].t0_us = segs[0].t0_us;
            segs.remove(0);
        } else {
            // Otherwise the band before it absorbs it entirely.
            segs[i - 1].t1_us = segs[i].t1_us;
            segs.remove(i);
        }
    }
    segs
}
