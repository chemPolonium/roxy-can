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
}
