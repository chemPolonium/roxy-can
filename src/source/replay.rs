use crate::can::frame::CanFrame;
use crate::log::vec_stream::VecStream;
use crate::source::{FrameSource, FrameStream};

/// Frames emitted by a single `poll` before we hand control back to the UI.
/// Bounds worst-case page-fault work when a high playback speed crosses a
/// dense stretch of the log; anything left over fires on the next poll.
const MAX_POLL_FRAMES: usize = 8_192;

pub struct ReplaySource {
    stream: Box<dyn FrameStream>,
    /// Wall-clock time of the last poll; gaps (pauses) are skipped via
    /// `shift_time` so they never advance the virtual clock.
    last: Option<u64>,
    /// Accumulated virtual (log) time in microseconds.
    pos_us: f64,
    speed: f64,
    /// Latched when the underlying stream reports EOF via `peek_t`. Held as
    /// a flag because `is_done(&self)` cannot call the `&mut self` peek.
    done: bool,
    /// The log time of the next undelivered frame, cached so the event
    /// loop's `next_deadline` can read it without `&mut` access to the
    /// stream. Refreshed everywhere the playhead moves.
    next_t: Option<u64>,
}

impl ReplaySource {
    pub fn new(stream: Box<dyn FrameStream>) -> Self {
        let mut src = ReplaySource {
            stream,
            last: None,
            pos_us: 0.0,
            speed: 1.0,
            done: false,
            next_t: None,
        };
        src.refresh_next();
        src
    }

    /// Convenience constructor for tests and small in-memory captures.
    #[allow(dead_code)]
    pub fn from_frames(frames: Vec<CanFrame>) -> Self {
        Self::new(Box::new(VecStream::new(frames)))
    }

    fn refresh_next(&mut self) {
        // The replay clock ends when both frame kinds run out; a pure
        // FlexRay log has no CAN timestamps at all.
        let can = self.stream.peek_t();
        let fr = self.stream.peek_fr_t();
        self.next_t = match (can, fr) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        };
    }
}

impl FrameSource for ReplaySource {
    fn poll(&mut self, now_us: u64, out: &mut Vec<CanFrame>) {
        // "As fast as possible" (infinite speed): the playhead jumps to
        // the end of the log at once. A plain `delta * inf` would turn
        // the first poll's zero delta into NaN, so the jump is explicit.
        if self.speed.is_infinite() {
            self.pos_us = f64::INFINITY;
        } else {
            let prev = self.last.unwrap_or(now_us);
            self.pos_us += now_us.saturating_sub(prev) as f64 * self.speed;
        }
        self.last = Some(now_us);
        let target = self.pos_us as u64;
        while out.len() < MAX_POLL_FRAMES {
            match self.stream.peek_t() {
                Some(t) if t <= target => match self.stream.next_frame() {
                    Some(f) => out.push(f),
                    // peek_t promised a frame; treat a miss as EOF but let
                    // the FlexRay side below decide `done`.
                    None => break,
                },
                Some(_) => break,
                None => break, // CAN EOF; FR rows may still be due
            }
        }
        // The done latch fires only when neither stream side has
        // anything left, so a pure FlexRay replay runs its full length.
        // The rows themselves drain through `poll_fr`.
        if self.stream.peek_t().is_none() && self.stream.peek_fr_t().is_none() {
            self.done = true;
        }
        self.refresh_next();
    }

    fn is_done(&self) -> bool {
        self.done
    }

    fn shift_time(&mut self, us: u64) {
        if let Some(l) = self.last.as_mut() {
            *l += us;
        }
    }

    fn set_speed(&mut self, s: f64) {
        self.speed = s.max(0.01);
    }

    fn position(&self) -> Option<u64> {
        let pos = self.pos_us as u64;
        if self.pos_us.is_finite() {
            return Some(pos);
        }
        // "As fast as possible" parks `pos_us` at infinity so the drain emits
        // every remaining frame in one lap; report the log's end instead of the
        // saturated `u64::MAX` that cast yields, so the plot windows -- which
        // read this as "now" -- stay on the data. The clamp belongs to the
        // infinite case only: a stream's duration estimate can lag the playhead
        // (`BlfStream` falls back to the last timestamp it has read when the
        // header states no length), and capping a finite playhead by that
        // estimate freezes the plot clock behind the traffic it should follow.
        match self.stream.duration_us() {
            Some(d) => Some(pos.min(d)),
            None => Some(pos),
        }
    }

    fn poll_fr(&mut self, _now_us: u64, out: &mut Vec<crate::trace::FrRow>) {
        // The playhead is the same one the CAN loop paced against, so
        // FR rows stream in log order interleaved with the CAN frames.
        let target = self.pos_us as u64;
        self.stream.poll_fr_rows(target, out);
    }

    fn duration(&self) -> Option<u64> {
        self.stream.duration_us()
    }

    fn next_deadline(&self, now_us: u64) -> Option<u64> {
        if self.done {
            return None;
        }
        let next_log = self.next_t?;
        // Log time accrues at `speed` wall microseconds per log microsecond:
        // the wall wait for the next frame is the remaining log span, scaled.
        let remaining = ((next_log as f64 - self.pos_us).max(0.0)) / self.speed;
        Some(self.last.unwrap_or(now_us) + remaining as u64)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::can::frame::{FrameFlags, MAX_CAN_FD_LEN};

    fn frame(t_us: u64) -> CanFrame {
        CanFrame {
            t_us,
            channel: 0,
            id: 0x100,
            extended: false,
            len: 8,
            data: [0; MAX_CAN_FD_LEN],
            dir: crate::can::frame::Direction::Rx,
            flags: FrameFlags::NONE,
        }
    }

    #[test]
    fn pause_shift_stops_the_replay_clock() {
        let mut src = ReplaySource::from_frames(vec![frame(0), frame(100_000), frame(200_000)]);
        let mut out = Vec::new();
        src.poll(1_000_000, &mut out);
        assert_eq!(out.len(), 1, "first frame at t=0");
        src.shift_time(500_000);
        out.clear();
        src.poll(1_500_000, &mut out);
        assert!(out.is_empty(), "paused time must not emit frames");
        src.poll(1_600_000, &mut out);
        assert_eq!(out.len(), 1, "resumes exactly where it stopped");
    }

    #[test]
    fn speed_scales_the_virtual_clock() {
        let mut src = ReplaySource::from_frames(vec![frame(0), frame(100_000), frame(200_000)]);
        src.set_speed(2.0);
        let mut out = Vec::new();
        src.poll(1_000_000, &mut out);
        assert_eq!(out.len(), 1, "first frame at t=0");
        out.clear();
        // 50 ms of wall time at 2x covers 100 ms of log time.
        src.poll(1_050_000, &mut out);
        assert_eq!(
            out.len(),
            1,
            "2x speed emits the 100ms frame twice as early"
        );
        out.clear();
        src.set_speed(0.5);
        src.poll(1_150_000, &mut out);
        // 100ms wall at 0.5x adds 50ms: pos=150ms, nothing new.
        assert!(out.is_empty(), "slowing down mid-replay is continuous");
        src.poll(1_250_000, &mut out);
        assert_eq!(out.len(), 1, "final frame once pos reaches 200ms");
    }

    #[test]
    fn exposes_position_and_duration() {
        let mut src = ReplaySource::from_frames(vec![frame(0), frame(100_000), frame(200_000)]);
        assert_eq!(src.duration(), Some(200_000));
        assert_eq!(src.position(), Some(0));
        src.poll(1_000_000, &mut Vec::new());
        src.poll(1_050_000, &mut Vec::new());
        assert_eq!(
            src.position(),
            Some(50_000),
            "position tracks the virtual clock"
        );
    }

    /// A length estimate that lags the traffic must not cap the playhead.
    /// `BlfStream` falls back to the last timestamp it has read whenever the
    /// file header states no duration -- capping a finite playhead by that
    /// estimate is how a recording's plot clock froze behind the traffic it
    /// should have been following.
    #[test]
    fn a_short_duration_estimate_never_caps_the_playhead() {
        struct Lagging;
        impl FrameStream for Lagging {
            fn peek_t(&mut self) -> Option<u64> {
                Some(10_000_000)
            }
            fn next_frame(&mut self) -> Option<CanFrame> {
                None
            }
            fn duration_us(&self) -> Option<u64> {
                Some(0)
            }
        }
        let mut src = ReplaySource::new(Box::new(Lagging));
        let mut out = Vec::new();
        // The first poll only anchors the clock; the second is the one that
        // advances three seconds of playback.
        src.poll(0, &mut out);
        src.poll(3_000_000, &mut out);
        assert_eq!(
            src.position(),
            Some(3_000_000),
            "three seconds of playback reports three seconds"
        );
    }

    #[test]
    fn as_fast_as_possible_reports_the_log_end() {
        // Infinite speed parks the playhead at infinity to drain the whole log
        // in one lap. position() must clamp to the last frame, not saturate to
        // u64::MAX -- which sent the plot windows' "now" ~1.8e13 s past data.
        let mut src = ReplaySource::from_frames(vec![frame(0), frame(100_000), frame(200_000)]);
        src.set_speed(f64::INFINITY);
        let mut out = Vec::new();
        src.poll(1_000_000, &mut out);
        assert_eq!(out.len(), 3, "one lap drains the whole log");
        assert_eq!(
            src.position(),
            Some(200_000),
            "position clamps to the log end, not u64::MAX"
        );
    }

    #[test]
    fn is_done_latches_after_stream_eof() {
        let mut src = ReplaySource::from_frames(vec![frame(0), frame(10)]);
        assert!(!src.is_done(), "not done before the first poll");
        let mut out = Vec::new();
        src.poll(1_000_000, &mut out);
        src.poll(1_000_000 + 100_000, &mut out);
        assert!(src.is_done(), "polling past the last frame marks done");
        let before = out.len();
        src.poll(1_000_000 + 200_000, &mut out);
        assert_eq!(out.len(), before, "done source emits nothing more");
    }

    #[test]
    fn empty_stream_becomes_done_on_first_poll() {
        let mut src = ReplaySource::from_frames(Vec::new());
        let mut out = Vec::new();
        src.poll(1_000_000, &mut out);
        assert!(out.is_empty());
        assert!(src.is_done(), "a stream with no frames reports done");
    }

    #[test]
    fn poll_respects_max_frames_cap() {
        // Build a stream of MAX_POLL_FRAMES + 100 frames at t=0.. so every
        // frame is due in one poll; only the cap fires.
        let frames: Vec<CanFrame> = (0..(MAX_POLL_FRAMES + 100) as u64).map(frame).collect();
        let mut src = ReplaySource::from_frames(frames);
        let mut out = Vec::new();
        // Advance the virtual clock far enough to include every frame.
        src.poll(1_000_000, &mut out);
        src.poll(1_000_000 + 10_000_000, &mut out);
        assert_eq!(
            out.len(),
            MAX_POLL_FRAMES,
            "single poll caps at MAX_POLL_FRAMES"
        );
        assert!(!src.is_done(), "capped poll must not mark the stream done");
    }

    #[test]
    fn next_deadline_tracks_the_next_frame_in_log_time() {
        let mut src = ReplaySource::from_frames(vec![frame(0), frame(100_000), frame(200_000)]);
        // Never polled: the anchor is "now", and the t=0 frame is due now.
        assert_eq!(src.next_deadline(0), Some(0));
        let mut out = Vec::new();
        src.poll(1_000_000, &mut out);
        // The 100 ms frame is due 100 ms after the poll that re-anchored.
        assert_eq!(src.next_deadline(1_000_000), Some(1_100_000));
    }

    #[test]
    fn next_deadline_scales_with_speed() {
        let mut src = ReplaySource::from_frames(vec![frame(0), frame(100_000)]);
        src.set_speed(2.0);
        let mut out = Vec::new();
        src.poll(1_000_000, &mut out);
        // 100 ms of log time at 2x: half the wall wait.
        assert_eq!(src.next_deadline(1_000_000), Some(1_050_000));
    }

    #[test]
    fn next_deadline_is_none_after_eof() {
        let mut src = ReplaySource::from_frames(vec![frame(0)]);
        let mut out = Vec::new();
        src.poll(1_000_000, &mut out);
        src.poll(2_000_000, &mut out);
        assert_eq!(src.next_deadline(2_000_000), None);
    }

    #[test]
    fn a_flexray_only_replay_runs_its_full_length() {
        // A pure-FlexRay log carries no CAN frames, so the CAN side is at EOF
        // from the first poll. `done` must wait for the FlexRay rows to drain,
        // or a FlexRay replay stops after one frame.
        use crate::trace::FrRow;
        let fr = |t_us: u64| FrRow {
            bus: 0,
            t_us,
            ab: 0,
            slot: 1,
            cycle: 0,
            payload: Vec::new(),
            header_crc: 0,
            flags: 0,
            name: None,
        };
        let stream = VecStream::with_fr(Vec::new(), vec![fr(0), fr(100_000), fr(200_000)]);
        let mut src = ReplaySource::new(Box::new(stream));
        let mut fr_out = Vec::new();
        // Drive the clock to and past the last row: `poll` paces, `poll_fr`
        // drains, exactly as `step` pairs them each lap.
        src.poll(0, &mut Vec::new());
        src.poll_fr(0, &mut fr_out);
        src.poll(300_000, &mut Vec::new());
        src.poll_fr(300_000, &mut fr_out);
        assert_eq!(fr_out.len(), 3, "all three FlexRay rows emit over the run");
        src.poll(400_000, &mut Vec::new());
        assert!(src.is_done(), "an exhausted FlexRay stream reports done");
    }
}
