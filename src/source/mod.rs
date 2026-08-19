pub mod replay;
pub mod virtual_source;

use crate::can::frame::CanFrame;

pub trait FrameSource {
    fn poll(&mut self, now_us: u64, out: &mut Vec<CanFrame>);

    fn is_done(&self) -> bool {
        false
    }
}
