//! What the bus did versus what the database said it should do.
//!
//! The DBC is read once for configuration (see [`crate::dbc`]) and the
//! generator obeys it. This module asks the other question: given the frames
//! that actually arrived, which of them break a promise the database made?
//! Four promises are checkable from the data already collected -- the frame
//! belongs to the database at all, it carries the declared number of bytes, it
//! arrives on the declared period, and a message that *was* arriving has not
//! gone quiet -- and all four are verdicts about a message identity over time,
//! so they are taken once per measurement step rather than per frame.

use std::collections::{BTreeMap, HashMap};

/// How far the observed period may stray from the declared one, in percent.
/// Exactly this much is still clean; see [`cycle_offender`].
pub const TOLERANCE_PERCENT: u64 = 10;

/// How many declared periods may pass in silence before a message that was
/// talking counts as dropped.
pub const GRACE_CYCLES: u64 = 3;

/// The four ways a frame or a silence can contradict the database. The order
/// is the order the report window groups and filters by.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Kind {
    /// No database attached, or this id is not in the one that is.
    Unknown,
    /// A different number of bytes than `BO_` declares.
    Dlc,
    /// Arriving faster or slower than the declared period.
    Cycle,
    /// Was arriving, then stopped for longer than the grace window.
    Missing,
}

impl Kind {
    pub const ALL: [Kind; 4] = [Kind::Unknown, Kind::Dlc, Kind::Cycle, Kind::Missing];

    pub fn label(self) -> &'static str {
        match self {
            Kind::Unknown => "Unknown id",
            Kind::Dlc => "DLC",
            Kind::Cycle => "Cycle",
            Kind::Missing => "Dropped",
        }
    }

    /// Position in [`Kind::ALL`], which the report window also indexes its
    /// per-rule filter flags with.
    pub fn index(self) -> usize {
        match self {
            Kind::Unknown => 0,
            Kind::Dlc => 1,
            Kind::Cycle => 2,
            Kind::Missing => 3,
        }
    }
}

/// One message breaking one rule. It latches: the first occurrence opens the
/// record, later ones only deepen the count, and nothing ever closes it but a
/// new run or an explicit clear. A report is a history, not a status light.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Latch {
    pub count: u64,
    pub first_t_us: u64,
    pub last_t_us: u64,
    /// What the database asked for: microseconds, or bytes for [`Kind::Dlc`].
    /// Zero where there is no declaration to compare against.
    pub declared: f64,
    /// What we saw on the offending step, in the same unit as `declared`.
    pub measured: f64,
}

/// The monitor's whole state: the latched rows plus just enough memory of the
/// previous step to measure an interval.
#[derive(Clone, Debug, Default)]
pub struct Spec {
    pub rows: BTreeMap<(u8, u32, Kind), Latch>,
    /// `last_t_us` of each message as of the previous step. Kept here rather
    /// than added to [`crate::app::MessageAgg`], which is the observers' shared
    /// ledger and should not grow a field for one consumer.
    previous: HashMap<(u8, u32), u64>,
}

impl Spec {
    /// The last time we saw this message, as recorded on the previous step.
    /// `None` on its first appearance, when there is no interval to measure.
    pub fn previous(&self, key: (u8, u32)) -> Option<u64> {
        self.previous.get(&key).copied()
    }

    pub fn note(&mut self, key: (u8, u32), last_t_us: u64) {
        self.previous.insert(key, last_t_us);
    }

    pub fn record(&mut self, key: (u8, u32, Kind), now_us: u64, declared: f64, measured: f64) {
        let row = self.rows.entry(key).or_insert(Latch {
            count: 0,
            first_t_us: now_us,
            last_t_us: now_us,
            declared,
            measured,
        });
        row.count += 1;
        row.last_t_us = now_us;
        row.declared = declared;
        row.measured = measured;
    }

    pub fn clear(&mut self) {
        self.rows.clear();
    }

    /// Follow a bus deletion: this bus's rows go, and the ones above it shift
    /// down. Both maps are keyed by channel, and every other channel-keyed
    /// structure gets the same treatment -- a row left at an old index would
    /// accuse a different bus of the deleted one's mistakes.
    pub fn drop_channel(&mut self, ch: u8) {
        let remap = |c: u8| -> Option<u8> {
            match (c as usize).cmp(&(ch as usize)) {
                std::cmp::Ordering::Less => Some(c),
                std::cmp::Ordering::Equal => None,
                std::cmp::Ordering::Greater => Some(c - 1),
            }
        };
        self.rows = std::mem::take(&mut self.rows)
            .into_iter()
            .filter_map(|((c, id, kind), l)| remap(c).map(|nc| ((nc, id, kind), l)))
            .collect();
        self.previous = std::mem::take(&mut self.previous)
            .into_iter()
            .filter_map(|((c, id), t)| remap(c).map(|nc| ((nc, id), t)))
            .collect();
    }
}

/// Is this interval outside the declared period by more than `tol_pct`?
///
/// Both directions count: a message arriving twice as fast is as much a
/// violation as one lagging, and the one-sided comparison is the easy mistake
/// to make here. `declared_us` of 0 is event-triggered, which has no period to
/// break, so it is rejected here rather than at each call site.
pub fn cycle_offender(interval_us: u64, declared_us: u64, tol_pct: u64) -> bool {
    if declared_us == 0 {
        return false;
    }
    let d = declared_us as f64;
    (interval_us as f64 - d).abs() > d * (tol_pct as f64 / 100.0)
}

/// Does the observed length differ from the declared one? Any difference is a
/// mismatch in either direction, so this is an equality test, not a bound.
pub fn dlc_offender(observed: u8, declared: u64) -> bool {
    u64::from(observed) != declared
}

/// Has a message that declared period `d` been silent for more than `grace * d`?
///
/// Saturation matters: `now_us` runs on the simulation clock and a seek or a
/// stopped generator can leave `last_t_us` far behind, so a plain subtraction
/// would panic on overflow in debug builds.
pub fn missing_offender(now_us: u64, last_t_us: u64, declared_us: u64, grace: u64) -> bool {
    if declared_us == 0 {
        return false;
    }
    now_us.saturating_sub(last_t_us) > declared_us.saturating_mul(grace)
}

/// A magnitude in the unit its rule uses: bytes for the length rule,
/// milliseconds for the two timing ones, and nothing at all for an unknown
/// id. The report window and the CSV export render the same wording.
pub fn qty(kind: Kind, v: f64) -> String {
    match kind {
        Kind::Unknown => "-".to_string(),
        Kind::Dlc => format!("{v:.0} B"),
        Kind::Cycle | Kind::Missing => format!("{:.1} ms", v / 1e3),
    }
}

#[cfg(test)]
mod tests {
    use super::{GRACE_CYCLES, Kind, Latch, Spec, cycle_offender, dlc_offender, missing_offender};

    #[test]
    fn cycle_tolerance_is_inclusive_at_the_boundary() {
        assert!(
            !cycle_offender(110_000, 100_000, 10),
            "exactly 10% over is still inside the tolerance"
        );
        assert!(cycle_offender(111_000, 100_000, 10));
    }

    #[test]
    fn a_stalled_message_fails_the_cycle_check_in_both_directions() {
        assert!(
            cycle_offender(50_000, 100_000, 10),
            "twice as fast is as much a violation as twice as slow"
        );
        assert!(cycle_offender(200_000, 100_000, 10));
        assert!(!cycle_offender(95_000, 100_000, 10));
    }

    #[test]
    fn an_event_triggered_declaration_offends_nothing() {
        // A database that says `0 ms` says "no period", so no interval can
        // contradict it. Treating the 0 as a period would convict every frame.
        assert!(!cycle_offender(1_000, 0, 10));
        assert!(!missing_offender(10_000_000, 0, 0, GRACE_CYCLES));
    }

    #[test]
    fn any_length_difference_is_a_dlc_mismatch() {
        assert!(dlc_offender(6, 8));
        assert!(dlc_offender(8, 6), "longer is a mismatch too");
        assert!(!dlc_offender(8, 8));
    }

    #[test]
    fn missing_never_fires_before_the_grace_window_closes() {
        // grace 3 x 100 ms: silent at 250 ms is fine, at 350 ms is not. The
        // mistake this catches is reading grace as an addition.
        assert!(!missing_offender(250_000, 0, 100_000, GRACE_CYCLES));
        assert!(missing_offender(350_000, 0, 100_000, GRACE_CYCLES));
    }

    #[test]
    fn a_latch_opens_once_and_only_deepens() {
        let mut spec = Spec::default();
        let key = (0, 0x64, Kind::Cycle);
        spec.record(key, 100, 100_000.0, 200_000.0);
        spec.record(key, 200, 100_000.0, 300_000.0);
        assert_eq!(
            spec.rows[&key],
            Latch {
                count: 2,
                first_t_us: 100,
                last_t_us: 200,
                declared: 100_000.0,
                measured: 300_000.0,
            }
        );
    }

    #[test]
    fn there_is_no_interval_to_measure_on_a_messages_first_step() {
        let mut spec = Spec::default();
        assert_eq!(spec.previous((0, 0x64)), None);
        spec.note((0, 0x64), 42);
        assert_eq!(spec.previous((0, 0x64)), Some(42));
        spec.clear();
        // Clearing the report must not forget the clock, or the next step would
        // measure an interval across the whole gap it just hid.
        assert_eq!(spec.previous((0, 0x64)), Some(42));
    }

    #[test]
    fn remapping_a_bus_drops_it_and_shifts_the_ones_above() {
        let mut spec = Spec::default();
        for ch in 0..3 {
            spec.record((ch, 1, Kind::Dlc), 0, 8.0, 6.0);
        }
        spec.note((2, 1), 500);
        spec.drop_channel(1);
        let buses: Vec<u8> = spec.rows.keys().map(|(c, _, _)| *c).collect();
        assert_eq!(buses, [0, 1], "gone, and the one above shifted down");
        assert_eq!(spec.previous((1, 1)), Some(500), "the clock moved with it");
        assert_eq!(spec.previous((2, 1)), None);
    }

    #[test]
    fn the_kinds_are_listed_in_report_order() {
        let labels: Vec<&str> = Kind::ALL.iter().map(|k| k.label()).collect();
        assert_eq!(labels, ["Unknown id", "DLC", "Cycle", "Dropped"]);
        let indexes: Vec<usize> = Kind::ALL.iter().map(|k| k.index()).collect();
        assert_eq!(indexes, [0, 1, 2, 3], "the filter flags index by position");
        assert!(Kind::Unknown < Kind::Missing, "Ord follows the listing");
    }
}
