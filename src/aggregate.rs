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
    /// Frames seen in each direction. The aggregate key does not split by
    /// direction (one row per (bus, id)), so a message both sent and
    /// received carries two counts.
    pub rx: u64,
    pub tx: u64,
    pub last_t_us: u64,
    pub cycle_us: f64,
    pub min_us: f64,
    pub max_us: f64,
    /// Mean absolute deviation of the intervals from the running mean
    /// cycle (an EMA, same smoothing as `cycle_us`).
    pub jitter_us: f64,
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

/// Per-slot FlexRay aggregate behind the Messages window's FR rows.
/// Cycle and jitter use the same EMA smoothing as [`MessageAgg`]; the
/// payload is the last frame the slot carried.
#[derive(Clone, Debug, Default)]
pub struct FrSlotAgg {
    pub slot: u16,
    /// Reception channel: 0 = A, 1 = B, 2 = unknown.
    pub ab: u8,
    /// The communication cycle the last frame arrived in: slots shared
    /// by several frames (cycle repetition) resolve their frame name
    /// against this, not against cycle 0.
    pub last_cycle: u8,
    pub count: u64,
    pub last_t_us: u64,
    pub cycle_us: f64,
    pub jitter_us: f64,
    pub payload: Vec<u8>,
}
