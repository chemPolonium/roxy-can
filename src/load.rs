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
            (arb_bits as f64, 8.0 * f.len as f64 + FD_DATA_PHASE_EXTRA)
        } else {
            ((arb_bits + 8 * f.len as u64) as f64, 0.0)
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

/// The traffic class behind the CAN statistics rows: standard/extended ×
/// data/remote, plus error frames, which are their own row and never enter
/// the identifier classes.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum FrameClass {
    StdData = 0,
    ExtData = 1,
    StdRemote = 2,
    ExtRemote = 3,
    Error = 4,
}

const CLASSES: usize = 5;

fn classify(f: &CanFrame) -> usize {
    if f.is_error() {
        FrameClass::Error as usize
    } else if f.flags.contains(FrameFlags::RTR) {
        if f.extended {
            FrameClass::ExtRemote as usize
        } else {
            FrameClass::StdRemote as usize
        }
    } else if f.extended {
        FrameClass::ExtData as usize
    } else {
        FrameClass::StdData as usize
    }
}

/// Frames with at most this much bus quiet between them count as one burst,
/// the way the CAN statistics window groups back-to-back traffic. A fixed
/// 1 ms read: nothing on a simulated bus argues for a configurable knob yet.
pub const BURST_GAP_US: u64 = 1_000;

/// One bus's rolling traffic window and its run-long statistics: load and
/// frame rate over the last [`WINDOW_US`], a 100 ms-bucketed history, the
/// CAN statistics rows (per-class rates and totals, send distance, bursts),
/// and error frames. Updated from the same frame stream the aggregates
/// read, so a frame never reaches one and not the other.
pub struct BusLoad {
    recent: VecDeque<(u64, f64, usize)>,
    window_wire_us: f64,
    /// (bucket start t_us, wire time within the bucket) at
    /// [`BUCKET_US`] resolution, newest last, [`HISTORY_BUCKETS`] kept.
    buckets: VecDeque<(u64, f64)>,
    pub errors: u64,
    /// Newest timestamp seen, so a paused bus drains its window forward in
    /// time instead of keeping stale frames alive forever.
    newest_t_us: u64,
    /// Frames currently inside the rolling window, per class; pruned frames
    /// leave their count.
    window_counts: [u64; CLASSES],
    totals: [u64; CLASSES],
    /// Per-step samples of the windowed numbers, so Min/Max/Avg columns
    /// have something run-long to show.
    load_min: Option<f64>,
    load_max: Option<f64>,
    load_sum: f64,
    load_n: u64,
    rates_min: [Option<f64>; CLASSES],
    rates_max: [Option<f64>; CLASSES],
    rates_sum: [f64; CLASSES],
    rates_n: u64,
    /// Start-to-start distance between consecutive frames on the bus.
    last_frame_t: Option<u64>,
    last_gap_us: Option<u64>,
    min_gap_us: Option<u64>,
    max_gap_us: Option<u64>,
    gap_sum: f64,
    gap_n: u64,
    /// Bursts: frames arriving within [`BURST_GAP_US`] of each other.
    bursts_total: u64,
    burst_open: bool,
    burst_frames: u64,
    burst_start_us: u64,
    last_burst_frames: u64,
    last_burst_time_us: u64,
    btime_min: Option<u64>,
    btime_max: Option<u64>,
    btime_sum: f64,
    btime_n: u64,
    fpb_min: Option<u64>,
    fpb_max: Option<u64>,
    fpb_sum: f64,
    fpb_n: u64,
}

/// Load and frame rate are measured over this much wall time.
pub const WINDOW_US: u64 = 1_000_000;
/// Sparkline resolution.
pub const BUCKET_US: u64 = 100_000;
/// How many buckets the history keeps: 60 x 100 ms = the last minute.
pub const HISTORY_BUCKETS: usize = 60;

impl BusLoad {
    pub fn new() -> Self {
        BusLoad {
            recent: VecDeque::new(),
            window_wire_us: 0.0,
            buckets: VecDeque::new(),
            errors: 0,
            newest_t_us: 0,
            window_counts: [0; CLASSES],
            totals: [0; CLASSES],
            load_min: None,
            load_max: None,
            load_sum: 0.0,
            load_n: 0,
            rates_min: [None; CLASSES],
            rates_max: [None; CLASSES],
            rates_sum: [0.0; CLASSES],
            rates_n: 0,
            last_frame_t: None,
            last_gap_us: None,
            min_gap_us: None,
            max_gap_us: None,
            gap_sum: 0.0,
            gap_n: 0,
            bursts_total: 0,
            burst_open: false,
            burst_frames: 0,
            burst_start_us: 0,
            last_burst_frames: 0,
            last_burst_time_us: 0,
            btime_min: None,
            btime_max: None,
            btime_sum: 0.0,
            btime_n: 0,
            fpb_min: None,
            fpb_max: None,
            fpb_sum: 0.0,
            fpb_n: 0,
        }
    }

    pub fn note(&mut self, f: &CanFrame, wire_us: f64) {
        let t = f.t_us;
        if f.is_error() {
            self.errors += 1;
        }
        // Frames out of order or before what we have seen (a seek backwards)
        // would poison the sliding window; the buckets and the burst and gap
        // tracking simply ignore them.
        if t < self.newest_t_us {
            return;
        }
        self.newest_t_us = t;

        let class = classify(f);
        self.totals[class] += 1;
        self.window_counts[class] += 1;
        self.recent.push_back((t, wire_us, class));
        self.window_wire_us += wire_us;
        self.prune_window(t);

        // Buckets feed the (undrawn) history.
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

        // Send distance: start-to-start gap against whichever frame went
        // before, regardless of identifier.
        let prev = self.last_frame_t;
        let gap = prev.map(|p| t - p);
        if let Some(g) = gap {
            self.last_gap_us = Some(g);
            self.min_gap_us = Some(self.min_gap_us.map_or(g, |m: u64| m.min(g)));
            self.max_gap_us = Some(self.max_gap_us.map_or(g, |m: u64| m.max(g)));
            self.gap_sum += g as f64;
            self.gap_n += 1;
        }
        self.last_frame_t = Some(t);

        // Bursts: close the open one when the bus went quiet longer than
        // the classification gap, then start a fresh one with this frame.
        // The closing burst's duration ends at its own last frame --
        // `prev` -- because by now `last_frame_t` already names the new
        // arrival.
        match gap {
            Some(g) if g <= BURST_GAP_US && self.burst_open => {
                self.burst_frames += 1;
            }
            _ => {
                self.close_burst(prev);
                self.burst_open = true;
                self.burst_frames = 1;
                self.burst_start_us = t;
            }
        }
    }

    /// Folds the open burst into the statistics; a no-op when nothing is
    /// open. `last_t` is the burst's own last frame, which ended before
    /// the gap that closed it.
    fn close_burst(&mut self, last_t: Option<u64>) {
        if !self.burst_open {
            return;
        }
        self.bursts_total += 1;
        let time = last_t.map_or(0, |l| l.saturating_sub(self.burst_start_us));
        self.last_burst_time_us = time;
        self.last_burst_frames = self.burst_frames;
        self.btime_min = Some(self.btime_min.map_or(time, |m: u64| m.min(time)));
        self.btime_max = Some(self.btime_max.map_or(time, |m: u64| m.max(time)));
        self.btime_sum += time as f64;
        self.btime_n += 1;
        self.fpb_min = Some(
            self.fpb_min
                .map_or(self.burst_frames, |m: u64| m.min(self.burst_frames)),
        );
        self.fpb_max = Some(
            self.fpb_max
                .map_or(self.burst_frames, |m: u64| m.max(self.burst_frames)),
        );
        self.fpb_sum += self.burst_frames as f64;
        self.fpb_n += 1;
        self.burst_open = false;
    }

    /// Takes one sample of the windowed numbers for the Min/Max/Avg
    /// columns. Called once per measurement step: between frames the
    /// windowed read holds still (documented freeze), and the samples
    /// should say that rather than skip it.
    pub fn sample(&mut self) {
        let l = self.load();
        self.load_min = Some(self.load_min.map_or(l, |m: f64| m.min(l)));
        self.load_max = Some(self.load_max.map_or(l, |m: f64| m.max(l)));
        self.load_sum += l;
        self.load_n += 1;
        for i in 0..CLASSES {
            let r = self.window_counts[i] as f64;
            self.rates_min[i] = Some(self.rates_min[i].map_or(r, |m: f64| m.min(r)));
            self.rates_max[i] = Some(self.rates_max[i].map_or(r, |m: f64| m.max(r)));
            self.rates_sum[i] += r;
        }
        self.rates_n += 1;
    }

    /// Drops frames that left the window and reports the load they carried.
    fn prune_window(&mut self, now: u64) {
        let horizon = now.saturating_sub(WINDOW_US);
        while let Some(&(t, w, class)) = self.recent.front() {
            if t > horizon {
                break;
            }
            self.recent.pop_front();
            self.window_wire_us -= w;
            self.window_counts[class] -= 1;
        }
    }

    /// Load as a fraction of the bus capacity over the last second, 0..=1+
    /// (a bus can be oversubscribed on paper).
    pub fn load(&self) -> f64 {
        self.window_wire_us / WINDOW_US as f64
    }

    /// Frames per second over the same window, all classes together. The
    /// statistics window splits by class now; the total stays for tests
    /// and future views.
    #[allow(dead_code)]
    pub fn frame_rate(&self) -> f64 {
        self.recent.len() as f64 / (WINDOW_US as f64 / 1_000_000.0)
    }

    /// The load history as (t_us, load fraction) pairs, one per 100 ms
    /// bucket. The sparkline that drew this was removed by decree --
    /// load reads as a plain percentage in the Bus Statistics window --
    /// but the collection stays: tests pin the bucketing, and a future
    /// export path gets the history for free.
    #[allow(dead_code)]
    pub fn history(&self) -> impl Iterator<Item = (u64, f64)> + '_ {
        self.buckets
            .iter()
            .map(|&(start, wire)| (start, wire / BUCKET_US as f64))
    }

    /// Frames of this class seen since start.
    pub fn class_total(&self, c: FrameClass) -> u64 {
        self.totals[c as usize]
    }

    /// Windowed frames-per-second of this class (the window is 1 s).
    pub fn class_rate(&self, c: FrameClass) -> f64 {
        self.window_counts[c as usize] as f64
    }

    /// (min, max, avg) of the class's sampled per-second rate since start.
    pub fn rate_stats(&self, c: FrameClass) -> (Option<f64>, Option<f64>, Option<f64>) {
        let i = c as usize;
        let avg = if self.rates_n > 0 {
            Some(self.rates_sum[i] / self.rates_n as f64)
        } else {
            None
        };
        (self.rates_min[i], self.rates_max[i], avg)
    }

    /// (min, max, avg) of the sampled load since start.
    pub fn load_stats(&self) -> (Option<f64>, Option<f64>, Option<f64>) {
        let avg = if self.load_n > 0 {
            Some(self.load_sum / self.load_n as f64)
        } else {
            None
        };
        (self.load_min, self.load_max, avg)
    }

    /// (last, min, max, avg) start-to-start frame distance in µs.
    pub fn send_dist_us(&self) -> (Option<u64>, Option<u64>, Option<u64>, Option<f64>) {
        let avg = if self.gap_n > 0 {
            Some(self.gap_sum / self.gap_n as f64)
        } else {
            None
        };
        (self.last_gap_us, self.min_gap_us, self.max_gap_us, avg)
    }

    pub fn bursts_total(&self) -> u64 {
        self.bursts_total
    }

    /// (last, min, max, avg) closed-burst duration in µs.
    pub fn burst_time_us(&self) -> (u64, Option<u64>, Option<u64>, Option<f64>) {
        let avg = if self.btime_n > 0 {
            Some(self.btime_sum / self.btime_n as f64)
        } else {
            None
        };
        (self.last_burst_time_us, self.btime_min, self.btime_max, avg)
    }

    /// (current, min, max, avg) frames per burst; current is the open
    /// burst's size, or the last closed one's.
    pub fn frames_per_burst(&self) -> (u64, Option<u64>, Option<u64>, Option<f64>) {
        let avg = if self.fpb_n > 0 {
            Some(self.fpb_sum / self.fpb_n as f64)
        } else {
            None
        };
        let cur = if self.burst_open {
            self.burst_frames
        } else {
            self.last_burst_frames
        };
        (cur, self.fpb_min, self.fpb_max, avg)
    }

    pub fn clear(&mut self) {
        self.recent.clear();
        self.buckets.clear();
        self.window_wire_us = 0.0;
        self.errors = 0;
        self.newest_t_us = 0;
        self.window_counts = [0; CLASSES];
        self.totals = [0; CLASSES];
        self.load_min = None;
        self.load_max = None;
        self.load_sum = 0.0;
        self.load_n = 0;
        self.rates_min = [None; CLASSES];
        self.rates_max = [None; CLASSES];
        self.rates_sum = [0.0; CLASSES];
        self.rates_n = 0;
        self.rates_n = 0;
        self.last_frame_t = None;
        self.last_gap_us = None;
        self.min_gap_us = None;
        self.max_gap_us = None;
        self.gap_sum = 0.0;
        self.gap_n = 0;
        self.bursts_total = 0;
        self.burst_open = false;
        self.burst_frames = 0;
        self.burst_start_us = 0;
        self.last_burst_frames = 0;
        self.last_burst_time_us = 0;
        self.btime_min = None;
        self.btime_max = None;
        self.btime_sum = 0.0;
        self.btime_n = 0;
        self.fpb_min = None;
        self.fpb_max = None;
        self.fpb_sum = 0.0;
        self.fpb_n = 0;
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
        assert!(
            (wire_time_us(&ext, 500, 2000) - wire_time_us(&std, 500, 2000) - 40.0).abs() < 1e-9,
            "the 20 extra identifier bits are 40 µs at 500 kbit/s"
        );
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
        assert!(
            (wire_time_us(&f, 500, 8000) - 222.0).abs() < 1e-9,
            "BRS without FD is nonsense; the arbitration rate decides"
        );
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
        assert_eq!(load.bursts_total(), 0);
        assert_eq!(load.class_total(FrameClass::StdData), 0);
    }

    /// The statistics rows split traffic standard/extended × data/remote;
    /// totals count forever, rates follow the rolling window.
    #[test]
    fn the_statistics_rows_split_frames_by_class() {
        let mut load = BusLoad::new();
        // Off the window's closed lower edge (t=0 prunes itself), like the
        // load test below.
        load.note(&frame(10_000, 8, false, FrameFlags::NONE), 222.0);
        load.note(&frame(10_010, 8, true, FrameFlags::NONE), 222.0);
        load.note(&frame(10_020, 0, false, FrameFlags::RTR), 100.0);
        load.note(&frame(10_030, 0, true, FrameFlags::RTR), 100.0);
        load.note(&frame(10_040, 0, false, FrameFlags::ERROR), 90.0);
        assert_eq!(load.class_total(FrameClass::StdData), 1);
        assert_eq!(load.class_total(FrameClass::ExtData), 1);
        assert_eq!(load.class_total(FrameClass::StdRemote), 1);
        assert_eq!(load.class_total(FrameClass::ExtRemote), 1);
        assert_eq!(load.class_total(FrameClass::Error), 1);
        // Everything is inside the 1 s window, so the rates read the same.
        assert_eq!(load.class_rate(FrameClass::StdData), 1.0);
        // A frame two seconds later pushes every earlier one out of the
        // window: the totals stay, the rates fall to the new frame only.
        load.note(&frame(2_000_000, 8, false, FrameFlags::NONE), 222.0);
        assert_eq!(
            load.class_total(FrameClass::StdData),
            2,
            "totals count forever"
        );
        assert_eq!(
            load.class_rate(FrameClass::StdData),
            1.0,
            "only the new frame is in the window"
        );
        assert_eq!(
            load.class_rate(FrameClass::ExtData),
            0.0,
            "pruned frames leave their count"
        );
    }

    /// Send distance is start-to-start over all frames regardless of
    /// identifier: gaps of 200 µs and 1300 µs give last/min/max/avg.
    #[test]
    fn send_distance_tracks_the_gaps_between_frames() {
        let mut load = BusLoad::new();
        load.note(&frame(0, 8, false, FrameFlags::NONE), 222.0);
        let (last, _min, _max, _avg) = load.send_dist_us();
        assert_eq!(last, None, "the first frame has no predecessor");
        load.note(&frame(200, 8, false, FrameFlags::NONE), 222.0);
        load.note(&frame(1_500, 8, true, FrameFlags::NONE), 222.0);
        let (last, min, max, avg) = load.send_dist_us();
        assert_eq!(last, Some(1_300));
        assert_eq!(min, Some(200));
        assert_eq!(max, Some(1_300));
        assert!((avg.unwrap() - 750.0).abs() < 1e-9);
    }

    /// Frames within the classification gap are one burst; a longer quiet
    /// stretch closes it and the statistics fold in, so re-opening later
    /// starts a fresh burst.
    #[test]
    fn a_burst_closes_when_the_bus_goes_quiet() {
        let mut load = BusLoad::new();
        load.note(&frame(0, 8, false, FrameFlags::NONE), 222.0);
        load.note(&frame(200, 8, false, FrameFlags::NONE), 222.0);
        assert_eq!(load.bursts_total(), 0, "still open");
        load.note(&frame(5_200, 8, false, FrameFlags::NONE), 222.0);
        assert_eq!(
            load.bursts_total(),
            1,
            "the 5 ms gap closed the first burst"
        );
        let (bt_last, bt_min, bt_max, bt_avg) = load.burst_time_us();
        assert_eq!(bt_last, 200, "burst time spans first to last frame");
        assert_eq!(bt_min, Some(200));
        assert_eq!(bt_max, Some(200));
        assert!((bt_avg.unwrap() - 200.0).abs() < 1e-9);
        let (cur, f_min, f_max, f_avg) = load.frames_per_burst();
        assert_eq!(cur, 1, "the new burst is open with one frame");
        assert_eq!(f_min, Some(2));
        assert_eq!(f_max, Some(2));
        assert!((f_avg.unwrap() - 2.0).abs() < 1e-9);
    }

    /// Min/Max/Avg come from per-step samples of the windowed numbers: a
    /// rate of 2 then a rate of 1 reads min 1, max 2, avg 1.5.
    #[test]
    fn sampling_feeds_the_min_max_avg_columns() {
        let mut load = BusLoad::new();
        load.note(&frame(10_000, 8, false, FrameFlags::NONE), 222.0);
        load.note(&frame(100_000, 8, false, FrameFlags::NONE), 222.0);
        load.sample();
        load.note(&frame(1_100_000, 8, false, FrameFlags::NONE), 222.0);
        load.sample();
        let (min, max, avg) = load.rate_stats(FrameClass::StdData);
        assert_eq!(min, Some(1.0));
        assert_eq!(max, Some(2.0));
        assert!((avg.unwrap() - 1.5).abs() < 1e-9);
        let (l_min, l_max, _) = load.load_stats();
        assert!(
            l_min.unwrap() < l_max.unwrap(),
            "load samples track the window too"
        );
    }
}
