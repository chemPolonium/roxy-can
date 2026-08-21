use crate::can::frame::CanFrame;
use crate::source::FrameSource;

pub struct ReplaySource {
    frames: Vec<CanFrame>,
    idx: usize,
    start: Option<u64>,
}

impl ReplaySource {
    pub fn new(frames: Vec<CanFrame>) -> Self {
        ReplaySource {
            frames,
            idx: 0,
            start: None,
        }
    }
}

impl FrameSource for ReplaySource {
    fn poll(&mut self, now_us: u64, out: &mut Vec<CanFrame>) {
        let start = *self.start.get_or_insert(now_us);
        let target = now_us.saturating_sub(start);
        while self.idx < self.frames.len() && self.frames[self.idx].t_us <= target {
            out.push(self.frames[self.idx]);
            self.idx += 1;
        }
    }

    fn is_done(&self) -> bool {
        self.idx >= self.frames.len() && !self.frames.is_empty()
    }

    fn shift_time(&mut self, us: u64) {
        if let Some(s) = self.start.as_mut() {
            *s += us;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(t_us: u64) -> CanFrame {
        CanFrame {
            t_us,
            channel: 0,
            id: 0x100,
            extended: false,
            dlc: 8,
            data: [0; 8],
            dir: crate::can::frame::Direction::Rx,
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
}
