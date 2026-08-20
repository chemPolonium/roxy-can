use crate::can::frame::CanFrame;
use crate::source::FrameSource;

/// Idle virtual bus. It never produces traffic by itself; all frames come
/// from the interactive generator (or a replay source).
pub struct VirtualSource;

impl VirtualSource {
    pub fn new() -> Self {
        VirtualSource
    }
}

impl Default for VirtualSource {
    fn default() -> Self {
        Self::new()
    }
}

impl FrameSource for VirtualSource {
    fn poll(&mut self, _now_us: u64, _out: &mut Vec<CanFrame>) {}
}
