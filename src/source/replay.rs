use crate::can::frame::CanFrame;
use crate::source::FrameSource;

pub struct ReplaySource {
    frames: Vec<CanFrame>,
    idx: usize,
    /// Wall-clock time of the last poll; gaps (pauses) are skipped via
    /// `shift_time` so they never advance the virtual clock.
    last: Option<u64>,
    /// Accumulated virtual (log) time in microseconds.
    pos_us: f64,
    speed: f64,
}

impl ReplaySource {
    pub fn new(frames: Vec<CanFrame>) -> Self {
        ReplaySource {
            frames,
            idx: 0,
            last: None,
            pos_us: 0.0,
            speed: 1.0,
        }
    }

    fn duration_us(&self) -> Option<u64> {
        self.frames.last().map(|f| f.t_us)
    }
}

impl FrameSource for ReplaySource {
    fn poll(&mut self, now_us: u64, out: &mut Vec<CanFrame>) {
        let prev = self.last.unwrap_or(now_us);
        self.pos_us += now_us.saturating_sub(prev) as f64 * self.speed;
        self.last = Some(now_us);
        let target = self.pos_us as u64;
        while self.idx < self.frames.len() && self.frames[self.idx].t_us <= target {
            out.push(self.frames[self.idx]);
            self.idx += 1;
        }
    }

    fn is_done(&self) -> bool {
        self.idx >= self.frames.len() && !self.frames.is_empty()
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
        self.duration_us()
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
        let mut src = ReplaySource::new(vec![frame(0), frame(100_000), frame(200_000)]);
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
        let mut src = ReplaySource::new(vec![frame(0), frame(100_000), frame(200_000)]);
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
        let mut src = ReplaySource::new(vec![frame(0), frame(100_000), frame(200_000)]);
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
}
