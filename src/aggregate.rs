//! Per-(bus, id) frame aggregation: the running tally behind the Messages
//! and Statistics views.

use crate::can::frame::{Direction, FrameFlags, MAX_CAN_FD_LEN};

/// Per-(bus, id) aggregate. Copied freely so window snapshots never alias the
/// live tallies.
#[derive(Clone, Copy, Debug)]
pub struct MessageAgg {
    pub id: u32,
    pub extended: bool,
    pub channel: u8,
    pub dir: Direction,
    pub count: u64,
    pub last_t_us: u64,
    pub cycle_us: f64,
    pub min_us: f64,
    pub max_us: f64,
    pub len: u8,
    pub data: [u8; MAX_CAN_FD_LEN],
    pub flags: FrameFlags,
}

impl MessageAgg {
    /// The most recent frame's payload slice; empty for error / remote frames
    /// so callers can render it without a separate kind check.
    pub fn payload(&self) -> &[u8] {
        if self.flags.contains(FrameFlags::ERROR) || self.flags.contains(FrameFlags::RTR) {
            return &[];
        }
        &self.data[..self.len as usize]
    }
}
