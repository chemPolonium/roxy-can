use crate::can::frame::CanFrame;
use crate::source::FrameStream;
use crate::trace::FrRow;

/// In-memory stream used by tests and by the small-ASC path in
/// [`crate::log::open_stream`], where a full `Vec<CanFrame>` is cheaper
/// than an mmap + line cursor.
pub struct VecStream {
    frames: Vec<CanFrame>,
    idx: usize,
    /// FlexRay rows the container carried, in file order.
    fr_rows: Vec<FrRow>,
    fr_idx: usize,
}

impl VecStream {
    pub fn new(frames: Vec<CanFrame>) -> Self {
        VecStream {
            frames,
            idx: 0,
            fr_rows: Vec::new(),
            fr_idx: 0,
        }
    }

    /// The small-ASC path: CAN frames plus the FlexRay rows the same
    /// parse produced.
    pub fn with_fr(frames: Vec<CanFrame>, fr_rows: Vec<FrRow>) -> Self {
        VecStream {
            frames,
            idx: 0,
            fr_rows,
            fr_idx: 0,
        }
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

    fn peek_fr_t(&mut self) -> Option<u64> {
        self.fr_rows.get(self.fr_idx).map(|r| r.t_us)
    }

    fn poll_fr_rows(&mut self, upto_t_us: u64, out: &mut Vec<FrRow>) {
        while let Some(r) = self.fr_rows.get(self.fr_idx) {
            if r.t_us > upto_t_us {
                break;
            }
            out.push(self.fr_rows[self.fr_idx].clone());
            self.fr_idx += 1;
        }
    }

    fn duration_us(&self) -> Option<u64> {
        self.frames
            .last()
            .map(|f| f.t_us)
            .or_else(|| self.fr_rows.last().map(|r| r.t_us))
    }

    fn describe(&self) -> String {
        format!("{} frames", self.frames.len())
    }
}
