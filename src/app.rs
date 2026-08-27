use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::time::Instant;

use crate::can::frame::{CanFrame, Direction, FrameFlags, MAX_CAN_FD_LEN};
use crate::config::{AUTOSAVE_PATH, Config, META_PATH, Meta, PROJECT_EXT, ProjectFile};
use crate::dbc::SymbolTable;
use crate::log::asc::{AscWriter, parse_asc};
use crate::source::FrameSource;
use crate::source::replay::ReplaySource;
use crate::source::virtual_source::VirtualSource;

pub const TRACE_LIMIT: usize = 50_000;
pub const TOOLBAR_H: f32 = 54.0;
pub const STATUSBAR_H: f32 = 28.0;
pub const TABSTRIP_H: f32 = 22.0;
const HISTORY_LIMIT: usize = 4_000;
const SAMPLE_INTERVAL_US: u64 = 50_000;
/// Speed ladder shared by the toolbar combo and the slower/faster buttons.
pub const REPLAY_SPEEDS: [f64; 4] = [0.5, 1.0, 2.0, 4.0];

pub const PALETTE: [[f32; 4]; 8] = [
    [0.30, 0.80, 1.00, 1.0],
    [1.00, 0.65, 0.20, 1.0],
    [0.45, 0.95, 0.45, 1.0],
    [1.00, 0.40, 0.40, 1.0],
    [0.75, 0.55, 1.00, 1.0],
    [1.00, 0.85, 0.30, 1.0],
    [0.35, 0.95, 0.85, 1.0],
    [0.95, 0.55, 0.85, 1.0],
];

pub struct Subscription {
    pub latest: f64,
    pub unit: String,
    pub min: f64,
    pub max: f64,
    /// Running average over the sampled values.
    pub avg: f64,
    sum: f64,
    n: u64,
    pub last_update_us: u64,
    pub last_sample_us: u64,
    pub history: VecDeque<(u64, f64)>,
    pub color: usize,
}

pub struct GfxSignal {
    pub key: (u8, u32, String),
    pub visible: bool,
}

pub struct GraphicsWindow {
    pub name: String,
    pub signals: Vec<GfxSignal>,
    pub time_window_s: f64,
    pub stacked: bool,
    pub opened: bool,
    pub t_offset_s: f64,
    pub show_cursor: bool,
    pub zoom_enabled: bool,
}

pub struct DataWindow {
    pub name: String,
    pub signals: Vec<GfxSignal>,
    pub opened: bool,
    /// Visualization column style: true = value bar, false = sparkline;
    /// clicking the column toggles it.
    pub viz_bar: bool,
}

/// One CAN bus: user-defined name, a DBC database, and the path it came from.
pub struct Channel {
    pub name: String,
    pub dbc: Option<SymbolTable>,
    pub dbc_path: String,
}

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

#[derive(Clone, Copy)]
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

pub struct TxMsg {
    pub channel: u8,
    pub id: u32,
    pub extended: bool,
    pub name: String,
    pub len: u8,
    pub data: [u8; MAX_CAN_FD_LEN],
    pub flags: FrameFlags,
    pub data_text: String,
    pub cycle_us: u64,
    pub active: bool,
    pub next_t_us: u64,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Virtual,
    Replay,
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
    pub show_id_filter: bool,
}

/// Deferred action waiting behind the "unsaved project" confirmation modal.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PendingAction {
    Quit,
    NewProject,
    OpenProject,
    OpenPath(PathBuf),
}

pub struct App {
    pub measuring: bool,
    pub recording: bool,
    pub mode: Mode,
    pub run_mode: Mode,
    pub quit: bool,
    pub t0: Instant,
    pub frame_counter: u64,
    pub trace: VecDeque<CanFrame>,
    pub trace_paused: bool,
    paused_at_us: Option<u64>,
    pub channels: Vec<Channel>,
    pub status: String,
    pub asc_path: String,
    asc_frames: Vec<CanFrame>,
    pub recent_dbc: Vec<String>,
    pub recent_asc: Vec<String>,
    /// Path of the currently open .rxproj project; None = untitled workspace.
    pub project_path: Option<PathBuf>,
    pub recent_projects: Vec<String>,
    /// imgui layout text captured every frame by main; embedded on save.
    pub layout_cache: String,
    /// Layout captured on the very first frame, restored by New Project.
    pub default_layout: String,
    /// Layout queued to be applied by main before the next imgui frame.
    pub pending_layout: Option<String>,
    /// Set when a destructive action must first pass the unsaved-project modal.
    pub pending_action: Option<PendingAction>,
    /// Config JSON snapshot of the last clean state (loaded/saved/reset);
    /// compared against the live config to decide if anything changed.
    baseline: String,
    /// Named workspace arrangements; always holds at least one desktop.
    pub desktops: Vec<Desktop>,
    pub active_desktop: usize,
    /// Target index of the rename popup; buffer holds the edited name.
    pub desktop_rename_target: Option<usize>,
    pub desktop_rename_buf: String,
    pub replay_speed: f64,
    pub record_path: String,
    pub last_record: String,
    pub subs: HashMap<(u8, u32, String), Subscription>,
    pub aggs: HashMap<(u8, u32), MessageAgg>,
    pub symbol_search: String,
    pub show_tx: bool,
    pub show_network: bool,
    pub show_measurement: bool,
    pub show_buses: bool,
    pub show_id_filter: bool,
    pub show_shortcuts: bool,
    pub show_about: bool,
    pub id_filter_search: String,
    pub gen_search: String,
    pub popup_target: Option<PopupTarget>,
    pub focus_title: Option<String>,
    pub net_selected: usize,
    pub tx_list: Vec<TxMsg>,
    pub tx_pick: usize,
    pub last_tick_us: u64,
    pub frame_rate: f64,
    pub trace_windows: Vec<TraceWin>,
    pub msg_windows: Vec<MsgWin>,
    pub stats_windows: Vec<StatsWin>,
    pub graphics: Vec<GraphicsWindow>,
    pub data_windows: Vec<DataWindow>,
    trace_counter: usize,
    msg_counter: usize,
    stats_counter: usize,
    graphics_counter: usize,
    data_counter: usize,
    bus_counter: usize,
    color_counter: usize,
    source: Box<dyn FrameSource>,
    writer: Option<AscWriter>,
    buf: Vec<CanFrame>,
}

/// Newest-first recent list: dedups and caps at 8 entries.
fn push_recent(list: &mut Vec<String>, path: String) {
    list.retain(|p| p != &path);
    list.insert(0, path);
    list.truncate(8);
}

impl App {
    pub fn new() -> Self {
        let mut app = App {
            measuring: false,
            recording: false,
            mode: Mode::Virtual,
            run_mode: Mode::Virtual,
            quit: false,
            t0: Instant::now(),
            frame_counter: 0,
            trace: VecDeque::new(),
            trace_paused: false,
            paused_at_us: None,
            channels: vec![
                Channel {
                    name: "CAN1".to_string(),
                    dbc: None,
                    dbc_path: "assets/sample.dbc".to_string(),
                },
                Channel {
                    name: "CAN2".to_string(),
                    dbc: None,
                    dbc_path: "assets/motbus.dbc".to_string(),
                },
            ],
            status: "stopped".to_string(),
            asc_path: String::new(),
            asc_frames: Vec::new(),
            recent_dbc: Vec::new(),
            recent_asc: Vec::new(),
            project_path: None,
            recent_projects: Vec::new(),
            layout_cache: String::new(),
            default_layout: String::new(),
            pending_layout: None,
            pending_action: None,
            baseline: String::new(),
            desktops: Vec::new(),
            active_desktop: 0,
            desktop_rename_target: None,
            desktop_rename_buf: String::new(),
            replay_speed: 1.0,
            record_path: String::new(),
            last_record: String::new(),
            subs: HashMap::new(),
            aggs: HashMap::new(),
            symbol_search: String::new(),
            show_tx: true,
            show_network: true,
            show_measurement: true,
            show_buses: false,
            show_id_filter: false,
            show_shortcuts: false,
            show_about: false,
            id_filter_search: String::new(),
            gen_search: String::new(),
            popup_target: None,
            focus_title: None,
            net_selected: 0,
            tx_list: Vec::new(),
            tx_pick: 0,
            last_tick_us: 0,
            frame_rate: 0.0,
            trace_windows: Vec::new(),
            msg_windows: Vec::new(),
            stats_windows: Vec::new(),
            graphics: Vec::new(),
            data_windows: Vec::new(),
            trace_counter: 0,
            msg_counter: 0,
            stats_counter: 0,
            graphics_counter: 0,
            data_counter: 0,
            bus_counter: 2,
            color_counter: 0,
            source: Box::new(VirtualSource::new()),
            writer: None,
            buf: Vec::new(),
        };
        app.load_dbcs();
        app.new_trace_window();
        app.new_msg_window();
        app.new_stats_window();
        app.new_graphics_window();
        app.new_data_window();
        let msgs: Vec<(u8, u32)> = app
            .channels
            .iter()
            .enumerate()
            .flat_map(|(ch, c)| {
                c.dbc
                    .as_ref()
                    .map(|db| db.order.iter().map(move |&id| (ch as u8, id)))
                    .into_iter()
                    .flatten()
            })
            .collect();
        for (ch, id) in msgs {
            app.add_tx(ch, id);
        }
        let mut first = app.desktop_snapshot();
        first.name = "Desktop 1".to_string();
        app.desktops = vec![first];
        app.active_desktop = 0;
        app.baseline = app.config_snapshot();
        app
    }

    pub fn channel_dbc(&self, ch: u8) -> Option<&SymbolTable> {
        self.channels.get(ch as usize).and_then(|c| c.dbc.as_ref())
    }

    pub fn message_name(&self, ch: u8, id: u32) -> Option<&str> {
        self.channel_dbc(ch).and_then(|db| db.message_name(id))
    }

    pub fn channel_name(&self, ch: u8) -> String {
        self.channels
            .get(ch as usize)
            .map(|c| c.name.clone())
            .unwrap_or_else(|| format!("CAN{}", ch + 1))
    }

    /// Adds a new bus, loads its default DBC, and pre-populates the generator.
    pub fn add_channel(&mut self) {
        self.bus_counter += 1;
        self.channels.push(Channel {
            name: format!("CAN{}", self.bus_counter),
            dbc: None,
            dbc_path: "assets/sample.dbc".to_string(),
        });
        let ch = self.channels.len() - 1;
        self.load_channel(ch);
        let ids: Vec<u32> = self.channels[ch]
            .dbc
            .as_ref()
            .map(|db| db.order.clone())
            .unwrap_or_default();
        for id in ids {
            self.add_tx(ch as u8, id);
        }
    }

    /// Removes a bus and remaps every channel-indexed reference one step down.
    pub fn remove_channel(&mut self, ch: usize) {
        if self.channels.len() <= 1 {
            self.status = "at least one bus is required".to_string();
            return;
        }
        if ch >= self.channels.len() {
            return;
        }
        let name = self.channels[ch].name.clone();
        self.channels.remove(ch);
        let remap = |c: u8| -> Option<u8> {
            if (c as usize) < ch {
                Some(c)
            } else if (c as usize) == ch {
                None
            } else {
                Some(c - 1)
            }
        };
        self.aggs = self
            .aggs
            .drain()
            .filter_map(|((c, id), mut a)| {
                remap(c).map(|nc| {
                    a.channel = nc;
                    ((nc, id), a)
                })
            })
            .collect();
        self.subs = self
            .subs
            .drain()
            .filter_map(|((c, id, sig), s)| remap(c).map(|nc| ((nc, id, sig), s)))
            .collect();
        let remap_set = |set: &mut HashSet<(u8, u32)>| {
            *set = set
                .drain()
                .filter_map(|(c, id)| remap(c).map(|nc| (nc, id)))
                .collect();
        };
        for w in &mut self.trace_windows {
            remap_set(&mut w.manual);
        }
        for w in &mut self.msg_windows {
            remap_set(&mut w.manual);
        }
        for w in &mut self.stats_windows {
            remap_set(&mut w.manual);
        }
        self.tx_list.retain(|t| t.channel as usize != ch);
        for t in &mut self.tx_list {
            if t.channel as usize > ch {
                t.channel -= 1;
            }
        }
        self.trace.retain(|f| f.channel as usize != ch);
        for f in self.trace.iter_mut() {
            if f.channel as usize > ch {
                f.channel -= 1;
            }
        }
        let remap_keys = |signals: &mut Vec<GfxSignal>| {
            signals.retain(|s| remap(s.key.0).is_some());
            for s in signals.iter_mut() {
                s.key.0 = remap(s.key.0).unwrap();
            }
        };
        for g in &mut self.graphics {
            remap_keys(&mut g.signals);
        }
        for d in &mut self.data_windows {
            remap_keys(&mut d.signals);
        }
        let fix_scope = |s: &mut SigScope| {
            if let SigScope::Bus(b) = *s {
                *s = match (b as usize).cmp(&ch) {
                    std::cmp::Ordering::Equal => SigScope::All,
                    std::cmp::Ordering::Greater => SigScope::Bus(b - 1),
                    std::cmp::Ordering::Less => *s,
                };
            }
        };
        for w in &mut self.trace_windows {
            fix_scope(&mut w.scope);
        }
        for w in &mut self.msg_windows {
            fix_scope(&mut w.scope);
        }
        for w in &mut self.stats_windows {
            fix_scope(&mut w.scope);
        }
        self.net_selected = 0;
        self.status = format!("bus {name} removed");
    }

    pub fn now_us(&self) -> u64 {
        self.t0.elapsed().as_micros() as u64
    }

    pub fn start_virtual(&mut self) {
        self.close_writer();
        self.source = Box::new(VirtualSource::new());
        self.mode = Mode::Virtual;
        self.run_mode = Mode::Virtual;
        self.reset_time();
        self.measuring = true;
        self.status = "measuring (virtual)".to_string();
        if self.recording {
            self.open_writer();
        }
    }

    /// Starts measurement in the mode selected by the Simulation/Replay
    /// dropdown; replay falls back to a file picker when no ASC is loaded.
    pub fn start_selected(&mut self) {
        match self.run_mode {
            Mode::Virtual => self.start_virtual(),
            Mode::Replay => {
                if !self.can_replay() {
                    self.pick_asc();
                }
                // Start expressed the intent to run, so begin playback as
                // soon as a log is actually available.
                if self.can_replay() {
                    self.replay();
                }
            }
        }
    }

    /// Switches between simulation and replay; a running measurement is
    /// stopped first so the transport never outlives its source.
    pub fn switch_run_mode(&mut self, mode: Mode) {
        if mode == self.run_mode {
            return;
        }
        if self.measuring {
            self.stop();
        }
        self.run_mode = mode;
    }

    fn can_replay(&self) -> bool {
        !self.asc_frames.is_empty()
            || !self.asc_path.trim().is_empty()
            || !self.last_record.trim().is_empty()
    }

    pub fn stop(&mut self) {
        self.measuring = false;
        self.close_writer();
        self.status = "stopped".to_string();
    }

    fn close_writer(&mut self) {
        if let Some(w) = self.writer.take() {
            w.finish().ok();
        }
    }

    fn open_writer(&mut self) -> bool {
        let b = self.record_path.trim();
        let b = b
            .strip_suffix(".asc")
            .or_else(|| b.strip_suffix(".ASC"))
            .unwrap_or(b);
        let base = if b.is_empty() { "record" } else { b };
        let path = format!(
            "{}_{}.asc",
            base,
            chrono::Local::now().format("%Y%m%d_%H%M%S")
        );
        match AscWriter::new(&path) {
            Ok(w) => {
                self.writer = Some(w);
                self.last_record = path.clone();
                self.status = format!("recording to {path}");
                true
            }
            Err(e) => {
                self.status = format!("record failed: {e}");
                false
            }
        }
    }

    fn reset_time(&mut self) {
        self.t0 = Instant::now();
        self.frame_counter = 0;
        // A fresh start must not inherit the previous run's pause state.
        self.trace_paused = false;
        self.paused_at_us = None;
        self.trace.clear();
        self.aggs.clear();
        for tx in &mut self.tx_list {
            tx.next_t_us = 0;
        }
        for sub in self.subs.values_mut() {
            sub.history.clear();
        }
    }

    pub fn toggle_record(&mut self) {
        if self.recording {
            self.close_writer();
            self.recording = false;
        } else {
            self.recording = true;
            // While stopped, the file is created by the next start_virtual;
            // checking Record must not leave an empty record file behind.
            if self.measuring {
                self.recording = self.open_writer();
            }
        }
    }

    pub fn load_channel(&mut self, ch: usize) -> bool {
        let Some(channel) = self.channels.get_mut(ch) else {
            return false;
        };
        let name = channel.name.clone();
        match std::fs::read_to_string(channel.dbc_path.trim()) {
            Ok(content) => match crate::dbc::load_dbc_str(&content) {
                Ok(table) => {
                    self.status = format!("{name} DBC loaded: {} messages", table.order.len());
                    channel.dbc = Some(table);
                    true
                }
                Err(e) => {
                    self.status = format!("{name} DBC error: {e}");
                    false
                }
            },
            Err(e) => {
                self.status = format!("{name} DBC read failed: {e}");
                false
            }
        }
    }

    pub fn load_dbcs(&mut self) {
        for ch in 0..self.channels.len() {
            self.load_channel(ch);
        }
    }

    pub fn pick_dbc(&mut self) {
        let ch = 0;
        let name = self.channel_name(ch as u8);
        if let Some(p) = rfd::FileDialog::new()
            .set_title(format!("Open DBC for {name}"))
            .add_filter("DBC files", &["dbc"])
            .pick_file()
        {
            self.open_dbc_for(ch, p.to_string_lossy().into_owned());
        }
    }

    /// Open a DBC directly for a given channel (used by the Buses window).
    pub fn pick_dbc_for(&mut self, ch: usize) {
        let name = self.channel_name(ch as u8);
        if let Some(p) = rfd::FileDialog::new()
            .set_title(format!("Open DBC for {name}"))
            .add_filter("DBC files", &["dbc"])
            .pick_file()
        {
            self.open_dbc_for(ch, p.to_string_lossy().into_owned());
        }
    }

    /// Sets a bus's DBC path and loads it; successful parses are recorded
    /// in the recent list.
    pub fn open_dbc_for(&mut self, ch: usize, path: String) {
        if let Some(channel) = self.channels.get_mut(ch) {
            channel.dbc_path = path.clone();
        }
        if self.load_channel(ch) {
            self.push_recent_dbc(path);
        }
    }

    /// Opens a file dropped onto the window: a DBC goes into the first
    /// bus, an ASC becomes the replay log.
    pub fn open_dropped(&mut self, path: &std::path::Path) {
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_ascii_lowercase());
        match ext.as_deref() {
            Some("dbc") => {
                self.open_dbc_for(0, path.to_string_lossy().into_owned());
            }
            Some("asc") => self.load_asc(&path.to_string_lossy()),
            _ => {
                let name = path
                    .file_name()
                    .map(|s| s.to_string_lossy().into_owned())
                    .unwrap_or_default();
                self.status = format!("unsupported file: {name}");
            }
        }
    }

    pub fn push_recent_dbc(&mut self, path: String) {
        push_recent(&mut self.recent_dbc, path);
    }

    pub fn push_recent_asc(&mut self, path: String) {
        push_recent(&mut self.recent_asc, path);
    }

    pub fn pick_asc(&mut self) {
        if let Some(p) = rfd::FileDialog::new()
            .set_title("Open ASC")
            .add_filter("ASC files", &["asc"])
            .pick_file()
        {
            self.load_asc(&p.to_string_lossy());
        }
    }

    /// Parses an ASC log and keeps it ready for replay without starting
    /// playback; Start (in Replay mode) begins it.
    pub fn load_asc(&mut self, path: &str) {
        match std::fs::read_to_string(path) {
            Ok(content) => {
                let frames = parse_asc(&content);
                if frames.is_empty() {
                    self.status = "ASC: no frames parsed".to_string();
                    return;
                }
                let name = std::path::Path::new(path)
                    .file_name()
                    .map(|s| s.to_string_lossy().into_owned())
                    .unwrap_or_else(|| path.to_string());
                self.asc_path = path.to_string();
                self.status = format!("loaded {} frames: {name}", frames.len());
                self.asc_frames = frames;
                self.push_recent_asc(path.to_string());
            }
            Err(e) => self.status = format!("ASC read failed [{path}]: {e}"),
        }
    }

    /// Exports the frames that pass the given Trace window's filter as ASC.
    pub fn export_trace(&mut self, win: usize, path: &str) {
        if self.trace.is_empty() {
            self.status = "export: trace is empty".to_string();
            return;
        }
        let Some(w) = self.trace_windows.get(win) else {
            return;
        };
        let frames: Vec<CanFrame> = self
            .trace
            .iter()
            .copied()
            .filter(|f| self.trace_match(w, f))
            .collect();
        if frames.is_empty() {
            self.status = "export: no frames pass the trace filter".to_string();
            return;
        }
        match AscWriter::new(path) {
            Ok(mut w) => {
                for f in &frames {
                    w.write(f).ok();
                }
                w.finish().ok();
                self.status = format!(
                    "exported {} of {} frames to {path}",
                    frames.len(),
                    self.trace.len()
                );
            }
            Err(e) => self.status = format!("export failed: {e}"),
        }
    }

    pub fn export_trace_dialog(&mut self, win: usize) {
        if let Some(p) = rfd::FileDialog::new()
            .set_title("Export Trace as ASC")
            .set_file_name("trace_export.asc")
            .add_filter("ASC files", &["asc"])
            .save_file()
        {
            let path = p.to_string_lossy().to_string();
            self.export_trace(win, &path);
        }
    }

    fn csv_save_dialog(title: &str, name: &str) -> Option<std::path::PathBuf> {
        rfd::FileDialog::new()
            .set_title(title)
            .set_file_name(name)
            .add_filter("CSV files", &["csv"])
            .save_file()
    }

    fn write_export(&mut self, path: &str, content: String) {
        match std::fs::write(path, content) {
            Ok(()) => self.status = format!("exported to {path}"),
            Err(e) => self.status = format!("export failed: {e}"),
        }
    }

    pub fn export_stats_dialog(&mut self, win: usize) {
        if let Some(p) = Self::csv_save_dialog("Export Statistics as CSV", "statistics.csv") {
            self.export_stats_csv(win, &p.to_string_lossy());
        }
    }

    pub fn export_stats_csv(&mut self, win: usize, path: &str) {
        let Some(w) = self.stats_windows.get(win) else {
            return;
        };
        let scope = w.scope;
        let manual = &w.manual;
        let mut rows: Vec<&MessageAgg> = self
            .aggs
            .values()
            .filter(|a| Self::scope_match(scope, manual, a.channel, a.id))
            .collect();
        rows.sort_by_key(|a| (a.channel, a.id));
        let mut s =
            String::from("bus,id,name,count,cycle_min_ms,cycle_avg_ms,cycle_max_ms,len,flags\n");
        for a in &rows {
            let name = self.message_name(a.channel, a.id).unwrap_or("-");
            let bus = self.channel_name(a.channel);
            let (cmin, cavg, cmax) = if a.count > 1 {
                (a.min_us / 1000.0, a.cycle_us / 1000.0, a.max_us / 1000.0)
            } else {
                (0.0, 0.0, 0.0)
            };
            s.push_str(&format!(
                "{bus},{},{},{},{cmin:.3},{cavg:.3},{cmax:.3},{},{}\n",
                a.id,
                name,
                a.count,
                a.len,
                a.flags.tag()
            ));
        }
        self.write_export(path, s);
    }

    pub fn export_messages_dialog(&mut self, win: usize) {
        if let Some(p) = Self::csv_save_dialog("Export Messages as CSV", "messages.csv") {
            self.export_messages_csv(win, &p.to_string_lossy());
        }
    }

    /// Exports the aggregation rows currently visible in the given Messages window.
    pub fn export_messages_csv(&mut self, win: usize, path: &str) {
        let Some(w) = self.msg_windows.get(win) else {
            return;
        };
        let scope = w.scope;
        let filter = w.filter.trim().to_lowercase();
        let dbc_only = w.dbc_only;
        let manual = &w.manual;
        let mut rows: Vec<MessageAgg> = self
            .aggs
            .values()
            .copied()
            .filter(|a| {
                if !Self::scope_match(scope, manual, a.channel, a.id) {
                    return false;
                }
                let name = self.message_name(a.channel, a.id).unwrap_or("-");
                if dbc_only && name == "-" {
                    return false;
                }
                if filter.is_empty() {
                    return true;
                }
                let id_str = format!("{:x}", a.id);
                name.to_lowercase().contains(&filter) || id_str.contains(&filter)
            })
            .collect();
        rows.sort_by_key(|a| (a.channel, a.id));
        let mut s = String::from("bus,id,name,dir,count,cycle_ms,len,flags,data\n");
        for a in &rows {
            let name = self.message_name(a.channel, a.id).unwrap_or("-");
            let bus = self.channel_name(a.channel);
            let dir = match a.dir {
                Direction::Rx => "Rx",
                Direction::Tx => "Tx",
            };
            let cycle = if a.count > 1 {
                a.cycle_us / 1000.0
            } else {
                0.0
            };
            let data: String = a
                .payload()
                .iter()
                .map(|b| format!("{b:02X}"))
                .collect::<Vec<_>>()
                .join(" ");
            s.push_str(&format!(
                "{bus},{},{},{},{},{cycle:.3},{},{},{}\n",
                a.id,
                name,
                dir,
                a.count,
                a.len,
                a.flags.tag(),
                data
            ));
        }
        self.write_export(path, s);
    }

    pub fn export_graphics_dialog(&mut self, i: usize) {
        let name = self
            .graphics
            .get(i)
            .map(|g| g.name.to_lowercase().replace(' ', "_"))
            .unwrap_or_else(|| "graphics".to_string());
        if let Some(p) =
            Self::csv_save_dialog("Export Graphics history as CSV", &format!("{name}.csv"))
        {
            self.export_graphics_csv(i, &p.to_string_lossy());
        }
    }

    /// Snapshot of the plotted signal history, long format: one row per sample.
    pub fn export_graphics_csv(&mut self, i: usize, path: &str) {
        let Some(g) = self.graphics.get(i) else {
            return;
        };
        let keys: Vec<(u8, u32, String)> = g
            .signals
            .iter()
            .filter(|s| s.visible)
            .map(|s| s.key.clone())
            .collect();
        let mut s = String::from("time_us,bus,signal,value\n");
        let mut n = 0usize;
        for key in &keys {
            let Some(sub) = self.subs.get(key) else {
                continue;
            };
            let bus = self.channel_name(key.0);
            for (t, v) in &sub.history {
                s.push_str(&format!("{t},{bus},{},{v}\n", key.2));
                n += 1;
            }
        }
        if n == 0 {
            self.status = "export: no signal history yet".to_string();
            return;
        }
        self.write_export(path, s);
    }

    pub fn export_data_dialog(&mut self, i: usize) {
        let name = self
            .data_windows
            .get(i)
            .map(|d| d.name.to_lowercase().replace(' ', "_"))
            .unwrap_or_else(|| "data".to_string());
        if let Some(p) = Self::csv_save_dialog("Export Data values as CSV", &format!("{name}.csv"))
        {
            self.export_data_csv(i, &p.to_string_lossy());
        }
    }

    /// Snapshot of the latest signal values shown in a Data window.
    pub fn export_data_csv(&mut self, i: usize, path: &str) {
        let Some(d) = self.data_windows.get(i) else {
            return;
        };
        let keys: Vec<(u8, u32, String)> = d
            .signals
            .iter()
            .filter(|s| s.visible)
            .map(|s| s.key.clone())
            .collect();
        let mut s = String::from("bus,signal,value,unit\n");
        for key in &keys {
            let Some(sub) = self.subs.get(key) else {
                continue;
            };
            let bus = self.channel_name(key.0);
            s.push_str(&format!("{bus},{},{},{}\n", key.2, sub.latest, sub.unit));
        }
        self.write_export(path, s);
    }

    pub fn replay(&mut self) {
        if !self.asc_frames.is_empty() {
            self.start_replay(self.asc_frames.clone());
            return;
        }
        let path = {
            let p = self.asc_path.trim();
            if p.is_empty() {
                self.last_record.clone()
            } else {
                p.to_string()
            }
        };
        match std::fs::read_to_string(&path) {
            Ok(content) => {
                let frames = parse_asc(&content);
                if frames.is_empty() {
                    self.status = "ASC: no frames parsed".to_string();
                    return;
                }
                self.start_replay(frames);
            }
            Err(e) => self.status = format!("ASC read failed [{path}]: {e}"),
        }
    }

    fn start_replay(&mut self, frames: Vec<CanFrame>) {
        let n = frames.len();
        self.close_writer();
        // Replay just re-emits an existing log; recording it would
        // only duplicate the file, so drop the Record state.
        self.recording = false;
        let mut source = ReplaySource::new(frames);
        source.set_speed(self.replay_speed);
        self.source = Box::new(source);
        self.mode = Mode::Replay;
        self.run_mode = Mode::Replay;
        self.reset_time();
        self.measuring = true;
        self.status = format!("replaying {n} frames at {}x", self.replay_speed);
    }

    /// Changes replay speed; takes effect immediately if a replay is
    /// running.
    pub fn set_replay_speed(&mut self, speed: f64) {
        self.replay_speed = speed;
        self.source.set_speed(speed);
    }

    /// Moves one notch along REPLAY_SPEEDS; negative slows down, positive
    /// speeds up, the ends clamp.
    pub fn step_replay_speed(&mut self, delta: i32) {
        let idx = REPLAY_SPEEDS
            .iter()
            .position(|s| (*s - self.replay_speed).abs() < 1e-9)
            .unwrap_or(1);
        let next = (idx as i32 + delta).clamp(0, REPLAY_SPEEDS.len() as i32 - 1) as usize;
        self.set_replay_speed(REPLAY_SPEEDS[next]);
    }

    /// Current replay position and total duration in seconds (None when
    /// the active source has no timeline).
    pub fn replay_position(&self) -> Option<(f64, f64)> {
        let pos = self.source.position()? as f64 / 1e6;
        let dur = self.source.duration()? as f64 / 1e6;
        Some((pos, dur))
    }

    /// Resets the pan offsets of all plot windows back to the live edge.
    pub fn jump_to_live(&mut self) {
        for g in &mut self.graphics {
            g.t_offset_s = 0.0;
        }
    }

    /// Rebuilds the whole workspace back to factory defaults, keeping only
    /// the machine-local recents and the captured default layout.
    pub fn reset_to_defaults(&mut self) {
        self.stop();
        let recent_dbc = std::mem::take(&mut self.recent_dbc);
        let recent_asc = std::mem::take(&mut self.recent_asc);
        let recent_projects = std::mem::take(&mut self.recent_projects);
        let default_layout = std::mem::take(&mut self.default_layout);
        *self = App::new();
        self.recent_dbc = recent_dbc;
        self.recent_asc = recent_asc;
        self.recent_projects = recent_projects;
        self.default_layout = default_layout;
    }

    /// Display name of the current project: the file stem, or "Untitled".
    pub fn project_name(&self) -> String {
        self.project_path
            .as_ref()
            .and_then(|p| p.file_stem())
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| "Untitled".to_string())
    }

    /// Project name with a `*` suffix when there are unsaved changes.
    pub fn display_name(&self) -> String {
        let name = self.project_name();
        if self.is_dirty() {
            format!("{name} *")
        } else {
            name
        }
    }

    pub fn push_recent_project(&mut self, path: String) {
        push_recent(&mut self.recent_projects, path);
    }

    /// Serializable snapshot of the workspace configuration, used to tell
    /// whether anything changed since the last load/save/reset.
    fn config_snapshot(&self) -> String {
        serde_json::to_string(&Config::from_app(self, None)).unwrap_or_default()
    }

    pub fn is_dirty(&self) -> bool {
        self.config_snapshot() != self.baseline
    }

    /// Marks the current workspace as the clean baseline (after load/save).
    pub fn mark_clean(&mut self) {
        self.baseline = self.config_snapshot();
    }

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
        Desktop {
            name: String::new(),
            layout: self.layout_cache.clone(),
            open_windows,
            show_tx: self.show_tx,
            show_network: self.show_network,
            show_measurement: self.show_measurement,
            show_buses: self.show_buses,
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
        self.show_tx = d.show_tx;
        self.show_network = d.show_network;
        self.show_measurement = d.show_measurement;
        self.show_buses = d.show_buses;
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

    pub fn run_action(&mut self, action: PendingAction) {
        match action {
            PendingAction::Quit => self.quit = true,
            PendingAction::NewProject => self.new_project(),
            PendingAction::OpenProject => self.open_project_dialog(),
            PendingAction::OpenPath(p) => self.open_project_path(&p),
        }
    }

    /// Saved projects auto-save and proceed; untouched untitled workspaces
    /// have nothing to lose and proceed directly; only a modified untitled
    /// workspace is routed through the confirmation modal.
    pub fn guarded_action(&mut self, action: PendingAction) {
        if let Some(p) = self.project_path.clone() {
            self.save_project(Some(p));
            self.run_action(action);
        } else if self.is_dirty() {
            self.pending_action = Some(action);
        } else {
            self.run_action(action);
        }
    }

    /// Saves the workspace as a .rxproj file. `None` opens a picker.
    /// Returns true when a file was written.
    pub fn save_project(&mut self, path: Option<PathBuf>) -> bool {
        let path = match path {
            Some(p) => p,
            None => {
                let mut dlg = rfd::FileDialog::new().add_filter("roxy-can project", &[PROJECT_EXT]);
                if let Some(dir) = self.project_path.as_ref().and_then(|p| p.parent()) {
                    dlg = dlg.set_directory(dir);
                }
                dlg = dlg.set_file_name(format!("{}.rxproj", self.project_name()));
                match dlg.save_file() {
                    Some(p) => p,
                    None => return false,
                }
            }
        };
        let base = path.parent().map(|d| d.to_path_buf());
        let proj = ProjectFile {
            version: 1,
            layout: self.layout_cache.clone(),
            project: None,
            config: Config::from_app(self, base.as_deref()),
        };
        let written = serde_json::to_string_pretty(&proj)
            .map_err(|e| e.to_string())
            .and_then(|j| std::fs::write(&path, j).map_err(|e| e.to_string()));
        match written {
            Ok(()) => {
                self.push_recent_project(path.to_string_lossy().to_string());
                self.project_path = Some(path.clone());
                self.baseline = self.config_snapshot();
                self.status = format!("project saved: {}", path.display());
                true
            }
            Err(e) => {
                self.status = format!("project save failed: {e}");
                false
            }
        }
    }

    /// Loads a .rxproj file, replacing the current workspace.
    pub fn open_project_path(&mut self, path: &Path) {
        let text = match std::fs::read_to_string(path) {
            Ok(t) => t,
            Err(e) => {
                self.status = format!("project read failed: {e}");
                return;
            }
        };
        match serde_json::from_str::<ProjectFile>(&text) {
            Ok(proj) => {
                self.reset_to_defaults();
                let mut cfg = proj.config;
                cfg.resolve_paths(path.parent());
                cfg.apply(self);
                self.project_path = Some(path.to_path_buf());
                if !proj.layout.is_empty() {
                    self.pending_layout = Some(proj.layout);
                }
                self.baseline = self.config_snapshot();
                self.push_recent_project(path.to_string_lossy().to_string());
                self.status = format!("project loaded: {}", path.display());
            }
            Err(e) => self.status = format!("project ignored: {e}"),
        }
    }

    pub fn open_project_dialog(&mut self) {
        let pick = rfd::FileDialog::new()
            .add_filter("roxy-can project", &[PROJECT_EXT])
            .pick_file();
        if let Some(p) = pick {
            self.open_project_path(&p);
        }
    }

    /// Periodic crash cache; the real project file is never touched.
    pub fn write_autosave(&self) {
        let base = self
            .project_path
            .as_ref()
            .and_then(|p| p.parent())
            .map(|d| d.to_path_buf());
        let proj = ProjectFile {
            version: 1,
            layout: self.layout_cache.clone(),
            project: self
                .project_path
                .as_ref()
                .map(|p| p.to_string_lossy().to_string()),
            config: Config::from_app(self, base.as_deref()),
        };
        if let Ok(json) = serde_json::to_string_pretty(&proj) {
            let _ = std::fs::write(AUTOSAVE_PATH, json);
        }
    }

    /// Restores the crash cache left behind by an abnormal exit.
    pub fn load_autosave(&mut self) -> bool {
        let Ok(text) = std::fs::read_to_string(AUTOSAVE_PATH) else {
            return false;
        };
        let Ok(proj) = serde_json::from_str::<ProjectFile>(&text) else {
            return false;
        };
        self.reset_to_defaults();
        let base = proj
            .project
            .as_deref()
            .map(Path::new)
            .and_then(|p| p.parent());
        let mut cfg = proj.config;
        cfg.resolve_paths(base);
        cfg.apply(self);
        self.project_path = proj.project.map(PathBuf::from);
        if !proj.layout.is_empty() {
            self.pending_layout = Some(proj.layout);
        }
        self.mark_clean();
        self.status = "restored autosave".to_string();
        true
    }

    /// Starts a fresh, completely empty untitled workspace: no DBCs, no
    /// observer windows, no generator entries, default layout.
    pub fn new_project(&mut self) {
        self.reset_to_defaults();
        for c in &mut self.channels {
            c.dbc_path.clear();
            c.dbc = None;
        }
        self.trace_windows.clear();
        self.msg_windows.clear();
        self.stats_windows.clear();
        self.graphics.clear();
        self.data_windows.clear();
        self.tx_list.clear();
        self.subs.clear();
        self.baseline = self.config_snapshot();
        if !self.default_layout.is_empty() {
            self.pending_layout = Some(self.default_layout.clone());
        }
        self.status = "new project".to_string();
    }

    /// Saved projects quit silently (auto-save on exit); untitled ones
    /// only go through the confirmation modal when actually modified.
    pub fn request_quit(&mut self) {
        self.guarded_action(PendingAction::Quit);
    }

    /// Writes the startup driver (last project + recent projects).
    pub fn write_meta(&self) {
        let meta = Meta {
            last_project: self
                .project_path
                .as_ref()
                .map(|p| p.to_string_lossy().to_string()),
            recent_projects: self.recent_projects.clone(),
        };
        if let Ok(json) = serde_json::to_string_pretty(&meta) {
            let _ = std::fs::write(META_PATH, json);
        }
    }

    /// Startup restore: an autosave left by a crash comes first, then the
    /// last project when one is known, then the legacy `roxy-can.json`
    /// import on a very first launch, else defaults.
    pub fn startup_workspace(&mut self) {
        if Path::new(AUTOSAVE_PATH).exists() {
            if self.load_autosave() {
                return;
            }
            let _ = std::fs::remove_file(AUTOSAVE_PATH);
        }
        if let Ok(text) = std::fs::read_to_string(META_PATH) {
            if let Ok(meta) = serde_json::from_str::<Meta>(&text) {
                self.recent_projects = meta.recent_projects;
                if let Some(last) = meta.last_project {
                    let path = PathBuf::from(last);
                    if path.exists() {
                        self.open_project_path(&path);
                    } else {
                        self.status = "last project missing, started empty".to_string();
                    }
                }
                return;
            }
        }
        self.load_config();
    }

    pub fn update(&mut self) {
        if !self.measuring {
            return;
        }
        if self.trace_paused {
            if self.paused_at_us.is_none() {
                self.paused_at_us = Some(self.now_us());
            }
            return;
        }
        if let Some(t) = self.paused_at_us.take() {
            // Skip the paused interval so replay resumes in place
            // instead of fast-forwarding through it.
            self.source.shift_time(self.now_us().saturating_sub(t));
        }
        let now = self.now_us();
        if self.last_tick_us > 0 && now > self.last_tick_us {
            let dt_s = (now - self.last_tick_us) as f64 / 1e6;
            let inst = (self.buf.len() as f64) / dt_s;
            self.frame_rate = if self.frame_rate == 0.0 {
                inst
            } else {
                self.frame_rate * 0.9 + inst * 0.1
            };
        }
        self.last_tick_us = now;
        self.buf.clear();
        self.source.poll(now, &mut self.buf);
        let source_empty = self.buf.is_empty();

        // Generators only transmit in live simulation; replaying an ASC must
        // not mix in synthetic frames from active generator entries.
        if matches!(self.mode, Mode::Virtual) {
            for tx in &mut self.tx_list {
                if tx.active && tx.cycle_us > 0 && tx.next_t_us <= now {
                    while tx.next_t_us <= now {
                        tx.next_t_us += tx.cycle_us;
                    }
                    self.buf.push(CanFrame {
                        t_us: now,
                        channel: tx.channel,
                        id: tx.id,
                        extended: tx.extended,
                        len: tx.len,
                        data: tx.data,
                        dir: Direction::Tx,
                        flags: tx.flags,
                    });
                }
            }
        }

        let replay_done =
            matches!(self.mode, Mode::Replay) && source_empty && self.source.is_done();

        for &f in &self.buf {
            if let Some(w) = &mut self.writer {
                w.write(&f).ok();
            }
            if self.trace.len() >= TRACE_LIMIT {
                self.trace.pop_front();
            }
            self.trace.push_back(f);
            self.frame_counter += 1;
            if f.is_error() {
                // Error frames carry no identifier and no payload; they are
                // intentionally kept out of per-message aggregation.
                continue;
            }
            let agg = self.aggs.entry((f.channel, f.id)).or_insert(MessageAgg {
                id: f.id,
                extended: f.extended,
                channel: f.channel,
                dir: f.dir,
                count: 0,
                last_t_us: 0,
                cycle_us: 0.0,
                min_us: f64::MAX,
                max_us: 0.0,
                len: f.len,
                data: f.data,
                flags: f.flags,
            });
            if agg.count > 0 {
                let dt = f.t_us.saturating_sub(agg.last_t_us) as f64;
                agg.cycle_us = if agg.count == 1 {
                    dt
                } else {
                    agg.cycle_us * 0.9 + dt * 0.1
                };
                if dt < agg.min_us {
                    agg.min_us = dt;
                }
                if dt > agg.max_us {
                    agg.max_us = dt;
                }
            }
            agg.count += 1;
            agg.last_t_us = f.t_us;
            agg.channel = f.channel;
            agg.dir = f.dir;
            agg.len = f.len;
            agg.data = f.data;
            agg.flags = f.flags;
            let db = self
                .channels
                .get(f.channel as usize)
                .and_then(|c| c.dbc.as_ref());
            if let Some(db) = db {
                for (name, phys, unit) in db.decode_signals(&f) {
                    let key = (f.channel, f.id, name.clone());
                    let Some(entry) = self.subs.get_mut(&key) else {
                        continue;
                    };
                    entry.latest = phys;
                    entry.unit = unit;
                    entry.last_update_us = f.t_us;
                    if f.t_us >= entry.last_sample_us + SAMPLE_INTERVAL_US
                        || entry.history.is_empty()
                    {
                        entry.history.push_back((f.t_us, phys));
                        entry.last_sample_us = f.t_us;
                        if entry.history.len() > HISTORY_LIMIT {
                            entry.history.pop_front();
                        }
                        if phys < entry.min {
                            entry.min = phys;
                        }
                        if phys > entry.max {
                            entry.max = phys;
                        }
                        entry.sum += phys;
                        entry.n += 1;
                        entry.avg = entry.sum / entry.n as f64;
                    }
                }
            }
        }

        if replay_done {
            self.measuring = false;
            self.close_writer();
            let dur = self.source.duration().unwrap_or(0) as f64 / 1e6;
            self.status = format!("replay finished at {dur:.2}s");
        }
    }

    pub fn subscribe(&mut self, key: (u8, u32, String)) {
        if !self.subs.contains_key(&key) {
            let color = self.color_counter;
            self.color_counter += 1;
            self.subs.insert(
                key,
                Subscription {
                    latest: 0.0,
                    unit: String::new(),
                    min: f64::INFINITY,
                    max: f64::NEG_INFINITY,
                    avg: 0.0,
                    sum: 0.0,
                    n: 0,
                    last_update_us: 0,
                    last_sample_us: 0,
                    history: VecDeque::new(),
                    color,
                },
            );
        }
    }

    /// Drops the subscription if no Data/Graphics window references the signal anymore.
    pub fn prune_signal(&mut self, key: &(u8, u32, String)) {
        let in_use = self
            .graphics
            .iter()
            .any(|g| g.signals.iter().any(|s| &s.key == key))
            || self
                .data_windows
                .iter()
                .any(|d| d.signals.iter().any(|s| &s.key == key));
        if !in_use {
            self.subs.remove(key);
        }
    }

    /// Adds or removes a signal in a Graphics/Data window's signal list
    /// (used by the Signal Selection popup).
    pub fn set_win_signal(&mut self, target: PopupTarget, key: (u8, u32, String), on: bool) {
        let signals: Option<&mut Vec<GfxSignal>> = match target {
            PopupTarget::Graphics(i) => self.graphics.get_mut(i).map(|w| &mut w.signals),
            PopupTarget::Data(i) => self.data_windows.get_mut(i).map(|w| &mut w.signals),
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
            });
        } else {
            signals.retain(|s| s.key != key);
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
        });
    }

    pub fn new_stats_window(&mut self) {
        self.stats_counter += 1;
        self.stats_windows.push(StatsWin {
            name: format!("Statistics {}", self.stats_counter),
            opened: true,
            scope: SigScope::All,
            manual: HashSet::new(),
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
        });
    }

    pub fn new_data_window(&mut self) {
        self.data_counter += 1;
        self.data_windows.push(DataWindow {
            name: format!("Data {}", self.data_counter),
            signals: Vec::new(),
            opened: true,
            viz_bar: true,
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
    /// and ID/name substring.
    pub fn trace_match(&self, w: &TraceWin, f: &CanFrame) -> bool {
        if !Self::scope_match(w.scope, &w.manual, f.channel, f.id) {
            return false;
        }
        match w.dir {
            1 => {
                if !matches!(f.dir, Direction::Rx) {
                    return false;
                }
            }
            2 => {
                if !matches!(f.dir, Direction::Tx) {
                    return false;
                }
            }
            _ => {}
        }
        let name = self.message_name(f.channel, f.id);
        if w.dbc_only && name.is_none() {
            return false;
        }
        let q = w.filter.trim();
        if !q.is_empty() {
            let q = q.to_ascii_uppercase();
            let hex = format!("{:X}", f.id);
            let in_name = name.is_some_and(|n| n.to_ascii_uppercase().contains(&q));
            if !hex.contains(&q) && !in_name {
                return false;
            }
        }
        true
    }

    /// Enables or disables every generator message of one bus; freshly
    /// enabled messages restart their cycle immediately.
    pub fn set_bus_tx(&mut self, ch: u8, on: bool) {
        for t in &mut self.tx_list {
            if t.channel == ch && t.active != on {
                t.active = on;
                if on {
                    t.next_t_us = 0;
                }
            }
        }
    }

    pub fn add_tx(&mut self, channel: u8, id: u32) {
        if self
            .tx_list
            .iter()
            .any(|t| t.channel == channel && t.id == id)
        {
            return;
        }
        let (name, len) = self
            .channel_dbc(channel)
            .and_then(|db| db.messages.get(&id))
            .map(|m| (m.name.clone(), m.dlc.min(MAX_CAN_FD_LEN as u64) as u8))
            .unwrap_or_else(|| (format!("{id:X}"), 8));
        let data_text = vec!["00"; len as usize].join(" ");
        self.tx_list.push(TxMsg {
            channel,
            id,
            extended: id > 0x7FF,
            name,
            len,
            data: [0; MAX_CAN_FD_LEN],
            flags: if len > 8 { FrameFlags::FD } else { FrameFlags::NONE },
            data_text,
            cycle_us: 100_000,
            active: false,
            next_t_us: 0,
        });
    }

    pub fn bus_counter(&self) -> usize {
        self.bus_counter
    }

    pub fn set_bus_counter(&mut self, n: usize) {
        self.bus_counter = n;
    }

    pub fn window_counters(&self) -> crate::config::Counters {
        crate::config::Counters {
            trace: self.trace_counter,
            msg: self.msg_counter,
            stats: self.stats_counter,
            graphics: self.graphics_counter,
            data: self.data_counter,
        }
    }

    pub fn set_window_counters(&mut self, c: crate::config::Counters) {
        self.trace_counter = c.trace.max(self.trace_windows.len());
        self.msg_counter = c.msg.max(self.msg_windows.len());
        self.stats_counter = c.stats.max(self.stats_windows.len());
        self.graphics_counter = c.graphics.max(self.graphics.len());
        self.data_counter = c.data.max(self.data_windows.len());
    }
}

impl Default for App {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn record_survives_start_and_writes_frames() {
        let mut app = App::new();
        let path = std::env::temp_dir().join("roxy_can_record_test.asc");
        app.record_path = path.to_string_lossy().to_string();
        app.toggle_record();
        assert!(app.recording);
        assert!(
            !app.tx_list.is_empty(),
            "DBC messages pre-populate the generator"
        );
        for tx in &mut app.tx_list {
            tx.active = true;
            tx.cycle_us = 10_000;
        }
        app.start_virtual();
        assert!(app.recording, "Start must not clear the Record checkbox");
        for _ in 0..12 {
            std::thread::sleep(std::time::Duration::from_millis(11));
            app.update();
        }
        app.stop();
        let actual = app.last_record.clone();
        assert!(actual.ends_with(".asc"), "no record file: {actual}");
        assert!(
            actual.contains("roxy_can_record_test_"),
            "generated name should keep the user base: {actual}"
        );
        let content = std::fs::read_to_string(&actual).unwrap();
        let frames = crate::log::asc::parse_asc(&content);
        assert!(frames.len() >= 10, "expected frames, got {}", frames.len());
        if let Some(dir) = std::path::Path::new(&actual).parent() {
            if let Ok(rd) = std::fs::read_dir(dir) {
                for e in rd.flatten() {
                    let n = e.file_name().to_string_lossy().to_string();
                    if n.starts_with("roxy_can_record_test") {
                        std::fs::remove_file(e.path()).ok();
                    }
                }
            }
        }
    }

    #[test]
    fn replay_after_recorded_simulation_creates_no_second_file() {
        let mut app = App::new();
        let path = std::env::temp_dir().join("roxy_can_replay_rec_test.asc");
        app.record_path = path.to_string_lossy().to_string();
        app.toggle_record();
        for tx in &mut app.tx_list {
            tx.active = true;
            tx.cycle_us = 10_000;
        }
        app.start_virtual();
        for _ in 0..12 {
            std::thread::sleep(std::time::Duration::from_millis(11));
            app.update();
        }
        app.stop();
        let first = app.last_record.clone();
        assert!(!first.is_empty(), "simulation should have recorded a file");
        app.asc_path = first.clone();
        app.replay();
        assert!(!app.recording, "replay must drop the Record state");
        assert_eq!(
            app.last_record, first,
            "replay must not open a second record file"
        );
        app.stop();
        std::fs::remove_file(&first).ok();
    }

    #[test]
    fn loading_asc_does_not_start_replay() {
        let mut app = App::new();
        let path = std::env::temp_dir().join("roxy_can_load_asc_test.asc");
        app.record_path = path.to_string_lossy().to_string();
        app.toggle_record();
        for tx in &mut app.tx_list {
            tx.active = true;
            tx.cycle_us = 10_000;
        }
        app.start_virtual();
        for _ in 0..12 {
            std::thread::sleep(std::time::Duration::from_millis(11));
            app.update();
        }
        app.stop();
        let first = app.last_record.clone();
        app.load_asc(&first);
        assert!(!app.measuring, "loading must not start playback");
        assert!(!app.asc_frames.is_empty(), "frames should be cached");
        app.replay();
        assert!(app.measuring, "replay starts on demand");
        assert!(matches!(app.mode, Mode::Replay));
        app.stop();
        std::fs::remove_file(&first).ok();
    }

    #[test]
    fn aggregates_frames_per_message_id() {
        let mut app = App::new();
        let tx = app
            .tx_list
            .iter_mut()
            .find(|t| t.id == 0x100)
            .expect("EngineStatus pre-populated in generator");
        tx.active = true;
        tx.cycle_us = 10_000;
        app.start_virtual();
        for _ in 0..8 {
            std::thread::sleep(std::time::Duration::from_millis(12));
            app.update();
        }
        let agg = app
            .aggs
            .get(&(0, 0x100))
            .expect("EngineStatus aggregated on CAN1");
        assert!(agg.count >= 5, "expected several frames, got {}", agg.count);
        assert!(
            (agg.cycle_us / 1000.0 - 12.0).abs() < 8.0,
            "cycle should track the update cadence, got {}ms",
            agg.cycle_us / 1000.0
        );
        assert!(agg.min_us > 0.0, "min cycle should be recorded");
        assert!(agg.max_us >= agg.min_us, "max cycle >= min cycle");
        app.stop();
    }

    #[test]
    fn tx_generator_emits_frames() {
        let mut app = App::new();
        app.add_tx(0, 0x777);
        let tx = app.tx_list.last_mut().expect("tx entry added");
        assert_eq!(tx.id, 0x777);
        assert_eq!(tx.channel, 0);
        assert!(!tx.active, "new entries start inactive");
        tx.cycle_us = 20_000;
        tx.active = true;
        app.start_virtual();
        app.update();
        assert!(
            app.trace
                .iter()
                .any(|f| f.id == 0x777 && matches!(f.dir, Direction::Tx)),
            "expected a Tx frame from the generator"
        );
        assert!(
            app.aggs.get(&(0, 0x777)).is_some(),
            "generator frames aggregate"
        );
        app.stop();
    }

    #[test]
    fn export_trace_writes_parseable_asc() {
        let mut app = App::new();
        for tx in &mut app.tx_list {
            tx.active = true;
            tx.cycle_us = 10_000;
        }
        app.start_virtual();
        for _ in 0..6 {
            std::thread::sleep(std::time::Duration::from_millis(11));
            app.update();
        }
        let n = app.trace.len();
        assert!(n > 0, "expected captured frames");
        let path = std::env::temp_dir().join("roxy_can_export_test.asc");
        let path_str = path.to_string_lossy().to_string();
        app.export_trace(0, &path_str);
        let content = std::fs::read_to_string(&path).unwrap();
        let frames = crate::log::asc::parse_asc(&content);
        assert_eq!(frames.len(), n, "exported frame count mismatch");
        std::fs::remove_file(&path).ok();
        app.stop();
    }

    #[test]
    fn two_channels_aggregate_separately() {
        let mut app = App::new();
        assert_eq!(app.channels.len(), 2);
        for (ch, c) in app.channels.iter().enumerate() {
            assert!(c.dbc.is_some(), "CAN{} should load its DBC", ch + 1);
        }
        assert!(
            app.tx_list.iter().any(|t| t.channel == 0 && t.id == 0x100)
                && app.tx_list.iter().any(|t| t.channel == 1 && t.id == 0xC8),
            "generator pre-populated on both buses"
        );
        for tx in &mut app.tx_list {
            if (tx.channel == 0 && tx.id == 0x100) || (tx.channel == 1 && tx.id == 0xC8) {
                tx.active = true;
                tx.cycle_us = 10_000;
            }
        }
        app.start_virtual();
        for _ in 0..6 {
            std::thread::sleep(std::time::Duration::from_millis(11));
            app.update();
        }
        let a = app.aggs.get(&(0, 0x100)).expect("CAN1 aggregate");
        let b = app.aggs.get(&(1, 0xC8)).expect("CAN2 aggregate");
        assert!(a.count >= 3, "CAN1 frames: {}", a.count);
        assert!(b.count >= 3, "CAN2 frames: {}", b.count);
        assert!(app.trace.iter().any(|f| f.channel == 1 && f.id == 0xC8));
        app.stop();
    }

    #[test]
    fn csv_exports_match_window_state() {
        let mut app = App::new();
        let db = app.channels[0].dbc.as_ref().expect("sample DBC loaded");
        let id = db.order[0];
        let sig = db.messages[&id].signals[0].name.clone();
        let key = (0u8, id, sig);
        app.subscribe(key.clone());
        app.graphics[0].signals.push(GfxSignal {
            key: key.clone(),
            visible: true,
        });
        app.data_windows[0].signals.push(GfxSignal {
            key: key.clone(),
            visible: true,
        });
        for tx in &mut app.tx_list {
            tx.active = true;
            tx.cycle_us = 10_000;
        }
        app.start_virtual();
        for _ in 0..8 {
            std::thread::sleep(std::time::Duration::from_millis(11));
            app.update();
        }
        let dir = std::env::temp_dir();
        let stats = dir.join("roxy_stats_test.csv");
        app.export_stats_csv(0, &stats.to_string_lossy());
        let s = std::fs::read_to_string(&stats).unwrap();
        assert!(s.lines().count() > 1, "stats should have data rows");

        let msgs = dir.join("roxy_msgs_test.csv");
        app.export_messages_csv(0, &msgs.to_string_lossy());
        let m = std::fs::read_to_string(&msgs).unwrap();
        assert!(m.lines().count() > 1, "messages should have data rows");

        let gfx = dir.join("roxy_gfx_test.csv");
        app.export_graphics_csv(0, &gfx.to_string_lossy());
        let g = std::fs::read_to_string(&gfx).unwrap();
        assert!(g.contains(&key.2), "graphics history names the signal");

        let data = dir.join("roxy_data_test.csv");
        app.export_data_csv(0, &data.to_string_lossy());
        let d = std::fs::read_to_string(&data).unwrap();
        assert!(d.contains(&key.2), "data snapshot names the signal");

        for p in [&stats, &msgs, &gfx, &data] {
            std::fs::remove_file(p).ok();
        }
        app.stop();
    }

    #[test]
    fn trace_filter_matches_by_name_id_and_direction() {
        let app = App::new();
        let rx = CanFrame {
            t_us: 0,
            channel: 0,
            id: 0x100,
            extended: false,
            len: 8,
            data: [0; MAX_CAN_FD_LEN],
            dir: Direction::Rx,
            flags: FrameFlags::NONE,
        };
        let tx = CanFrame {
            id: 0x320,
            dir: Direction::Tx,
            ..rx
        };
        let rx_ch1 = CanFrame { channel: 1, ..rx };
        let unknown = CanFrame { id: 0x777, ..rx };
        let mut w = app.trace_windows[0].clone();
        assert!(app.trace_match(&w, &rx));
        assert!(app.trace_match(&w, &unknown));

        w.filter = "eng".to_string();
        assert!(app.trace_match(&w, &rx), "name match is case-insensitive");
        assert!(!app.trace_match(&w, &unknown));

        w.filter = "77".to_string();
        assert!(app.trace_match(&w, &unknown), "hex id substring");
        assert!(!app.trace_match(&w, &rx));

        w.filter.clear();
        w.dir = 1;
        assert!(app.trace_match(&w, &rx));
        assert!(!app.trace_match(&w, &tx), "Rx-only filter drops Tx frames");

        w.dir = 0;
        w.dbc_only = true;
        assert!(app.trace_match(&w, &rx));
        assert!(!app.trace_match(&w, &unknown), "DBC-only drops unknown IDs");

        w.dbc_only = false;
        w.scope = SigScope::Bus(0);
        assert!(app.trace_match(&w, &rx), "Bus scope passes its own bus");
        assert!(!app.trace_match(&w, &rx_ch1), "Bus scope drops other buses");

        w.scope = SigScope::Manual;
        w.manual.insert((0, 0x320));
        assert!(
            !app.trace_match(&w, &rx),
            "Manual selection drops unselected IDs"
        );
        assert!(
            app.trace_match(&w, &tx),
            "Manual selection passes the chosen ID"
        );
        w.manual.clear();
        assert!(
            !app.trace_match(&w, &tx),
            "empty Manual selection passes nothing"
        );
        w.scope = SigScope::All;
        assert!(app.trace_match(&w, &rx), "All scope passes everything");
    }

    #[test]
    fn channels_can_be_added_removed_and_renamed() {
        let mut app = App::new();
        assert_eq!(app.channels.len(), 2);
        app.channels[0].name = "Powertrain".to_string();
        assert_eq!(app.channel_name(0), "Powertrain");

        app.add_channel();
        assert_eq!(app.channels.len(), 3);
        assert_eq!(app.channel_name(2), "CAN3");

        app.aggs.insert(
            (1, 0x100),
            MessageAgg {
                id: 0x100,
                channel: 1,
                extended: false,
                dir: Direction::Rx,
                count: 1,
                last_t_us: 0,
                cycle_us: 0.0,
                min_us: 0.0,
                max_us: 0.0,
                len: 8,
                data: [0; MAX_CAN_FD_LEN],
                flags: FrameFlags::NONE,
            },
        );
        app.trace_windows[0].manual.insert((2, 0x200));
        app.trace_windows[0].scope = SigScope::Bus(2);
        let w = app.trace_windows[0].clone();

        app.remove_channel(0);
        assert_eq!(app.channels.len(), 2);
        assert_eq!(app.channel_name(0), "CAN2", "remaining buses shift down");
        assert!(app.aggs.contains_key(&(0, 0x100)), "agg remapped 1 -> 0");
        assert!(
            app.trace_windows[0].manual.contains(&(1, 0x200)),
            "filter remapped 2 -> 1"
        );
        assert_eq!(w.scope, SigScope::Bus(2), "cloned window is untouched");
        assert_eq!(
            app.trace_windows[0].scope,
            SigScope::Bus(1),
            "Bus scope indices shift with the channels"
        );

        while app.channels.len() > 1 {
            app.remove_channel(0);
        }
        assert_eq!(app.channels.len(), 1, "last bus cannot be removed");
    }

    #[test]
    fn replay_position_tracks_playback() {
        let mut app = App::new();
        let path = std::env::temp_dir().join("roxy_can_seek_test.asc");
        app.record_path = path.to_string_lossy().to_string();
        app.toggle_record();
        for tx in &mut app.tx_list {
            tx.active = true;
            tx.cycle_us = 10_000;
        }
        app.start_virtual();
        for _ in 0..12 {
            std::thread::sleep(std::time::Duration::from_millis(11));
            app.update();
        }
        app.stop();
        let file = app.last_record.clone();
        app.load_asc(&file);
        app.replay();
        let (pos0, dur) = app.replay_position().expect("replay has a timeline");
        assert!(dur > 0.0, "timeline covers the whole log");
        assert!(pos0 < 0.01, "playback starts at the beginning");
        // The first poll only anchors the replay clock, so a second
        // cycle is needed before the position actually advances.
        std::thread::sleep(std::time::Duration::from_millis(15));
        app.update();
        std::thread::sleep(std::time::Duration::from_millis(15));
        app.update();
        let (pos1, _) = app.replay_position().unwrap();
        assert!(pos1 > pos0, "position advances while replaying");
        app.stop();
        std::fs::remove_file(&file).ok();
    }

    #[test]
    fn replay_does_not_inject_generator_frames() {
        let mut app = App::new();
        let path = std::env::temp_dir().join("roxy_can_replay_pure.asc");
        std::fs::write(
            &path,
            "date Thu Jan 01\nbase hex  timestamps hex\ninternal events logged\n\
             0.000000 Start of measurement\n\
             0.001000  1  100  Tx  8  00 00 00 00 00 00 00 00\n",
        )
        .unwrap();
        app.load_asc(&path.to_string_lossy());
        app.tx_list[0].active = true;
        app.tx_list[0].cycle_us = 1_000;
        app.tx_list[0].data = [0xDE; MAX_CAN_FD_LEN];
        app.replay();
        for _ in 0..6 {
            std::thread::sleep(std::time::Duration::from_millis(11));
            app.update();
        }
        assert!(
            !app.trace.iter().any(|f| f.data == [0xDE; MAX_CAN_FD_LEN]),
            "replay must not mix in generator frames"
        );
        app.stop();
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn replay_speed_steps_along_the_ladder() {
        let mut app = App::new();
        assert_eq!(app.replay_speed, 1.0);
        app.step_replay_speed(1);
        assert_eq!(app.replay_speed, 2.0, "one notch faster");
        app.step_replay_speed(-1);
        app.step_replay_speed(-1);
        assert_eq!(app.replay_speed, 0.5, "two notches slower");
        app.step_replay_speed(-1);
        assert_eq!(app.replay_speed, 0.5, "clamped at the slow end");
        app.step_replay_speed(99);
        assert_eq!(app.replay_speed, 4.0, "clamped at the fast end");
    }

    #[test]
    fn starting_clears_the_previous_pause() {
        let mut app = App::new();
        app.start_virtual();
        app.trace_paused = true;
        app.stop();
        app.start_virtual();
        assert!(!app.trace_paused, "a new start must not stay paused");
        app.update();
        app.stop();
    }

    #[test]
    fn switching_run_mode_stops_a_running_measurement() {
        let mut app = App::new();
        app.start_virtual();
        assert!(app.measuring);
        app.switch_run_mode(Mode::Replay);
        assert!(!app.measuring, "switching mode stops the run");
        assert!(matches!(app.run_mode, Mode::Replay));
        app.switch_run_mode(Mode::Replay);
        assert!(matches!(app.run_mode, Mode::Replay), "no-op keeps the mode");
    }

    #[test]
    fn recent_lists_dedup_and_cap() {
        let mut app = App::new();
        for i in 0..10 {
            app.push_recent_dbc(format!("f{i}.dbc"));
        }
        assert_eq!(app.recent_dbc.len(), 8, "recent list is capped");
        assert_eq!(app.recent_dbc[0], "f9.dbc", "newest first");
        app.push_recent_dbc("f3.dbc".to_string());
        assert_eq!(app.recent_dbc[0], "f3.dbc", "reopen moves to the front");
        assert_eq!(
            app.recent_dbc.iter().filter(|p| *p == "f3.dbc").count(),
            1,
            "no duplicates"
        );
    }

    #[test]
    fn dropping_a_dbc_loads_it_into_the_first_bus() {
        let mut app = App::new();
        app.open_dropped(std::path::Path::new("assets/motbus.dbc"));
        assert_eq!(app.channels[0].dbc_path, "assets/motbus.dbc");
        assert!(
            app.channels[0].dbc.is_some(),
            "dropped DBC is parsed into the first bus"
        );
        assert_eq!(app.recent_dbc[0], "assets/motbus.dbc");
    }

    #[test]
    fn jump_to_live_resets_plot_offsets() {
        let mut app = App::new();
        app.graphics[0].t_offset_s = -42.0;
        app.jump_to_live();
        assert_eq!(app.graphics[0].t_offset_s, 0.0);
    }

    #[test]
    fn reset_restores_the_default_workspace() {
        let mut app = App::new();
        app.new_trace_window();
        app.push_recent_dbc("keep.dbc".to_string());
        app.start_virtual();
        app.reset_to_defaults();
        assert!(!app.measuring, "reset stops a running measurement");
        assert_eq!(app.trace_windows.len(), 1, "default has one trace window");
        assert!(app.project_path.is_none());
        assert_eq!(app.recent_dbc[0], "keep.dbc", "recents survive the reset");
    }

    #[test]
    fn new_project_starts_completely_empty() {
        let mut app = App::new();
        app.new_project();
        assert!(
            app.channels
                .iter()
                .all(|c| c.dbc.is_none() && c.dbc_path.is_empty()),
            "no DBCs on any bus"
        );
        assert!(app.trace_windows.is_empty());
        assert!(app.msg_windows.is_empty());
        assert!(app.stats_windows.is_empty());
        assert!(app.graphics.is_empty());
        assert!(app.data_windows.is_empty());
        assert!(app.tx_list.is_empty());
        assert!(app.project_path.is_none());
        assert!(!app.is_dirty(), "a fresh project has nothing to save");
    }

    #[test]
    fn untouched_workspace_quits_without_prompting() {
        let mut app = App::new();
        app.request_quit();
        assert!(app.quit, "clean untitled workspace quits silently");
        assert!(app.pending_action.is_none());

        let mut app = App::new();
        app.new_trace_window();
        app.request_quit();
        assert!(!app.quit, "modified workspace must confirm first");
        assert_eq!(app.pending_action, Some(crate::app::PendingAction::Quit));
    }

    #[test]
    fn autosave_round_trips_the_workspace() {
        let mut app = App::new();
        let path = std::env::temp_dir().join("roxy_can_autosave.rxproj");
        assert!(app.save_project(Some(path.clone())));
        app.trace_windows[0].filter = "Motor".to_string();
        app.layout_cache = "[Window][Dockspace]\n".to_string();
        app.write_autosave();

        let mut restored = App::new();
        assert!(restored.load_autosave());
        assert_eq!(restored.project_path.as_deref(), Some(path.as_path()));
        assert_eq!(restored.trace_windows[0].filter, "Motor");
        assert_eq!(
            restored.pending_layout.as_deref(),
            Some("[Window][Dockspace]\n")
        );
        assert!(!restored.is_dirty(), "restored autosave starts clean");
        std::fs::remove_file(&path).ok();
        std::fs::remove_file(crate::config::AUTOSAVE_PATH).ok();
    }

    #[test]
    fn desktop_switching_restores_window_visibility() {
        let mut app = App::new();
        assert!(app.trace_windows[0].opened);
        assert!(app.show_network);
        app.add_desktop();
        assert!(!app.trace_windows[0].opened, "a new desktop starts empty");
        assert!(!app.show_network, "a new desktop hides all panels");
        app.switch_desktop(0);
        assert_eq!(app.active_desktop, 0);
        assert!(app.trace_windows[0].opened, "desktop 1 reopens its windows");
        assert!(app.show_network, "desktop 1 restores the panel state");
        app.switch_desktop(1);
        assert!(!app.trace_windows[0].opened, "desktop 2 keeps it closed");
        assert!(!app.show_network);
    }

    #[test]
    fn desktops_round_trip_through_config() {
        let mut app = App::new();
        app.add_desktop();
        app.switch_desktop(0);
        let cfg = Config::from_app(&app, None);
        let mut restored = App::new();
        cfg.apply(&mut restored);
        assert_eq!(restored.desktops.len(), 2);
        assert_eq!(restored.desktops[0].name, "Desktop 1");
        assert_eq!(restored.desktops[1].name, "Desktop 2");
        assert_eq!(restored.active_desktop, 0);
        assert_eq!(
            restored.desktops[0].open_windows.len(),
            app.desktops[0].open_windows.len()
        );
    }

    #[test]
    fn delete_desktop_keeps_at_least_one() {
        let mut app = App::new();
        app.delete_desktop(0);
        assert_eq!(app.desktops.len(), 1, "the last desktop cannot be deleted");
        app.add_desktop();
        app.add_desktop();
        assert_eq!(app.active_desktop, 2);
        app.delete_desktop(2);
        assert_eq!(app.desktops.len(), 2);
        assert_eq!(app.active_desktop, 1, "deleting the active one falls back");
        app.delete_desktop(0);
        assert_eq!(app.active_desktop, 0, "indices shift when deleting below");
        assert_eq!(app.desktops.len(), 1);
    }

    #[test]
    fn new_project_resets_to_single_desktop() {
        let mut app = App::new();
        app.add_desktop();
        app.rename_desktop(1, "Analysis".to_string());
        app.new_project();
        assert_eq!(app.desktops.len(), 1);
        assert_eq!(app.active_desktop, 0);
        assert_eq!(app.desktops[0].name, "Desktop 1");
    }

    #[test]
    fn project_round_trips_through_an_rxproj_file() {
        let mut app = App::new();
        app.trace_windows[0].filter = "Motor".to_string();
        let path = std::env::temp_dir().join("roxy_can_test.rxproj");
        assert!(app.save_project(Some(path.clone())), "save writes the file");
        assert_eq!(app.project_path.as_deref(), Some(path.as_path()));

        let mut restored = App::new();
        restored.open_project_path(&path);
        assert_eq!(restored.project_path.as_deref(), Some(path.as_path()));
        assert_eq!(restored.trace_windows[0].filter, "Motor");
        assert_eq!(restored.channels.len(), app.channels.len());
        assert_eq!(restored.tx_list.len(), app.tx_list.len());
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn signal_stats_track_min_avg_max() {
        let mut app = App::new();
        let key = {
            let db = app.channel_dbc(0).expect("sample DBC loaded");
            let id = db.order[0];
            (0u8, id, db.messages[&id].signals[0].name.clone())
        };
        app.subscribe(key.clone());
        for tx in &mut app.tx_list {
            tx.active = true;
            tx.cycle_us = 10_000;
        }
        app.start_virtual();
        for _ in 0..8 {
            std::thread::sleep(std::time::Duration::from_millis(11));
            app.update();
        }
        app.stop();
        let sub = app.subs.get(&key).expect("signal subscribed");
        assert!(
            sub.min.is_finite() && sub.max.is_finite(),
            "samples update min/max"
        );
        assert!(sub.min <= sub.avg && sub.avg <= sub.max, "avg within range");
        assert!(!sub.history.is_empty(), "history sampled");
    }

    #[test]
    fn restored_signals_are_resubscribed() {
        let mut app = App::new();
        let key = {
            let db = app.channel_dbc(0).expect("sample DBC loaded");
            let id = db.order[0];
            (0u8, id, db.messages[&id].signals[0].name.clone())
        };
        app.subscribe(key.clone());
        app.graphics[0].signals.push(GfxSignal {
            key: key.clone(),
            visible: true,
        });
        let path = std::env::temp_dir().join("roxy_can_resub.rxproj");
        assert!(app.save_project(Some(path.clone())));
        let mut restored = App::new();
        restored.open_project_path(&path);
        assert!(
            restored.subs.contains_key(&key),
            "restored signal is resubscribed so it is not grey"
        );
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn recording_captures_generator_data_faithfully() {
        let mut app = App::new();
        app.tx_list[0].active = true;
        app.tx_list[0].cycle_us = 10_000;
        let mut payload = [0u8; MAX_CAN_FD_LEN];
        payload[..8].copy_from_slice(&[0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88]);
        app.tx_list[0].data = payload;
        app.record_path = "target/test_record".to_string();
        app.toggle_record();
        app.start_virtual();
        for _ in 0..8 {
            std::thread::sleep(std::time::Duration::from_millis(11));
            app.update();
        }
        app.stop();
        let path = app.last_record.clone();
        assert!(!path.is_empty(), "recording produced a file");
        let content = std::fs::read_to_string(&path).expect("record file readable");
        let parsed = crate::log::asc::parse_asc(&content);
        let (id, ch) = (app.tx_list[0].id, app.tx_list[0].channel);
        let hit = parsed
            .iter()
            .find(|f| f.id == id && f.channel == ch)
            .expect("recorded frames parsed back");
        assert_eq!(
            hit.payload(),
            &[0x11u8, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88][..],
            "recorded data matches what the generator sent"
        );
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn set_bus_tx_toggles_a_whole_bus() {
        let mut app = App::new();
        assert!(app.tx_list.iter().all(|t| !t.active));
        app.set_bus_tx(0, true);
        assert!(
            app.tx_list
                .iter()
                .filter(|t| t.channel == 0)
                .all(|t| t.active),
            "bus 0 fully enabled"
        );
        assert!(
            app.tx_list
                .iter()
                .filter(|t| t.channel == 1)
                .all(|t| !t.active),
            "other buses untouched"
        );
        app.set_bus_tx(0, false);
        assert!(app.tx_list.iter().all(|t| !t.active));
    }
}
