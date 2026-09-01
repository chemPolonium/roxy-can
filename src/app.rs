use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::time::Instant;

use crate::can::frame::{CanFrame, Direction};
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
use crate::generator::tx_payload;
pub use crate::generator::{TX_CYCLE_MAX_MS, TxMsg, cycle_from_ms_text};
pub use crate::observe::{DataWindow, GfxSignal, GraphicsWindow, SampleCache, Subscription};
pub use crate::project::PendingAction;
pub use crate::workspace::{
    Desktop, MsgWin, PopupTarget, SigScope, StatsWin, TraceWin, WindowKind,
};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Virtual,
    Replay,
}

pub struct App {
    pub measuring: bool,
    /// ASC recording state: the checkbox intent plus the open file.
    pub recorder: crate::recorder::Recorder,
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
    pub subs: HashMap<(u8, u32, String), Subscription>,
    pub aggs: HashMap<(u8, u32), MessageAgg>,
    /// Per-bus load / frame-rate / error rolling state, one entry per
    /// channel. Fed from the same frame loop as `aggs`.
    pub bus_loads: Vec<crate::load::BusLoad>,
    pub triggers: Vec<crate::trigger::Trigger>,
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
    buf: Vec<CanFrame>,
}

impl App {
    pub fn new() -> Self {
        let mut app = App {
            measuring: false,
            recorder: crate::recorder::Recorder::new(),
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
            subs: HashMap::new(),
            aggs: HashMap::new(),
            bus_loads: vec![crate::load::BusLoad::new(), crate::load::BusLoad::new()],
            triggers: Vec::new(),
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
        self.recorder.close();
        self.source = Box::new(VirtualSource::new());
        self.mode = Mode::Virtual;
        self.run_mode = Mode::Virtual;
        self.reset_time();
        self.measuring = true;
        self.status = "measuring (virtual)".to_string();
        if self.recorder.recording {
            match self.recorder.open() {
                Ok(path) => self.status = format!("recording to {path}"),
                Err(e) => self.status = format!("record failed: {e}"),
            }
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
        !self.log_path.trim().is_empty() || !self.recorder.last_record.trim().is_empty()
    }

    pub fn stop(&mut self) {
        self.measuring = false;
        self.recorder.close();
        self.replay_reset_pending = true;
        self.status = "stopped".to_string();
    }

    pub fn toggle_record(&mut self) {
        if self.recorder.recording {
            self.recorder.close();
            self.recorder.recording = false;
        } else {
            self.recorder.recording = true;
            // While stopped, the file is created by the next start_virtual;
            // checking Record must not leave an empty record file behind.
            if self.measuring {
                let opened = self.recorder.open();
                self.recorder.recording = opened.is_ok();
                match opened {
                    Ok(path) => self.status = format!("recording to {path}"),
                    Err(e) => self.status = format!("record failed: {e}"),
                }
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
                self.recorder.last_record.clone()
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
        self.recorder.close();
        // Replay just re-emits an existing log; recording it would
        // only duplicate the file, so drop the Record state.
        self.recorder.recording = false;
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
        // In replay the simulation clock is owned by `tick`, which reads it
        // off the log frames; adding wall time here would let the generator
        // march on while the log is quiet and snap back on the next frame.
        if !matches!(self.mode, Mode::Replay) {
            self.sim_t_us += now.saturating_sub(self.sim_prev_us);
        }
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
        self.buf.clear();
        self.source.poll(now_us, &mut self.buf);
        let source_empty = self.buf.is_empty();

        // The log clock is primary while replaying: `sim_t_us` follows the
        // newest log frame's own stamp and holds between frames, so injected
        // frames land on the same timeline the log carries and every
        // consumer -- aggregates, plots, the spec check -- sees one clock.
        // Live simulation keeps the wall-derived clock `update` maintains.
        if matches!(self.mode, Mode::Replay)
            && let Some(t) = self.buf.last().map(|f| f.t_us)
        {
            self.sim_t_us = t;
        }
        let sim = self.sim_t_us;

        // Generators transmit in every running mode: an active entry during
        // replay injects onto the log timeline, which is what makes
        // "replay a real log and stir a few frames in" possible.
        let channels = &self.channels;
        for tx in &mut self.tx_list {
            if matches!(self.mode, Mode::Replay) && tx.next_t_us == 0 {
                // Log time has no slot zero: an entry picked up mid-log is
                // anchored at the playhead instead of emitting one frame
                // dated the epoch.
                tx.next_t_us = sim;
            }
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

        let replay_done =
            matches!(self.mode, Mode::Replay) && source_empty && self.source.is_done();

        // Index walk rather than `for &f in &self.buf`: a frame is
        // copied out one at a time so `eval_triggers` can take `&mut
        // self` without the iterator holding `buf` borrowed.
        for i in 0..self.buf.len() {
            let f = self.buf[i];
            // Triggers judge the frame before anything else consumes it,
            // so a trigger that starts a recording captures the very
            // frame that fired it.
            self.eval_triggers(&f);
            self.recorder.write(&f);
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
            self.recorder.close();
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
#[path = "app_tests.rs"]
mod tests;
