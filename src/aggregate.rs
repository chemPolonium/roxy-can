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

/// The L2 identity one Ethernet aggregate row counts: bus, VLAN, MAC pair,
/// EtherType and payload length. AUTOSAR adds no address of its own, so this
/// is the only key a capture offers.
#[derive(
    Clone,
    Copy,
    Debug,
    PartialEq,
    Eq,
    Hash,
    PartialOrd,
    Ord,
    Default,
    serde::Serialize,
    serde::Deserialize,
)]
#[serde(default)]
pub struct EthFlowKey {
    pub bus: u8,
    pub vlan: Option<u16>,
    pub dst: [u8; 6],
    pub src: [u8; 6],
    pub ethertype: u16,
    pub len: usize,
}

impl EthFlowKey {
    /// The flow one captured frame belongs to: its L2 identity and length.
    pub fn of_frame(f: &crate::eth::EthFrame) -> Self {
        Self {
            bus: f.bus,
            vlan: f.vlan,
            dst: f.dst,
            src: f.src,
            ethertype: f.ethertype,
            len: f.len(),
        }
    }

    /// The same identity in the matcher/decoder's flow shape.
    pub fn flow(&self) -> crate::eth::db::EthFlow {
        crate::eth::db::EthFlow {
            vlan: self.vlan,
            dst: self.dst,
            src: self.src,
            ethertype: self.ethertype,
            payload_len: self.len,
        }
    }

    /// Stable Messages row key: bus, EtherType, VLAN, both MACs and length.
    pub fn message_key(&self) -> String {
        let vlan = self
            .vlan
            .map(|vlan| vlan.to_string())
            .unwrap_or_else(|| "-".to_string());
        let mac = |address: &[u8; 6]| {
            address
                .iter()
                .map(|byte| format!("{byte:02X}"))
                .collect::<String>()
        };
        format!(
            "eth:{}:{}:{}:{}:{}:{}",
            self.bus,
            self.ethertype,
            vlan,
            mac(&self.dst),
            mac(&self.src),
            self.len
        )
    }
}

/// One Ethernet flow's running tally behind Messages/Statistics. Cycle and
/// jitter use the same EMA smoothing as the CAN/FlexRay aggregates.
#[derive(Clone, Debug, Default)]
pub struct EthAgg {
    pub key: EthFlowKey,
    pub count: u64,
    pub last_t_us: u64,
    pub cycle_us: f64,
    pub min_us: f64,
    pub max_us: f64,
    pub jitter_us: f64,
    pub payload: std::sync::Arc<[u8]>,
}

/// Which occupant of a slot one FlexRay aggregate row counts -- the identity
/// [`FrFrameAgg`] is keyed by. See its doc for why this is not the slot.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Default)]
pub enum FrOccupant {
    /// The cluster description resolved this slot's frame for the cycle that
    /// arrived: its index in that cluster's frame list, so the row decodes
    /// against the same frame it names.
    Frame(usize),
    /// Nothing resolved, but the log named the frame (CANoe's ASC does). Still
    /// a frame to track; without it these rows would merge by slot.
    Logged(Box<str>),
    /// Neither: no description for this bus and no name in the log. One row
    /// per slot collects whatever arrives there.
    #[default]
    Unknown,
}

impl FrOccupant {
    /// The description index to decode against, when there is one.
    pub fn frame_ix(&self) -> Option<usize> {
        match self {
            FrOccupant::Frame(i) => Some(*i),
            _ => None,
        }
    }
}

/// One FlexRay **frame**: the aggregate behind a Messages/Statistics FR row.
///
/// The unit is the frame, not the slot. A static slot is scheduled per cycle
/// *phase*, so one slot can belong to several frames in turn -- the bundled
/// ARXML holds six (`Frame_71_0_8` .. `Frame_71_5_8`, cycle repetition 8) in
/// slot 71. Aggregating `(bus, slot)` therefore mixed their traffic into one
/// row whose name changed with whichever frame arrived last, and whose
/// observed period was the slot's, not any frame's.
///
/// Cycle and jitter use the same EMA smoothing as [`MessageAgg`]; the payload
/// is the last frame this one carried.
#[derive(Clone, Debug, Default)]
pub struct FrFrameAgg {
    /// The bus this frame belongs to -- slot numbers repeat across clusters.
    pub bus: u8,
    pub slot: u16,
    /// What of this slot the row counts.
    pub occupant: FrOccupant,
    /// The frame's name: the description's for a resolved occupant, else the
    /// name the log carried. `None` = unnamed, which is its own row.
    pub name: Option<String>,
    /// Reception channel of the last arrival: 0 = A, 1 = B, 2 = unknown.
    pub ab: u8,
    pub count: u64,
    pub last_t_us: u64,
    pub cycle_us: f64,
    /// Extremes of the observed inter-arrival intervals, as on the CAN side --
    /// the smoothed `cycle_us` alone cannot show a frame that is mostly steady
    /// with the occasional late cycle. Zero means no interval yet (CAN keeps
    /// its floor at `f64::MAX` instead; a FlexRay row starts at zero because
    /// `or_default` builds it, and the views only read these from `count >= 2`).
    pub min_us: f64,
    pub max_us: f64,
    pub jitter_us: f64,
    pub payload: Vec<u8>,
}
