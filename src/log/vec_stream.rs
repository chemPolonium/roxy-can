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

    fn seek_to_us(&mut self, target: u64) -> Option<u64> {
        // Log timestamps ascend, so the first frame with t >= target is a
        // plain lower bound over the whole buffer -- no checkpoint needed.
        let hit = self.frames.partition_point(|f| f.t_us < target);
        self.idx = hit;
        let fr_hit = self.fr_rows.partition_point(|r| r.t_us < target);
        self.fr_idx = fr_hit;
        // Land on the earliest row of either kind. A FlexRay-only stream has
        // no CAN frames, so returning only the CAN hit would report EOF and
        // leave the caller unable to unlatch `done` after a finished replay.
        let can = self.frames.get(hit).map(|f| f.t_us);
        let fr = self.fr_rows.get(fr_hit).map(|r| r.t_us);
        can.into_iter().chain(fr).min()
    }

    fn duration_us(&self) -> Option<u64> {
        self.frames
            .last()
            .map(|f| f.t_us)
            .or_else(|| self.fr_rows.last().map(|r| r.t_us))
    }

    fn has_can_frames(&self) -> bool {
        !self.frames.is_empty()
    }

    fn describe(&self) -> String {
        format!("{} frames", self.frames.len())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::can::frame::{Direction, FrameFlags, MAX_CAN_FD_LEN};

    fn frame(t_us: u64) -> CanFrame {
        CanFrame {
            t_us,
            channel: 0,
            id: 0x100,
            extended: false,
            len: 0,
            data: [0u8; MAX_CAN_FD_LEN],
            dir: Direction::Rx,
            flags: FrameFlags::NONE,
        }
    }

    fn stream(times: &[u64]) -> VecStream {
        VecStream::new(times.iter().copied().map(frame).collect())
    }

    #[test]
    fn seek_lands_on_the_first_frame_at_or_after_target() {
        let mut s = stream(&[10, 20, 30]);
        assert_eq!(s.seek_to_us(20), Some(20));
        assert_eq!(s.next_frame().map(|f| f.t_us), Some(20));
        assert_eq!(s.seek_to_us(21), Some(30));
        assert_eq!(s.peek_t(), Some(30));
    }

    #[test]
    fn seek_to_zero_rewinds() {
        let mut s = stream(&[10, 20, 30]);
        assert_eq!(s.next_frame().map(|f| f.t_us), Some(10));
        assert_eq!(s.next_frame().map(|f| f.t_us), Some(20));
        assert_eq!(s.seek_to_us(0), Some(10));
        assert_eq!(s.peek_t(), Some(10));
    }

    #[test]
    fn seek_resolves_duplicates_to_the_first_match() {
        let mut s = stream(&[5, 5, 5, 9]);
        assert_eq!(s.seek_to_us(5), Some(5));
        assert_eq!(s.idx, 0, "must not skip the repeated frames");
    }

    #[test]
    fn seek_past_the_end_lands_at_eof() {
        let mut s = stream(&[10, 20]);
        assert_eq!(s.seek_to_us(21), None);
        assert_eq!(s.peek_t(), None);
        assert!(s.next_frame().is_none());
    }

    #[test]
    fn seek_on_an_empty_stream_reports_eof() {
        let mut s = stream(&[]);
        assert_eq!(s.seek_to_us(0), None);
        assert_eq!(s.peek_t(), None);
    }

    #[test]
    fn seek_lands_on_a_flexray_row_when_there_is_no_can() {
        let fr = |t_us| FrRow {
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
        let mut s = VecStream::with_fr(Vec::new(), vec![fr(10), fr(20)]);
        assert_eq!(s.seek_to_us(15), Some(20), "no CAN frames: land on the FR row");
        s.seek_to_us(0);
        assert_eq!(s.peek_t(), None, "still no CAN frames to report");
        assert_eq!(s.peek_fr_t(), Some(10), "FR head restored to the start");
        assert_eq!(s.seek_to_us(25), None, "past the end of both kinds");
    }
}
