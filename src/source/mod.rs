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

    /// Repositions the cursor so the next unread frame is the first one with
    /// `t_us >= target`. Returns the timestamp actually landed on (>= target),
    /// which lets the caller sync its clock to a real frame instead of the
    /// requested value and avoid accumulating drift. `None` means `target` is
    /// past the end and the cursor now sits at EOF.
    ///
    /// Deliberately has no default impl: positioning differs per container
    /// format, and a wrong inherited default would silently mis-date playback.
    fn seek_to_us(&mut self, target: u64) -> Option<u64>;

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

    /// Jumps the source's clock to `us` and returns the timestamp actually
    /// landed on. The no-op default is how a source without a timeline tells
    /// the UI it cannot be scrubbed.
    fn set_position_us(&mut self, _us: u64) -> Option<u64> {
        None
    }

    /// Collects every frame inside `[from_us, to_us]` into `out` **without**
    /// moving the playhead, and reports whether the span was read completely
    /// (a capped scan returns false). This is what lets a plot show a window the
    /// playback cursor has not walked into yet. Sources with no file to re-read
    /// do nothing and report true, meaning "there is no pending work".
    fn scan_range(
        &mut self,
        _from_us: u64,
        _to_us: u64,
        _max_frames: usize,
        _out: &mut Vec<CanFrame>,
    ) -> bool {
        true
    }

    /// Total length of the source's timeline, in microseconds.
    fn duration(&self) -> Option<u64> {
        None
    }
}
