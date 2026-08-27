use crate::can::frame::CanFrame;
use crate::source::FrameStream;

/// In-memory stream used by tests and by the small-ASC path in
/// [`crate::log::open_stream`], where a full `Vec<CanFrame>` is cheaper
/// than an mmap + line cursor.
pub struct VecStream {
    frames: Vec<CanFrame>,
    idx: usize,
}

impl VecStream {
    pub fn new(frames: Vec<CanFrame>) -> Self {
        VecStream { frames, idx: 0 }
    }
}

impl FrameStream for VecStream {
    fn peek_t(&mut self) -> Option<u64> {
        self.frames.get(self.idx).map(|f| f.t_us)
    }

    fn next_frame(&mut self) -> Option<CanFrame> {
        let f = *self.frames.get(self.idx)?;
        self.idx += 1;
        Some(f)
    }

    fn duration_us(&self) -> Option<u64> {
        self.frames.last().map(|f| f.t_us)
    }

    fn describe(&self) -> String {
        format!("{} frames", self.frames.len())
    }
}
