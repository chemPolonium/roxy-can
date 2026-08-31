//! Per-bus load and frame rate, the CANoe Statistics view's headline numbers.
//!
//! Load is **bit-time weighted**, not frame-count weighted: a 64-byte FD frame
//! with BRS spends most of its bits in the data phase at the data bitrate, an
//! order of magnitude shorter than counting it at the arbitration rate -- and
//! a plain frame count does not distinguish the two at all. Each frame is
//! converted to its approximate time on wire, and load is the share of a
//! rolling window that wire time occupies.

use std::collections::VecDeque;

use crate::can::frame::{CanFrame, FrameFlags};

/// Wire time of one frame in microseconds, from the bit counts below. An
/// error frame carries no identifier; it is charged the standard-frame
/// overhead only, which matches its dominant-flag behaviour closely enough
/// for a load display.
pub fn wire_time_us(f: &CanFrame, arb_kbps: u32, data_kbps: u32) -> f64 {
    if f.is_error() {
        return STD_ARBITRATION_BITS as f64 / kbits_per_us(arb_kbps);
    }
    let brs = f.flags.contains(FrameFlags::BRS) && f.flags.contains(FrameFlags::FD);
    if f.flags.contains(FrameFlags::FD) {
        let arb_bits = if f.extended {
            EXT_ARBITRATION_BITS + FD_ARBITRATION_EXTRA
        } else {
            STD_ARBITRATION_BITS + FD_ARBITRATION_EXTRA
        };
        // The payload and the (longer) FD CRC clock out of the data phase at
        // the data bitrate; frame tail (EOF/ACK/IFS) stays in arbitration.
        let (arb_bits, data_bits) = if brs {
            (
                arb_bits as f64,
                8.0 * f.len as f64 + FD_DATA_PHASE_EXTRA,
            )
        } else {
            (
                (arb_bits + 8 * f.len as u64) as f64,
                0.0,
            )
        };
        arb_bits / kbits_per_us(arb_kbps) + data_bits / kbits_per_us(data_kbps)
    } else {
        let bits = if f.extended {
            EXT_ARBITRATION_BITS
        } else {
            STD_ARBITRATION_BITS
        } + 8 * f.len as u64;
        bits as f64 / kbits_per_us(arb_kbps)
    }
}

const fn kbits_per_us(kbps: u32) -> f64 {
    kbps as f64 / 1_000.0
}

/// Stuff-bit-free frame overhead incl. EOF and intermission: an 8-byte
/// standard frame counts 111 bits, the usual ballpark figure (its worst-case
/// stuffed form reaches ~1.3x; a load display does not pretend to that).
const STD_ARBITRATION_BITS: u64 = 47;
const EXT_ARBITRATION_BITS: u64 = 67;
/// FD's longer control field and the FDF/BRS/ESI flags.
const FD_ARBITRATION_EXTRA: u64 = 8;
/// CRC-21, stuff bits, and the frame tail for the data phase.
const FD_DATA_PHASE_EXTRA: f64 = 40.0;

/// One bus's rolling traffic window: load and frame rate over the last
/// [`WINDOW_US`], a 100 ms-bucketed sparkline history, and the error frame
/// total. Updated from the same frame stream the aggregates read, so a frame
/// never reaches one and not the other.
pub struct BusLoad {
    recent: VecDeque<(u64, f64)>,
    window_wire_us: f64,
    /// (bucket start t_us, wire time within the bucket) at
    /// [`BUCKET_US`] resolution, newest last, [`HISTORY_BUCKETS`] kept.
    buckets: VecDeque<(u64, f64)>,
    pub errors: u64,
    /// Newest timestamp seen, so a paused bus drains its window forward in
    /// time instead of keeping stale frames alive forever.
    newest_t_us: u64,
}

/// Load and frame rate are measured over this much wall time.
pub const WINDOW_US: u64 = 1_000_000;
/// Sparkline resolution.
pub const BUCKET_US: u64 = 100_000;
/// How many buckets the sparkline keeps: 60 x 100 ms = the last minute.
pub const HISTORY_BUCKETS: usize = 60;

impl BusLoad {
    pub fn new() -> Self {
        BusLoad {
            recent: VecDeque::new(),
            window_wire_us: 0.0,
            buckets: VecDeque::new(),
            errors: 0,
            newest_t_us: 0,
        }
    }

    pub fn note(&mut self, f: &CanFrame, wire_us: f64) {
        let t = f.t_us;
        if f.is_error() {
            self.errors += 1;
        }
        // Frames out of order or before what we have seen (a seek backwards)
        // would poison the sliding window; the buckets simply ignore them.
        if t < self.newest_t_us {
            return;
        }
        self.newest_t_us = t;
        self.recent.push_back((t, wire_us));
        self.window_wire_us += wire_us;
        self.prune_window(t);

        match self.buckets.back() {
            Some(&(start, _)) if t < start + BUCKET_US => {
                self.buckets.back_mut().unwrap().1 += wire_us;
            }
            _ => {
                let start = t - t % BUCKET_US;
                self.buckets.push_back((start, wire_us));
                if self.buckets.len() > HISTORY_BUCKETS {
                    self.buckets.pop_front();
                }
            }
        }
    }

    /// Drops frames that left the window and reports the load they carried.
    fn prune_window(&mut self, now: u64) {
        let horizon = now.saturating_sub(WINDOW_US);
        while let Some(&(t, w)) = self.recent.front() {
            if t > horizon {
                break;
            }
            self.recent.pop_front();
            self.window_wire_us -= w;
        }
    }

    /// Load as a fraction of the bus capacity over the last second, 0..=1+
    /// (a bus can be oversubscribed on paper).
    pub fn load(&self) -> f64 {
        self.window_wire_us / WINDOW_US as f64
    }

    /// Frames per second over the same window.
    pub fn frame_rate(&self) -> f64 {
        self.recent.len() as f64 / (WINDOW_US as f64 / 1_000_000.0)
    }

    /// The sparkline as (t_us, load fraction) pairs, one per 100 ms bucket.
    pub fn history(&self) -> impl Iterator<Item = (u64, f64)> + '_ {
        self.buckets
            .iter()
            .map(|&(start, wire)| (start, wire / BUCKET_US as f64))
    }

    pub fn clear(&mut self) {
        self.recent.clear();
        self.buckets.clear();
        self.window_wire_us = 0.0;
        self.errors = 0;
        self.newest_t_us = 0;
    }
}

impl Default for BusLoad {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::can::frame::{Direction, MAX_CAN_FD_LEN};

    fn frame(t_us: u64, len: u8, extended: bool, flags: FrameFlags) -> CanFrame {
        CanFrame {
            t_us,
            channel: 0,
            id: 0x100,
            extended,
            len,
            data: [0u8; MAX_CAN_FD_LEN],
            dir: Direction::Rx,
            flags,
        }
    }

    #[test]
    fn a_classic_8byte_frame_counts_111_bits() {
        // The documented ballpark figure: 47 overhead + 64 payload bits at
        // 500 kbit/s = 222 µs on wire.
        let f = frame(0, 8, false, FrameFlags::NONE);
        assert!((wire_time_us(&f, 500, 2000) - 222.0).abs() < 1e-9);
    }

    #[test]
    fn an_extended_frame_costs_more_than_a_standard_one() {
        let std = frame(0, 8, false, FrameFlags::NONE);
        let ext = frame(0, 8, true, FrameFlags::NONE);
        assert!((wire_time_us(&ext, 500, 2000) - wire_time_us(&std, 500, 2000) - 40.0).abs() < 1e-9,
            "the 20 extra identifier bits are 40 µs at 500 kbit/s");
    }

    #[test]
    fn brs_moves_the_payload_into_the_faster_data_phase() {
        // 64 bytes of payload at 2 Mbit/s data phase is four times cheaper
        // than clocking it through the 500 kbit/s arbitration phase.
        let slow = frame(0, 64, false, FrameFlags::FD);
        let fast = frame(0, 64, false, FrameFlags::FD.union(FrameFlags::BRS));
        let t_slow = wire_time_us(&slow, 500, 2000);
        let t_fast = wire_time_us(&fast, 500, 2000);
        assert!(
            t_fast < t_slow * 0.5,
            "BRS 64B frame {t_fast} µs should be far under the no-BRS {t_slow} µs"
        );
        // Hand calculation: arb = (47+8)/500k, data = (512+40)/2M.
        let expected = 55.0 / 0.5 + 552.0 / 2.0;
        assert!((t_fast - expected).abs() < 1e-9);
    }

    #[test]
    fn a_non_fd_frame_ignores_the_data_rate() {
        let f = frame(0, 8, false, FrameFlags::BRS);
        assert!((wire_time_us(&f, 500, 8000) - 222.0).abs() < 1e-9,
            "BRS without FD is nonsense; the arbitration rate decides");
    }

    #[test]
    fn an_error_frame_charges_the_overhead_and_counts_itself() {
        let mut load = BusLoad::new();
        let e = frame(1, 0, false, FrameFlags::ERROR);
        load.note(&e, wire_time_us(&e, 500, 2000));
        assert_eq!(load.errors, 1);
        assert!(load.frame_rate() > 0.0, "error frames occupy the bus too");
    }

    #[test]
    fn load_is_wire_time_over_the_window() {
        let mut load = BusLoad::new();
        // 100 frames/s of 111 bits at 500 kbit/s: 100 x 222 µs = 22.2 ms of
        // wire time in the second, i.e. 2.22 % load. Frames start at 10 ms so
        // none sits on the window's closed lower edge.
        for i in 0..100u64 {
            let f = frame((i + 1) * 10_000, 8, false, FrameFlags::NONE);
            load.note(&f, wire_time_us(&f, 500, 2000));
        }
        assert!((load.frame_rate() - 100.0).abs() < 1e-9);
        assert!((load.load() - 0.0222).abs() < 1e-9);
    }

    #[test]
    fn the_window_drains_as_time_moves_on() {
        let mut load = BusLoad::new();
        for i in 0..100u64 {
            let f = frame((i + 1) * 10_000, 8, false, FrameFlags::NONE);
            load.note(&f, wire_time_us(&f, 500, 2000));
        }
        // A later frame is what advances the window; the burst a second back
        // falls out entirely, leaving only the advancing frame itself.
        let f = frame(1_000_000 + WINDOW_US, 0, false, FrameFlags::NONE);
        load.note(&f, 0.0);
        assert!(load.load() < 1e-9, "a burst a second ago is not load now");
        assert!((load.frame_rate() - 1.0).abs() < 1e-9);
    }

    #[test]
    fn a_backwards_timestamp_is_ignored_not_merged() {
        let mut load = BusLoad::new();
        let f = frame(1_000_000, 8, false, FrameFlags::NONE);
        load.note(&f, 222.0);
        // A replay seek rewinds to before what we saw; folding those frames
        // in would double-count traffic the window already measured.
        let old = frame(500_000, 8, false, FrameFlags::NONE);
        load.note(&old, 222.0);
        assert!(
            (load.load() - 0.000222).abs() < 1e-9,
            "only the first frame counts"
        );
    }

    #[test]
    fn the_sparkline_buckets_at_100ms() {
        let mut load = BusLoad::new();
        for i in 0..10u64 {
            let f = frame(i * 100_000, 8, false, FrameFlags::NONE);
            load.note(&f, wire_time_us(&f, 500, 2000));
        }
        let buckets: Vec<(u64, f64)> = load.history().collect();
        assert_eq!(buckets.len(), 10, "one bucket per 100 ms");
        for (start, v) in &buckets {
            assert_eq!(start % BUCKET_US, 0);
            // One 222 µs frame in a 100 ms bucket is 0.222 % load.
            assert!((v - 0.00222).abs() < 1e-9, "each bucket is a load fraction");
        }
    }

    #[test]
    fn clear_forgets_everything() {
        let mut load = BusLoad::new();
        let f = frame(0, 8, false, FrameFlags::ERROR);
        load.note(&f, wire_time_us(&f, 500, 2000));
        load.clear();
        assert_eq!(load.errors, 0);
        assert_eq!(load.load(), 0.0);
        assert_eq!(load.history().count(), 0);
    }
}
