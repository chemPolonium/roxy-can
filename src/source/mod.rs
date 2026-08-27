pub mod replay;
pub mod virtual_source;

use crate::can::frame::CanFrame;

/// Sequential iterator over decoded log frames. Unlike `FrameSource` this
/// has no clock — `ReplaySource` owns the pacing and pulls one frame at a
/// time via `peek_t`/`next_frame`. Keeping the split lets big files stream
/// straight out of an mmap without ever materializing a `Vec<CanFrame>`.
pub trait FrameStream {
    /// Timestamp of the next frame without consuming it; `None` at EOF.
    /// Implementations may decompress a chunk to service this call.
    fn peek_t(&mut self) -> Option<u64>;

    /// Consumes the frame `peek_t` returned.
    fn next_frame(&mut self) -> Option<CanFrame>;

    /// Total length of the log timeline in microseconds, relative to the
    /// same zero point the frames use.
    fn duration_us(&self) -> Option<u64> {
        None
    }

    /// One-line summary for the status bar (e.g. `"BLF4, 41.2 s, 312 blocks"`).
    fn describe(&self) -> String {
        String::new()
    }
}

pub trait FrameSource {
    fn poll(&mut self, now_us: u64, out: &mut Vec<CanFrame>);

    fn is_done(&self) -> bool {
        false
    }

    /// Advances the source's internal clock by `us`, used to skip over
    /// time spent paused so playback resumes where it stopped.
    fn shift_time(&mut self, _us: u64) {}

    /// Playback speed multiplier (1.0 = real time); only sources with a
    /// replay clock honor this.
    fn set_speed(&mut self, _speed: f64) {}

    /// Current position on the source's clock, in microseconds.
    fn position(&self) -> Option<u64> {
        None
    }

    /// Total length of the source's timeline, in microseconds.
    fn duration(&self) -> Option<u64> {
        None
    }
}
