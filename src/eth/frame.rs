use crate::can::frame::Direction;
use std::sync::Arc;

/// One captured Ethernet frame, stripped of its L2 header.
///
/// `payload` excludes the 14-byte Ethernet II header (18 with a VLAN tag)
/// and the FCS; `len()` is the payload length the capture reported. The
/// `Arc<[u8]>` keeps published snapshots cheap to share.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EthFrame {
    pub t_us: u64,
    /// Which Ethernet bus (its own numbering space, never a CAN channel).
    pub bus: u8,
    pub dir: Direction,
    pub dst: [u8; 6],
    pub src: [u8; 6],
    pub vlan: Option<u16>,
    /// The VLAN tag protocol id the capture carried (0x8100 C-VLAN,
    /// 0x88A8 S-VLAN). Kept so an export writes the tag back with the same
    /// meaning; absent on frames created by this program without a capture.
    pub vlan_tpid: Option<u16>,
    pub ethertype: u16,
    pub payload: Arc<[u8]>,
    /// Assignment order in the capture ring: the identity a Trace pick uses
    /// to name one row when several frames share a microsecond.
    pub seq: u64,
    /// The capture said the frame was cut short (snaplen/hardware loss).
    pub truncated: bool,
}

impl EthFrame {
    #[allow(clippy::too_many_arguments)]
    #[cfg(test)]
    pub fn new(
        t_us: u64,
        bus: u8,
        dir: Direction,
        dst: [u8; 6],
        src: [u8; 6],
        vlan: Option<u16>,
        ethertype: u16,
        payload: impl Into<Arc<[u8]>>,
    ) -> Self {
        Self {
            t_us,
            bus,
            dir,
            dst,
            src,
            vlan,
            vlan_tpid: None,
            ethertype,
            payload: payload.into(),
            seq: 0,
            truncated: false,
        }
    }

    pub fn len(&self) -> usize {
        self.payload.len()
    }
}
