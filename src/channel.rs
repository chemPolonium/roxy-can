//! One CAN bus: user identity, its DBC, and the bitrate declarations the
//! load view divides wire bits by.

use crate::dbc::SymbolTable;

/// One CAN bus: user-defined name, a DBC database, the path it came from, and
/// the DBC nodes this tool transmits as.
pub struct Channel {
    pub name: String,
    pub dbc: Option<SymbolTable>,
    pub dbc_path: String,
    /// Names of the DBC nodes marked as simulated on this bus. Kept on the
    /// channel itself so deleting or renumbering a bus takes its nodes along
    /// without a second remap pass.
    pub sim_nodes: Vec<String>,
    /// Arbitration bitrate in kbit/s, as the load view divides wire bits by
    /// it. There is no hardware behind the simulation, so the value is a
    /// declaration about the bus being analysed, not a device setting.
    pub bitrate_kbps: u32,
    /// CAN FD data-phase bitrate in kbit/s, applied to BRS frames only.
    pub fd_data_kbps: u32,
}

impl Channel {
    pub const DEFAULT_BITRATE_KBPS: u32 = 500;
    pub const DEFAULT_FD_DATA_KBPS: u32 = 2000;
}
