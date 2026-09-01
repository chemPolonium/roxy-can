//! Window and desktop bookkeeping: the five observer-window models, the
//! window-kind registry, and named desktops over them.

use std::collections::HashSet;

/// Which buses/messages an analysis window looks at.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum SigScope {
    All,
    Bus(u8),
    Manual,
}

/// Which analysis window the Filter Selection popup edits.
#[derive(Clone, Copy, PartialEq)]
pub enum PopupTarget {
    Trace(usize),
    Messages(usize),
    Stats(usize),
    Graphics(usize),
    Data(usize),
}

#[derive(Clone)]
pub struct TraceWin {
    pub name: String,
    pub opened: bool,
    pub scope: SigScope,
    pub manual: HashSet<(u8, u32)>,
    pub filter: String,
    pub dir: usize,
    pub dbc_only: bool,
}

#[derive(Clone)]
pub struct MsgWin {
    pub name: String,
    pub opened: bool,
    pub scope: SigScope,
    pub manual: HashSet<(u8, u32)>,
    pub filter: String,
    pub dbc_only: bool,
}

#[derive(Clone)]
pub struct StatsWin {
    pub name: String,
    pub opened: bool,
    pub scope: SigScope,
    pub manual: HashSet<(u8, u32)>,
}

/// Observer window categories a desktop tracks.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum WindowKind {
    Trace,
    Messages,
    Statistics,
    Graphics,
    Data,
}

impl WindowKind {
    pub fn to_u8(self) -> u8 {
        match self {
            WindowKind::Trace => 0,
            WindowKind::Messages => 1,
            WindowKind::Statistics => 2,
            WindowKind::Graphics => 3,
            WindowKind::Data => 4,
        }
    }

    pub fn from_u8(v: u8) -> Option<Self> {
        match v {
            0 => Some(WindowKind::Trace),
            1 => Some(WindowKind::Messages),
            2 => Some(WindowKind::Statistics),
            3 => Some(WindowKind::Graphics),
            4 => Some(WindowKind::Data),
            _ => None,
        }
    }
}

/// A named workspace arrangement: which windows/panels are open and where.
#[derive(Clone)]
pub struct Desktop {
    pub name: String,
    /// imgui ini text captured when the desktop was last active.
    pub layout: String,
    pub open_windows: Vec<(WindowKind, String)>,
    pub show_tx: bool,
    pub show_network: bool,
    pub show_measurement: bool,
    pub show_buses: bool,
    pub show_spec: bool,
    pub show_id_filter: bool,
}
