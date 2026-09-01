use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::time::Instant;

use crate::can::frame::{CanFrame, Direction};
use crate::log::AscWriter;
use crate::log::open_stream;
use crate::source::replay::ReplaySource;
use crate::source::virtual_source::VirtualSource;
use crate::source::{FrameSource, FrameStream};
use crate::spec::{
    GRACE_CYCLES, Kind, Spec, TOLERANCE_PERCENT, cycle_offender, dlc_offender, missing_offender,
};

pub const TRACE_LIMIT: usize = 50_000;
pub const TOOLBAR_H: f32 = 54.0;
pub const STATUSBAR_H: f32 = 28.0;
pub const TABSTRIP_H: f32 = 22.0;
/// How far back sampled signal history is kept. Deliberately in seconds, not
/// points: the Graphics window ladder goes up to an hour, and a point-count
/// cap silently dropped the head of the curve mid-run the moment it filled,
/// whatever width the user had chosen.
pub(crate) const HISTORY_SPAN_US: u64 = 3_600_000_000;
pub(crate) const SAMPLE_INTERVAL_US: u64 = 50_000;
/// Frames one window backfill may collect. Bounds a single synchronous pass so
/// a dense hour-wide window cannot lock the UI; the plot shows what it got and
/// asks again on the next change.
pub(crate) const MAX_SCAN_FRAMES: usize = 300_000;
/// Speed ladder shared by the toolbar combo and the slower/faster buttons.
pub const REPLAY_SPEEDS: [f64; 4] = [0.5, 1.0, 2.0, 4.0];
/// Cycle a new generator entry gets when its DBC declares none. A declared
/// value always wins; this is only the invention we fall back to.
pub(crate) const DEFAULT_TX_CYCLE_US: u64 = 100_000;

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
pub use crate::aggregate::MessageAgg;
pub use crate::channel::Channel;
pub use crate::generator::{cycle_from_ms_text, TxMsg, TX_CYCLE_MAX_MS};
pub use crate::observe::{DataWindow, GfxSignal, GraphicsWindow, SampleCache, Subscription};
pub use crate::project::PendingAction;
pub use crate::workspace::{
    Desktop, MsgWin, PopupTarget, SigScope, StatsWin, TraceWin, WindowKind,
};
use crate::generator::tx_payload;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Virtual,
    Replay,
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
    pub(crate) sample_cover: Option<(u64, u64)>,
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
    pub(crate) baseline: String,
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
    /// Per-bus load / frame-rate / error rolling state, one entry per
    /// channel. Fed from the same frame loop as `aggs`.
    pub bus_loads: Vec<crate::load::BusLoad>,
    /// Observed-versus-declared violations, recomputed on every measurement
    /// step from `aggs` and the loaded databases.
    pub spec: Spec,
    /// Which violation kinds the report window lists, indexed by
    /// [`crate::spec::Kind::ALL`]. A noise control, deliberately not part of the
    /// project: hiding third-party traffic today should not hide it next week.
    pub spec_show: [bool; 4],
    /// How far an observed period may stray from the declared one, in percent.
    pub spec_tol_pct: u64,
    /// How many declared periods of silence count as a dropped message.
    pub spec_grace: u64,
    pub symbol_search: String,
    pub show_tx: bool,
    pub show_network: bool,
    pub show_measurement: bool,
    pub show_buses: bool,
    pub show_spec: bool,
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
    /// The value-source dialog's in-progress parameters. Applied to the row only
    /// when the dialog confirms, so typing 8000 into `hi` cannot make the
    /// running waveform pass through 8 and 80 first.
    pub src_draft: Option<crate::sim::ValueSrc>,
    /// A generator slider's value while it is being dragged or typed; the model
    /// waits for the edit to end. See [`crate::ui::Draft`].
    pub num_draft: crate::ui::Draft,
    /// Generator row whose send period the cycle dialog is drafting. The value
    /// stays a draft until the dialog confirms it: as an inline number box it
    /// applied every keystroke, so dialing in 100 sent at 1 ms first.
    pub tx_cycle_edit: Option<usize>,
    /// Draft period in whole milliseconds for that row.
    pub tx_cycle_buf: String,
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
    pub(crate) trace_counter: usize,
    pub(crate) msg_counter: usize,
    pub(crate) stats_counter: usize,
    pub(crate) graphics_counter: usize,
    pub(crate) data_counter: usize,
    pub(crate) bus_counter: usize,
    pub(crate) color_counter: usize,
    pub(crate) source: Box<dyn FrameSource>,
    writer: Option<AscWriter>,
    buf: Vec<CanFrame>,
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
                    sim_nodes: Vec::new(),
                    bitrate_kbps: Channel::DEFAULT_BITRATE_KBPS,
                    fd_data_kbps: Channel::DEFAULT_FD_DATA_KBPS,
                },
                Channel {
                    name: "CAN2".to_string(),
                    dbc: None,
                    dbc_path: "assets/motbus.dbc".to_string(),
                    sim_nodes: Vec::new(),
                    bitrate_kbps: Channel::DEFAULT_BITRATE_KBPS,
                    fd_data_kbps: Channel::DEFAULT_FD_DATA_KBPS,
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
            bus_loads: vec![
                crate::load::BusLoad::new(),
                crate::load::BusLoad::new(),
            ],
            spec: Spec::default(),
            spec_show: [true; 4],
            spec_tol_pct: TOLERANCE_PERCENT,
            spec_grace: GRACE_CYCLES,
            symbol_search: String::new(),
            show_tx: true,
            show_network: true,
            show_measurement: true,
            show_buses: false,
            show_spec: false,
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
            src_draft: None,
            num_draft: crate::ui::Draft::default(),
            tx_cycle_edit: None,
            tx_cycle_buf: String::new(),
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
        for load in &mut self.bus_loads {
            load.clear();
        }
        // Along with the aggregates it reads: keeping the previous run's
        // interval memory would turn the first step of a new run into one
        // enormous measured period.
        self.spec = Spec::default();
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
    /// reopens it 鈥?Windows refuses to move/rename a mapped file.
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
            // Error frames included: they occupy the bus and the load view
            // counts them; per-message aggregation below skips them.
            if let Some(load) = self.bus_loads.get_mut(f.channel as usize) {
                let (arb, data) = {
                    let ch = &self.channels[f.channel as usize];
                    (ch.bitrate_kbps, ch.fd_data_kbps)
                };
                let wire = crate::load::wire_time_us(&f, arb, data);
                load.note(&f, wire);
            }
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
            for (key, d) in self.subscribed_values(&f) {
                let Some(entry) = self.subs.get_mut(&key) else {
                    continue;
                };
                entry.latest = d.phys;
                entry.unit = d.unit;
                entry.type_tag = d.type_tag;
                entry.label = d.label;
                entry.last_update_us = f.t_us;
                if f.t_us >= entry.last_sample_us + SAMPLE_INTERVAL_US || entry.history.is_empty() {
                    entry.push_sample(f.t_us, d.phys);
                }
            }
        }

        self.check_spec();

        if replay_done {
            self.measuring = false;
            self.close_writer();
            let dur = self.source.duration().unwrap_or(0) as f64 / 1e6;
            self.status = format!("replay finished at {dur:.2}s");
        }
    }

    /// Compare what arrived against what the databases promise.
    ///
    /// Once per step, not per frame: every verdict here is a claim about a
    /// *message's* timing or identity, and the loop above has already folded the
    /// frames into one aggregate per `(bus, id)`. Sweeping the aggregates turns a
    /// step of two hundred frames into one verdict each instead of two hundred
    /// latch writes, and it cannot read an aggregate mid-update the way a check
    /// inside that loop would.
    fn check_spec(&mut self) {
        let now = self.sim_t_us;
        // "Dropped" is the only verdict that needs a present tense. Replay runs
        // on the log's own timestamps, where "still going" has no meaning, so
        // only live simulation may call a message gone; pausing is covered
        // separately by `tick` not running at all. The other three are facts
        // about frames already seen and stay on in every mode.
        let live = matches!(self.mode, Mode::Virtual) && !self.trace_paused;
        let mut hits: Vec<((u8, u32, Kind), f64, f64)> = Vec::new();
        let mut seen: Vec<((u8, u32), u64)> = Vec::with_capacity(self.aggs.len());
        for (&key, agg) in &self.aggs {
            let (ch, id) = key;
            seen.push((key, agg.last_t_us));
            // No database on this bus means no opinion, not a clean bill.
            let Some(db) = self.channel_dbc(ch) else {
                continue;
            };
            let Some(m) = db.messages.get(&id) else {
                hits.push(((ch, id, Kind::Unknown), 0.0, 0.0));
                continue;
            };
            // A declaration of 0 is event-triggered, which the two timing
            // predicates below reject on their own; `None` is the database
            // saying nothing, and neither is a period to check against.
            let declared = m.cycle_us;
            // Our own frames are exempt from the length and period verdicts.
            // Driving a signal that reaches past the base length widens the
            // frame on purpose (see `tx_payload`), so judging a Tx frame by the
            // declared DLC would convict a configuration chosen deliberately;
            // and the generator row already offers to restore a hand-tuned
            // period. Transmitting an id the database lacks is still reported.
            if matches!(agg.dir, Direction::Rx) {
                if dlc_offender(agg.len, m.dlc) {
                    hits.push(((ch, id, Kind::Dlc), m.dlc as f64, f64::from(agg.len)));
                }
                // The interval since the previous step, never the running
                // average in `agg.cycle_us`: an EMA reads a five-fold stall as
                // 1.4x and takes twenty samples to converge, and `min_us` /
                // `max_us` latch forever, so either would hide or keep a
                // violation that a single real interval states plainly. A
                // message with no previous step, or with a step that brought it
                // no new frame, has no interval to judge yet.
                let elapsed = self
                    .spec
                    .previous(key)
                    .and_then(|from| agg.last_t_us.checked_sub(from))
                    .filter(|i| *i > 0);
                if let (Some(d), Some(interval)) = (declared, elapsed)
                    && cycle_offender(interval, d, self.spec_tol_pct)
                {
                    hits.push(((ch, id, Kind::Cycle), d as f64, interval as f64));
                }
            }
            if let (true, Some(d)) = (live, declared)
                && missing_offender(now, agg.last_t_us, d, self.spec_grace)
            {
                hits.push((
                    (ch, id, Kind::Missing),
                    d as f64,
                    now.saturating_sub(agg.last_t_us) as f64,
                ));
            }
        }
        for (key, declared, measured) in hits {
            self.spec.record(key, now, declared, measured);
        }
        for (key, last_t_us) in seen {
            self.spec.note(key, last_t_us);
        }
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
    // The tests rely on the parent's imports through `use super::*`; the
    // ones app.rs itself no longer needs are imported directly here.
    use crate::can::frame::{FrameFlags, MAX_CAN_FD_LEN};
    use crate::config::Config;
    use crate::sim::ValueSrc;

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
    fn simulated_node_state_follows_the_bus_it_lives_on() {
        let mut app = App::new();
        app.channels[0].sim_nodes.push("EngineECU".to_string());
        app.channels[1].sim_nodes.push("ABS".to_string());
        app.remove_channel(0);
        assert_eq!(
            app.channels[0].sim_nodes,
            ["ABS"],
            "the survivor keeps its own nodes instead of inheriting the deleted bus's"
        );
    }

    /// What one bus is actually putting on the wire right now.
    fn active_ids(app: &App, ch: u8) -> Vec<u32> {
        let mut ids: Vec<u32> = app
            .tx_list
            .iter()
            .filter(|t| t.channel == ch && t.active)
            .map(|t| t.id)
            .collect();
        ids.sort_unstable();
        ids
    }

    fn entry_of(app: &App, ch: u8, id: u32) -> &TxMsg {
        app.tx_list
            .iter()
            .find(|t| t.channel == ch && t.id == id)
            .expect("entry exists")
    }

    #[test]
    fn ticking_a_node_activates_only_its_own_messages() {
        let mut app = App::new();
        app.set_node_sim(1, "ABS", true);
        // assets/motbus.dbc:31,35,54 -- ABS owns these three, nobody else.
        assert_eq!(active_ids(&app, 1), [199, 200, 201]);
        assert!(active_ids(&app, 0).is_empty(), "the other bus untouched");
        assert!(
            app.tx_list
                .iter()
                .filter(|t| t.channel == 1 && t.active)
                .all(|t| t.next_t_us == 0),
            "a ticked node starts on the next tick, not one period later"
        );
        assert!(app.is_node_simulated(1, "ABS"));
        assert!(!app.is_node_simulated(1, "GearBox"), "not a side effect");
    }

    #[test]
    fn ticking_a_node_creates_the_entries_it_lacks() {
        let mut app = App::new();
        app.tx_list.clear();
        app.set_node_sim(1, "ABS", true);
        assert_eq!(
            active_ids(&app, 1),
            [199, 200, 201],
            "the generator refills from the DBC"
        );
        assert_eq!(app.tx_list.len(), 3, "and only this node's messages");
        assert_eq!(entry_of(&app, 1, 201).name, "ABSdata");
        assert_eq!(entry_of(&app, 1, 201).cycle_us, 50_000);
    }

    #[test]
    fn ticking_a_node_never_overwrites_a_tuned_cycle() {
        let mut app = App::new();
        let tuned = entry_of(&app, 1, 201).cycle_us;
        assert_eq!(tuned, 50_000, "what the DBC declares");
        let i = app
            .tx_list
            .iter()
            .position(|t| t.channel == 1 && t.id == 201)
            .unwrap();
        app.tx_list[i].cycle_us = 250_000;
        app.set_node_sim(1, "ABS", true);
        assert_eq!(
            app.tx_list[i].cycle_us, 250_000,
            "a period someone dialed in outlives the click"
        );
        assert!(app.tx_list[i].active, "but the entry is switched on");
    }

    #[test]
    fn unticking_a_node_keeps_its_entries_and_their_stimulus() {
        let mut app = App::new();
        app.set_node_sim(1, "ABS", true);
        let i = app
            .tx_list
            .iter()
            .position(|t| t.channel == 1 && t.id == 201)
            .unwrap();
        app.set_source(
            i,
            ValueSrc::new("CarSpeed", crate::sim::SrcKind::Ramp, 0.0, 300.0),
        );
        let before = app.tx_list.len();

        app.set_node_sim(1, "ABS", false);
        assert!(active_ids(&app, 1).is_empty(), "stopped sending");
        assert_eq!(app.tx_list.len(), before, "entries survive");
        assert_eq!(app.tx_list[i].srcs.len(), 1, "with the waveform attached");
        assert_eq!(app.tx_list[i].cycle_us, 50_000, "and the declared period");

        app.set_node_sim(1, "ABS", true);
        assert_eq!(
            app.tx_list[i].srcs.len(),
            1,
            "ticking it back on does not rebuild the entry"
        );
        assert!(app.tx_list[i].active);
    }

    /// Loading a different database does not rebuild the generator, so the
    /// only thing left to go by is the node stamped on each entry.
    #[test]
    fn unticking_still_silences_a_node_after_its_dbc_is_gone() {
        let mut app = App::new();
        app.set_node_sim(1, "ABS", true);
        assert_eq!(active_ids(&app, 1).len(), 3);
        app.channels[1].dbc = None;
        app.set_node_sim(1, "ABS", false);
        assert!(
            active_ids(&app, 1).is_empty(),
            "unchecking must work even with nothing to look up"
        );
        assert!(!app.is_node_simulated(1, "ABS"));
    }

    #[test]
    fn a_receive_only_node_can_still_be_ticked() {
        let mut app = App::new();
        app.set_node_sim(1, "DashBoard", true);
        assert!(active_ids(&app, 1).is_empty(), "it has no messages to send");
        assert!(
            app.is_node_simulated(1, "DashBoard"),
            "the intent is remembered anyway"
        );
    }

    /// The restore chip compares a row against this, so it has to stay the
    /// database's own opinion even after the row disagrees with it.
    #[test]
    fn the_declared_cycle_survives_a_hand_tuned_row() {
        let mut app = App::new();
        assert_eq!(app.dbc_cycle_us(1, 0xC9), Some(50_000), "ABSdata");
        assert_eq!(
            app.dbc_cycle_us(0, 0x100),
            None,
            "sample.dbc declares nothing"
        );
        assert_eq!(app.dbc_cycle_us(1, 0x5AA), None, "no such message");
        let i = app
            .tx_list
            .iter()
            .position(|t| t.channel == 1 && t.id == 0xC9)
            .unwrap();
        app.tx_list[i].cycle_us = 250_000;
        assert_eq!(
            app.dbc_cycle_us(1, 0xC9),
            Some(50_000),
            "not whatever the row currently says"
        );

        app.channels[0].dbc = Some(crate::dbc::load_dbc_str(CYCLE_TEST_DBC).unwrap());
        assert_eq!(
            app.dbc_cycle_us(0, 4096),
            Some(0),
            "a declared 0 is event-triggered, not undeclared"
        );
    }

    #[test]
    fn the_cycle_box_accepts_only_whole_milliseconds_in_range() {
        assert_eq!(cycle_from_ms_text("100"), Some(100_000));
        assert_eq!(cycle_from_ms_text("  133 "), Some(133_000));
        assert_eq!(cycle_from_ms_text("0"), Some(0), "0 is event-triggered");
        assert_eq!(
            cycle_from_ms_text("60000"),
            Some(60_000_000),
            "top of the range"
        );
        assert_eq!(cycle_from_ms_text(""), None, "half-deleted text");
        assert_eq!(cycle_from_ms_text("1.5"), None, "no sub-millisecond step");
        assert_eq!(cycle_from_ms_text("-1"), None);
        assert_eq!(cycle_from_ms_text("60001"), None, "past the ceiling");
        assert_eq!(cycle_from_ms_text("abc"), None);
    }

    /// The parser writes `""` for a transmitter the DBC never assigned, and
    /// that matches every unassigned message at once.
    const NO_OWNER_DBC: &str = r#"VERSION "roxy-can orphan test"

NS_ :

BU_: ECU

BO_ 4096 Orphan: 8 Vector__XXX
 SG_ S : 0|8@1+ (1,0) [0|0] "" ECU

"#;

    #[test]
    fn a_node_with_no_name_simulates_nothing() {
        let mut app = App::new();
        app.channels[0].dbc = Some(crate::dbc::load_dbc_str(NO_OWNER_DBC).unwrap());
        app.tx_list.retain(|t| t.channel != 0);
        app.add_tx(0, 4096);
        assert_eq!(entry_of(&app, 0, 4096).node, "", "unassigned");

        app.set_node_sim(0, "", true);
        assert!(
            app.channels[0].sim_nodes.is_empty(),
            "not even recorded as a tick"
        );
        assert!(
            active_ids(&app, 0).is_empty(),
            "an empty name must not adopt every message without an owner"
        );
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
            label: None,
            type_tag: String::new(),
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

    use crate::spec::Kind;

    /// A database covering the three declarations the monitor distinguishes:
    /// 100 on a declared 100 ms period, 300 declared event-triggered, and 200
    /// with no cycle at all. There is deliberately no `BA_DEF_DEF_` line, so an
    /// unannotated message gets no declaration rather than a default one.
    const SPEC_DBC: &str = r#"VERSION "roxy-can spec test"

NS_ :

BU_: ECU

BO_ 100 Periodic: 8 ECU
 SG_ S : 0|8@1+ (1,0) [0|0] "" ECU

BO_ 200 Undeclared: 8 ECU
 SG_ S : 0|8@1+ (1,0) [0|0] "" ECU

BO_ 300 EventMsg: 8 ECU
 SG_ S : 0|8@1+ (1,0) [0|0] "" ECU

BA_DEF_ BO_  "GenMsgCycleTime" INT 0 10000;
BA_ "GenMsgCycleTime" BO_ 100 100;
BA_ "GenMsgCycleTime" BO_ 300 0;
"#;

    /// A virtual bus with a silent generator, so everything the monitor sees
    /// arrived through `receive`.
    fn spec_app() -> App {
        let mut app = App::new();
        app.channels[0].dbc = Some(crate::dbc::load_dbc_str(SPEC_DBC).unwrap());
        app.tx_list.retain(|t| t.channel != 0);
        app.start_virtual();
        app
    }

    fn frame_at(t_us: u64, id: u32, len: u8, dir: Direction) -> CanFrame {
        CanFrame {
            t_us,
            channel: 0,
            id,
            extended: false,
            len,
            data: [0; MAX_CAN_FD_LEN],
            dir,
            flags: FrameFlags::NONE,
        }
    }

    /// Runs exactly one measurement step in which `frames` arrive and the
    /// simulation clock reads `t_us`. A scripted source is the only way to get
    /// received traffic through the real aggregation path without writing a log
    /// file, and it keeps the step exact: no wall clock, no sleeping.
    fn receive(app: &mut App, t_us: u64, frames: Vec<CanFrame>) {
        struct Scripted(Option<Vec<CanFrame>>);
        impl FrameSource for Scripted {
            fn poll(&mut self, _now_us: u64, out: &mut Vec<CanFrame>) {
                if let Some(v) = self.0.take() {
                    out.extend(v);
                }
            }
        }
        app.sim_t_us = t_us;
        app.source = Box::new(Scripted(Some(frames)));
        app.tick(t_us);
    }

    fn flagged(app: &App, ch: u8, id: u32, kind: Kind) -> bool {
        app.spec.rows.contains_key(&(ch, id, kind))
    }

    fn verdict(app: &App, ch: u8, id: u32, kind: Kind) -> crate::spec::Latch {
        app.spec.rows[&(ch, id, kind)]
    }

    #[test]
    fn an_identifier_the_database_lacks_is_reported_unknown() {
        let mut app = spec_app();
        receive(&mut app, 0, vec![frame_at(0, 0x777, 8, Direction::Rx)]);
        assert!(flagged(&app, 0, 0x777, Kind::Unknown));
        receive(
            &mut app,
            10_000,
            vec![frame_at(10_000, 100, 8, Direction::Rx)],
        );
        assert!(
            !flagged(&app, 0, 100, Kind::Unknown),
            "a declared id is not a violation"
        );
        app.stop();
    }

    #[test]
    fn a_bus_with_no_database_reports_nothing_at_all() {
        let mut app = spec_app();
        app.channels[0].dbc = None;
        assert!(app.channel_dbc(0).is_none(), "test setup: no database");
        receive(&mut app, 0, vec![frame_at(0, 0x777, 3, Direction::Rx)]);
        receive(&mut app, 900_000, vec![]);
        assert!(app.spec.rows.is_empty(), "no database, no opinion");
        app.stop();
    }

    /// The frame facts are judged only for traffic we did not produce: driving a
    /// signal past the base length widens a Tx frame on purpose, and the
    /// generator row already offers to restore a hand-tuned period.
    #[test]
    fn our_own_transmission_is_never_a_cycle_or_dlc_violation() {
        let mut tx = spec_app();
        receive(&mut tx, 0, vec![frame_at(0, 100, 6, Direction::Tx)]);
        receive(
            &mut tx,
            115_000,
            vec![frame_at(115_000, 100, 6, Direction::Tx)],
        );
        assert!(!flagged(&tx, 0, 100, Kind::Dlc), "we chose that length");
        assert!(!flagged(&tx, 0, 100, Kind::Cycle));

        let mut rx = spec_app();
        receive(&mut rx, 0, vec![frame_at(0, 100, 6, Direction::Rx)]);
        receive(
            &mut rx,
            115_000,
            vec![frame_at(115_000, 100, 6, Direction::Rx)],
        );
        assert!(
            flagged(&rx, 0, 100, Kind::Dlc),
            "the same frame received is"
        );
        assert!(flagged(&rx, 0, 100, Kind::Cycle));
        rx.stop();
        tx.stop();
    }

    #[test]
    fn a_frame_shorter_than_the_declared_size_is_a_dlc_mismatch() {
        let mut app = spec_app();
        receive(&mut app, 0, vec![frame_at(0, 100, 6, Direction::Rx)]);
        assert_eq!(
            verdict(&app, 0, 100, Kind::Dlc),
            crate::spec::Latch {
                count: 1,
                first_t_us: 0,
                last_t_us: 0,
                declared: 8.0,
                measured: 6.0,
            }
        );
        app.stop();
    }

    #[test]
    fn timing_is_silent_where_the_database_declares_no_cycle() {
        let mut app = spec_app();
        assert!(
            app.dbc_cycle_us(0, 200).is_none(),
            "test setup: 200 declares no period"
        );
        receive(&mut app, 0, vec![frame_at(0, 200, 8, Direction::Rx)]);
        receive(
            &mut app,
            5_000_000,
            vec![frame_at(5_000_000, 200, 8, Direction::Rx)],
        );
        assert!(
            !flagged(&app, 0, 200, Kind::Cycle),
            "five seconds between two frames promises nothing was broken"
        );
        assert!(!flagged(&app, 0, 200, Kind::Missing));
        app.stop();
    }

    #[test]
    fn an_event_triggered_message_is_never_reported_missing() {
        let mut app = spec_app();
        assert_eq!(
            app.dbc_cycle_us(0, 300),
            Some(0),
            "test setup: a declared 0 means event-triggered"
        );
        receive(&mut app, 0, vec![frame_at(0, 300, 8, Direction::Rx)]);
        receive(&mut app, 5_000_000, vec![]);
        assert!(!flagged(&app, 0, 300, Kind::Missing));
        assert!(!flagged(&app, 0, 300, Kind::Cycle));
        app.stop();
    }

    /// The report must not become a list of everyone we chose not to simulate:
    /// a virtual bus only ever carries the nodes the user switched on.
    #[test]
    fn a_message_that_never_appeared_is_not_reported_missing() {
        let mut app = spec_app();
        assert_eq!(
            app.dbc_cycle_us(0, 100),
            Some(100_000),
            "test setup: 100 is declared periodic"
        );
        run_sim(&mut app, 20, 50_000);
        assert!(
            app.spec.rows.is_empty(),
            "never seen is not the same as dropped: {:?}",
            app.spec.rows.keys().collect::<Vec<_>>()
        );
        app.stop();
    }

    #[test]
    fn a_message_that_went_silent_beyond_the_grace_is_reported_missing() {
        let mut app = spec_app();
        receive(&mut app, 0, vec![frame_at(0, 100, 8, Direction::Rx)]);
        receive(
            &mut app,
            100_000,
            vec![frame_at(100_000, 100, 8, Direction::Rx)],
        );
        receive(&mut app, 300_000, vec![]);
        assert!(
            !flagged(&app, 0, 100, Kind::Missing),
            "two silent periods is still inside a grace of three"
        );
        receive(&mut app, 420_000, vec![]);
        assert!(flagged(&app, 0, 100, Kind::Missing));
        assert_eq!(verdict(&app, 0, 100, Kind::Missing).measured, 320_000.0);
        app.stop();
    }

    #[test]
    fn the_cycle_check_uses_the_last_interval_not_the_running_average() {
        let mut app = spec_app();
        for i in 0..10u64 {
            let t = i * 100_000;
            receive(&mut app, t, vec![frame_at(t, 100, 8, Direction::Rx)]);
        }
        assert!(!flagged(&app, 0, 100, Kind::Cycle), "ten on-time frames");
        // 115 ms is 15% late, which the aggregate's running average smooths down
        // to 1.5% and would never report.
        receive(
            &mut app,
            1_015_000,
            vec![frame_at(1_015_000, 100, 8, Direction::Rx)],
        );
        assert!(
            flagged(&app, 0, 100, Kind::Cycle),
            "agg.cycle_us reads 1.5% off here; only the raw interval is 15%"
        );
        assert_eq!(verdict(&app, 0, 100, Kind::Cycle).measured, 115_000.0);
        app.stop();
    }

    #[test]
    fn the_tolerance_setting_decides_how_late_is_late() {
        let mut app = spec_app();
        for i in 0..10u64 {
            let t = i * 100_000;
            receive(&mut app, t, vec![frame_at(t, 100, 8, Direction::Rx)]);
        }
        app.spec_tol_pct = 20;
        receive(
            &mut app,
            1_015_000,
            vec![frame_at(1_015_000, 100, 8, Direction::Rx)],
        );
        assert!(
            !flagged(&app, 0, 100, Kind::Cycle),
            "15% late is clean at a 20% tolerance"
        );
        app.spec_tol_pct = 5;
        receive(
            &mut app,
            1_126_000,
            vec![frame_at(1_126_000, 100, 8, Direction::Rx)],
        );
        assert!(
            flagged(&app, 0, 100, Kind::Cycle),
            "the next interval is only 11% late, but the tolerance now says 5"
        );
        app.stop();
    }

    #[test]
    fn the_grace_setting_decides_when_silence_counts_as_dropout() {
        let mut app = spec_app();
        receive(&mut app, 0, vec![frame_at(0, 100, 8, Direction::Rx)]);
        app.spec_grace = 10;
        receive(&mut app, 950_000, vec![]);
        assert!(
            !flagged(&app, 0, 100, Kind::Missing),
            "9.5 periods of silence is inside a grace of ten"
        );
        app.spec_grace = 2;
        receive(&mut app, 960_000, vec![]);
        assert!(
            flagged(&app, 0, 100, Kind::Missing),
            "tightening the grace convicts the same continuing silence"
        );
        app.stop();
    }

    #[test]
    fn one_bad_interval_is_counted_once_and_never_forgotten() {
        let mut app = spec_app();
        receive(&mut app, 0, vec![frame_at(0, 100, 8, Direction::Rx)]);
        receive(
            &mut app,
            200_000,
            vec![frame_at(200_000, 100, 8, Direction::Rx)],
        );
        assert_eq!(verdict(&app, 0, 100, Kind::Cycle).count, 1);
        for i in 1..=5u64 {
            // Continue the declared spacing from 200 ms, so each of these is a
            // clean interval rather than another gap.
            let t = 200_000 + i * 100_000;
            receive(&mut app, t, vec![frame_at(t, 100, 8, Direction::Rx)]);
        }
        assert_eq!(
            verdict(&app, 0, 100, Kind::Cycle).count,
            1,
            "a verdict from min/max would keep convicting"
        );
        app.stop();
    }

    #[test]
    fn the_first_sample_of_a_message_is_never_a_cycle_violation() {
        let mut app = spec_app();
        // Arrives five periods late, so the only thing standing between this and
        // a verdict is that nothing preceded it.
        receive(
            &mut app,
            500_000,
            vec![frame_at(500_000, 100, 8, Direction::Rx)],
        );
        assert_eq!(
            app.aggs[&(0, 100)].count,
            1,
            "test setup: one frame, no interval yet"
        );
        assert!(!flagged(&app, 0, 100, Kind::Cycle));
        app.stop();
    }

    #[test]
    fn a_step_that_brought_no_new_frame_is_not_a_new_interval() {
        let mut app = spec_app();
        receive(&mut app, 0, vec![frame_at(0, 100, 8, Direction::Rx)]);
        receive(
            &mut app,
            100_000,
            vec![frame_at(100_000, 100, 8, Direction::Rx)],
        );
        assert!(!flagged(&app, 0, 100, Kind::Cycle), "test setup: on time");
        // Nothing arrives for two steps. The aggregate has not moved, so there
        // is no period to measure -- only the silence clock runs here.
        receive(&mut app, 200_000, vec![]);
        receive(&mut app, 300_000, vec![]);
        assert!(!flagged(&app, 0, 100, Kind::Cycle));
        app.stop();
    }

    #[test]
    fn replay_traffic_never_raises_a_missing_violation() {
        let mut app = spec_app();
        app.mode = Mode::Replay;
        receive(&mut app, 0, vec![frame_at(0, 100, 8, Direction::Rx)]);
        receive(&mut app, 5_000_000, vec![]);
        assert!(
            !flagged(&app, 0, 100, Kind::Missing),
            "a log's clock cannot say what is still talking"
        );
        // The frame facts stay judged in replay, because they need no clock.
        receive(
            &mut app,
            5_100_000,
            vec![frame_at(5_100_000, 100, 5, Direction::Rx)],
        );
        assert!(flagged(&app, 0, 100, Kind::Dlc));
        app.stop();
    }

    #[test]
    fn a_paused_clock_does_not_make_a_message_missing() {
        let mut app = spec_app();
        receive(&mut app, 0, vec![frame_at(0, 100, 8, Direction::Rx)]);
        app.trace_paused = true;
        assert!(app.trace_paused, "test setup: paused");
        // The real loop never calls `tick` while paused; driving it anyway
        // shows the verdict is gated on the pause itself, not on a missing step.
        for i in 1..=10u64 {
            receive(&mut app, i * 200_000, vec![]);
        }
        assert!(!flagged(&app, 0, 100, Kind::Missing));
        app.stop();
    }

    #[test]
    fn a_verdict_follows_its_bus_when_an_earlier_bus_is_deleted() {
        let mut app = spec_app();
        // Move the fixture database onto the second bus and leave the first
        // without one, then delete the first.
        app.channels[1].dbc = app.channels[0].dbc.take();
        receive(
            &mut app,
            0,
            vec![CanFrame {
                channel: 1,
                ..frame_at(0, 100, 6, Direction::Rx)
            }],
        );
        assert!(flagged(&app, 1, 100, Kind::Dlc), "test setup: on bus 1");
        app.remove_channel(0);
        assert!(
            flagged(&app, 0, 100, Kind::Dlc),
            "the row must move with the bus, not stay behind at the old index"
        );
        app.stop();
    }

    #[test]
    fn the_monitor_forgets_everything_when_a_new_run_starts() {
        let mut app = spec_app();
        receive(&mut app, 0, vec![frame_at(0, 100, 8, Direction::Rx)]);
        receive(
            &mut app,
            200_000,
            vec![frame_at(200_000, 100, 8, Direction::Rx)],
        );
        assert!(
            flagged(&app, 0, 100, Kind::Cycle),
            "test setup: a verdict worth forgetting"
        );
        app.start_virtual();
        assert!(app.spec.rows.is_empty(), "the report belongs to a run");
        assert_eq!(
            app.spec.previous((0, 100)),
            None,
            "and so does the interval memory"
        );
        // Five seconds later on a clock that started over. Against the previous
        // run's interval this would read as one enormous measured period.
        receive(
            &mut app,
            5_000_000,
            vec![frame_at(5_000_000, 100, 8, Direction::Rx)],
        );
        assert!(
            !flagged(&app, 0, 100, Kind::Cycle),
            "a stale interval from the old run would convict this frame"
        );
        app.stop();
    }

    /// Same mux layout as `MUX_DBC` in the dbc tests, one bus, generator
    /// silenced so only the frames built here reach the sampler.
    const MUX_SAMPLE_DBC: &str = r#"VERSION "roxy-can mux sampling test"

NS_ :

BU_: ECU

BO_ 400 Muxed: 8 ECU
 SG_ Switch M : 0|8@1+ (1,0) [0|0] "" ECU
 SG_ G1_A m1 : 16|16@1+ (0.1,0) [0|0] "" ECU
 SG_ G2_C m2 : 16|16@1+ (0.5,0) [0|0] "" ECU
"#;

    fn mux_app() -> App {
        let mut app = App::new();
        app.channels[0].dbc = Some(crate::dbc::load_dbc_str(MUX_SAMPLE_DBC).unwrap());
        app.tx_list.retain(|t| t.channel != 0);
        app.start_virtual();
        app
    }

    fn mux_frame(t_us: u64, switch: u8) -> CanFrame {
        let mut f = frame_at(t_us, 400, 8, Direction::Rx);
        f.data[0] = switch;
        f.data[2] = 100;
        f
    }

    #[test]
    fn a_signal_of_the_inactive_group_is_not_sampled() {
        let mut app = mux_app();
        let g1 = (0u8, 400u32, "G1_A".to_string());
        let g2 = (0u8, 400u32, "G2_C".to_string());
        app.subscribe(g1.clone());
        app.subscribe(g2.clone());
        assert!(app.subs.contains_key(&g1), "both signals are subscribed");
        assert!(app.subs.contains_key(&g2));

        receive(&mut app, 100_000, vec![mux_frame(100_000, 1)]);
        assert!(
            app.subs[&g2].history.is_empty(),
            "group 2 was not in the frame, so it has no samples"
        );
        assert!(
            !app.subs[&g1].history.is_empty(),
            "group 1 was active and got sampled"
        );
        app.stop();
    }

    #[test]
    fn a_group_signal_gains_samples_once_its_group_is_switched_in() {
        let mut app = mux_app();
        let g2 = (0u8, 400u32, "G2_C".to_string());
        app.subscribe(g2.clone());

        receive(&mut app, 100_000, vec![mux_frame(100_000, 1)]);
        let before = app.subs[&g2].history.len();
        receive(&mut app, 200_000, vec![mux_frame(200_000, 2)]);
        assert_eq!(before, 0, "inactive until the switch changes");
        assert!(
            app.subs[&g2].history.len() > before,
            "once its group is switched in, the signal is sampled again"
        );
        app.stop();
    }

    fn rx_frame(t_us: u64, id: u32, len: u8, flags: FrameFlags) -> CanFrame {
        CanFrame {
            t_us,
            channel: 0,
            id,
            extended: false,
            len,
            data: [0u8; MAX_CAN_FD_LEN],
            dir: Direction::Rx,
            flags,
        }
    }

    fn quiet_app() -> App {
        let mut app = App::new();
        app.start_virtual();
        // Only frames built by the test may reach the bus statistics.
        app.tx_list.retain(|t| t.channel != 0);
        app
    }

    /// The load view's whole point: the same traffic reads differently at
    /// different bitrates, and each number agrees with a hand calculation.
    /// 100 classic 8-byte frames over one second are 111 bits each, 222 碌s of
    /// wire time at 500 kbit/s -- 2.22 % of the bus.
    #[test]
    fn bus_load_matches_the_hand_calculation() {
        let mut app = quiet_app();
        let frames: Vec<CanFrame> = (0..100)
            .map(|i| rx_frame((i + 1) * 10_000, 0x100, 8, FrameFlags::NONE))
            .collect();
        receive(&mut app, 1_000_000, frames);
        assert!((app.bus_loads[0].frame_rate() - 100.0).abs() < 1e-9);
        assert!(
            (app.bus_loads[0].load() - 0.0222).abs() < 1e-9,
            "got {}, expected 2.22 %",
            app.bus_loads[0].load()
        );
        app.stop();
    }

    /// A 64-byte BRS payload clocks out of the data phase, so the same frame
    /// stream is far cheaper at a 2 Mbit/s data phase than at 500 kbit/s --
    /// the acceptance case from the capability backlog, and exactly what a
    /// frame-counting "load" would get wrong.
    #[test]
    fn a_brs_payload_gets_cheaper_at_a_faster_data_phase() {
        let mut app = quiet_app();
        let frames: Vec<CanFrame> = (0..100)
            .map(|i| {
                rx_frame(
                    (i + 1) * 10_000,
                    0x200,
                    64,
                    FrameFlags::FD.union(FrameFlags::BRS),
                )
            })
            .collect();
        // 55 arbitration bits at 500 kbit/s + 552 data bits at 2 Mbit/s =
        // 110 + 276 = 386 碌s per frame -> 3.86 % load.
        receive(&mut app, 1_000_000, frames.clone());
        assert!(
            (app.bus_loads[0].load() - 0.0386).abs() < 1e-9,
            "got {}, expected 3.86 %",
            app.bus_loads[0].load()
        );
        // Same frames, data phase throttled to the arbitration rate: all 607
        // bits at 500 kbit/s = 1214 碌s -> 12.14 %.
        app.channels[0].fd_data_kbps = 500;
        for load in &mut app.bus_loads {
            load.clear();
        }
        receive(&mut app, 3_000_000, frames);
        assert!(
            (app.bus_loads[0].load() - 0.1214).abs() < 1e-9,
            "got {}, expected 12.14 %",
            app.bus_loads[0].load()
        );
        app.stop();
    }

    /// Error frames never enter per-message aggregation, but the bus view
    /// must still report them: they occupy the bus and are the thing you are
    /// usually hunting.
    #[test]
    fn error_frames_are_counted_per_bus() {
        let mut app = quiet_app();
        receive(
            &mut app,
            1_000,
            vec![rx_frame(1_000, 0x300, 0, FrameFlags::ERROR)],
        );
        assert_eq!(app.bus_loads[0].errors, 1);
        receive(
            &mut app,
            2_000,
            vec![rx_frame(2_000, 0x300, 0, FrameFlags::ERROR)],
        );
        assert_eq!(app.bus_loads[0].errors, 2);
        assert!(
            app.bus_loads[1].errors == 0,
            "bus 1 saw nothing and says so"
        );
        app.stop();
    }

    /// A fresh run must not inherit the previous run's load: the window is
    /// cleared with the aggregates it accompanies.
    #[test]
    fn restarting_measurement_clears_the_bus_windows() {
        let mut app = quiet_app();
        receive(
            &mut app,
            1_000,
            vec![rx_frame(1_000, 0x100, 8, FrameFlags::NONE)],
        );
        assert!(app.bus_loads[0].load() > 0.0);
        app.reset_time();
        assert_eq!(app.bus_loads[0].load(), 0.0);
        assert_eq!(app.bus_loads[0].errors, 0);
    }
}


