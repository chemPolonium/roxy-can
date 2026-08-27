#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Direction {
    Rx,
    Tx,
}

/// Maximum CAN FD data payload in bytes.
pub const MAX_CAN_FD_LEN: usize = 64;

/// Bit flags carried alongside a frame. A hand-rolled bitfield (no external
/// dep) so future frame kinds (error / remote / XL) just add a bit without
/// touching every `CanFrame { .. }` literal.
#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
pub struct FrameFlags(u32);

impl FrameFlags {
    pub const NONE: Self = Self(0);
    /// CAN FD frame (variable payload, data-phase bitrate possible).
    pub const FD: Self = Self(1 << 0);
    /// Bit Rate Switch: data phase runs at a higher bitrate than arbitration.
    pub const BRS: Self = Self(1 << 1);
    /// Error State Indicator (sender reports error-active/passive).
    pub const ESI: Self = Self(1 << 2);
    /// CAN error frame (no ID, no payload; raised by any node detecting an error).
    pub const ERROR: Self = Self(1 << 3);
    /// Remote transmission request (classic CAN only; requests data by ID).
    pub const RTR: Self = Self(1 << 4);
    // Reserved for later: XL = 1 << 5.

    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }
    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }
    /// Compact frame-type marker for the UI: "" for classic data, "FD" (with
    /// "·B" for BRS and "·E" for ESI) for CAN FD, plus "ERR" and "RTR" for the
    /// two non-data kinds. Error / RTR are mutually exclusive with FD because
    /// CAN FD dropped RTR and reports errors separately.
    pub fn tag(self) -> &'static str {
        if self.contains(Self::ERROR) {
            return "ERR";
        }
        if self.contains(Self::RTR) {
            return "RTR";
        }
        if !self.contains(Self::FD) {
            return "";
        }
        match (self.contains(Self::BRS), self.contains(Self::ESI)) {
            (false, false) => "FD",
            (true, false) => "FD·B",
            (false, true) => "FD·E",
            (true, true) => "FD·BE",
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct CanFrame {
    pub t_us: u64,
    pub channel: u8,
    pub id: u32,
    pub extended: bool,
    /// Actual number of valid data bytes (0..=64). This is the source of
    /// truth; the on-wire DLC code is derived via [`CanFrame::dlc_code`].
    pub len: u8,
    /// Fixed 64-byte buffer keeps the frame `Copy` (relied on by the trace
    /// ring buffer, aggregations and the `static` context in the UI). Only
    /// the first `len` bytes are meaningful.
    pub data: [u8; MAX_CAN_FD_LEN],
    pub dir: Direction,
    pub flags: FrameFlags,
}

impl CanFrame {
    pub fn is_fd(&self) -> bool {
        self.flags.contains(FrameFlags::FD)
    }
    pub fn brs(&self) -> bool {
        self.flags.contains(FrameFlags::BRS)
    }
    pub fn esi(&self) -> bool {
        self.flags.contains(FrameFlags::ESI)
    }
    pub fn is_error(&self) -> bool {
        self.flags.contains(FrameFlags::ERROR)
    }
    pub fn is_remote(&self) -> bool {
        self.flags.contains(FrameFlags::RTR)
    }
    /// The meaningful data slice (`data[..len]`). Error and remote frames never
    /// carry data — return an empty slice so the UI and DBC decoder can rely
    /// on it without special-casing.
    pub fn payload(&self) -> &[u8] {
        if self.is_error() || self.is_remote() {
            return &[];
        }
        &self.data[..self.len as usize]
    }
    /// The CAN DLC code (0..=15) for this frame's byte length.
    pub fn dlc_code(&self) -> u8 {
        len2dlc(self.len)
    }
}

/// DLC code (0..=15) -> payload length in bytes. Codes 9..=15 map to the
/// fixed CAN FD lengths.
pub fn dlc2len(code: u8) -> u8 {
    match code {
        0..=8 => code,
        9 => 12,
        10 => 16,
        11 => 20,
        12 => 24,
        13 => 32,
        14 => 48,
        _ => 64,
    }
}

/// Payload length in bytes -> smallest DLC code whose length covers it
/// (rounds up to the next valid CAN FD length).
pub fn len2dlc(len: u8) -> u8 {
    match len {
        0..=8 => len,
        9..=12 => 9,
        13..=16 => 10,
        17..=20 => 11,
        21..=24 => 12,
        25..=32 => 13,
        33..=48 => 14,
        _ => 15,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dlc_code_length_round_trip() {
        // Classic codes are identity.
        for c in 0..=8u8 {
            assert_eq!(dlc2len(c), c);
            assert_eq!(len2dlc(c), c);
        }
        // FD codes map to the fixed ladder.
        assert_eq!(dlc2len(9), 12);
        assert_eq!(dlc2len(15), 64);
        // len2dlc rounds up to the next valid length code.
        assert_eq!(len2dlc(12), 9);
        assert_eq!(len2dlc(13), 10); // 13 is not a valid FD length -> 16 (code 10)
        assert_eq!(len2dlc(64), 15);
        // dlc2len(len2dlc(len)) >= len for every length we can carry.
        for len in 0..=64u8 {
            assert!(dlc2len(len2dlc(len)) >= len);
        }
    }

    #[test]
    fn payload_tracks_len() {
        let mut f = CanFrame {
            t_us: 0,
            channel: 0,
            id: 0x100,
            extended: false,
            len: 3,
            data: [0xAA; MAX_CAN_FD_LEN],
            dir: Direction::Rx,
            flags: FrameFlags::NONE,
        };
        assert_eq!(f.payload(), &[0xAA, 0xAA, 0xAA]);
        assert_eq!(f.dlc_code(), 3);
        f.flags = FrameFlags::FD.union(FrameFlags::BRS);
        assert!(f.is_fd());
        assert!(f.brs());
        assert!(!f.esi());
    }

    #[test]
    fn canframe_is_88_bytes() {
        // TRACE_LIMIT × sizeof(CanFrame) sizes the replay ring; the
        // streaming work in 0.3.0 assumes ~4.5 MB for the 50 000-frame
        // trace buffer. If the struct grows past 88 B, revisit TRACE_LIMIT
        // or box the data payload.
        assert_eq!(std::mem::size_of::<CanFrame>(), 88);
    }
}
