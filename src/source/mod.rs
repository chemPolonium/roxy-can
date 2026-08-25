pub mod replay;
pub mod virtual_source;

use crate::can::frame::CanFrame;

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
