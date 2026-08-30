use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::time::Instant;

use crate::can::frame::{CanFrame, Direction, FrameFlags, MAX_CAN_FD_LEN, dlc2len, len2dlc};
use crate::config::{AUTOSAVE_PATH, Config, META_PATH, Meta, PROJECT_EXT, ProjectFile};
use crate::dbc::SymbolTable;
use crate::log::AscWriter;
use crate::log::open_stream;
use crate::sim::{ValueSrc, eval_phys};
use crate::source::replay::ReplaySource;
use crate::source::virtual_source::VirtualSource;
use crate::source::{FrameSource, FrameStream};

pub const TRACE_LIMIT: usize = 50_000;
pub const TOOLBAR_H: f32 = 54.0;
pub const STATUSBAR_H: f32 = 28.0;
pub const TABSTRIP_H: f32 = 22.0;
/// How far back sampled signal history is kept. Deliberately in seconds, not
/// points: the Graphics window ladder goes up to an hour, and a point-count
/// cap silently dropped the head of the curve mid-run the moment it filled,
/// whatever width the user had chosen.
pub(crate) const HISTORY_SPAN_US: u64 = 3_600_000_000;
const SAMPLE_INTERVAL_US: u64 = 50_000;
/// Frames one window backfill may collect. Bounds a single synchronous pass so
/// a dense hour-wide window cannot lock the UI; the plot shows what it got and
/// asks again on the next change.
const MAX_SCAN_FRAMES: usize = 300_000;
/// Speed ladder shared by the toolbar combo and the slower/faster buttons.
pub const REPLAY_SPEEDS: [f64; 4] = [0.5, 1.0, 2.0, 4.0];
/// Cycle a new generator entry gets when its DBC declares none. A declared
/// value always wins; this is only the invention we fall back to.
const DEFAULT_TX_CYCLE_US: u64 = 100_000;

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

/// Signal samples, kept ascending by timestamp.
///
/// A `VecDeque` sufficed while samples only ever arrived through the playback
/// stream. Window backfill also decodes the stretch *behind* the playhead, so
/// insertion must work from either end: the hot streaming path still appends in
/// O(1), and out-of-order points fall back to a search + splice. Entries stay
/// 16 bytes with no per-node allocation, which matters because an hour of
/// 50 ms sampling is 72 000 points per signal.
#[derive(Debug, Default)]
pub struct SampleCache {
    points: Vec<(u64, f64)>,
}

impl SampleCache {
    pub fn len(&self) -> usize {
        self.points.len()
    }

    pub fn is_empty(&self) -> bool {
        self.points.is_empty()
    }

    pub fn first(&self) -> Option<(u64, f64)> {
        self.points.first().copied()
    }

    pub fn iter(&self) -> std::slice::Iter<'_, (u64, f64)> {
        self.points.iter()
    }

    pub fn clear(&mut self) {
        self.points.clear();
    }

    /// Merges an ascending batch, keeping the buffer sorted and rejecting any
    /// candidate that lands within `min_gap` of a point already held -- or of
    /// another accepted candidate. Returns the values it actually took, so the
    /// caller only folds those into its statistics.
    ///
    /// The gap rule matters: overlapping window requests re-read ground that is
    /// already cached, and without it each re-scan injects near-duplicate
    /// timestamps whose values differ, so the polyline zig-zags between them and
    /// the curve reads as a thick band instead of a line.
    pub fn merge(&mut self, batch: &[(u64, f64)], min_gap: u64) -> Vec<f64> {
        let gap = min_gap.max(1);
        if batch.is_empty() {
            return Vec::new();
        }
        // Fast path: the batch starts beyond everything held, so nothing can
        // collide except its own first element.
        if self
            .points
            .last()
            .is_none_or(|&(last_t, _)| batch[0].0 >= last_t + gap)
        {
            let taken = batch.iter().map(|&(_, v)| v).collect();
            self.points.extend_from_slice(batch);
            return taken;
        }
        let mut out = Vec::with_capacity(self.points.len() + batch.len());
        let mut taken = Vec::with_capacity(batch.len());
        let mut last_emitted: Option<u64> = None;
        let mut j = 0usize;
        for &(et, ev) in &self.points {
            while j < batch.len() && batch[j].0 < et {
                let (bt, bv) = batch[j];
                j += 1;
                let after_prev = last_emitted.is_none_or(|lt| bt >= lt + gap);
                let before_next = bt.saturating_add(gap) <= et;
                if after_prev && before_next {
                    out.push((bt, bv));
                    taken.push(bv);
                    last_emitted = Some(bt);
                }
            }
            out.push((et, ev));
            last_emitted = Some(et);
        }
        while j < batch.len() {
            let (bt, bv) = batch[j];
            j += 1;
            if last_emitted.is_none_or(|lt| bt >= lt + gap) {
                out.push((bt, bv));
                taken.push(bv);
                last_emitted = Some(bt);
            }
        }
        self.points = out;
        taken
    }

    /// Samples inside `[t_from, t_to]`, ascending. This is the only view the
    /// plotter needs: the visible window is a query, not a mutable buffer.
    pub fn range(&self, t_from: u64, t_to: u64) -> &[(u64, f64)] {
        let lo = self.points.partition_point(|(t, _)| *t < t_from);
        let hi = self.points.partition_point(|(t, _)| *t <= t_to);
        &self.points[lo..hi]
    }

    /// Value of the most recent sample at or before `t_us`.
    pub fn at(&self, t_us: u64) -> Option<f64> {
        let idx = self.points.partition_point(|&(ts, _)| ts <= t_us);
        idx.checked_sub(1).map(|i| self.points[i].1)
    }

    /// Drops the oldest samples until what remains fits inside `span_us` of the
    /// newest point.
    pub fn trim_oldest(&mut self, span_us: u64) {
        let Some(&(newest, _)) = self.points.last() else {
            return;
        };
        let horizon = newest.saturating_sub(span_us);
        let stale = self.points.partition_point(|(t, _)| *t < horizon);
        if stale > 0 {
            self.points.drain(..stale);
        }
    }
}

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
    pub history: SampleCache,
    pub color: usize,
}

impl Subscription {
    /// Records one sample and drops whatever has fallen out of the retention
    /// span. `min`/`max`/`avg` stay cumulative over everything sampled since the
    /// run began -- recomputing them every time the oldest point is trimmed would
    /// cost an O(n) rescan per sample.
    fn push_sample(&mut self, t_us: u64, v: f64) {
        // Same spacing rule as a backfill merge. Comparing only against the
        // newest cached point would be wrong after a rewind, where the cache
        // legitimately still holds points ahead of the playhead.
        if self
            .history
            .merge(&[(t_us, v)], SAMPLE_INTERVAL_US)
            .is_empty()
        {
            return;
        }
        self.last_sample_us = t_us;
        self.observe(v);
        self.history.trim_oldest(HISTORY_SPAN_US);
    }

    /// Folds one sampled value into the running statistics.
    fn observe(&mut self, v: f64) {
        if v < self.min {
            self.min = v;
        }
        if v > self.max {
            self.max = v;
        }
        self.sum += v;
        self.n += 1;
        self.avg = self.sum / self.n as f64;
    }

    /// Forgets everything the sampler accumulates, keeping the signal's identity
    /// (`unit`, `color`). Emptying the cache alone is not a reset: the sampler
    /// gates on `t_us >= last_sample_us + SAMPLE_INTERVAL_US`, so a baseline
    /// inherited from the previous run silently rejects every frame until the
    /// playhead climbs past where the old run ended -- which reads as the start
    /// of the trace going missing.
    fn reset_measurement(&mut self) {
        self.history.clear();
        self.clear_accumulators();
    }

    fn clear_accumulators(&mut self) {
        self.latest = 0.0;
        self.min = f64::INFINITY;
        self.max = f64::NEG_INFINITY;
        self.avg = 0.0;
        self.sum = 0.0;
        self.n = 0;
        self.last_update_us = 0;
        self.last_sample_us = 0;
    }

    /// Lets the sampler resume at a scrubbed playhead. The cache is deliberately
    /// left alone: points ahead of the playhead are simply outside the visible
    /// window, and deleting them is what used to blank the curve on a rewind.
    fn resume_sampling_at(&mut self, t_us: u64) {
        if self.last_sample_us > t_us {
            self.last_sample_us = t_us;
        }
    }
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
    /// Draw a dot on each sample when the visible points are sparse enough to
    /// read individually.
    pub show_markers: bool,
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
    /// Signals whose value is generated over time rather than held at whatever
    /// `data` says. Applied on top of the base payload at emit time; `data`
    /// itself is never rewritten by them. See [`crate::sim`].
    pub srcs: Vec<ValueSrc>,
}

/// Whitespace-separated hex bytes, as typed in the generator's data box.
/// Returns None on an empty, over-long or non-hex string.
fn parse_hex_bytes(s: &str) -> Option<Vec<u8>> {
    let toks: Vec<&str> = s.split_whitespace().collect();
    if toks.is_empty() || toks.len() > MAX_CAN_FD_LEN {
        return None;
    }
    let mut out = Vec::with_capacity(toks.len());
    for t in toks {
        out.push(u8::from_str_radix(t, 16).ok()?);
    }
    Some(out)
}

fn hex_text(data: &[u8; MAX_CAN_FD_LEN], len: u8) -> String {
    data[..len.min(MAX_CAN_FD_LEN as u8) as usize]
        .iter()
        .map(|b| format!("{b:02X}"))
        .collect::<Vec<_>>()
        .join(" ")
}

/// Payload for one generated frame: `tx`'s base bytes with every driven signal
/// overwritten by its value at `at_us`. Never mutates `tx`, so `data` and
/// `len` stay whatever the user set and a save captures the base rather than
/// wherever the waveform happened to be.
///
/// Takes `channels` explicitly rather than `&self` so the generator can call it
/// while holding a mutable borrow of `tx_list`.
fn tx_payload(
    channels: &[Channel],
    tx: &TxMsg,
    at_us: u64,
) -> ([u8; MAX_CAN_FD_LEN], u8, FrameFlags) {
    let mut data = tx.data;
    let mut len = tx.len;
    let mut flags = tx.flags;
    let Some((table, msg)) = channels
        .get(tx.channel as usize)
        .and_then(|c| c.dbc.as_ref())
        .and_then(|db| db.messages.get(&tx.id).map(|m| (db, m)))
    else {
        return (data, len, flags);
    };
    for src in &tx.srcs {
        let Some(s) = msg.signals.iter().find(|s| s.name == src.name) else {
            continue;
        };
        // The whole 64-byte array goes in: encode_signal skips bits that fall
        // outside its argument, so a narrowed slice would silently drop them.
        if !table.encode_signal(tx.id, &src.name, eval_phys(src, at_us), &mut data) {
            continue;
        }
        // A driven signal reaching past the base length must widen the frame,
        // or decoders -- including our own plots -- read the value as zero.
        let needed = ((s.start_bit + s.size) as usize)
            .div_ceil(8)
            .min(MAX_CAN_FD_LEN);
        len = len.max(needed as u8);
    }
    if len > 8 {
        // Snap to a length a real FD frame can carry, and say so.
        len = dlc2len(len2dlc(len));
        flags = flags.union(FrameFlags::FD);
    }
    (data, len, flags)
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
    /// Absolute path of the log currently loaded for replay (`.asc`, `.blf`).
    pub log_path: String,
    /// One-line summary from the stream's `describe()`, e.g. "BLF4, 41.2 s".
    pub log_info: Option<String>,
    /// Contiguous log-time span whose frames have already been decoded into the
    /// signal caches. A Graphics window asking for a range outside it triggers a
    /// backfill scan.
    sample_cover: Option<(u64, u64)>,
    pub recent_dbc: Vec<String>,
    pub recent_log: Vec<String>,
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
    /// Set by `stop`: the next Play re-opens the log from zero instead of
    /// resuming wherever the scrub bar left the playhead.
    pub replay_reset_pending: bool,
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
    /// Generator row whose value-source parameters the modal is editing:
    /// index into `tx_list` plus the DBC signal name.
    pub src_edit: Option<(usize, String)>,
    /// Edit buffer for the step sequence, kept on `App` so the text box keeps
    /// its caret and partial input across frames.
    pub src_seq_buf: String,
    pub last_tick_us: u64,
    /// Simulation clock: accumulates only while measuring and unpaused.
    /// Generator frames are stamped on it and their signal values are evaluated
    /// from it, so a pause freezes the bus in place instead of letting it jump
    /// phase. Replay polling still uses the wall clock (`last_tick_us`).
    pub sim_t_us: u64,
    /// `now_us()` at the previous accepted tick, the reference for `sim_t_us`.
    sim_prev_us: u64,
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
            log_path: String::new(),
            log_info: None,
            sample_cover: None,
            recent_dbc: Vec::new(),
            recent_log: Vec::new(),
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
            replay_reset_pending: false,
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
            src_edit: None,
            src_seq_buf: String::new(),
            last_tick_us: 0,
            sim_t_us: 0,
            sim_prev_us: 0,
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
    /// dropdown; replay falls back to a file picker when no log is loaded.
    pub fn start_selected(&mut self) {
        match self.run_mode {
            Mode::Virtual => self.start_virtual(),
            Mode::Replay => {
                if !self.can_replay() {
                    self.pick_log();
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
        !self.log_path.trim().is_empty() || !self.last_record.trim().is_empty()
    }

    pub fn stop(&mut self) {
        self.measuring = false;
        self.close_writer();
        self.replay_reset_pending = true;
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
        self.sim_t_us = 0;
        self.sim_prev_us = 0;
        self.last_tick_us = 0;
        self.frame_counter = 0;
        // A fresh start must not inherit the previous run's pause state.
        self.trace_paused = false;
        self.paused_at_us = None;
        self.trace.clear();
        self.aggs.clear();
        self.sample_cover = None;
        for tx in &mut self.tx_list {
            tx.next_t_us = 0;
        }
        for sub in self.subs.values_mut() {
            sub.reset_measurement();
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
            Some("asc") | Some("blf") | Some("mf4") => {
                self.load_log(&path.to_string_lossy());
            }
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

    pub fn push_recent_log(&mut self, path: String) {
        push_recent(&mut self.recent_log, path);
    }

    pub fn pick_log(&mut self) {
        if let Some(p) = rfd::FileDialog::new()
            .set_title("Open CAN log")
            .add_filter("CAN logs", &["asc", "blf"])
            .pick_file()
        {
            self.load_log(&p.to_string_lossy());
        }
    }

    /// Validates a log and selects it for replay without starting playback.
    /// The stream is dropped here so the mmap handle closes before `replay`
    /// reopens it — Windows refuses to move/rename a mapped file.
    ///
    /// Refused while a replay is running: `log_path`, the status bar and the
    /// scrub bar's length would describe the new file while the live source
    /// keeps streaming the old one.
    pub fn load_log(&mut self, path: &str) {
        if self.measuring && matches!(self.mode, Mode::Replay) {
            self.status = "stop the replay before loading another log".to_string();
            return;
        }
        let p = std::path::Path::new(path);
        match open_stream(p) {
            Ok(stream) => {
                let name = p
                    .file_name()
                    .map(|s| s.to_string_lossy().into_owned())
                    .unwrap_or_else(|| path.to_string());
                let info = stream.describe();
                self.log_path = path.to_string();
                self.log_info = if info.is_empty() {
                    None
                } else {
                    Some(info.clone())
                };
                // A newly selected log must be opened from scratch; without
                // this, Play after a finished run would resume the previous
                // file's source while the UI named the new one.
                self.replay_reset_pending = true;
                self.status = if info.is_empty() {
                    format!("loaded {name}")
                } else {
                    format!("loaded {name} [{info}]")
                };
                self.push_recent_log(path.to_string());
            }
            Err(e) => self.status = format!("log load failed [{path}]: {e}"),
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
            for (t, v) in sub.history.iter() {
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
        let path = {
            let p = self.log_path.trim();
            if p.is_empty() {
                self.last_record.clone()
            } else {
                p.to_string()
            }
        };
        if path.trim().is_empty() {
            self.status = "replay: no log selected".to_string();
            return;
        }
        match open_stream(Path::new(&path)) {
            Ok(stream) => self.start_replay(stream),
            Err(e) => self.status = format!("replay failed [{path}]: {e}"),
        }
    }

    fn start_replay(&mut self, stream: Box<dyn FrameStream>) {
        let info = stream.describe();
        self.close_writer();
        // Replay just re-emits an existing log; recording it would
        // only duplicate the file, so drop the Record state.
        self.recording = false;
        let mut source = ReplaySource::new(stream);
        source.set_speed(self.replay_speed);
        self.source = Box::new(source);
        self.mode = Mode::Replay;
        self.run_mode = Mode::Replay;
        self.replay_reset_pending = false;
        self.reset_time();
        self.measuring = true;
        let tag = if info.is_empty() {
            String::new()
        } else {
            format!(" [{info}]")
        };
        self.status = format!("replaying{tag} at {}x", self.replay_speed);
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

    /// Moves the replay playhead to `t_s` seconds. Works while running,
    /// paused, or stopped after the log ran out; the log's own duration bounds
    /// the request so a drag past the right edge lands on the last frame.
    pub fn seek_replay_seconds(&mut self, t_s: f64) {
        if !matches!(self.mode, Mode::Replay) {
            return;
        }
        let dur_us = self.source.duration();
        let target = match dur_us {
            Some(d) => ((t_s.max(0.0) * 1e6) as u64).min(d),
            None => (t_s.max(0.0) * 1e6) as u64,
        };
        match self.source.set_position_us(target) {
            Some(landed) => {
                self.rewind_samples_to(landed);
                let dur = dur_us.map(|d| d as f64 / 1e6);
                self.status = match dur {
                    Some(d) => format!("seek {:.2} / {:.2} s", landed as f64 / 1e6, d),
                    None => format!("seek {:.2} s", landed as f64 / 1e6),
                };
            }
            None => self.status = "seek: past end of log".to_string(),
        }
    }

    /// Decodes whatever frames cover `[t_from_us, t_to_us]` into the signal
    /// caches, unless an earlier scan already read that span.
    ///
    /// This is what makes the plot independent of the playback cursor. Deriving
    /// samples only as playback walks past them meant a Graphics window showing
    /// ground the cursor had not reached contained no points at all -- which is
    /// exactly what a forward scrub looked like.
    pub fn ensure_samples_in(&mut self, t_from_us: u64, t_to_us: u64) {
        if t_to_us <= t_from_us {
            return;
        }
        // Read only the edges the cache does not already cover. During playback
        // the visible window slides forward by tens of milliseconds every frame,
        // and re-reading the whole window each time would both cost that much
        // again and overwrite the cache's spacing.
        let (cov_lo, cov_hi) = self.sample_cover.unwrap_or((t_from_us, t_from_us));
        let mut edges = Vec::new();
        if t_from_us < cov_lo {
            edges.push((t_from_us, t_to_us.min(cov_lo)));
        }
        if t_to_us > cov_hi {
            edges.push((cov_hi.max(t_from_us), t_to_us));
        }
        for (from, to) in edges {
            if to > from {
                self.backfill(from, to);
            }
        }
    }

    /// One scan + merge over a span known to be uncovered.
    fn backfill(&mut self, t_from_us: u64, t_to_us: u64) {
        let mut frames = Vec::new();
        let capped = !self
            .source
            .scan_range(t_from_us, t_to_us, MAX_SCAN_FRAMES, &mut frames);
        // A scan-local stride: the shared per-signal baseline sits at the
        // playhead and would reject every point that lies behind it.
        let mut stride: HashMap<(u8, u32, String), u64> = HashMap::new();
        let mut batches: HashMap<(u8, u32, String), Vec<(u64, f64)>> = HashMap::new();
        for f in &frames {
            for (key, phys, _) in self.subscribed_values(f) {
                if stride
                    .get(&key)
                    .is_some_and(|&lt| f.t_us < lt + SAMPLE_INTERVAL_US)
                {
                    continue;
                }
                stride.insert(key.clone(), f.t_us);
                batches.entry(key).or_default().push((f.t_us, phys));
            }
        }
        for (key, pts) in batches {
            if let Some(sub) = self.subs.get_mut(&key) {
                let taken = sub.history.merge(&pts, SAMPLE_INTERVAL_US);
                for v in taken {
                    sub.observe(v);
                }
            }
        }
        // Claim the span that was *asked for*, not merely what was read. Repeating
        // an unsatisfied request every frame would rescan the same stretch
        // forever; a scan stopped by the frame cap can therefore leave the tail of
        // a very dense window thin until the view moves enough to ask again.
        self.sample_cover = match self.sample_cover {
            Some((lo, hi)) if t_from_us <= hi && t_to_us >= lo => {
                Some((lo.min(t_from_us), hi.max(t_to_us)))
            }
            _ => Some((t_from_us, t_to_us)),
        };
        if capped {
            self.status = format!(
                "plot: window too dense to decode fully ({} frames)",
                MAX_SCAN_FRAMES
            );
        }
    }

    /// Lets every signal's sampler resume at a scrubbed playhead. Retained
    /// samples are left in place; see [`Subscription::resume_sampling_at`].
    fn rewind_samples_to(&mut self, t_us: u64) {
        for sub in self.subs.values_mut() {
            sub.resume_sampling_at(t_us);
        }
    }

    /// True when a scrubbed replay is parked mid-log and Play should pick up
    /// from there instead of re-opening the file at zero.
    fn can_resume_replay(&self) -> bool {
        matches!(self.mode, Mode::Replay)
            && !self.replay_reset_pending
            && self.source.position().is_some()
    }

    /// Restarts the wall clock without touching captured history, so playback
    /// continues from the scrubbed position. `reset_time` is deliberately not
    /// used: it would wipe the trace and aggregates the user just scrubbed
    /// through. The stale `last` in the replay source is harmless because the
    /// poll clock uses `saturating_sub` against the new, smaller `now_us`.
    fn resume_replay(&mut self) {
        self.t0 = Instant::now();
        self.sim_prev_us = 0;
        self.trace_paused = false;
        self.paused_at_us = None;
        self.measuring = true;
        self.status = format!("resumed at {}x", self.replay_speed);
    }

    /// Starts playback, resuming a scrubbed replay in place when there is one.
    pub fn play(&mut self) {
        if self.can_resume_replay() {
            self.resume_replay();
        } else {
            self.start_selected();
        }
    }

    /// Play/pause as a single action, shared by the toolbar button, Space and
    /// F9 so the resume rule lives in exactly one place.
    pub fn toggle_play(&mut self) {
        if self.measuring {
            self.trace_paused = !self.trace_paused;
        } else {
            self.play();
        }
    }

    /// Current replay position and total duration in seconds (None when
    /// the active source has no timeline).
    pub fn replay_position(&self) -> Option<(f64, f64)> {
        let pos = self.source.position()? as f64 / 1e6;
        let dur = self.source.duration()? as f64 / 1e6;
        Some((pos, dur))
    }

    /// Time in seconds the plotting windows should treat as "now".
    ///
    /// While replaying that is the playhead, not the wall clock: samples are
    /// stamped with the log's own `t_us`, so anchoring a window on wall time
    /// slides the curve out of view as soon as the speed is not exactly 1x or
    /// as soon as the user scrubs.
    ///
    /// While simulating it is `sim_t_us`, which stops during a pause; the wall
    /// clock does not, so a paused window used to drift its own curve away.
    pub fn plot_now_s(&self) -> f64 {
        if matches!(self.mode, Mode::Replay)
            && let Some(pos) = self.source.position()
        {
            return pos as f64 / 1e6;
        }
        self.sim_t_us as f64 / 1e6
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
        let recent_log = std::mem::take(&mut self.recent_log);
        let recent_projects = std::mem::take(&mut self.recent_projects);
        let default_layout = std::mem::take(&mut self.default_layout);
        *self = App::new();
        self.recent_dbc = recent_dbc;
        self.recent_log = recent_log;
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
        let now = self.now_us();
        if let Some(t) = self.paused_at_us.take() {
            // Skip the paused interval so replay resumes in place
            // instead of fast-forwarding through it.
            self.source.shift_time(now.saturating_sub(t));
            // The simulation clock must skip it too, or every generator's
            // schedule and waveform phase jumps by the paused span.
            self.sim_prev_us = now;
        }
        self.sim_t_us += now.saturating_sub(self.sim_prev_us);
        self.sim_prev_us = now;
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
        self.tick(now);
    }

    /// One step of the measurement loop, polled against wall clock `now_us`.
    /// Split out of [`Self::update`] so a test can run a single step at a time
    /// of its choosing; the generator reads `sim_t_us`, which `update` maintains.
    pub fn tick(&mut self, now_us: u64) {
        let sim = self.sim_t_us;
        self.buf.clear();
        self.source.poll(now_us, &mut self.buf);
        let source_empty = self.buf.is_empty();

        // Generators only transmit in live simulation; replaying an ASC must
        // not mix in synthetic frames from active generator entries.
        let channels = &self.channels;
        if matches!(self.mode, Mode::Virtual) {
            for tx in &mut self.tx_list {
                if !tx.active || tx.cycle_us == 0 || tx.next_t_us > sim {
                    continue;
                }
                // Emit the slot this frame was due on, not the wall clock: a
                // stalled UI drops backlog rather than bursting, but the
                // spacing of what does go out stays exactly `cycle_us`.
                let slot = tx.next_t_us;
                tx.next_t_us += tx.cycle_us;
                if tx.next_t_us < sim {
                    let behind = (sim - tx.next_t_us) / tx.cycle_us + 1;
                    tx.next_t_us += behind * tx.cycle_us;
                }
                // Values are read at the slot, not at `sim`: a frame stamped
                // `slot` must carry the waveform's value at `slot`, or every
                // payload would lead its own timestamp by up to a full cycle.
                let (data, len, flags) = tx_payload(channels, tx, slot);
                self.buf.push(CanFrame {
                    t_us: slot,
                    channel: tx.channel,
                    id: tx.id,
                    extended: tx.extended,
                    len,
                    data,
                    dir: Direction::Tx,
                    flags,
                });
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
            // Only a strictly later timestamp marks a real cycle. A backwards
            // or repeated one is a discontinuity -- a seek, or an out-of-order
            // log row -- and folding it in used to pin `min_us` at zero for the
            // rest of the run. The running average keeps its pre-seek value and
            // resumes blending at the next real interval.
            if agg.count > 0 && f.t_us > agg.last_t_us {
                let dt = (f.t_us - agg.last_t_us) as f64;
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
            for (key, phys, unit) in self.subscribed_values(&f) {
                let Some(entry) = self.subs.get_mut(&key) else {
                    continue;
                };
                entry.latest = phys;
                entry.unit = unit;
                entry.last_update_us = f.t_us;
                if f.t_us >= entry.last_sample_us + SAMPLE_INTERVAL_US || entry.history.is_empty() {
                    entry.push_sample(f.t_us, phys);
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

    /// Decodes one frame into `(signal key, physical value, unit)` for the
    /// signals that are currently subscribed. Shared by the playback loop and
    /// the Graphics window backfill so the two cannot drift on what a signal's
    /// value is.
    fn subscribed_values(&self, f: &CanFrame) -> Vec<((u8, u32, String), f64, String)> {
        let Some(db) = self
            .channels
            .get(f.channel as usize)
            .and_then(|c| c.dbc.as_ref())
        else {
            return Vec::new();
        };
        db.decode_signals(f)
            .into_iter()
            .filter_map(|(name, phys, unit)| {
                let key = (f.channel, f.id, name);
                self.subs.contains_key(&key).then_some((key, phys, unit))
            })
            .collect()
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
                    history: SampleCache::default(),
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
            show_markers: true,
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
        let (name, len, cycle_us) = self
            .channel_dbc(channel)
            .and_then(|db| db.messages.get(&id))
            .map(|m| {
                (
                    m.name.clone(),
                    m.dlc.min(MAX_CAN_FD_LEN as u64) as u8,
                    // A declared 0 is event-triggered, so `unwrap_or` rather
                    // than `unwrap_or_default` on the Option: only "the DBC
                    // said nothing" gets our invented period.
                    m.cycle_us.unwrap_or(DEFAULT_TX_CYCLE_US),
                )
            })
            .unwrap_or_else(|| (format!("{id:X}"), 8, DEFAULT_TX_CYCLE_US));
        let data_text = vec!["00"; len as usize].join(" ");
        self.tx_list.push(TxMsg {
            channel,
            id,
            srcs: Vec::new(),
            extended: id > 0x7FF,
            name,
            len,
            data: [0; MAX_CAN_FD_LEN],
            flags: if len > 8 {
                FrameFlags::FD
            } else {
                FrameFlags::NONE
            },
            data_text,
            cycle_us,
            active: false,
            next_t_us: 0,
        });
    }

    /// Adds or replaces the source driving `src.name` on generator `i`.
    pub fn set_source(&mut self, i: usize, src: ValueSrc) {
        let Some(tx) = self.tx_list.get_mut(i) else {
            return;
        };
        match tx.srcs.iter_mut().find(|s| s.name == src.name) {
            Some(held) => *held = src,
            None => tx.srcs.push(src),
        }
    }

    /// Stops driving `name`, which leaves the base bytes in charge again.
    pub fn clear_source(&mut self, i: usize, name: &str) {
        if let Some(tx) = self.tx_list.get_mut(i) {
            tx.srcs.retain(|s| s.name != name);
        }
    }

    /// Writes a physical value into the base payload and pins that signal by
    /// dropping only its source: grabbing a moving slider means "hold here".
    pub fn pin_signal(&mut self, i: usize, name: &str, phys: f64) -> bool {
        let Some(tx) = self.tx_list.get(i) else {
            return false;
        };
        let (channel, id, mut data) = (tx.channel, tx.id, tx.data);
        let Some(table) = self.channel_dbc(channel) else {
            return false;
        };
        if !table.encode_signal(id, name, phys, &mut data) {
            return false;
        }
        let msg_size = table
            .messages
            .get(&id)
            .map(|m| m.dlc.min(MAX_CAN_FD_LEN as u64) as u8)
            .unwrap_or(0);
        let tx = &mut self.tx_list[i];
        tx.srcs.retain(|s| s.name != name);
        let len = tx.len.max(msg_size);
        Self::set_tx_base(tx, data, len);
        true
    }

    /// Replaces the base payload from the generator's hex box. Active sources
    /// deliberately survive: correcting one byte must not throw away a whole
    /// stimulus setup. Returns false if the text is not whole hex bytes.
    pub fn set_tx_hex(&mut self, i: usize, text: &str) -> bool {
        let Some(bytes) = parse_hex_bytes(text) else {
            return false;
        };
        let Some(tx) = self.tx_list.get_mut(i) else {
            return false;
        };
        let mut data = [0u8; MAX_CAN_FD_LEN];
        data[..bytes.len()].copy_from_slice(&bytes);
        let len = bytes.len() as u8;
        Self::set_tx_base(tx, data, len);
        true
    }

    /// Installs base bytes and keeps length, the FD flag and the hex text in
    /// step with them.
    fn set_tx_base(tx: &mut TxMsg, data: [u8; MAX_CAN_FD_LEN], len: u8) {
        let len = len.min(MAX_CAN_FD_LEN as u8);
        tx.data = data;
        tx.len = len;
        if len > 8 {
            tx.flags = tx.flags.union(FrameFlags::FD);
        }
        tx.data_text = hex_text(&data, len);
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
        app.log_path = first.clone();
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
    fn loading_log_does_not_start_replay() {
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
        app.load_log(&first);
        assert!(!app.measuring, "loading must not start playback");
        assert!(app.log_info.is_some(), "load should cache a stream summary");
        app.replay();
        assert!(app.measuring, "replay starts on demand");
        assert!(matches!(app.mode, Mode::Replay));
        app.stop();
        std::fs::remove_file(&first).ok();
    }

    #[test]
    fn loading_blf_does_not_start_replay() {
        let mut app = App::new();
        let path = std::env::temp_dir().join("roxy_can_load_blf_test.blf");
        let bytes = crate::log::blf::tests::minimal_file();
        std::fs::write(&path, &bytes).unwrap();
        app.load_log(&path.to_string_lossy());
        assert!(
            app.log_info.is_some(),
            "BLF load should cache a stream summary, got status {:?}",
            app.status
        );
        assert!(!app.measuring, "loading must not start playback");
        app.replay();
        assert!(app.measuring, "replay starts on demand");
        assert!(matches!(app.mode, Mode::Replay));
        app.stop();
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn open_dropped_reports_unsupported_for_mf4() {
        let mut app = App::new();
        app.open_dropped(std::path::Path::new("/tmp/does-not-exist.mf4"));
        assert!(
            app.status.contains("unsupported format: MF4"),
            "MF4 should surface a clear reason, got {:?}",
            app.status
        );
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

    /// Drives `ticks` steps of the loop, each `step_us` of simulation time
    /// apart. `tick` reads `sim_t_us` directly, so no wall clock is involved.
    fn run_sim(app: &mut App, ticks: u32, step_us: u64) {
        for i in 1..=ticks {
            app.sim_t_us = u64::from(i) * step_us;
            app.tick(app.sim_t_us);
        }
    }

    fn slots_of(app: &App, id: u32) -> Vec<u64> {
        app.trace
            .iter()
            .filter(|f| f.id == id)
            .map(|f| f.t_us)
            .collect()
    }

    #[test]
    fn generator_frames_are_spaced_exactly_one_cycle() {
        let mut app = App::new();
        app.add_tx(0, 0x777);
        let tx = app.tx_list.last_mut().unwrap();
        tx.cycle_us = 20_000;
        tx.active = true;
        app.start_virtual();
        // Ticks land on multiples of 7 ms, which the 20 ms cycle never lines up
        // with: slots must still come out on exact 20 ms boundaries.
        run_sim(&mut app, 12, 7_000);
        assert_eq!(
            slots_of(&app, 0x777),
            vec![0, 20_000, 40_000, 60_000, 80_000]
        );
        let agg = app.aggs.get(&(0, 0x777)).expect("aggregate");
        assert_eq!((agg.min_us, agg.max_us), (20_000.0, 20_000.0));
        app.stop();
    }

    #[test]
    fn a_tick_exactly_on_a_slot_does_not_drop_a_cycle() {
        let mut app = App::new();
        app.add_tx(0, 0x779);
        let tx = app.tx_list.last_mut().unwrap();
        tx.cycle_us = 20_000;
        tx.active = true;
        app.start_virtual();
        // Ticks land precisely on slot boundaries.
        run_sim(&mut app, 6, 20_000);
        assert_eq!(
            slots_of(&app, 0x779),
            vec![0, 20_000, 40_000, 60_000, 80_000, 100_000],
            "one frame per cycle, none skipped at the boundary"
        );
        app.stop();
    }

    #[test]
    fn a_stalled_ui_drops_cycles_instead_of_bursting() {
        let mut app = App::new();
        app.add_tx(0, 0x778);
        let tx = app.tx_list.last_mut().unwrap();
        tx.cycle_us = 20_000;
        tx.active = true;
        app.start_virtual();
        app.sim_t_us = 0;
        app.tick(0);
        // Twelve cycles' worth of stall must produce one frame, and the
        // schedule must end up past the stall rather than queued behind it.
        app.sim_t_us = 250_000;
        app.tick(250_000);
        assert_eq!(slots_of(&app, 0x778), vec![0, 20_000], "one frame per tick");
        assert_eq!(
            app.tx_list.last().unwrap().next_t_us,
            260_000,
            "the backlog was skipped, not deferred"
        );
        app.stop();
    }

    #[test]
    fn a_pause_freezes_the_simulation_clock_and_its_phase() {
        let mut app = App::new();
        app.start_virtual();
        app.update();
        let before = app.sim_t_us;
        // Age the wall clock by 400 ms the way a real pause would, then check
        // the simulation clock neither moves during the pause nor absorbs the
        // paused span afterwards.
        app.trace_paused = true;
        app.t0 = Instant::now() - std::time::Duration::from_millis(400);
        app.update();
        assert_eq!(app.sim_t_us, before, "a paused clock must not advance");
        app.trace_paused = false;
        app.update();
        assert!(
            app.sim_t_us - before < 50_000,
            "resuming absorbed the paused span: sim advanced {} us",
            app.sim_t_us - before
        );
        app.stop();
    }

    /// A 16-byte message with one signal in the classic area and one starting at
    /// byte 9, so payload widening is testable without an FD asset.
    const WIDE_DBC: &str = r#"VERSION "roxy-can test database"

NS_ :

BU_: ECU

BO_ 768 WideMsg: 16 ECU
 SG_ NearSig : 0|16@1+ (1,0) [0|65535] "" ECU
 SG_ FarSig : 72|16@1+ (1,0) [0|65535] "" ECU
"#;

    /// Channel 0 on [`WIDE_DBC`] with one active, source-driven `WideMsg`. The
    /// default period is one second, so a slot `t` microseconds into the run
    /// carries `(t as f64 / 1e6) * hi`.
    fn driven_app(signal: &str, kind: crate::sim::SrcKind, lo: f64, hi: f64) -> App {
        let mut app = App::new();
        app.channels[0].dbc = Some(crate::dbc::load_dbc_str(WIDE_DBC).expect("wide dbc parses"));
        app.add_tx(0, 0x300);
        let tx = app.tx_list.last_mut().expect("tx entry added");
        tx.cycle_us = 20_000;
        tx.active = true;
        tx.srcs.push(ValueSrc::new(signal, kind, lo, hi));
        app.start_virtual();
        app
    }

    fn emitted(app: &App, id: u32) -> Vec<CanFrame> {
        app.trace.iter().filter(|f| f.id == id).copied().collect()
    }

    fn raw_at(f: &CanFrame, start_bit: u64) -> u64 {
        crate::decode::extract_raw(&f.data, start_bit, 16, false)
    }

    #[test]
    fn a_driven_signal_carries_the_value_of_its_own_timestamp() {
        // Ticks land off the 20 ms slot grid on purpose, so a payload read at
        // the tick instead of at the stamp it carries would come out wrong.
        let mut app = driven_app("NearSig", crate::sim::SrcKind::Ramp, 0.0, 1000.0);
        run_sim(&mut app, 12, 7_000);
        let frames = emitted(&app, 0x300);
        let slots: Vec<u64> = frames.iter().map(|f| f.t_us).collect();
        assert_eq!(slots, vec![0, 20_000, 40_000, 60_000, 80_000]);
        let vals: Vec<u64> = frames.iter().map(|f| raw_at(f, 0)).collect();
        assert_eq!(
            vals,
            vec![0, 20, 40, 60, 80],
            "each frame must hold the ramp value at its own stamp"
        );
        app.stop();
    }

    #[test]
    fn the_wall_clock_cannot_move_a_generated_value() {
        let value_at = |now_us: u64| {
            let mut app = driven_app("NearSig", crate::sim::SrcKind::Ramp, 0.0, 1000.0);
            app.tx_list.last_mut().unwrap().next_t_us = 40_000;
            app.sim_t_us = 45_000;
            app.tick(now_us);
            raw_at(&emitted(&app, 0x300)[0], 0)
        };
        assert_eq!(
            value_at(45_000),
            value_at(9_999_999),
            "payloads must depend on simulation time only"
        );
        assert_eq!(value_at(9_999_999), 40, "the value at the slot stamped");
    }

    #[test]
    fn driving_a_signal_leaves_the_base_payload_alone() {
        let mut app = driven_app("NearSig", crate::sim::SrcKind::Sine, 0.0, 1000.0);
        let base = app.tx_list.last().unwrap().data;
        run_sim(&mut app, 30, 7_000);
        assert!(
            emitted(&app, 0x300).iter().any(|f| raw_at(f, 0) != 0),
            "the source should have moved something by now"
        );
        assert_eq!(
            app.tx_list.last().unwrap().data,
            base,
            "a waveform sample must not become the saved base payload"
        );
        app.stop();
    }

    #[test]
    fn a_driven_signal_past_byte_8_widens_the_frame() {
        let mut app = driven_app("FarSig", crate::sim::SrcKind::Ramp, 0.0, 1000.0);
        let i = app.tx_list.len() - 1;
        assert!(app.set_tx_hex(i, "00 01 02 03 04 05 06 07"));
        app.tx_list[i].next_t_us = 70_000;
        app.sim_t_us = 70_000;
        app.tick(70_000);
        let f = emitted(&app, 0x300).pop().expect("one frame");
        // Bits 72..88 need 11 bytes; 11 is not a legal FD length, so the frame
        // goes out at 12 with the FD flag set.
        assert_eq!(f.len, 12, "widened to the next legal FD length");
        assert!(f.flags.contains(FrameFlags::FD), "widening implies FD");
        assert_eq!(raw_at(&f, 72), 70, "the driven bytes are really there");
        assert_eq!(raw_at(&f, 0), 0x0100, "the base bytes still come through");
        assert_eq!(
            app.tx_list[i].len, 8,
            "only the emitted frame grows, not the base"
        );
        app.stop();
    }

    #[test]
    fn pin_signal_stops_only_that_signal() {
        let mut app = driven_app("NearSig", crate::sim::SrcKind::Ramp, 0.0, 1000.0);
        let i = app.tx_list.len() - 1;
        app.set_source(
            i,
            ValueSrc::new("FarSig", crate::sim::SrcKind::Sine, 0.0, 100.0),
        );
        assert_eq!(app.tx_list[i].srcs.len(), 2);
        assert!(app.pin_signal(i, "NearSig", 250.0));
        let names: Vec<&str> = app.tx_list[i]
            .srcs
            .iter()
            .map(|s| s.name.as_str())
            .collect();
        assert_eq!(names, ["FarSig"], "pinning must not stop the other source");
        assert_eq!(
            crate::decode::extract_raw(&app.tx_list[i].data, 0, 16, false),
            250,
            "pinned into the base"
        );
        assert_eq!(
            app.tx_list[i].data_text,
            "FA 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00"
        );
        assert!(
            !app.pin_signal(i, "NoSuchSignal", 1.0),
            "unknown signal refused"
        );
        app.stop();
    }

    #[test]
    fn a_hex_edit_keeps_the_sources_running() {
        let mut app = driven_app("NearSig", crate::sim::SrcKind::Ramp, 0.0, 1000.0);
        let i = app.tx_list.len() - 1;
        assert!(!app.set_tx_hex(i, "0 zz"), "non-hex must not apply");
        assert_eq!(app.tx_list[i].len, 16, "the rejected edit changed nothing");
        assert!(app.set_tx_hex(i, "11 22 33"));
        assert_eq!(app.tx_list[i].len, 3);
        assert_eq!(app.tx_list[i].data_text, "11 22 33", "text stays canonical");
        assert_eq!(
            app.tx_list[i].srcs.len(),
            1,
            "fixing one byte must not throw away the stimulus setup"
        );
        app.stop();
    }

    #[test]
    fn set_source_replaces_by_name() {
        let mut app = driven_app("NearSig", crate::sim::SrcKind::Ramp, 0.0, 1000.0);
        let i = app.tx_list.len() - 1;
        app.set_source(
            i,
            ValueSrc::new("NearSig", crate::sim::SrcKind::Sine, 0.0, 50.0),
        );
        assert_eq!(app.tx_list[i].srcs.len(), 1, "the same name is one source");
        assert_eq!(app.tx_list[i].srcs[0].hi, 50.0, "and the later one wins");
        app.clear_source(i, "NearSig");
        assert!(app.tx_list[i].srcs.is_empty());
        app.stop();
    }

    /// A DBC that declares 0 ms means event-triggered; one that declares
    /// nothing at all must still get the invented fallback period.
    const CYCLE_TEST_DBC: &str = r#"VERSION "roxy-can cycle test"

NS_ :

BU_: ECU

BO_ 4096 EventMsg: 8 ECU
 SG_ S : 0|8@1+ (1,0) [0|0] "" ECU

BO_ 4097 DefaultedMsg: 8 ECU
 SG_ S : 0|8@1+ (1,0) [0|0] "" ECU

BA_DEF_ BO_  "GenMsgCycleTime" INT 0 10000;
BA_DEF_DEF_  "GenMsgCycleTime" 77;
BA_ "GenMsgCycleTime" BO_ 4096 0;
"#;

    #[test]
    fn new_generator_entries_inherit_the_declared_cycle() {
        let app = App::new();
        let cycle = |ch: u8, id: u32| {
            app.tx_list
                .iter()
                .find(|t| t.channel == ch && t.id == id)
                .map(|t| t.cycle_us)
        };
        // assets/motbus.dbc:62-63 declare these two explicitly...
        assert_eq!(cycle(1, 0x64), Some(133_000), "EngineData 133ms");
        assert_eq!(cycle(1, 0xC9), Some(50_000), "ABSdata 50ms");
        // ...and its BA_DEF_DEF_ puts 100ms on the rest.
        assert_eq!(cycle(1, 0xC7), Some(100_000), "declared default");
        // sample.dbc declares nothing, so the fallback is what shows up.
        assert_eq!(cycle(0, 0x100), Some(100_000), "no declaration -> fallback");
    }

    #[test]
    fn an_event_triggered_message_is_never_auto_sent() {
        let mut app = App::new();
        app.channels[0].dbc = Some(crate::dbc::load_dbc_str(CYCLE_TEST_DBC).unwrap());
        app.tx_list.retain(|t| t.channel != 0);
        app.add_tx(0, 4096);
        app.add_tx(0, 4097);
        let i = app.tx_list.len() - 2;
        assert_eq!(
            app.tx_list[i].cycle_us, 0,
            "an explicit 0 is not 'undeclared'"
        );
        assert_eq!(
            app.tx_list[i + 1].cycle_us,
            77_000,
            "the default still applies"
        );
        for t in &mut app.tx_list {
            t.active = true;
        }
        app.start_virtual();
        run_sim(&mut app, 20, 10_000);
        assert!(
            slots_of(&app, 4096).is_empty(),
            "event-triggered means no timer"
        );
        assert_eq!(
            slots_of(&app, 4097),
            vec![0, 77_000, 154_000],
            "the declared 77ms period is what runs"
        );
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
        app.load_log(&file);
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

    /// A log of `n` frames spaced `step_us` apart, so the timeline is exactly
    /// `(n-1) * step_us` long and every frame sits on a round second.
    fn write_timed_asc(name: &str, n: u32, step_us: u64) -> std::path::PathBuf {
        let path = std::env::temp_dir().join(name);
        {
            let mut w = AscWriter::new(&path.to_string_lossy()).unwrap();
            for i in 0..n {
                let mut f = CanFrame {
                    t_us: u64::from(i) * step_us,
                    channel: 0,
                    id: 0x100,
                    extended: false,
                    len: 2,
                    data: [0; MAX_CAN_FD_LEN],
                    dir: Direction::Rx,
                    flags: FrameFlags::NONE,
                };
                f.data[0] = 0xAA;
                f.data[1] = 0xBB;
                w.write(&f).unwrap();
            }
            w.finish().unwrap();
        }
        path
    }

    #[test]
    fn seek_replay_moves_the_playhead() {
        let mut app = App::new();
        let path = write_timed_asc("roxy_can_scrub.asc", 100, 10_000);
        app.load_log(&path.to_string_lossy());
        app.replay();
        assert!(app.measuring);

        app.seek_replay_seconds(0.5);
        let (pos, dur) = app.replay_position().expect("replay has a timeline");
        assert!(
            (pos - 0.5).abs() < 1e-6,
            "playhead should land on the 0.5 s frame, got {pos}"
        );
        assert!(dur > 0.9, "timeline covers the log, got {dur}");

        // The first update after a seek only re-anchors the clock, so exactly
        // the landing frame is emitted -- a scrub must not dump the prefix.
        app.update();
        assert_eq!(app.trace.len(), 1, "no flood of skipped frames");
        assert_eq!(app.trace.back().unwrap().t_us, 500_000);

        app.seek_replay_seconds(0.1);
        app.update();
        assert_eq!(app.trace.len(), 2, "seeking backwards replays earlier rows");
        assert_eq!(app.trace.back().unwrap().t_us, 100_000);
        app.stop();
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn play_after_a_scrub_resumes_in_place() {
        let mut app = App::new();
        let path = write_timed_asc("roxy_can_scrub_resume.asc", 100, 10_000);
        app.load_log(&path.to_string_lossy());
        app.replay();

        // Run the log out the far end without touching Stop.
        let (_, dur) = app.replay_position().unwrap();
        app.seek_replay_seconds(dur);
        app.update();
        app.update();
        assert!(!app.measuring, "the replay finished on its own");

        app.seek_replay_seconds(0.3);
        assert!(
            app.replay_position().is_some(),
            "the timeline must survive the end of the log so the scrub bar stays usable"
        );
        app.toggle_play();
        assert!(app.measuring, "Play resumes a finished, scrubbed replay");
        let (pos, _) = app.replay_position().unwrap();
        assert!(
            (pos - 0.3).abs() < 1e-6,
            "must continue from the scrubbed position, got {pos}"
        );
        app.stop();
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn stop_makes_the_next_play_restart_from_zero() {
        let mut app = App::new();
        let path = write_timed_asc("roxy_can_scrub_stop.asc", 100, 10_000);
        app.load_log(&path.to_string_lossy());
        app.replay();
        app.seek_replay_seconds(0.5);
        app.update();
        app.stop();
        app.toggle_play();
        let (pos, _) = app.replay_position().unwrap();
        assert!(
            pos < 0.01,
            "Stop is an explicit request to re-open from the beginning, got {pos}"
        );
        app.stop();
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn the_plot_clock_follows_the_replay_playhead() {
        let mut app = App::new();
        let path = write_timed_asc("roxy_can_plot_clock.asc", 100, 10_000);
        app.load_log(&path.to_string_lossy());
        app.replay();
        app.seek_replay_seconds(0.5);
        assert!(
            (app.plot_now_s() - 0.5).abs() < 1e-6,
            "the Graphics axis must track the scrub bar, got {}",
            app.plot_now_s()
        );
        app.seek_replay_seconds(0.2);
        assert!(
            (app.plot_now_s() - 0.2).abs() < 1e-6,
            "and track a rewind, got {}",
            app.plot_now_s()
        );
        app.stop();
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn loading_another_log_mid_replay_is_refused() {
        let mut app = App::new();
        let a = write_timed_asc("roxy_can_guard_a.asc", 50, 10_000);
        let b = write_timed_asc("roxy_can_guard_b.asc", 50, 10_000);
        app.load_log(&a.to_string_lossy());
        app.replay();
        assert!(app.measuring);

        app.load_log(&b.to_string_lossy());
        assert_eq!(
            app.log_path,
            a.to_string_lossy(),
            "the running log must stay selected"
        );
        assert!(
            app.status.contains("stop the replay"),
            "the refusal should say why, got {:?}",
            app.status
        );
        assert!(
            !app.recent_log
                .iter()
                .any(|p| p == &b.to_string_lossy().to_string()),
            "a refused load must not enter the recent list"
        );

        // Stopped, the same selection goes through and demands a fresh open.
        app.stop();
        app.load_log(&b.to_string_lossy());
        assert_eq!(app.log_path, b.to_string_lossy());
        assert!(
            app.replay_reset_pending,
            "a newly selected log must not be resumed over"
        );
        app.stop();
        std::fs::remove_file(&a).ok();
        std::fs::remove_file(&b).ok();
    }

    #[test]
    fn play_after_choosing_a_new_log_opens_that_log() {
        let mut app = App::new();
        let a = write_timed_asc("roxy_can_switch_a.asc", 100, 10_000);
        let b = write_timed_asc("roxy_can_switch_b.asc", 20, 10_000);
        app.load_log(&a.to_string_lossy());
        app.replay();
        // Let the first log run to its natural end, which leaves the source
        // parked but replay-able -- exactly where the old code could resume the
        // wrong file.
        let (_, dur_a) = app.replay_position().unwrap();
        app.seek_replay_seconds(dur_a);
        app.update();
        app.update();
        assert!(!app.measuring, "setup: the first log finished on its own");

        app.load_log(&b.to_string_lossy());
        app.play();
        let (_, dur_b) = app.replay_position().expect("the new log has a timeline");
        assert!(
            dur_b < dur_a,
            "Play must open the newly selected log ({dur_b}s) rather than resume \
             the finished one ({dur_a}s)"
        );
        app.stop();
        std::fs::remove_file(&a).ok();
        std::fs::remove_file(&b).ok();
    }

    fn blank_sub() -> Subscription {
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
            history: SampleCache::default(),
            color: 0,
        }
    }

    #[test]
    fn the_head_of_the_curve_survives_past_the_old_point_cap() {
        // 250 s of samples at the real 50 ms interval: beyond the 4000-point
        // cap this replaces, which began popping the head at exactly 200 s and
        // made the left end of a running trace vanish on its own.
        let mut sub = blank_sub();
        for i in 0..5_000u64 {
            sub.push_sample(i * SAMPLE_INTERVAL_US, (i % 97) as f64);
        }
        assert_eq!(sub.history.len(), 5_000, "250 s fits inside the span");
        assert_eq!(
            sub.history.first().unwrap().0,
            0,
            "the head must still be there mid-run"
        );
    }

    #[test]
    fn eviction_begins_only_past_the_retention_span() {
        let mut sub = blank_sub();
        let n = HISTORY_SPAN_US / SAMPLE_INTERVAL_US + 10;
        for i in 0..n {
            sub.push_sample(i * SAMPLE_INTERVAL_US, 1.0);
        }
        assert!(sub.history.len() < n as usize, "stale head is dropped");
        let kept = sub.history.len() as u64 * SAMPLE_INTERVAL_US;
        assert!(
            kept <= HISTORY_SPAN_US + SAMPLE_INTERVAL_US,
            "retained span {kept} us exceeds the cap"
        );
        assert!(
            sub.n > sub.history.len() as u64,
            "min/max/avg stay cumulative over the whole run"
        );
    }

    #[test]
    fn retention_backs_the_widest_plot_window() {
        assert!(
            HISTORY_SPAN_US as f64 / 1e6 >= crate::ui::graphics::MAX_TIME_WINDOW_S,
            "the widest window is {} s but history only holds {} s",
            crate::ui::graphics::MAX_TIME_WINDOW_S,
            HISTORY_SPAN_US as f64 / 1e6,
        );
    }

    /// Records `iters` frames of generator traffic to an ASC, then returns an
    /// App with the first sample.dbc signal subscribed and that log loaded but
    /// not yet playing. The traffic has to be DBC-decodable for the Graphics
    /// history to fill, so a hand-written fixture will not do.
    fn app_with_replayable_recording(name: &str, iters: usize) -> (App, (u8, u32, String), String) {
        let mut app = App::new();
        let key = {
            let db = app.channel_dbc(0).expect("sample DBC loaded");
            let id = db.order[0];
            (0u8, id, db.messages[&id].signals[0].name.clone())
        };
        app.subscribe(key.clone());
        let out = std::env::temp_dir().join(format!("roxy_can_{name}.asc"));
        app.record_path = out.to_string_lossy().to_string();
        app.toggle_record();
        for tx in &mut app.tx_list {
            tx.active = true;
            tx.cycle_us = 10_000;
        }
        app.start_virtual();
        for _ in 0..iters {
            std::thread::sleep(std::time::Duration::from_millis(11));
            app.update();
        }
        app.stop();
        std::fs::remove_file(&out).ok();
        let file = app.last_record.clone();
        app.load_log(&file);
        (app, key, file)
    }

    #[test]
    fn a_backward_scrub_rewinds_signal_state() {
        let (mut app, key, file) = app_with_replayable_recording("scrub_history", 60);
        app.replay();
        // Let the clock actually run so sampling fills history across the log;
        // a forward seek cannot do it, since seeking discards the prefix.
        app.set_replay_speed(4.0);
        let deadline = std::time::Instant::now() + std::time::Duration::from_millis(250);
        while std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(11));
            app.update();
        }
        let filled = app.subs.get(&key).expect("subscribed");
        assert!(
            filled.history.len() > 3,
            "expected samples across the log, got {}",
            filled.history.len()
        );
        let (_, dur) = app.replay_position().unwrap();

        app.seek_replay_seconds(dur / 3.0);
        let landed = app.replay_position().unwrap().0;
        let sub = app.subs.get(&key).unwrap();
        assert!(
            sub.history.iter().any(|(t, _)| *t as f64 / 1e6 > landed),
            "the cache keeps samples ahead of a rewound playhead; the window \
             slice hides them, and deleting them was what blanked the curve"
        );
        assert!(
            sub.history
                .iter()
                .zip(sub.history.iter().skip(1))
                .all(|(a, b)| a.0 <= b.0),
            "the cache must stay ascending for the binary search in value_at"
        );
        let after_rewind = sub.history.len();

        // Replaying across ground the cache already holds must not inject
        // near-duplicates: the sampler's own baseline was pulled back to the
        // rewind point, so only the cache's spacing rule keeps it honest.
        app.play();
        assert!(app.measuring, "Play resumes the rewound replay");
        app.set_replay_speed(4.0);
        for _ in 0..5 {
            std::thread::sleep(std::time::Duration::from_millis(11));
            app.update();
        }
        let (pos_now, _) = app.replay_position().unwrap();
        assert!(
            pos_now * 1e6 > landed,
            "the playhead should advance past the rewind point, got {pos_now}"
        );
        let sub = app.subs.get(&key).unwrap();
        assert_eq!(
            sub.history.len(),
            after_rewind,
            "replaying cached ground must add nothing, not even near-duplicates"
        );
        assert!(
            sub.history
                .iter()
                .zip(sub.history.iter().skip(1))
                .all(|(a, b)| a.0 <= b.0),
            "re-sampled history must remain ascending"
        );
        app.stop();
        std::fs::remove_file(&file).ok();
    }

    #[test]
    fn sample_cache_stays_ascending_when_filled_from_either_end() {
        let mut c = SampleCache::default();
        // Streaming fills the later stretch first, then a backfill lands behind
        // it; the buffer must still read ascending for the plot and value_at.
        c.merge(
            &(100..110u64)
                .map(|i| (i * 1_000, i as f64))
                .collect::<Vec<_>>(),
            1_000,
        );
        c.merge(
            &(0..10).map(|i| (i * 1_000, i as f64)).collect::<Vec<_>>(),
            1_000,
        );
        assert_eq!(c.len(), 20);
        assert!(
            c.iter().zip(c.iter().skip(1)).all(|(a, b)| a.0 <= b.0),
            "merge behind existing points must keep the buffer sorted"
        );
        assert_eq!(c.first().unwrap().0, 0);
    }

    #[test]
    fn sample_cache_range_and_lookup_are_inclusive_and_ordered() {
        let mut c = SampleCache::default();
        c.merge(
            &(0..10u64)
                .map(|i| (i * 1_000, i as f64))
                .collect::<Vec<_>>(),
            1_000,
        );
        let win = c.range(2_000, 5_000);
        assert_eq!(
            win.iter().map(|(t, _)| *t).collect::<Vec<_>>(),
            vec![2_000, 3_000, 4_000, 5_000],
            "both ends of the window are included"
        );
        assert_eq!(c.at(4_500), Some(4.0), "last value at or before");
        assert_eq!(
            c.at(0),
            Some(0.0),
            "the first sample resolves on its own edge"
        );
        assert_eq!(c.at(999), Some(0.0), "step-signal semantics hold");
        assert_eq!(
            SampleCache::default().at(999),
            None,
            "an empty cache has no value to report"
        );
    }

    #[test]
    fn sample_cache_trims_by_span_not_by_count() {
        let mut c = SampleCache::default();
        c.merge(
            &(0..30u64).map(|i| (i * 10_000, 1.0)).collect::<Vec<_>>(),
            10_000,
        );
        c.trim_oldest(100_000);
        assert_eq!(
            c.first().unwrap().0,
            190_000,
            "newest is 290 s, so everything from 190 s on survives"
        );
        assert_eq!(c.len(), 11);
    }

    #[test]
    fn overlapping_backfills_do_not_pile_up_near_duplicates() {
        let (mut app, key, file) = app_with_replayable_recording("dupstride", 60);
        app.replay();
        // Two requests overlapping by most of their span -- exactly what
        // happens on consecutive frames as the playhead advances.
        app.ensure_samples_in(100_000, 400_000);
        app.ensure_samples_in(110_000, 410_000);
        let sub = app.subs.get(&key).unwrap();
        let mut tight = 0usize;
        let mut prev: Option<u64> = None;
        for &(t, _) in sub.history.iter() {
            if let Some(p) = prev
                && t.saturating_sub(p) < SAMPLE_INTERVAL_US
            {
                tight += 1;
            }
            prev = Some(t);
        }
        assert_eq!(
            tight, 0,
            "{} samples landed within one stride of a neighbour; the polyline \
             then zig-zags between them and reads as a thick band",
            tight
        );
        app.stop();
        std::fs::remove_file(&file).ok();
    }

    #[test]
    fn a_rewind_does_not_record_a_zero_cycle_time() {
        let (mut app, _key, file) = app_with_replayable_recording("cycle_rebase", 60);
        app.replay();
        app.set_replay_speed(4.0);
        let deadline = std::time::Instant::now() + std::time::Duration::from_millis(250);
        while std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(11));
            app.update();
        }
        let (_, dur) = app.replay_position().unwrap();
        assert!(!app.aggs.is_empty(), "setup: messages should be aggregated");

        // Walk back over ground already seen: every replayed frame is a
        // backwards timestamp for its message, which used to be folded in as a
        // zero-length cycle.
        app.seek_replay_seconds(dur / 3.0);
        app.play();
        for _ in 0..10 {
            std::thread::sleep(std::time::Duration::from_millis(11));
            app.update();
        }
        for agg in app.aggs.values() {
            if agg.count > 1 {
                assert!(
                    agg.min_us > 0.0,
                    "message {:#05X} reports a {} us minimum cycle after a rewind",
                    agg.id,
                    agg.min_us
                );
            }
        }
        app.stop();
        std::fs::remove_file(&file).ok();
    }

    #[test]
    fn a_plot_window_decodes_without_waiting_for_playback() {
        let (mut app, key, file) = app_with_replayable_recording("backfill", 60);
        app.replay();
        let (pos0, dur) = app.replay_position().unwrap();
        assert!(
            dur > 0.4,
            "setup: the log should span the window under test, got {dur} s"
        );

        {
            // Ask for a stretch the playback cursor has never walked through.
            // Under the old streaming-only design this window was simply empty.
            app.ensure_samples_in(200_000, 400_000);
            let sub = app.subs.get(&key).unwrap();
            let win = sub.history.range(200_000, 400_000);
            assert!(
                win.len() > 3,
                "the window must decode on demand, got {} points",
                win.len()
            );
            assert!(
                win.iter()
                    .zip(win.iter().skip(1))
                    .all(|(a, b)| b.0 - a.0 >= SAMPLE_INTERVAL_US),
                "a backfill must honour the sampling stride"
            );
            assert!(
                win.first().unwrap().0 >= 200_000 && win.last().unwrap().0 <= 400_000,
                "returned points must lie inside the request"
            );
        }
        let (pos1, _) = app.replay_position().unwrap();
        assert_eq!(pos1, pos0, "a backfill must not move the playhead");
        app.stop();
        std::fs::remove_file(&file).ok();
    }

    #[test]
    fn a_second_replay_run_samples_from_the_top() {
        let (mut app, key, file) = app_with_replayable_recording("resample", 60);

        // First run: play the log out so the subscription ends up with a
        // sampling baseline near the end of it.
        app.replay();
        app.set_replay_speed(4.0);
        let deadline = std::time::Instant::now() + std::time::Duration::from_millis(250);
        while std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(11));
            app.update();
        }
        let stale_baseline = app.subs.get(&key).unwrap().last_sample_us;
        assert!(
            stale_baseline > 200_000,
            "setup: the first run should have sampled deep into the log, got {stale_baseline} us"
        );

        // Second run. Replaying used to inherit that baseline, and the sampler
        // gate then rejected every frame until the playhead climbed past it --
        // visibly, the start of the curve was simply missing.
        app.replay();
        {
            let sub = app.subs.get(&key).unwrap();
            assert!(sub.history.is_empty(), "a fresh run drops the old trace");
            assert_eq!(
                sub.last_sample_us, 0,
                "a fresh run must not inherit the sampling baseline"
            );
            assert_eq!(sub.n, 0, "a fresh run resets the sample count");
            assert!(
                !sub.min.is_finite() && sub.max == f64::NEG_INFINITY,
                "a fresh run resets min/max instead of keeping the old extremes"
            );
        }
        // The first poll only anchors the replay clock; the second is what
        // moves the playhead past the log's opening frames.
        std::thread::sleep(std::time::Duration::from_millis(20));
        app.update();
        std::thread::sleep(std::time::Duration::from_millis(20));
        app.update();
        let sub = app.subs.get(&key).unwrap();
        assert!(
            !sub.history.is_empty(),
            "sampling must start at the top of the log, not at {stale_baseline} us"
        );
        assert!(
            sub.history.first().unwrap().0 < stale_baseline,
            "the first sample of the new run should precede the previous run's last"
        );
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
        app.load_log(&path.to_string_lossy());
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
