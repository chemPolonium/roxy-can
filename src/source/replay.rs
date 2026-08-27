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
}

impl ReplaySource {
    pub fn new(stream: Box<dyn FrameStream>) -> Self {
        ReplaySource {
            stream,
            last: None,
            pos_us: 0.0,
            speed: 1.0,
            done: false,
        }
    }

    /// Convenience constructor for tests and small in-memory captures.
    #[allow(dead_code)]
    pub fn from_frames(frames: Vec<CanFrame>) -> Self {
        Self::new(Box::new(VecStream::new(frames)))
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

    fn duration(&self) -> Option<u64> {
        self.stream.duration_us()
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
}
