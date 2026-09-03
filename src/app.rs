use std::path::PathBuf;
use std::time::Instant;

use crate::can::frame::{CanFrame, Direction, FrameFlags};
use crate::log::open_stream;
use crate::spec::{GRACE_CYCLES, TOLERANCE_PERCENT};

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
/// Live sampling aims for this many points across the smallest open
/// Graphics window, then clamps into `[MIN_STRIDE_US .. SAMPLE_INTERVAL_US]`:
/// a 0.1 s window samples every 500 µs (CANoe-style -- zooming in reveals
/// every update), while the hour window keeps the 50 ms memory bound.
pub(crate) const STRIDE_POINTS_PER_WINDOW: u64 = 200;
pub(crate) const MIN_STRIDE_US: u64 = 1_000;
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
pub use crate::generator::{TX_CYCLE_MAX_MS, cycle_from_ms_text};
pub use crate::observe::{DataWindow, GfxSignal, GraphicsWindow, SampleCache, YMode};
pub use crate::project::PendingAction;
pub use crate::workspace::{
    Desktop, MsgWin, PopupTarget, SigScope, StatsWin, TraceWin, WindowKind,
};

/// "{:.2}" milliseconds, the Min/Avg/Max cell format shared by the snapshot
/// builders below.
fn ms_text(v: f64) -> String {
    format!("{:.2}", v / 1000.0)
}

/// One Message Statistics row as throttled text, refreshed on the text gate
/// (see [`App::sync_stats_text`]): built strings, ready to draw.
#[derive(Clone)]
pub struct StatsRowText {
    pub label: String,
    pub bus: String,
    pub count: String,
    pub min: String,
    pub avg: String,
    pub max: String,
    pub len: String,
    pub flags: FrameFlags,
    pub share: String,
}

/// One Messages row as throttled text (see [`App::sync_msg_text`]); the
/// expanded tree draws the pre-decoded signal name/value pairs.
#[derive(Clone)]
pub struct MsgRowText {
    pub label: String,
    pub bus: String,
    pub dir: &'static str,
    pub count: String,
    pub cycle: String,
    pub flags: FrameFlags,
    pub data: String,
    pub signals: Vec<(String, String)>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Mode {
    #[default]
    Virtual,
    Replay,
}

pub struct App {
    /// The bus half (TODO.md 主线): simulation clock, channels, generator.
    /// Fields migrate here slice by slice; the `Deref` below keeps them
    /// visible at the old paths during the move.
    pub core: crate::bus::BusCore,
    /// The bus as of this frame's one snapshot read; UI rendering never
    /// touches `core` state directly, only this copy.
    pub snap: crate::bus::Snapshot,
    pub quit: bool,
    pub t0: Instant,
    pub status: String,
    /// Absolute path of the log currently loaded for replay (`.asc`, `.blf`).
    pub log_path: String,
    /// One-line summary from the stream's `describe()`, e.g. "BLF4, 41.2 s".
    pub log_info: Option<String>,
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
    /// The trigger the Triggers window is editing, if any.
    pub trigger_sel: Option<usize>,
    pub(crate) trig_id_buf: String,
    pub(crate) trig_edit_sel: Option<usize>,
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
    pub show_triggers: bool,
    pub show_bus_stats: bool,
    pub show_spec: bool,
    pub show_id_filter: bool,
    pub show_shortcuts: bool,
    pub show_about: bool,
    pub id_filter_search: String,
    pub gen_search: String,
    pub popup_target: Option<PopupTarget>,
    pub focus_title: Option<String>,
    pub net_selected: usize,
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
    /// The generator's editable data box while it has focus: row index plus
    /// the text typed so far. The live buffer is frontend draft state -- the
    /// bus only sees the parsed payload once the edit commits (via
    /// `SetEntryHex`).
    pub tx_data_edit: Option<(usize, String)>,
    pub last_tick_us: u64,
    /// How often number readouts (Data values, Statistics, Messages, the
    /// status bar) re-render, in Hz; 0 follows the frame rate. Curves and
    /// bars always render at full frame rate -- a 60 fps stream of changing
    /// digits is unreadable, which is what this throttles.
    pub text_rate_hz: u32,
    /// True on frames where throttled text content should re-render.
    pub text_fresh: bool,
    last_text_refresh: std::time::Instant,
    /// The status bar's "| frames: .. | f/s | trace | signals" line as of the
    /// last throttled text refresh; the state and replay readouts beside it
    /// stay live. See [`App::sync_status_text`].
    pub(crate) status_counters: String,
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
}

impl std::ops::Deref for App {
    type Target = crate::bus::BusCore;

    /// Migration scaffolding (TODO.md 主线 阶段 1): bus fields moved to
    /// `BusCore` stay reachable at `self.<field>` while the slices land.
    /// The stage-2 command boundary starts unwinding this.
    fn deref(&self) -> &Self::Target {
        &self.core
    }
}

impl std::ops::DerefMut for App {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.core
    }
}

impl App {
    pub fn new() -> Self {
        let mut core = crate::bus::BusCore::new(vec![
            crate::load::BusLoad::new(),
            crate::load::BusLoad::new(),
        ]);
        core.channels = vec![
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
        ];
        let mut app = App {
            core,
            snap: crate::bus::Snapshot::default(),
            quit: false,
            t0: Instant::now(),
            status: "stopped".to_string(),
            log_path: String::new(),
            log_info: None,
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
            trigger_sel: None,
            trig_id_buf: String::new(),
            trig_edit_sel: None,
            spec_show: [true; 4],
            spec_tol_pct: TOLERANCE_PERCENT,
            spec_grace: GRACE_CYCLES,
            symbol_search: String::new(),
            show_tx: true,
            show_network: true,
            show_measurement: true,
            show_buses: false,
            show_triggers: false,
            show_bus_stats: false,
            show_spec: false,
            show_id_filter: false,
            show_shortcuts: false,
            show_about: false,
            id_filter_search: String::new(),
            gen_search: String::new(),
            popup_target: None,
            focus_title: None,
            net_selected: 0,
            tx_pick: 0,
            src_edit: None,
            src_seq_buf: String::new(),
            src_draft: None,
            num_draft: crate::ui::Draft::default(),
            tx_cycle_edit: None,
            tx_cycle_buf: String::new(),
            tx_data_edit: None,
            last_tick_us: 0,
            text_rate_hz: 10,
            text_fresh: true,
            last_text_refresh: std::time::Instant::now(),
            status_counters: String::new(),
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

    /// Re-reads the snapshot. The frame loop calls this via `update`/`tick`
    /// /`send`; tests that poke core state directly call it explicitly.
    pub(crate) fn refresh_snapshot(&mut self) {
        self.snap = self.core.snapshot();
    }

    /// The frontend's post office to the bus. Single-threaded this applies
    /// the command immediately and surfaces whatever status it produced;
    /// stage 3 swaps the body for an mpsc push, and status comes back in
    /// the snapshot instead. Every UI write action funnels through here --
    /// that funnel is what makes the core movable.
    pub fn send(&mut self, cmd: crate::bus::BusCommand) {
        let mut status = String::new();
        self.core.handle(cmd, &mut status);
        if !status.is_empty() {
            self.status = status;
        }
        // Re-read after every write, so code that acts right after a
        // command sees the post-command bus, never a stale snapshot.
        self.refresh_snapshot();
    }

    pub fn start_virtual(&mut self) {
        // The wall-clock anchors are frontend state (`now_us` reads `t0`);
        // everything the reset blanks on the bus itself is `reset_run`.
        self.t0 = Instant::now();
        self.last_tick_us = 0;
        self.send(crate::bus::BusCommand::StartVirtual);
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
        if mode == self.snap.run_mode {
            return;
        }
        if self.snap.measuring {
            self.stop();
        }
        self.send(crate::bus::BusCommand::SetRunMode(mode));
    }

    fn can_replay(&self) -> bool {
        !self.log_path.trim().is_empty() || !self.recorder.last_record.trim().is_empty()
    }

    pub fn stop(&mut self) {
        self.send(crate::bus::BusCommand::Stop);
        // Set by `stop`: the next Play re-opens the log from zero instead
        // of resuming wherever the scrub bar left the playhead.
        self.replay_reset_pending = true;
    }

    pub fn toggle_record(&mut self) {
        self.send(crate::bus::BusCommand::ToggleRecord);
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

    /// Starts playback of the selected log (the path the user loaded, or
    /// the last recording as a fallback). The open, the silence-set scan
    /// and the source swap live in the command; what stays here is the
    /// frontend's own memory: which path it selected, the wall-clock
    /// anchors, and the not-a-resume mark.
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
        self.t0 = Instant::now();
        self.last_tick_us = 0;
        self.replay_reset_pending = false;
        self.send(crate::bus::BusCommand::StartReplay {
            path,
            speed: self.replay_speed,
        });
    }

    /// Changes replay speed; takes effect immediately if a replay is
    /// running. The remembered choice is frontend state -- the combo
    /// displays it and a fresh replay starts at it -- while the applied
    /// rate belongs to the bus's source.
    pub fn set_replay_speed(&mut self, speed: f64) {
        self.replay_speed = speed;
        self.send(crate::bus::BusCommand::SetReplaySpeed(speed));
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

    /// Moves the replay playhead to `t_s` seconds.
    pub fn seek_replay_seconds(&mut self, t_s: f64) {
        self.send(crate::bus::BusCommand::SeekReplay(t_s));
    }

    /// True when a scrubbed replay is parked mid-log and Play should pick up
    /// from there instead of re-opening the file at zero.
    fn can_resume_replay(&self) -> bool {
        matches!(self.mode, Mode::Replay)
            && !self.replay_reset_pending
            && self.source.position().is_some()
    }

    /// Resumes a scrubbed replay in place: the wall clock restarts here on
    /// the frontend, the bus unfreezes and resumes measuring via the
    /// command. Captured history is untouched, so playback continues from
    /// the scrubbed position.
    fn resume_replay(&mut self) {
        self.t0 = Instant::now();
        self.send(crate::bus::BusCommand::ResumeReplay {
            speed: self.replay_speed,
        });
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
            self.send(crate::bus::BusCommand::SetTracePaused(!self.trace_paused));
        } else {
            self.play();
        }
    }

    /// Current replay position and total duration in seconds (None when
    /// the active source has no timeline). Reads this frame's snapshot.
    pub fn replay_position(&self) -> Option<(f64, f64)> {
        let (pos, dur) = self.snap.replay?;
        Some((pos as f64 / 1e6, dur as f64 / 1e6))
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

    /// Refreshes Message Statistics window `i`'s throttled text snapshot:
    /// the header line and one pre-formatted struct per row. Same gate as
    /// [`App::sync_data_text`] -- a no-op unless the text gate says so or the
    /// visible message set changed, so a new id shows up without waiting a
    /// full period.
    pub(crate) fn sync_stats_text(&mut self, i: usize) {
        let (scope, manual) = {
            let w = &self.stats_windows[i];
            (w.scope, w.manual.clone())
        };
        // Aggregates come from this frame's snapshot, never the live map.
        let mut aggs: Vec<&MessageAgg> = self
            .snap
            .aggs
            .iter()
            .filter(|a| App::scope_match(scope, &manual, a.channel, a.id))
            .collect();
        aggs.sort_by_key(|a| (a.channel, a.id));
        let keys: Vec<(u8, u32)> = aggs.iter().map(|a| (a.channel, a.id)).collect();
        if self.stats_windows[i].text_keys == keys && !self.text_fresh {
            return;
        }
        let total: u64 = aggs.iter().map(|a| a.count).sum();
        let mut rows = Vec::with_capacity(aggs.len());
        for agg in &aggs {
            let agg = **agg;
            let id_str = if agg.extended {
                format!("{:08X}x", agg.id)
            } else {
                format!("{:03X}", agg.id)
            };
            let name = self.message_name(agg.channel, agg.id).unwrap_or("-");
            rows.push(StatsRowText {
                label: format!("{id_str}  {name}"),
                bus: self.channel_name(agg.channel),
                count: agg.count.to_string(),
                min: if agg.count >= 2 {
                    ms_text(agg.min_us)
                } else {
                    "-".to_string()
                },
                avg: if agg.count >= 2 {
                    ms_text(agg.cycle_us)
                } else {
                    "-".to_string()
                },
                max: if agg.count >= 2 {
                    ms_text(agg.max_us)
                } else {
                    "-".to_string()
                },
                len: agg.len.to_string(),
                flags: agg.flags,
                share: format!(
                    "{:.1}%",
                    if total > 0 {
                        agg.count as f64 / total as f64 * 100.0
                    } else {
                        0.0
                    }
                ),
            });
        }
        let win = &mut self.stats_windows[i];
        win.text_keys = keys;
        win.text_rows = rows;
        win.text_header = format!(
            "{} messages, {} frames since start",
            win.text_keys.len(),
            total
        );
    }

    /// Refreshes Messages window `i`'s throttled text snapshot: the header
    /// count and one pre-formatted struct per row, including the decoded
    /// signal pairs the expanded tree shows. Same gate as
    /// [`App::sync_data_text`].
    pub(crate) fn sync_msg_text(&mut self, i: usize) {
        let (scope, manual, dbc_only, filter) = {
            let w = &self.msg_windows[i];
            (
                w.scope,
                w.manual.clone(),
                w.dbc_only,
                w.filter.trim().to_lowercase(),
            )
        };
        let mut aggs: Vec<&MessageAgg> = self
            .snap
            .aggs
            .iter()
            .filter(|a| {
                if !App::scope_match(scope, &manual, a.channel, a.id) {
                    return false;
                }
                if dbc_only || !filter.is_empty() {
                    let name = self.message_name(a.channel, a.id).unwrap_or("-");
                    if dbc_only && name == "-" {
                        return false;
                    }
                    if !filter.is_empty() {
                        let id_str = format!("{:x}", a.id);
                        if !name.to_lowercase().contains(&filter) && !id_str.contains(&filter) {
                            return false;
                        }
                    }
                }
                true
            })
            .collect();
        aggs.sort_by_key(|a| (a.channel, a.id));
        let keys: Vec<(u8, u32)> = aggs.iter().map(|a| (a.channel, a.id)).collect();
        if self.msg_windows[i].text_keys == keys && !self.text_fresh {
            return;
        }
        let mut rows = Vec::with_capacity(aggs.len());
        for agg in &aggs {
            let agg = **agg;
            let id_str = if agg.extended {
                format!("{:08X}x", agg.id)
            } else {
                format!("{:03X}", agg.id)
            };
            let name = self.message_name(agg.channel, agg.id).unwrap_or("-");
            // The expanded tree reads the last frame's signals exactly as
            // before; only the refresh rate changed.
            let frame = CanFrame {
                t_us: agg.last_t_us,
                channel: agg.channel,
                id: agg.id,
                extended: agg.extended,
                len: agg.len,
                data: agg.data,
                dir: agg.dir,
                flags: agg.flags,
            };
            let signals = self
                .channel_dbc(agg.channel)
                .map(|db| db.decode_signals(&frame))
                .unwrap_or_default()
                .into_iter()
                .map(|d| {
                    (
                        d.name,
                        crate::dbc::fmt_signal_value(
                            d.phys,
                            &d.unit,
                            &d.type_tag,
                            d.label.as_deref(),
                        ),
                    )
                })
                .collect();
            rows.push(MsgRowText {
                label: format!("{id_str}  {name}"),
                bus: self.channel_name(agg.channel),
                dir: match agg.dir {
                    Direction::Rx => "Rx",
                    Direction::Tx => "Tx",
                },
                count: agg.count.to_string(),
                cycle: if agg.count > 1 {
                    format!("{:.1}", agg.cycle_us / 1000.0)
                } else {
                    "-".to_string()
                },
                flags: agg.flags,
                data: agg.payload().iter().map(|b| format!("{b:02X} ")).collect(),
                signals,
            });
        }
        let win = &mut self.msg_windows[i];
        win.text_keys = keys;
        win.text_rows = rows;
        win.text_header = format!("{} messages", win.text_keys.len());
    }

    /// Advances Trace window `i`'s reveal watermark on text frames: the rows
    /// already on screen never change, so the throttle decides how quickly
    /// *new* rows may appear. A run restart or a backward seek re-stamps
    /// frames below the watermark, and [`App::trace_revealed`] shows those
    /// immediately -- only the fresh tail is batched.
    pub(crate) fn sync_trace_text(&mut self, i: usize) {
        if !self.text_fresh {
            return;
        }
        let newest = self.snap.trace.last().map(|f| f.t_us).unwrap_or(u64::MAX);
        let shown = self.snap.trace.len();
        let w = &mut self.trace_windows[i];
        w.shown_t_us = newest;
        w.shown_count = shown;
    }

    /// Trace window `w`'s revealed frames, newest first: the whole buffer
    /// minus the not-yet-revealed tail beyond the watermark.
    pub(crate) fn trace_revealed<'a>(
        &'a self,
        w: &'a TraceWin,
    ) -> impl Iterator<Item = &'a CanFrame> {
        self.snap
            .trace
            .iter()
            .rev()
            .skip_while(|f| f.t_us > w.shown_t_us)
    }

    /// Refreshes the status bar's throttled counters line. The state, the REC
    /// marker and the replay position beside it stay live on purpose: they
    /// change rarely or must track the scrub bar.
    pub(crate) fn sync_status_text(&mut self) {
        if !self.text_fresh {
            return;
        }
        // Counters come from the snapshot, the f/s figure is the
        // frontend's own EMA.
        self.status_counters = format!(
            "| frames: {:>8}  | {:7.0} f/s  | trace: {:>6}  | signals: {:>4}",
            self.snap.frame_counter, self.frame_rate, self.snap.trace_len, self.snap.sub_count
        );
    }

    pub fn update(&mut self) {
        // The text-throttle gate runs before anything that can early-return:
        // every frame decides once whether throttled readouts re-render.
        self.text_fresh = self.text_rate_hz == 0
            || self.last_text_refresh.elapsed()
                >= std::time::Duration::from_millis(1000 / self.text_rate_hz.max(1) as u64);
        if self.text_fresh {
            self.last_text_refresh = std::time::Instant::now();
        }
        // One snapshot per frame, before anything can early-return: every
        // frontend read of the bus goes through it, so the rendering code
        // cannot tell a same-thread bus from the threaded one of stage 3.
        self.snap = self.core.snapshot();
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
        self.core.advance_clock(now);
        if self.last_tick_us > 0 && now > self.last_tick_us {
            let dt_s = (now - self.last_tick_us) as f64 / 1e6;
            let inst = (self.core.buf.len() as f64) / dt_s;
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
    /// The bus work itself is [`BusCore::step`]; what stays here is the
    /// frontend's own policy: the sampling stride it wants, the spec check
    /// it configured, and the status line it displays.
    pub fn tick(&mut self, now_us: u64) {
        let stride = self.wanted_stride_us();
        let mut status = String::new();
        self.core.step(
            now_us,
            stride,
            self.spec_tol_pct,
            self.spec_grace,
            &mut status,
        );
        if !status.is_empty() {
            self.status = status;
        }
        // Post-step re-read: the snapshot must never be staler than the
        // last step, whether it ran from the frame loop or a test.
        self.refresh_snapshot();
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
