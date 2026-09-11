//! Window and desktop bookkeeping: the five observer-window models, the
//! window-kind registry, and named desktops over them.

use std::collections::{HashMap, HashSet};

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
    State(usize),
}

/// A trace window's filter lens, cloned out of the window so the per-gate
/// row refresh can walk the ring without holding the window borrowed.
#[derive(Clone)]
pub struct TraceFilter {
    pub scope: SigScope,
    pub manual: HashSet<(u8, u32)>,
    pub filter: String,
    pub dir: usize,
    pub dbc_only: bool,
    pub payload: String,
    pub flags_kind: usize,
    /// Time-range filter (seconds); blank = no bound.
    pub time_from: String,
    pub time_to: String,
    /// Signal-value conditions parsed from the filter text
    /// (`Name>10`, `Name<=5`, ...).
    pub value_conds: Vec<ValueCond>,
}

/// One signal-value condition from the filter text: `Name>10`,
/// `Name<=5`, `Name==3`, ... The name matches a decoded signal on the
/// frame, the comparison runs on the physical value.
#[derive(Clone, Debug, PartialEq)]
pub struct ValueCond {
    pub signal: String,
    pub op: CmpOp,
    pub value: f64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum CmpOp {
    Gt,
    Ge,
    Lt,
    Le,
    Eq,
    Ne,
}

/// Parses one `Name<op>number` condition. Operators longest-first so
/// `>=` wins over `>`. `None` when the text is not a condition.
pub fn parse_value_cond(text: &str) -> Option<ValueCond> {
    let ops = [
        (">=", CmpOp::Ge),
        ("<=", CmpOp::Le),
        ("!=", CmpOp::Ne),
        ("==", CmpOp::Eq),
        (">", CmpOp::Gt),
        ("<", CmpOp::Lt),
        ("=", CmpOp::Eq),
    ];
    let t = text.trim();
    if t.is_empty() {
        return None;
    }
    for (sym, op) in ops {
        if let Some(pos) = t.find(sym) {
            let signal = t[..pos].trim().to_string();
            let value = t[pos + sym.len()..].trim().parse::<f64>().ok()?;
            if signal.is_empty() {
                return None;
            }
            return Some(ValueCond {
                signal,
                op,
                value,
            });
        }
    }
    None
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
    /// Payload byte search: hex pairs, spaces optional (`11 22 3F` or
    /// `11223f`). Rows whose payload contains the sequence match.
    pub payload: String,
    /// Frame-kind filter: 0 any / 1 classic data / 2 FD / 3 RTR / 4 error.
    pub flags_kind: usize,
    /// Time-range filter text (seconds): blank = no bound. Parsed to f64
    /// at match time; invalid text is ignored.
    pub time_from: String,
    pub time_to: String,
    /// The filtered, newest-first row cache the window draws (virtual
    /// scrolling: only the visible slice is submitted per frame).
    /// Rebuilt on the text gate; session state only.
    pub(crate) rows: Vec<CanFrame>,
    /// The newest frame timestamp this window has revealed. Rows stream in
    /// batches on the text gate (see [`crate::app::App::sync_trace_rows`])
    /// instead of churning every frame; `u64::MAX` means everything so far.
    /// Survives ring wrap-around because committed rows only ever leave the
    /// front of the deque. Session state only.
    pub(crate) shown_t_us: u64,
    /// Matching-row count as of the last refresh, for the throttled header.
    pub(crate) shown_count: usize,
}

impl TraceWin {
    /// The filter lens, cloned out of the window: the per-gate row refresh
    /// walks the ring without holding the window borrowed.
    pub fn filter_lens(&self) -> TraceFilter {
        TraceFilter {
            scope: self.scope,
            manual: self.manual.clone(),
            // A `Name>10` style text is a signal-value condition, not an
            // id/name search.
            filter: if parse_value_cond(&self.filter).is_some() {
                String::new()
            } else {
                self.filter.clone()
            },
            dir: self.dir,
            dbc_only: self.dbc_only,
            payload: self.payload.clone(),
            flags_kind: self.flags_kind,
            time_from: self.time_from.clone(),
            time_to: self.time_to.clone(),
            value_conds: parse_value_cond(&self.filter).into_iter().collect(),
        }
    }
}

#[derive(Clone)]
pub struct MsgWin {
    pub name: String,
    pub opened: bool,
    pub scope: SigScope,
    pub manual: HashSet<(u8, u32)>,
    pub filter: String,
    pub dbc_only: bool,
    /// Message rows as of the last throttled text refresh (see
    /// [`crate::app::App::sync_msg_text`]); the table draws these so the
    /// counters hold still long enough to read. Session state only.
    pub(crate) text_keys: Vec<(u8, u32)>,
    pub(crate) text_header: String,
    pub(crate) text_rows: Vec<crate::app::MsgRowText>,
}

#[derive(Clone)]
pub struct StatsWin {
    pub name: String,
    pub opened: bool,
    pub scope: SigScope,
    pub manual: HashSet<(u8, u32)>,
    /// Message Statistics rows as of the last throttled text refresh (see
    /// [`crate::app::App::sync_stats_text`]). Session state only.
    pub(crate) text_keys: Vec<(u8, u32)>,
    pub(crate) text_header: String,
    pub(crate) text_rows: Vec<crate::app::StatsRowText>,
}

/// Observer window categories a desktop tracks.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum WindowKind {
    Trace,
    Messages,
    Statistics,
    Graphics,
    Data,
    StateTracker,
}

impl WindowKind {
    pub fn to_u8(self) -> u8 {
        match self {
            WindowKind::Trace => 0,
            WindowKind::Messages => 1,
            WindowKind::Statistics => 2,
            WindowKind::Graphics => 3,
            WindowKind::Data => 4,
            WindowKind::StateTracker => 5,
        }
    }

    pub fn from_u8(v: u8) -> Option<Self> {
        match v {
            0 => Some(WindowKind::Trace),
            1 => Some(WindowKind::Messages),
            2 => Some(WindowKind::Statistics),
            3 => Some(WindowKind::Graphics),
            4 => Some(WindowKind::Data),
            5 => Some(WindowKind::StateTracker),
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
    pub show_triggers: bool,
    pub show_bus_stats: bool,
    pub show_spec: bool,
    pub show_id_filter: bool,
}

use crate::app::App;
use crate::can::frame::{CanFrame, Direction, FrameFlags};
use crate::observe::{DataWindow, GfxSignal, GraphicsWindow};
impl App {
    /// Live snapshot of the current window/panel arrangement.
    pub fn desktop_snapshot(&self) -> Desktop {
        let mut open_windows = Vec::new();
        for w in &self.trace_windows {
            if w.opened {
                open_windows.push((WindowKind::Trace, w.name.clone()));
            }
        }
        for w in &self.msg_windows {
            if w.opened {
                open_windows.push((WindowKind::Messages, w.name.clone()));
            }
        }
        for w in &self.stats_windows {
            if w.opened {
                open_windows.push((WindowKind::Statistics, w.name.clone()));
            }
        }
        for w in &self.graphics {
            if w.opened {
                open_windows.push((WindowKind::Graphics, w.name.clone()));
            }
        }
        for w in &self.data_windows {
            if w.opened {
                open_windows.push((WindowKind::Data, w.name.clone()));
            }
        }
        for w in &self.state_trackers {
            if w.opened {
                open_windows.push((WindowKind::StateTracker, w.name.clone()));
            }
        }
        Desktop {
            name: String::new(),
            layout: self.layout_cache.clone(),
            open_windows,
            show_tx: self.show_tx,
            show_network: self.show_network,
            show_measurement: self.show_measurement,
            show_buses: self.show_buses,
            show_triggers: self.show_triggers,
            show_bus_stats: self.show_bus_stats,
            show_spec: self.show_spec,
            show_id_filter: self.show_id_filter,
        }
    }

    /// Opens/closes windows and panels to match the given desktop.
    pub fn apply_desktop(&mut self, d: &Desktop) {
        let has = |kind: WindowKind, name: &str| {
            d.open_windows.iter().any(|(k, n)| *k == kind && n == name)
        };
        for w in &mut self.trace_windows {
            w.opened = has(WindowKind::Trace, &w.name);
        }
        for w in &mut self.msg_windows {
            w.opened = has(WindowKind::Messages, &w.name);
        }
        for w in &mut self.stats_windows {
            w.opened = has(WindowKind::Statistics, &w.name);
        }
        for w in &mut self.graphics {
            w.opened = has(WindowKind::Graphics, &w.name);
        }
        for w in &mut self.data_windows {
            w.opened = has(WindowKind::Data, &w.name);
        }
        for w in &mut self.state_trackers {
            w.opened = has(WindowKind::StateTracker, &w.name);
        }
        self.show_tx = d.show_tx;
        self.show_network = d.show_network;
        self.show_measurement = d.show_measurement;
        self.show_buses = d.show_buses;
        self.show_triggers = d.show_triggers;
        self.show_bus_stats = d.show_bus_stats;
        self.show_spec = d.show_spec;
        self.show_id_filter = d.show_id_filter;
        let layout = if d.layout.is_empty() {
            self.default_layout.clone()
        } else {
            d.layout.clone()
        };
        if !layout.is_empty() {
            self.pending_layout = Some(layout);
        }
    }

    /// Refreshes the stored state of the active desktop from live state.
    pub fn sync_active_desktop(&mut self) {
        let mut snap = self.desktop_snapshot();
        if let Some(d) = self.desktops.get_mut(self.active_desktop) {
            snap.name = d.name.clone();
            *d = snap;
        }
    }

    pub fn switch_desktop(&mut self, idx: usize) {
        if idx >= self.desktops.len() || idx == self.active_desktop {
            return;
        }
        self.sync_active_desktop();
        self.active_desktop = idx;
        let target = self.desktops[idx].clone();
        self.apply_desktop(&target);
    }

    /// Adds an empty desktop (no windows, no panels) and switches to it.
    pub fn add_desktop(&mut self) {
        self.sync_active_desktop();
        let snap = Desktop {
            name: format!("Desktop {}", self.desktops.len() + 1),
            layout: String::new(),
            open_windows: Vec::new(),
            show_tx: false,
            show_network: false,
            show_measurement: false,
            show_buses: false,
            show_triggers: false,
            show_bus_stats: false,
            show_spec: false,
            show_id_filter: false,
        };
        self.desktops.push(snap);
        self.active_desktop = self.desktops.len() - 1;
        let target = self.desktops[self.active_desktop].clone();
        self.apply_desktop(&target);
    }

    pub fn delete_desktop(&mut self, idx: usize) {
        if self.desktops.len() <= 1 || idx >= self.desktops.len() {
            return;
        }
        let was_active = idx == self.active_desktop;
        self.desktops.remove(idx);
        if idx < self.active_desktop {
            self.active_desktop -= 1;
        } else if self.active_desktop >= self.desktops.len() {
            self.active_desktop = self.desktops.len() - 1;
        }
        if was_active {
            let target = self.desktops[self.active_desktop].clone();
            self.apply_desktop(&target);
        }
    }

    pub fn rename_desktop(&mut self, idx: usize, name: String) {
        let name = name.trim().to_string();
        if name.is_empty() {
            return;
        }
        if let Some(d) = self.desktops.get_mut(idx) {
            d.name = name;
        }
    }

    pub fn move_desktop(&mut self, from: usize, to: usize) {
        if from >= self.desktops.len() || to >= self.desktops.len() || from == to {
            return;
        }
        let d = self.desktops.remove(from);
        self.desktops.insert(to, d);
        match self.active_desktop {
            a if a == from => self.active_desktop = to,
            a if a < from && to <= a => self.active_desktop = a + 1,
            a if a > from && to >= a => self.active_desktop = a - 1,
            _ => {}
        }
    }

    /// Adds or removes a signal in a Graphics/Data window's signal list
    /// (used by the Signal Selection popup).
    pub fn set_win_signal(&mut self, target: PopupTarget, key: crate::observe::SigKey, on: bool) {
        let signals: Option<&mut Vec<GfxSignal>> = match target {
            PopupTarget::Graphics(i) => self.graphics.get_mut(i).map(|w| &mut w.signals),
            PopupTarget::Data(i) => self.data_windows.get_mut(i).map(|w| &mut w.signals),
            PopupTarget::State(i) => self.state_trackers.get_mut(i).map(|w| &mut w.signals),
            _ => None,
        };
        let Some(signals) = signals else {
            return;
        };
        let present = signals.iter().any(|s| s.key == key);
        if on == present {
            return;
        }
        if on {
            signals.push(GfxSignal {
                key: key.clone(),
                visible: true,
                y_mode: crate::observe::YMode::Auto,
            });
        } else {
            signals.retain(|s| s.key != key);
            // Session color memory and per-signal state state of a removed
            // State Tracker row are dead weight; a re-added signal starts
            // fresh.
            if let PopupTarget::State(i) = target
                && let Some(w) = self.state_trackers.get_mut(i)
            {
                w.color_slots.remove(&key);
                w.rules.remove(&key);
                w.overrides.remove(&key);
            }
        }
        if on {
            self.subscribe(key);
        } else {
            self.prune_signal(&key);
        }
    }

    pub fn new_trace_window(&mut self) {
        self.trace_counter += 1;
        self.trace_windows.push(TraceWin {
            name: format!("Trace {}", self.trace_counter),
            opened: true,
            scope: SigScope::All,
            manual: HashSet::new(),
            filter: String::new(),
            dir: 0,
            dbc_only: false,
            payload: String::new(),
            flags_kind: 0,
            time_from: String::new(),
            time_to: String::new(),
            rows: Vec::new(),
            shown_t_us: self.snap.trace.last().map(|f| f.t_us).unwrap_or(u64::MAX),
            shown_count: self.snap.trace.len(),
        });
    }

    pub fn new_msg_window(&mut self) {
        self.msg_counter += 1;
        self.msg_windows.push(MsgWin {
            name: format!("Messages {}", self.msg_counter),
            opened: true,
            scope: SigScope::All,
            manual: HashSet::new(),
            filter: String::new(),
            dbc_only: false,
            text_keys: Vec::new(),
            text_header: String::new(),
            text_rows: Vec::new(),
        });
    }

    pub fn new_stats_window(&mut self) {
        self.stats_counter += 1;
        self.stats_windows.push(StatsWin {
            name: format!("Message Statistics {}", self.stats_counter),
            opened: true,
            scope: SigScope::All,
            manual: HashSet::new(),
            text_keys: Vec::new(),
            text_header: String::new(),
            text_rows: Vec::new(),
        });
    }

    pub fn new_graphics_window(&mut self) {
        self.graphics_counter += 1;
        self.graphics.push(GraphicsWindow {
            name: format!("Graphics {}", self.graphics_counter),
            signals: Vec::new(),
            time_window_s: 10.0,
            stacked: false,
            opened: true,
            t_offset_s: 0.0,
            show_cursor: true,
            zoom_enabled: false,
            show_markers: true,
            y_locks: HashMap::new(),
            legend_keys: Vec::new(),
            legend: Vec::new(),
        });
    }

    pub fn new_data_window(&mut self) {
        self.data_counter += 1;
        self.data_windows.push(DataWindow {
            name: format!("Data {}", self.data_counter),
            signals: Vec::new(),
            opened: true,
            text_keys: Vec::new(),
            text_cache: Vec::new(),
        });
    }

    pub fn new_state_window(&mut self) {
        self.state_counter += 1;
        self.state_trackers.push(crate::observe::StateWin {
            name: format!("State Tracker {}", self.state_counter),
            opened: true,
            signals: Vec::new(),
            time_window_s: 20.0,
            min_shown_ms: 0,
            color_slots: HashMap::new(),
            rules: HashMap::new(),
            overrides: HashMap::new(),
        });
    }

    /// Scope check shared by all analysis windows: All passes everything,
    /// Bus passes one channel, Manual uses that window's own selection set.
    pub fn scope_match(scope: SigScope, manual: &HashSet<(u8, u32)>, channel: u8, id: u32) -> bool {
        match scope {
            SigScope::All => true,
            SigScope::Bus(ch) => channel == ch,
            SigScope::Manual => manual.contains(&(channel, id)),
        }
    }

    /// Manual selection set of the window named by `t` (None for
    /// Graphics/Data, which filter at the signal level).
    pub fn win_manual(&self, t: PopupTarget) -> Option<&HashSet<(u8, u32)>> {
        match t {
            PopupTarget::Trace(i) => self.trace_windows.get(i).map(|w| &w.manual),
            PopupTarget::Messages(i) => self.msg_windows.get(i).map(|w| &w.manual),
            PopupTarget::Stats(i) => self.stats_windows.get(i).map(|w| &w.manual),
            _ => None,
        }
    }

    pub fn win_manual_mut(&mut self, t: PopupTarget) -> Option<&mut HashSet<(u8, u32)>> {
        match t {
            PopupTarget::Trace(i) => self.trace_windows.get_mut(i).map(|w| &mut w.manual),
            PopupTarget::Messages(i) => self.msg_windows.get_mut(i).map(|w| &mut w.manual),
            PopupTarget::Stats(i) => self.stats_windows.get_mut(i).map(|w| &mut w.manual),
            _ => None,
        }
    }

    /// Applies one Trace window's filter: scope, direction, DBC-only,
    /// payload search, frame kind, and ID/name substring.
    pub fn trace_match(&self, w: &TraceWin, f: &CanFrame) -> bool {
        self.trace_match_lens(&w.filter_lens(), f)
    }

    /// The filter as a borrowable value: the per-gate row refresh walks
    /// the ring against this without holding the window borrowed.
    pub fn trace_match_lens(&self, flt: &TraceFilter, f: &CanFrame) -> bool {
        if !Self::scope_match(flt.scope, &flt.manual, f.channel, f.id) {
            return false;
        }
        match flt.dir {
            1 => {
                if !matches!(f.dir, Direction::Rx) {
                    return false;
                }
            }
            2 if !matches!(f.dir, Direction::Tx) => {
                return false;
            }
            _ => {}
        }
        let name = self.message_name(f.channel, f.id);
        if flt.dbc_only && name.is_none() {
            return false;
        }
        let q = flt.filter.trim();
        // With value conditions present the filter text IS the condition
        // (e.g. `EngineSpeed>100`), so the id/name search is skipped.
        if flt.value_conds.is_empty() && !q.is_empty() {
            let q = q.to_ascii_uppercase();
            let hex = format!("{:X}", f.id);
            let in_name = name.is_some_and(|n| n.to_ascii_uppercase().contains(&q));
            if !hex.contains(&q) && !in_name {
                return false;
            }
        }
        // Payload byte search: hex pairs, spaces optional. A pattern that
        // does not parse is ignored rather than filtering everything away.
        let pat = flt.payload.replace(' ', "");
        if !pat.is_empty()
            && pat.len().is_multiple_of(2)
            && let Ok(needle) = (0..pat.len() / 2)
                .map(|i| u8::from_str_radix(&pat[i * 2..i * 2 + 2], 16))
                .collect::<Result<Vec<u8>, _>>()
            && !f.payload().windows(needle.len().max(1)).any(|w| w == needle)
        {
            return false;
        }
        // Frame-kind filter: 0 any / 1 classic data / 2 FD / 3 RTR / 4 error.
        if let Some(ok) = (match flt.flags_kind {
            1 => Some(!f.flags.contains(FrameFlags::FD)
                && !f.flags.contains(FrameFlags::RTR)
                && !f.flags.contains(FrameFlags::ERROR)),
            2 => Some(f.flags.contains(FrameFlags::FD)),
            3 => Some(f.flags.contains(FrameFlags::RTR)),
            4 => Some(f.flags.contains(FrameFlags::ERROR)),
            _ => None,
        }) && !ok
        {
            return false;
        }
        // Time-range check: if either bound is set and the frame is outside
        // it, drop the frame.
        let t_s = f.t_us as f64 / 1e6;
        if let Ok(from) = flt.time_from.trim().parse::<f64>()
            && t_s < from
        {
            return false;
        }
        if let Ok(to) = flt.time_to.trim().parse::<f64>()
            && t_s > to
        {
            return false;
        }
        // Signal-value conditions written into the filter box, like
        // `EngineSpeed>100` (operators: > >= < <= == !=). A frame whose
        // database decodes to no matching signal value is dropped. Frames
        // without a database skip decoding entirely.
        for cond in &flt.value_conds {
            let Some(db) = self.channel_dbc(f.channel) else {
                return false;
            };
            let Some(m) = db.messages.get(&(f.id, f.extended)) else {
                return false;
            };
            let Some(sig) = m.signals.iter().find(|s| s.name == cond.signal) else {
                return false;
            };
            let raw = crate::decode::extract_raw(
                &f.data[..f.len as usize],
                sig.start_bit,
                sig.size,
                sig.big_endian,
            );
            let v = crate::decode::to_physical(raw, sig.size, sig.signed, sig.factor, sig.offset);
            let ok = match cond.op {
                CmpOp::Gt => v > cond.value,
                CmpOp::Ge => v >= cond.value,
                CmpOp::Lt => v < cond.value,
                CmpOp::Le => v <= cond.value,
                CmpOp::Eq => v == cond.value,
                CmpOp::Ne => v != cond.value,
            };
            if !ok {
                return false;
            }
        }
        true
    }

    pub fn window_counters(&self) -> crate::config::Counters {
        crate::config::Counters {
            trace: self.trace_counter,
            msg: self.msg_counter,
            stats: self.stats_counter,
            graphics: self.graphics_counter,
            data: self.data_counter,
            state: self.state_counter,
        }
    }

    pub fn set_window_counters(&mut self, c: crate::config::Counters) {
        self.trace_counter = c.trace.max(self.trace_windows.len());
        self.msg_counter = c.msg.max(self.msg_windows.len());
        self.stats_counter = c.stats.max(self.stats_windows.len());
        self.graphics_counter = c.graphics.max(self.graphics.len());
        self.data_counter = c.data.max(self.data_windows.len());
        self.state_counter = c.state.max(self.state_trackers.len());
    }
}
