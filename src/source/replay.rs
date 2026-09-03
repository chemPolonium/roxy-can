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
        self.next_t = self.stream.peek_t();
    }
}

impl FrameSource for ReplaySource {
    fn poll(&mut self, now_us: u64, out: &mut Vec<CanFrame>) {
        let prev = self.last.unwrap_or(now_us);
        self.pos_us += now_us.saturating_sub(prev) as f64 * self.speed;
        self.last = Some(now_us);
        let target = self.pos_us as u64;
        while out.len() < MAX_POLL_FRAMES {
            match self.stream.peek_t() {
                Some(t) if t <= target => match self.stream.next_frame() {
                    Some(f) => out.push(f),
                    None => {
                        self.done = true;
                        break;
                    }
                },
                Some(_) => break,
                None => {
                    self.done = true;
                    break;
                }
            }
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
        Some(self.pos_us as u64)
    }

    fn set_position_us(&mut self, us: u64) -> Option<u64> {
        let landed = self.stream.seek_to_us(us)?;
        // Adopt the frame we actually landed on rather than the requested
        // value, so scrubbing back and forth never accumulates a bias.
        self.pos_us = landed as f64;
        // `done` is otherwise latched forever, which would leave the source
        // silent after a jump back from the end.
        self.done = false;
        // Re-anchor the wall clock: the next poll sees `last == None`, adds
        // nothing, and playback continues from the landing point. Without
        // this the seek itself is credited as elapsed playback time.
        self.last = None;
        self.refresh_next();
        Some(landed)
    }

    fn scan_range(
        &mut self,
        from_us: u64,
        to_us: u64,
        max_frames: usize,
        out: &mut Vec<CanFrame>,
    ) -> bool {
        let playhead = self.pos_us as u64;
        self.stream.seek_to_us(from_us);
        let mut complete = true;
        while let Some(t) = self.stream.peek_t() {
            if t > to_us {
                break;
            }
            if out.len() >= max_frames {
                complete = false;
                break;
            }
            match self.stream.next_frame() {
                Some(f) => out.push(f),
                None => {
                    complete = false;
                    break;
                }
            }
        }
        // Put the cursor back where playback left it. `done` is cleared because
        // the scan may have looked at the tail; the next poll re-latches it from
        // the playhead. `last` goes to None so the scan's own wall time is not
        // credited to playback on the next poll.
        self.stream.seek_to_us(playhead);
        self.done = false;
        self.last = None;
        self.refresh_next();
        complete
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
    fn seek_forward_does_not_re_emit_discarded_frames() {
        let frames: Vec<CanFrame> = (0..10u64).map(|i| frame(i * 100_000)).collect();
        let mut src = ReplaySource::from_frames(frames);
        let mut out = Vec::new();
        src.poll(0, &mut out);
        src.poll(250_000, &mut out);
        assert_eq!(out.len(), 3, "t=0,100k,200k due by 250ms");
        out.clear();
        assert_eq!(src.set_position_us(700_000), Some(700_000));
        src.poll(250_000, &mut out);
        assert_eq!(
            out.iter().map(|f| f.t_us).collect::<Vec<_>>(),
            vec![700_000],
            "playhead continues from the landing frame, skipping 300k..600k"
        );
    }

    #[test]
    fn seek_backward_clears_done_and_resumes() {
        let mut src = ReplaySource::from_frames(vec![frame(0), frame(100_000)]);
        let mut out = Vec::new();
        src.poll(0, &mut out);
        src.poll(1_000_000, &mut out);
        assert!(src.is_done(), "stream exhausted");
        out.clear();
        assert_eq!(src.set_position_us(0), Some(0));
        assert!(!src.is_done(), "a jump back from the end must unlatch done");
        src.poll(1_000_000, &mut out);
        assert_eq!(out.len(), 1, "first poll only re-anchors the clock");
        src.poll(2_000_000, &mut out);
        assert_eq!(out.len(), 2, "playhead advances over both frames again");
    }

    #[test]
    fn seek_lands_on_a_real_frame_instead_of_the_request() {
        let mut src = ReplaySource::from_frames(vec![frame(0), frame(100_000), frame(200_000)]);
        // Nothing sits at 150 ms, so the clock must adopt 200 ms -- using the
        // request would leave the playhead between frames and drift.
        assert_eq!(src.set_position_us(150_000), Some(200_000));
        assert_eq!(src.position(), Some(200_000));
    }

    #[test]
    fn seek_does_not_credit_its_own_cost_as_playback_time() {
        let mut src = ReplaySource::from_frames(vec![frame(0), frame(500_000)]);
        let mut out = Vec::new();
        src.poll(10_000, &mut out);
        src.poll(20_000, &mut out);
        assert_eq!(src.set_position_us(500_000), Some(500_000));
        // The next poll re-anchors instead of adding the 10 ms gap on top.
        src.poll(30_000, &mut out);
        assert_eq!(src.position(), Some(500_000), "no phantom advance");
    }

    #[test]
    fn a_failed_seek_leaves_the_source_alone() {
        let mut src = ReplaySource::from_frames(vec![frame(0), frame(100_000)]);
        let mut out = Vec::new();
        src.poll(0, &mut out);
        src.poll(40_000, &mut out);
        let before = src.position();
        assert_eq!(src.set_position_us(9_000_000), None, "past the end");
        assert_eq!(src.position(), before, "a rejected seek is a no-op");
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
        // A seek back relights the schedule.
        assert_eq!(src.set_position_us(0), Some(0));
        assert!(src.next_deadline(2_000_000).is_some());
    }
}
