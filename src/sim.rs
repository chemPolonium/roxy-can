//! Signal stimulus: how a generated signal's value moves with simulation time.
//!
//! Everything here is a pure function of `(source, t_us)`, with `t_us` in the
//! app's simulation clock. A source never samples a clock of its own and keeps
//! no mutable state, so a waveform is reproducible across runs and a stalled or
//! paused UI cannot warp its phase.

/// Shape of a driven signal's value over time. A signal with no source is not
/// listed here: it simply holds whatever the base payload says.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SrcKind {
    /// `lo` rising to `hi` over one period, then wrapping. Descending when
    /// `hi < lo`.
    Ramp,
    /// Raised cosine: starts exactly at `lo`, peaks exactly at `hi`.
    Sine,
    /// Walks [`ValueSrc::seq`] at equal duration, or toggles `lo`/`hi` if empty.
    Step,
    /// Uniform in `[lo, hi]`, redrawn every [`ValueSrc::redraw_us`].
    Random,
}

/// Combo-box order, so the UI and the persisted `u8` agree.
pub const KINDS: [SrcKind; 4] = [SrcKind::Ramp, SrcKind::Sine, SrcKind::Step, SrcKind::Random];

impl SrcKind {
    pub fn label(self) -> &'static str {
        match self {
            SrcKind::Ramp => "Ramp",
            SrcKind::Sine => "Sine",
            SrcKind::Step => "Step",
            SrcKind::Random => "Random",
        }
    }

    /// Persisted code. Derived from [`KINDS`] so it stays equal to the index the
    /// generator's combo shows.
    pub fn to_u8(self) -> u8 {
        KINDS.iter().position(|&k| k == self).unwrap_or(0) as u8
    }

    /// None for a code written by a newer version, so the caller drops the
    /// source rather than inventing a shape for it.
    pub fn from_u8(v: u8) -> Option<Self> {
        KINDS.get(v as usize).copied()
    }
}

/// One driven signal. `name` is the DBC signal name; values are physical, so
/// factor and offset stay with the encoder that applies them.
#[derive(Clone, Debug, PartialEq)]
pub struct ValueSrc {
    pub name: String,
    pub kind: SrcKind,
    pub lo: f64,
    pub hi: f64,
    /// One full cycle, in microseconds. Zero means one second.
    pub period_us: u64,
    /// Shifted onto `t_us` before wrapping.
    pub phase_us: u64,
    /// Equal-duration values for [`SrcKind::Step`].
    pub seq: Vec<f64>,
    pub seed: u64,
    /// Redraw interval for [`SrcKind::Random`]; zero means every frame.
    pub redraw_us: u64,
}

impl ValueSrc {
    pub fn new(name: &str, kind: SrcKind, lo: f64, hi: f64) -> Self {
        ValueSrc {
            name: name.to_string(),
            kind,
            lo,
            hi,
            period_us: 1_000_000,
            phase_us: 0,
            seq: Vec::new(),
            seed: 0x5EED_1234,
            redraw_us: 0,
        }
    }
}

/// Position within the current cycle, in `[0, 1)`.
fn frac(src: &ValueSrc, t_us: u64) -> f64 {
    let period = if src.period_us == 0 {
        1_000_000
    } else {
        src.period_us
    };
    let at = t_us.saturating_add(src.phase_us);
    (at % period) as f64 / period as f64
}

/// SplitMix64, advanced once. Canonical form, so a value depends only on the
/// state it was fed and not on how many times it has been called.
fn splitmix64(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// The physical value `src` carries at simulation time `t_us`.
pub fn eval_phys(src: &ValueSrc, t_us: u64) -> f64 {
    let span = src.hi - src.lo;
    match src.kind {
        SrcKind::Ramp => src.lo + span * frac(src, t_us),
        // cos rather than sin so the cycle starts at `lo` and its endpoints are
        // exact instead of one sample off.
        SrcKind::Sine => {
            let u = 2.0 * std::f64::consts::PI * frac(src, t_us);
            src.lo + span * (0.5 - 0.5 * u.cos())
        }
        SrcKind::Step => {
            let n = src.seq.len().max(2);
            let i = ((frac(src, t_us) * n as f64) as usize).min(n - 1);
            if src.seq.is_empty() {
                if i == 0 { src.lo } else { src.hi }
            } else {
                src.seq[i]
            }
        }
        SrcKind::Random => {
            let hold = src.redraw_us.max(1);
            let mut state = src.seed ^ (t_us / hold);
            src.lo + span * (splitmix64(&mut state) as f64 / u64::MAX as f64)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn src(kind: SrcKind, lo: f64, hi: f64, period_us: u64) -> ValueSrc {
        ValueSrc {
            period_us,
            ..ValueSrc::new("S", kind, lo, hi)
        }
    }

    #[test]
    fn ramp_rises_linearly_then_wraps() {
        let s = src(SrcKind::Ramp, 0.0, 100.0, 1_000_000);
        assert_eq!(eval_phys(&s, 0), 0.0);
        assert_eq!(eval_phys(&s, 250_000), 25.0);
        assert_eq!(
            eval_phys(&s, 999_999),
            99.9999,
            "approaches but excludes hi"
        );
        assert_eq!(eval_phys(&s, 1_000_000), 0.0, "wraps at the period");
        assert_eq!(eval_phys(&s, 1_500_000), 50.0, "second cycle");
    }

    #[test]
    fn ramp_descends_when_hi_is_below_lo() {
        let s = src(SrcKind::Ramp, 100.0, 0.0, 1_000_000);
        assert_eq!(eval_phys(&s, 0), 100.0);
        assert_eq!(eval_phys(&s, 500_000), 50.0);
    }

    #[test]
    fn sine_starts_at_lo_and_peaks_at_hi() {
        let s = src(SrcKind::Sine, 10.0, 20.0, 1_000_000);
        assert_eq!(eval_phys(&s, 0), 10.0, "must start at lo, not mid-span");
        assert!((eval_phys(&s, 500_000) - 20.0).abs() < 1e-9);
        assert!(
            (eval_phys(&s, 250_000) - 15.0).abs() < 1e-9,
            "quarter cycle"
        );
    }

    #[test]
    fn sine_stays_inside_its_range() {
        let s = src(SrcKind::Sine, -3.5, 7.25, 977_000);
        for t in (0..2_000_000).step_by(1_013) {
            let v = eval_phys(&s, t);
            assert!((-3.5..=7.25).contains(&v), "out of range at t={t}: {v}");
        }
    }

    #[test]
    fn step_walks_its_sequence_at_equal_duration() {
        let mut s = src(SrcKind::Step, 0.0, 9.0, 900_000);
        s.seq = vec![1.0, 4.0, 9.0];
        assert_eq!(eval_phys(&s, 0), 1.0);
        assert_eq!(eval_phys(&s, 299_999), 1.0);
        assert_eq!(eval_phys(&s, 300_000), 4.0);
        assert_eq!(eval_phys(&s, 600_000), 9.0);
        assert_eq!(eval_phys(&s, 899_999), 9.0);
        assert_eq!(eval_phys(&s, 900_000), 1.0, "wraps");
    }

    #[test]
    fn empty_step_sequence_toggles_the_endpoints() {
        let s = src(SrcKind::Step, 2.0, 8.0, 1_000_000);
        assert_eq!(eval_phys(&s, 0), 2.0);
        assert_eq!(eval_phys(&s, 500_000), 8.0);
        assert_eq!(eval_phys(&s, 999_999), 8.0);
    }

    /// Every shape moves; holding a value is the base payload's job, not a
    /// shape's.
    #[test]
    fn every_shape_changes_over_a_cycle() {
        for &kind in &KINDS {
            let s = src(kind, 0.0, 100.0, 1_000_000);
            assert_ne!(
                eval_phys(&s, 0),
                eval_phys(&s, 500_000),
                "{kind:?} must not hold one value through the cycle"
            );
        }
    }

    #[test]
    fn random_is_a_pure_function_of_seed_and_bucket() {
        let mut s = src(SrcKind::Random, 0.0, 1.0, 1_000_000);
        s.redraw_us = 10_000;
        assert_eq!(eval_phys(&s, 5_000), eval_phys(&s, 9_999), "same bucket");
        assert_eq!(eval_phys(&s, 5_000), eval_phys(&s, 5_001));
        assert_ne!(
            eval_phys(&s, 5_000),
            eval_phys(&s, 15_000),
            "a new bucket must redraw"
        );
        let other_seed = ValueSrc {
            seed: s.seed + 1,
            ..s.clone()
        };
        assert_ne!(
            eval_phys(&s, 5_000),
            eval_phys(&other_seed, 5_000),
            "the seed must select the stream"
        );
    }

    #[test]
    fn random_stays_in_range_and_defaults_to_a_redraw_per_frame() {
        let s = src(SrcKind::Random, -5.0, 5.0, 0);
        let mut seen = 0;
        let mut prev = f64::NAN;
        for t in 0..200u64 {
            let v = eval_phys(&s, t * 1_000);
            assert!((-5.0..=5.0).contains(&v), "out of range at t={t}: {v}");
            if v != prev {
                seen += 1;
            }
            prev = v;
        }
        assert!(
            seen > 150,
            "redraw_us == 0 must change on essentially every stamp, saw {seen}"
        );
    }

    #[test]
    fn zero_period_falls_back_to_one_second() {
        let s = src(SrcKind::Ramp, 0.0, 10.0, 0);
        assert_eq!(eval_phys(&s, 500_000), 5.0);
        assert_eq!(eval_phys(&s, 1_000_000), 0.0);
    }

    #[test]
    fn kinds_round_trip_through_u8() {
        for (i, &k) in KINDS.iter().enumerate() {
            assert_eq!(u8::try_from(i).ok().and_then(SrcKind::from_u8), Some(k));
            assert_eq!(usize::from(k.to_u8()), i, "the code is the combo index");
        }
        assert_eq!(SrcKind::from_u8(KINDS.len() as u8), None, "unknown code");
    }

    #[test]
    fn phase_shifts_the_cycle() {
        let mut s = src(SrcKind::Ramp, 0.0, 10.0, 1_000_000);
        assert_eq!(eval_phys(&s, 200_000), 2.0);
        s.phase_us = 200_000;
        assert_eq!(eval_phys(&s, 0), 2.0, "phase moves the whole waveform");
    }
}
