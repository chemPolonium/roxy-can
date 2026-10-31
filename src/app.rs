use std::path::PathBuf;
use std::time::Instant;

#[cfg(test)]
use crate::can::frame::CanFrame;
// The tests module reaches `Direction` through this module's namespace.
#[allow(unused_imports)]
use crate::can::frame::Direction;
use crate::log::open_stream;
use crate::spec::{GRACE_CYCLES, TOLERANCE_PERCENT};

pub const TRACE_LIMIT: usize = 50_000;
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
/// Row cache cap of one Trace window (R1 virtual scrolling): the newest
/// matching frames the clipper draws from.
#[cfg(test)]
pub(crate) const MAX_CACHED_ROWS: usize = 200_000;
/// Synthetic id prefix for derived signals: `emit_value` streams are
/// keyed `(ch, EMITTED_ID_BASE | node_id, false, name)`. The prefix sits
/// far above every real identifier (max 0x1FFFFFFF), so a derived stream
/// can never collide with a frame's own signals.
pub const EMITTED_ID_BASE: u32 = 0x8000_0000;

/// The replay speed ladder. The last entry is "as fast as possible":
/// infinity makes the replay clock jump to the end immediately while the
/// UI keeps stepping between polls, so every observer updates in chunks.
#[cfg(test)]
pub const REPLAY_SPEEDS: [f64; 5] = [0.5, 1.0, 2.0, 4.0, f64::INFINITY];
/// Cycle a new generator entry gets when its DBC declares none. A declared
/// value always wins; this is only the invention we fall back to.
pub(crate) const DEFAULT_TX_CYCLE_US: u64 = 100_000;

pub use crate::aggregate::MessageAgg;
pub use crate::channel::NodeRole;
#[cfg(test)]
pub use crate::generator::cycle_from_ms_text;
#[cfg(test)]
pub use crate::observe::SampleCache;
pub use crate::observe::{DataWindow, GfxSignal, GraphicsWindow, StateWin, YMode};
#[cfg(test)]
pub use crate::workspace::PopupTarget;
pub use crate::workspace::{MsgWin, SigScope, StatsWin, TraceWin};
#[cfg(test)]
pub use crate::workspace::{RowsBuild, TraceFilter, TraceRow};

/// "{:.2}" milliseconds, the Min/Avg/Max cell format shared by the snapshot
/// builders below.
#[cfg(test)]
fn ms_text(v: f64) -> String {
    format!("{:.2}", v / 1000.0)
}

/// The record draft minus a trailing `.asc`/`.blf` (any case), so the
/// format combo can stamp its own extension on arm. A draft that names
/// some other extension keeps it — the stem is the user's text.
fn strip_record_ext(path: &str) -> &str {
    let lower = path.to_ascii_lowercase();
    if lower.ends_with(".asc") || lower.ends_with(".blf") {
        &path[..path.len() - 4]
    } else {
        path
    }
}

/// A Message Statistics row as throttled text, used by the text-sync tests.
#[cfg(test)]
#[derive(Clone)]
pub struct StatsRowText {
    pub label: String,
    pub bus: String,
    pub count: String,
    pub min: String,
    pub avg: String,
    pub max: String,
    pub len: String,
    pub share: String,
}

/// A Messages row as throttled text, used by the text-sync tests.
#[cfg(test)]
#[derive(Clone)]
pub struct MsgRowText {
    pub label: String,
    /// Stable row identity. Not derived from `label`: the same `(id, name)`
    /// legitimately appears on two buses when both load one DBC.
    pub id_key: String,
    pub bus: String,
    pub count: String,
    /// Why the expanded row has no signals to show.
    pub empty_note: Option<String>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Mode {
    #[default]
    Virtual,
    Replay,
}

/// One row of the Monitor window: a watched signal, its display name,
/// the decimals shown, and an optional coloring rule (`rising` compares
/// `value >= threshold`, otherwise `<=`; the palette `color` slots the
/// row's text when the rule holds). The lightweight big-screen panel.
#[derive(Clone, Debug, PartialEq)]
pub struct MonitorRow {
    pub key: crate::observe::SigKey,
    pub label: String,
    pub digits: usize,
    pub rule_on: bool,
    pub rising: bool,
    pub threshold: f64,
    pub color: usize,
}

/// Rows watched by the Monitor panel; the web Monitor tab serializes them.
#[derive(Default)]
pub struct MonitorView {
    pub rows: Vec<MonitorRow>,
}

impl Default for MonitorRow {
    fn default() -> Self {
        Self {
            key: crate::observe::SigKey::can_in(
                crate::model::ClusterId(0),
                0,
                false,
                String::new(),
            ),
            label: String::new(),
            digits: 1,
            rule_on: false,
            rising: true,
            threshold: 0.0,
            color: 0,
        }
    }
}

/// Which generator row a test helper is addressing.
#[cfg(test)]
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum GenRow {
    /// Index into `Snapshot::fr_tx`.
    Fr(usize),
}

pub struct App {
    /// Where the bus actually lives. `Threaded` -- the production drive:
    /// the core runs on its own thread and only commands and snapshots
    /// cross. `Manual` -- tests and the headless CLI keep the core here
    /// and crank its loop by hand, with `send` staying synchronous.
    pub(crate) drive: CoreDrive,
    /// The bus as of this frame's one mailbox read; UI rendering never
    /// touches core state directly, only this shared copy.
    pub snap: std::sync::Arc<crate::bus::Snapshot>,
    /// Command pipe to the core: `send` pushes here; the receiving end
    /// is the core thread's inbox (or the manual lane's).
    inbox_tx: std::sync::mpsc::Sender<crate::bus::BusCommand>,
    /// Where the core publishes snapshots and the frontend picks them up,
    /// never by waiting.
    mail: crate::bus::SnapshotMailbox,
    /// The stepping policy the frontend keeps retuning; the core thread
    /// reads it every lap. (Manual drives pass the same numbers straight
    /// into their lap.)
    knobs: std::sync::Arc<crate::core_loop::BusKnobs>,
    /// Previous `(frame counter, tick instant)` for the frame-rate EMA,
    /// which now reads snapshot deltas: frames arrive on the core's
    /// clock, whatever drive is underneath.
    ema_prev: Option<(u64, u64)>,
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
    /// Frontend workspace embedded in the project: tabs, cursors, column
    /// layout and splitters. Opaque to the core.
    pub web_workspace: Option<serde_json::Value>,
    /// Bumped whenever a project is loaded so the web shell knows to adopt
    /// the file's workspace exactly once.
    pub workspace_rev: u64,
    /// Offline reference run decoded from another log, shown behind the live
    /// chart; session state, not part of the project.
    pub reference: Option<crate::reference::ReferenceRun>,
    pub reference_rev: u64,
    /// Derived-channel definitions. The project file stores this list and the
    /// core samples the curves; the web modal reads it back from the snapshot.
    pub derived_defs: Vec<crate::derived::DerivedDef>,
    /// Chart lanes the web shell keeps subscribed even though they are not in
    /// the first Graphics window (session-only Signals picks). Refreshed
    /// from the session selection just before an observer drops a row, so a
    /// Data/Monitor/State clear never unmounts a live chart lane. Session
    /// state: the project's Graphics rows are what persist real picks.
    pub(crate) session_chart_keep: std::collections::HashSet<crate::observe::SigKey>,
    /// Message rows whose signal children the web Messages table has open.
    /// Session state: only open rows pay for signal decoding in the state
    /// snapshot, so an idle Messages view adds no per-poll work.
    pub expanded_messages: std::collections::HashSet<String>,
    /// Last Windows PnP signature for Vector USB devices and when it was
    /// polled. A transition to/from "no Vector USB at all" reopens the XL
    /// driver automatically (USB replug without pressing Refresh).
    pnp_signature: Option<Vec<String>>,
    last_pnp_poll: Instant,
    /// ProjectV2 JSON snapshot of the last clean state (loaded/saved/reset);
    /// compared against the live config to decide if anything changed.
    pub(crate) baseline: String,
    pub replay_speed: f64,
    /// The Monitor rows shared by the web observers.
    pub(crate) monitor_view: MonitorView,
    /// Which Write categories the text export carries.
    pub write_filter: [bool; 4],
    /// How far an observed period may stray from the declared one, in percent.
    pub spec_tol_pct: u64,
    /// How many declared periods of silence count as a dropped message.
    pub spec_grace: u64,
    /// The recording file format: 0 = ASC, 1 = BLF. The toolbar combo
    /// selects this; the recorder picks the backend by extension.
    pub record_format: usize,
    /// The port each CAN row's 硬件 picker last chose, by bus. A failed attach
    /// leaves the driver holding nothing, so without this the cell falls back
    /// to the list's first entry and the row ends up showing a channel while
    /// the status line blames another one for failing. Session state: which
    /// port answers is a bench fact, not a project one.
    pub hw_pick: std::collections::BTreeMap<u8, (crate::hw::HwDriver, i32)>,
    /// The same for a FlexRay 路's port picker -- see [`Self::hw_pick`].
    pub fr_pick: std::collections::BTreeMap<u8, i32>,
    /// The FlexRay descriptions behind the watches, one per bus, parsed when
    /// the user picked each description file. Slot numbers repeat across
    /// clusters, so a frame's identity is `(bus, slot)` and every lookup goes
    /// through the bus the row arrived on. The core is handed the same `Arc`s,
    /// so no file is read or parsed twice.
    pub fr_buses: std::collections::BTreeMap<u8, std::sync::Arc<crate::fr_db::FrDb>>,
    /// What the user calls each FlexRay 路. Apart from [`Self::fr_buses`] on
    /// purpose: a name is not a description, so a cluster seen only in a
    /// replayed log -- which is exactly the case where `FR0`/`FR1` say nothing
    /// -- can still be named. Unnamed 路 fall back to `FR{n}`.
    pub fr_names: std::collections::BTreeMap<u8, String>,
    /// Ethernet buses and the ARXML matrices loaded for them. Separate index
    /// space from CAN and FlexRay.
    pub eth_buses: std::collections::BTreeMap<u8, std::sync::Arc<crate::eth::db::EthDb>>,
    /// CAN ARXML descriptions bound to a bus, in binding order: each entry is
    /// (file path, cluster inside the matrix). A bus may carry several, which
    /// is how files from different suppliers describe one physical bus.
    pub can_arxml: std::collections::BTreeMap<u8, Vec<(String, String)>>,
    /// FlexRay descriptions bound to a bus, in binding order. An ARXML entry
    /// names its cluster; a FIBEX entry addresses the whole file.
    pub fr_sources: std::collections::BTreeMap<u8, Vec<crate::model::ClusterSource>>,
    /// Ethernet AUTOSAR matrices bound to a bus, in binding order.
    pub eth_sources: std::collections::BTreeMap<u8, Vec<crate::model::ClusterSource>>,
    /// The last uploaded ARXML's cluster list, waiting for the user to pick
    /// which one belongs to the bus they were binding.
    pub arxml_scan: Option<crate::channel::ArxmlScan>,
    /// Whether the matrix's `FRAME-LENGTH` includes the 14-byte L2 header.
    /// The capture payload never does; this is the project-level convention
    /// that makes the two lengths comparable.
    pub eth_length_includes_header: bool,
    /// The record file's stem as the input box shows it -- frontend
    /// draft state; the recorder receives a copy via `SetRecordPath`
    /// when recording arms.
    pub record_path_buf: String,
    /// The record filter draft: hex id list gating what lands in the
    /// recorded ASC. Session state, like the record path stem.
    pub record_filter_text: String,
    /// The trace ring's retention, mirrored for the UI and the project
    /// file; the core holds the authoritative clamped value.
    pub trace_limit: usize,
    /// Trigger-recording context sizes, mirrored for the UI and the
    /// project file like `trace_limit`.
    pub limits: crate::config::LimitsCfg,
    pub last_tick_us: u64,
    /// How often number readouts (Data values, Statistics, Messages, the
    /// status bar) re-render, in Hz; 0 follows the frame rate. Curves and
    /// bars always render at full frame rate -- a 60 fps stream of changing
    /// digits is unreadable, which is what this throttles.
    pub text_rate_hz: u32,
    /// True on frames where throttled text content should re-render.
    #[cfg(test)]
    pub text_fresh: bool,
    #[cfg(test)]
    last_text_refresh: std::time::Instant,
    /// The status bar's "| frames: .. | f/s | trace | signals" line as of the
    /// last throttled text refresh; the state and replay readouts beside it
    /// stay live. See [`App::sync_status_text`].
    #[cfg(test)]
    pub(crate) status_counters: String,
    pub frame_rate: f64,
    pub trace_windows: Vec<TraceWin>,
    pub msg_windows: Vec<MsgWin>,
    pub stats_windows: Vec<StatsWin>,
    pub graphics: Vec<GraphicsWindow>,
    /// Sampling stride the web plot asks for itself, ignoring the observer
    /// windows' zoom so fine curves need no config mutation.
    pub(crate) plot_stride_us: Option<u64>,
    pub data_windows: Vec<DataWindow>,
    pub state_trackers: Vec<StateWin>,
    pub(crate) trace_counter: usize,
    pub(crate) msg_counter: usize,
    pub(crate) stats_counter: usize,
    pub(crate) graphics_counter: usize,
    pub(crate) data_counter: usize,
    pub(crate) state_counter: usize,
}

/// Which drive owns the bus core.
pub(crate) enum CoreDrive {
    /// The core runs on its own thread (`App::new`); this App holds only
    /// the command sender and the mailbox reader.
    Threaded,
    /// The core is a plain value here (`App::headless`): tests and the
    /// headless CLI crank the same loop laps by hand, with synchronous
    /// `send`.
    Manual(Box<crate::core_loop::CoreLoop>),
}

impl std::ops::Deref for App {
    type Target = crate::bus::BusCore;

    /// The bus as a plain value. Manual drives hand out their local core,
    /// so test setup can poke it directly (with a `refresh_snapshot`
    /// afterwards); the threaded drive has nothing to hand out -- the
    /// core lives on its thread, and only commands and snapshots cross.
    fn deref(&self) -> &Self::Target {
        match &self.drive {
            CoreDrive::Manual(lane) => &lane.core,
            CoreDrive::Threaded => {
                panic!("the bus core runs on its own thread; use commands and snapshots")
            }
        }
    }
}

impl std::ops::DerefMut for App {
    fn deref_mut(&mut self) -> &mut Self::Target {
        match &mut self.drive {
            CoreDrive::Manual(lane) => &mut lane.core,
            CoreDrive::Threaded => {
                panic!("the bus core runs on its own thread; use commands and snapshots")
            }
        }
    }
}

impl App {
    /// The bus every drive starts from: default buses with their sample
    /// databases loaded and the generator pre-populated. Pure bootstrap,
    /// finished before any thread exists or any snapshot is published.
    fn build_core() -> crate::bus::BusCore {
        let mut core = crate::bus::BusCore::new(vec![
            crate::load::BusLoad::new(),
            crate::load::BusLoad::new(),
        ]);
        for (name, path) in [
            ("CAN1", "builtin:sample.dbc"),
            ("CAN2", "builtin:motbus.dbc"),
        ] {
            let cluster = core.cluster_ids.allocate();
            core.can_cluster_ids.push(cluster);
            core.model_comms
                .push(Some(std::sync::Arc::new(crate::model::empty_can_comm(
                    name,
                    cluster,
                    &[crate::model::ClusterSource::Dbc {
                        path: path.to_string(),
                    }],
                    crate::model::CanParams::DEFAULT_BITRATE_KBPS,
                    crate::model::CanParams::DEFAULT_FD_DATA_KBPS,
                ))));
            core.model_plans.push(None);
            core.model_dbs.push(None);
            core.can_xml_models.push(None);
        }
        core.bootstrap_dbcs();
        core.populate_generator();
        core
    }

    fn shell(
        drive: CoreDrive,
        inbox_tx: std::sync::mpsc::Sender<crate::bus::BusCommand>,
        mail: crate::bus::SnapshotMailbox,
        knobs: std::sync::Arc<crate::core_loop::BusKnobs>,
        every_window_kind: bool,
    ) -> Self {
        // Baseline only in the GUI: the headless suite builds hundreds of
        // App shells and must not call SetupAPI for each one.
        let pnp_signature = if matches!(drive, CoreDrive::Threaded) {
            crate::hw::vector::present_usb_signature()
        } else {
            None
        };
        let mut app = App {
            drive,
            inbox_tx,
            mail,
            knobs,
            snap: std::sync::Arc::new(crate::bus::Snapshot::default()),
            ema_prev: None,
            t0: Instant::now(),
            status: "stopped".to_string(),
            log_path: String::new(),
            log_info: None,
            recent_dbc: Vec::new(),
            recent_log: Vec::new(),
            project_path: None,
            recent_projects: Vec::new(),
            web_workspace: None,
            workspace_rev: 0,
            reference: None,
            reference_rev: 0,
            derived_defs: Vec::new(),
            session_chart_keep: std::collections::HashSet::new(),
            expanded_messages: std::collections::HashSet::new(),
            pnp_signature,
            last_pnp_poll: Instant::now(),
            baseline: String::new(),
            replay_speed: 1.0,
            monitor_view: Default::default(),
            write_filter: [true; 4],
            spec_tol_pct: TOLERANCE_PERCENT,
            spec_grace: GRACE_CYCLES,
            // The record draft: a file name, shown as-is in the toolbar
            // input. The directory is implicit (the open project's
            // Record/ folder, resolved when recording arms); the
            // recorder receives the full path via `SetRecordPath`.
            record_path_buf: String::new(),
            // Toolbar combo: 0 = ASC, 1 = BLF. Decides the extension
            // `toggle_record` stamps onto the draft stem.
            record_format: 0,
            hw_pick: std::collections::BTreeMap::new(),
            fr_pick: std::collections::BTreeMap::new(),
            fr_buses: Default::default(),
            fr_names: Default::default(),
            eth_buses: Default::default(),
            can_arxml: Default::default(),
            fr_sources: Default::default(),
            eth_sources: Default::default(),
            arxml_scan: None,
            eth_length_includes_header: false,
            record_filter_text: String::new(),
            trace_limit: TRACE_LIMIT,
            limits: Default::default(),
            last_tick_us: 0,
            text_rate_hz: 10,
            #[cfg(test)]
            text_fresh: true,
            #[cfg(test)]
            last_text_refresh: std::time::Instant::now(),
            #[cfg(test)]
            status_counters: String::new(),
            frame_rate: 0.0,
            trace_windows: Vec::new(),
            msg_windows: Vec::new(),
            stats_windows: Vec::new(),
            graphics: Vec::new(),
            plot_stride_us: None,
            data_windows: Vec::new(),
            state_trackers: Vec::new(),
            trace_counter: 0,
            msg_counter: 0,
            stats_counter: 0,
            graphics_counter: 0,
            data_counter: 0,
            state_counter: 0,
        };
        // The bootstrap published its first frame into the mailbox before
        // the thread (if any) exists, so the workspace baseline and the
        // first UI frame see the full bus, not an empty placeholder.
        app.read_snapshot();
        app.seed_startup_windows(every_window_kind);
        app.baseline = app.config_snapshot();
        app
    }

    /// The observer windows a session boots with: **Trace and Messages**, the two
    /// an operator looks at first. Statistics / Graphics / Data / State Tracker
    /// are one `+` in Measurement Setup away; every window the workspace holds
    /// is a window the project file has to carry.
    ///
    /// `every_kind` is what the manual drive (tests, the headless CLI) passes to
    /// keep one of each: those suites index each kind by position, and a window
    /// that never renders costs nothing.
    pub(crate) fn seed_startup_windows(&mut self, every_kind: bool) {
        self.new_trace_window();
        self.new_msg_window();
        if every_kind {
            self.new_stats_window();
            self.new_graphics_window();
            self.new_data_window();
            self.new_state_window();
        }
    }

    /// The production drive: the core boots here, then moves to its own
    /// thread. The UI keeps the command sender and the mailbox reader.
    pub fn new() -> Self {
        let core = Self::build_core();
        let (inbox_tx, inbox_rx) = std::sync::mpsc::channel();
        let mail = crate::bus::new_mailbox();
        let knobs = std::sync::Arc::new(crate::core_loop::BusKnobs::default());
        let mut lane = crate::core_loop::CoreLoop::new(core, inbox_rx, mail.clone());
        lane.publish();
        crate::core_loop::spawn_lane(lane, knobs.clone());
        Self::shell(CoreDrive::Threaded, inbox_tx, mail, knobs, false)
    }

    /// The manual drive: the same booted core stays a plain value on this
    /// thread, and `send`/`tick` crank its loop by hand. Deterministic
    /// clocks, synchronous commands -- what tests and a headless CLI want.
    pub fn headless() -> Self {
        Self::headless_with(Self::build_core())
    }

    /// A manual drive with no buses at all. The web shell starts here until
    /// the user opens a project or generates the file-based sample.
    pub fn blank() -> Self {
        Self::headless_with(crate::bus::BusCore::new(Vec::new()))
    }

    fn headless_with(core: crate::bus::BusCore) -> Self {
        let (inbox_tx, inbox_rx) = std::sync::mpsc::channel();
        let mail = crate::bus::new_mailbox();
        let knobs = std::sync::Arc::new(crate::core_loop::BusKnobs::default());
        let mut lane = crate::core_loop::CoreLoop::new(core, inbox_rx, mail.clone());
        lane.publish();
        Self::shell(
            CoreDrive::Manual(Box::new(lane)),
            inbox_tx,
            mail,
            knobs,
            true,
        )
    }

    pub fn now_us(&self) -> u64 {
        self.t0.elapsed().as_micros() as u64
    }

    /// Frontend side of the mailbox: pick up the newest snapshot without
    /// ever waiting. If the writer holds the mailbox this frame, last
    /// frame's copy renders -- one frame of staleness beats a blocked UI.
    /// Re-surfacing a snapshot's status is guarded by pointer identity, so
    /// a carried text flushes once, not every frame it stays on screen.
    fn read_snapshot(&mut self) {
        if let Some(fresh) = Self::peek_mailbox(&self.mail, &self.snap) {
            if let Some(msg) = &fresh.status {
                self.status = msg.clone();
            }
            self.snap = fresh;
        }
    }

    /// Writes a line into the Write ring, where it stays. The status bar holds
    /// one message and whatever writes next erases it, so anything the user may
    /// need to find again -- a failed export, a refused operation, half a
    /// project load -- goes through here as well as onto the bar.
    pub fn log_status(&mut self, kind: crate::bus::WriteKind, text: impl Into<String>) {
        self.send(crate::bus::BusCommand::LogStatus {
            kind,
            text: text.into(),
        });
    }

    /// Reports a failure: the line goes on the bar and into the Write ring.
    /// Errors that carry a reason -- an OS error, a parser message, the path it
    /// happened to -- are exactly what the user needs a minute later, and the
    /// bar holds one line until the next action erases it.
    pub fn fail(&mut self, text: impl Into<String>) {
        let text = text.into();
        self.log_status(crate::bus::WriteKind::Error, text.clone());
        self.status = text;
    }

    /// Records what a project load could not restore. Each problem goes to the
    /// Write ring in full and the bar gets the count: the load reports "project
    /// loaded" immediately afterwards, which would otherwise erase the only
    /// notice that anything was missing. Call it *after* that line is set.
    pub fn report_load_problems(&mut self, problems: Vec<String>) {
        if problems.is_empty() {
            return;
        }
        for p in &problems {
            self.log_status(crate::bus::WriteKind::Error, p.clone());
        }
        self.status = format!(
            "{} · {} item(s) failed to load (details in Write)",
            self.status,
            problems.len()
        );
    }

    /// The mailbox read as a pure lookup: the newest snapshot, or `None`
    /// when the writer holds the lock or nothing has been published since
    /// `current`. Split out so the never-blocks rule is testable under a
    /// held lock.
    fn peek_mailbox(
        mail: &crate::bus::SnapshotMailbox,
        current: &std::sync::Arc<crate::bus::Snapshot>,
    ) -> Option<std::sync::Arc<crate::bus::Snapshot>> {
        let latest = mail.try_lock().ok()?.clone();
        if std::sync::Arc::ptr_eq(&latest, current) {
            None
        } else {
            Some(latest)
        }
    }

    /// Publish + read. Production flows never need this -- `send` and
    /// `tick` publish as part of their laps, and the threaded drive can't
    /// publish from here at all -- but tests that poke core state
    /// directly call it to make the poke visible to snapshot reads.
    #[cfg(test)]
    pub(crate) fn refresh_snapshot(&mut self) {
        if let CoreDrive::Manual(lane) = &mut self.drive {
            lane.publish();
        }
        self.read_snapshot();
    }

    /// Waits for the core thread to catch up with what was sent so far
    /// (threaded drives only): blocks until the snapshot's lap counter
    /// goes quiet, then returns with the freshest state. Manual drives
    /// are always settled. Used by flows that read the bus back
    /// mid-sequence -- project restore -- which is rare and user-paced,
    /// so a bounded block is the honest shape.
    pub(crate) fn settle(&mut self) {
        if matches!(self.drive, CoreDrive::Threaded) {
            let deadline = Instant::now() + std::time::Duration::from_millis(2_000);
            let mut last = self.snap.laps;
            let mut quiet = 0;
            while Instant::now() < deadline {
                std::thread::sleep(std::time::Duration::from_millis(2));
                self.read_snapshot();
                if self.snap.laps == last {
                    quiet += 1;
                    if quiet >= 3 {
                        break;
                    }
                } else {
                    quiet = 0;
                }
                last = self.snap.laps;
            }
        }
    }

    /// The frontend's post office to the bus. On the manual drive the
    /// push is followed by the core's wake-up work, so callers act on the
    /// post-command bus immediately; on the threaded drive the push is
    /// the whole story, and callers see the effect one published snapshot
    /// later. Every UI write action funnels through here -- that funnel
    /// is what made the core movable.
    pub fn send(&mut self, cmd: crate::bus::BusCommand) {
        let _ = self.inbox_tx.send(cmd);
        if let CoreDrive::Manual(lane) = &mut self.drive {
            lane.drain();
            lane.publish();
        }
        self.read_snapshot();
    }

    /// Replaces the derived-channel list. Rows in the observers that point at
    /// a definition the user just deleted go with it -- a lane with no
    /// expression behind it would sit there empty forever.
    pub fn set_derived_defs(&mut self, defs: Vec<crate::derived::DerivedDef>) {
        let ids: std::collections::HashSet<String> =
            defs.iter().map(|def| def.id.clone()).collect();
        // The key carries the display name, so a rename is a key change. Move
        // the observer rows and their per-key State maps to the new key
        // instead of leaving them pointing at a subscription that is gone.
        let renames: std::collections::HashMap<crate::observe::SigKey, crate::observe::SigKey> =
            defs.iter()
                .filter_map(|def| {
                    let old = self
                        .derived_defs
                        .iter()
                        .find(|old| old.id == def.id && old.name != def.name)?;
                    Some((
                        crate::observe::SigKey::derived(&def.id, &old.name),
                        crate::observe::SigKey::derived(&def.id, &def.name),
                    ))
                })
                .collect();
        if !renames.is_empty() {
            for row in &mut self.monitor_view.rows {
                if let Some(new) = renames.get(&row.key) {
                    if row.label == row.key.name() {
                        row.label = new.name().to_string();
                    }
                    row.key = new.clone();
                }
            }
            for window in &mut self.graphics {
                for signal in &mut window.signals {
                    if let Some(new) = renames.get(&signal.key) {
                        signal.key = new.clone();
                    }
                }
            }
            for window in &mut self.data_windows {
                for signal in &mut window.signals {
                    if let Some(new) = renames.get(&signal.key) {
                        signal.key = new.clone();
                    }
                }
            }
            fn remap_map<V>(
                map: &mut std::collections::HashMap<crate::observe::SigKey, V>,
                renames: &std::collections::HashMap<crate::observe::SigKey, crate::observe::SigKey>,
            ) {
                for (old, new) in renames {
                    if let Some(value) = map.remove(old) {
                        map.insert(new.clone(), value);
                    }
                }
            }
            for window in &mut self.state_trackers {
                for signal in &mut window.signals {
                    if let Some(new) = renames.get(&signal.key) {
                        signal.key = new.clone();
                    }
                }
                remap_map(&mut window.rules, &renames);
                remap_map(&mut window.overrides, &renames);
                remap_map(&mut window.color_slots, &renames);
            }
        }
        let keep = |key: &crate::observe::SigKey| !matches!(key, crate::observe::SigKey::Derived { id, .. } if !ids.contains(id));
        self.monitor_view.rows.retain(|row| keep(&row.key));
        for window in &mut self.data_windows {
            window.signals.retain(|signal| keep(&signal.key));
        }
        for window in &mut self.state_trackers {
            let dropped: Vec<crate::observe::SigKey> = window
                .signals
                .iter()
                .filter(|signal| !keep(&signal.key))
                .map(|signal| signal.key.clone())
                .collect();
            window.signals.retain(|signal| keep(&signal.key));
            for key in dropped {
                window.rules.remove(&key);
                window.overrides.remove(&key);
                window.color_slots.remove(&key);
            }
        }
        self.derived_defs = defs.clone();
        self.send(crate::bus::BusCommand::SetDerived(defs));
    }

    /// Replaces the session chart keep set. The web shell calls this from the
    /// selection store immediately before an observer removes a row.
    pub(crate) fn set_session_chart_keep(
        &mut self,
        keys: impl IntoIterator<Item = crate::observe::SigKey>,
    ) {
        self.session_chart_keep = keys.into_iter().collect();
    }

    /// Opens or closes one Messages row's signal children. The web layer
    /// owns the expansion UI; the core only decides what to include.
    pub fn set_message_expanded(&mut self, key: &str, on: bool) {
        if on {
            self.expanded_messages.insert(key.to_string());
        } else {
            self.expanded_messages.remove(key);
        }
        self.send(crate::bus::BusCommand::SetExpandedMessages(
            self.expanded_messages.iter().cloned().collect(),
        ));
    }

    pub fn clear_message_expanded(&mut self) {
        if !self.expanded_messages.is_empty() {
            self.expanded_messages.clear();
            self.send(crate::bus::BusCommand::SetExpandedMessages(Vec::new()));
        }
    }

    /// The State bands a derived channel carries, if its definition names any.
    pub fn derived_rule(&self, id: &str) -> Option<crate::observe::StateRule> {
        let def = self.derived_defs.iter().find(|def| def.id == id)?;
        if !def.has_rule() {
            return None;
        }
        Some(crate::observe::StateRule {
            cuts: def.cuts.clone(),
            names: def.names.clone(),
            colors: vec![None; def.names.len().max(def.cuts.len() + 1)],
        })
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
        match self.snap.run_mode {
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
    #[cfg(test)]
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
        !self.log_path.trim().is_empty() || !self.snap.last_record.trim().is_empty()
    }

    pub fn stop(&mut self) {
        self.send(crate::bus::BusCommand::Stop);
    }

    pub fn toggle_record(&mut self) {
        // The input holds a file name (or an explicit path the user
        // typed); the directory is implicit -- the open project's
        // Record/ folder. The combo is the single source of truth for
        // ASC vs BLF: its extension is stamped onto the stem, and the
        // recorder follows whatever extension it receives.
        let stem = strip_record_ext(self.record_path_buf.trim());
        let ext = if self.record_format == 1 {
            "blf"
        } else {
            "asc"
        };
        let path = self.record_full_path(&format!("{stem}.{ext}"));
        self.send(crate::bus::BusCommand::SetRecordPath(path));
        self.send(crate::bus::BusCommand::ToggleRecord);
    }

    /// Resolves a record draft to the path the recorder derives its
    /// file from. Relative names land in the open project's Record/
    /// folder; absolute paths are honored exactly as typed; with no
    /// project the name goes to the recorder unchanged (its own
    /// "record" default sits beside the executable, as before).
    fn record_full_path(&self, name: &str) -> String {
        if std::path::Path::new(name).is_absolute() {
            return name.to_string();
        }
        match self.project_path.as_ref().and_then(|p| p.parent()) {
            Some(dir) => dir
                .join("Record")
                .join(if name.is_empty() { "record" } else { name })
                .to_string_lossy()
                .into_owned(),
            None => name.to_string(),
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
    /// Refused while a replay is running: `log_path` and the status bar would
    /// describe the new file while the live source keeps streaming the old one.
    pub fn load_log(&mut self, path: &str) {
        if self.snap.measuring && matches!(self.snap.mode, Mode::Replay) {
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
                self.status = if info.is_empty() {
                    format!("loaded {name}")
                } else {
                    format!("loaded {name} [{info}]")
                };
                self.push_recent_log(path.to_string());
            }
            Err(e) => self.fail(format!("log load failed [{path}]: {e}")),
        }
    }

    /// Starts playback of the selected log (the path the user loaded, or
    /// the last recording as a fallback). The open, the silence-set scan
    /// and the source swap live in the command; what stays here is the
    /// frontend's own memory: which path it selected and the wall-clock
    /// anchors.
    pub fn replay(&mut self) {
        let path = {
            let p = self.log_path.trim();
            if p.is_empty() {
                self.snap.last_record.clone()
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
    #[cfg(test)]
    pub fn step_replay_speed(&mut self, delta: i32) {
        let idx = REPLAY_SPEEDS
            .iter()
            .position(|s| *s == self.replay_speed || (*s - self.replay_speed).abs() < 1e-9)
            .unwrap_or(1);
        let next = (idx as i32 + delta).clamp(0, REPLAY_SPEEDS.len() as i32 - 1) as usize;
        self.set_replay_speed(REPLAY_SPEEDS[next]);
    }

    /// Play/pause as a single action, shared by the toolbar button, Space and
    /// F9. A pause freezes the replay clock in place; resuming is the same
    /// action, and starting when nothing runs re-opens the selected log.
    #[cfg(test)]
    pub fn toggle_play(&mut self) {
        if self.snap.measuring {
            self.send(crate::bus::BusCommand::SetTracePaused(
                !self.snap.trace_paused,
            ));
        } else {
            self.start_selected();
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
    /// slides the curve out of view as soon as the speed is not exactly 1x.
    ///
    /// While simulating it is `sim_t_us`, which stops during a pause; the wall
    /// clock does not, so anchoring on it would drift the curve away.
    pub fn plot_now_s(&self) -> f64 {
        if matches!(self.snap.mode, Mode::Replay)
            && let Some((pos, _)) = self.snap.replay
        {
            return pos as f64 / 1e6;
        }
        self.snap.sim_t_us as f64 / 1e6
    }

    /// Resets the pan offsets of all plot windows back to the live edge.
    #[cfg(test)]
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
    #[cfg(test)]
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
        // Ethernet flows follow the same scope rules as the other two
        // streams: an Ethernet bus scope keeps its own, a CAN or cluster
        // scope drops them, and Manual admits exactly the hand-picked flows.
        let eth_aggs: Vec<&crate::aggregate::EthAgg> = self
            .snap
            .eth_aggs
            .iter()
            .filter(|a| App::scope_match_eth(scope, &manual, &a.key))
            .collect();
        if self.stats_windows[i].text_keys == keys && !self.text_fresh && eth_aggs.is_empty() {
            return;
        }
        // FlexRay slots share the table, so they count in its denominator too.
        // One row per frame the run carried: a slot scheduled by cycle
        // repetition belongs to several frames, and their traffic, names and
        // periods are separate things. A CAN channel scope keeps this side out
        // (a FlexRay cluster is not a CAN channel), a cluster scope shows that
        // cluster's frames so the percentage is its own traffic, and a Manual
        // scope admits exactly the slots hand-picked for this window -- the same
        // rule the Trace and Messages tables apply.
        let fr_aggs: Vec<&crate::aggregate::FrFrameAgg> = self
            .snap
            .fr_aggs
            .iter()
            .filter(|a| App::scope_match_fr(scope, &manual, a.bus, a.slot))
            .collect();
        let total: u64 = aggs
            .iter()
            .map(|a| a.count)
            .chain(fr_aggs.iter().map(|a| a.count))
            .chain(eth_aggs.iter().map(|a| a.count))
            .sum();
        let share = |count: u64| {
            format!(
                "{:.1}%",
                if total > 0 {
                    count as f64 / total as f64 * 100.0
                } else {
                    0.0
                }
            )
        };
        let mut rows = Vec::with_capacity(aggs.len() + fr_aggs.len() + eth_aggs.len());
        for agg in &aggs {
            let agg = **agg;
            let id_str = if agg.extended {
                format!("{:08X}x", agg.id)
            } else {
                format!("{:03X}", agg.id)
            };
            let name = self
                .message_name(agg.channel, agg.id, agg.extended)
                .unwrap_or_else(|| "-".to_string());
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
                share: share(agg.count),
            });
        }
        for agg in fr_aggs {
            let name = Self::fr_frame_name(agg);
            rows.push(StatsRowText {
                label: if name.is_empty() {
                    format!("slot {}", agg.slot)
                } else {
                    format!("slot {}  {name}", agg.slot)
                },
                bus: self.fr_bus_name(agg.bus),
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
                len: agg.payload.len().to_string(),
                share: share(agg.count),
            });
        }
        for agg in eth_aggs {
            let flow = crate::eth::db::EthFlow {
                vlan: agg.key.vlan,
                dst: agg.key.dst,
                src: agg.key.src,
                ethertype: agg.key.ethertype,
                payload_len: agg.key.len,
            };
            let name = self
                .eth_db(agg.key.bus)
                .and_then(
                    |db| match db.match_flow(&flow, self.eth_length_includes_header) {
                        crate::eth::db::EthMatch::Unique(i) => Some(db.frames[i].name.clone()),
                        crate::eth::db::EthMatch::Ambiguous(n) => Some(format!("<{n} candidates>")),
                        crate::eth::db::EthMatch::None => None,
                    },
                )
                .unwrap_or_else(|| {
                    format!(
                        "{} > {}",
                        crate::eth::db::mac_text(&agg.key.src),
                        crate::eth::db::mac_text(&agg.key.dst)
                    )
                });
            rows.push(StatsRowText {
                label: format!("ETH{}  {name}", agg.key.bus),
                bus: format!("ETH{}", agg.key.bus),
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
                len: agg.key.len.to_string(),
                share: share(agg.count),
            });
        }
        let win = &mut self.stats_windows[i];
        win.text_keys = keys;
        win.text_rows = rows;
        win.text_header = format!(
            "{} messages, {} frames since start",
            win.text_rows.len(),
            total
        );
    }

    /// The frame an FlexRay aggregate row counts, in a table or an export. The
    /// row was keyed by that frame when the first arrival resolved it -- either
    /// through this bus's cluster description, or through the name the log
    /// carried -- so the name is stored, not re-resolved: re-resolving would
    /// let a slot's row change frames as the cycles ran by.
    pub(crate) fn fr_frame_name(agg: &crate::aggregate::FrFrameAgg) -> &str {
        agg.name.as_deref().unwrap_or("")
    }

    /// The same precedence for a Trace row, which carries its own name column.
    pub fn fr_row_name<'a>(&'a self, row: &'a crate::trace::FrRow) -> Option<&'a str> {
        self.fr_db(row.bus)
            .and_then(|db| db.frame_at(row.slot, row.cycle, row.ab))
            .map(|f| f.name.as_str())
            .or(row.name.as_deref())
    }

    /// The frame a FlexRay tally row was keyed by, when that row's own cluster
    /// description still holds it. `None` means "not described", and it covers
    /// both ways a row can be undescribed: no description loaded on this bus, or
    /// one that does not schedule this slot's cycle phase. The Messages window
    /// and its CSV export gate "仅 DBC" on this, so the file matches the table.
    pub(crate) fn fr_described_frame(&self, agg: &crate::aggregate::FrFrameAgg) -> Option<usize> {
        self.fr_db(agg.bus).and_then(|db| {
            agg.occupant
                .frame_ix()
                .filter(|&i| db.frame_index(i).is_some())
        })
    }

    /// Does this row's own cluster description account for the arrival -- that
    /// is, does it schedule a frame for this slot *in this cycle phase*? The
    /// "仅 DBC" gate asks this rather than "is it a FlexRay row": a described
    /// cluster's frame is described by a database exactly as a DBC message is,
    /// and the two failure kinds stay distinct -- no description on the bus
    /// gives no frame for any row, a description that does not schedule this
    /// phase gives none for this one.
    pub fn fr_row_described(&self, row: &crate::trace::FrRow) -> bool {
        self.fr_db(row.bus)
            .is_some_and(|db| db.frame_ix_at(row.slot, row.cycle, row.ab).is_some())
    }

    /// The cluster description loaded for one FlexRay bus. `None` means the
    /// bus is watched without a description: rows still reach the Trace
    /// window, they just stay undecoded.
    pub fn fr_db(&self, bus: u8) -> Option<&crate::fr_db::FrDb> {
        self.fr_buses.get(&bus).map(|db| &**db)
    }

    /// Every latched spec violation, CAN and FlexRay alike: what the report
    /// window lists and what its CSV writes. The two buses keep separate
    /// storage because their identities do not share a key (see
    /// [`crate::spec::Spec`]); a report reads both.
    pub(crate) fn spec_rows(&self) -> Vec<crate::spec::SpecRow> {
        let mut out = Vec::new();
        for ((ch, id, ext, kind), l) in &self.snap.spec.rows {
            let name = self
                .model_can_name(*ch, *id, *ext)
                .unwrap_or_else(|| "not in database".to_string());
            out.push(crate::spec::SpecRow {
                bus: self.channel_name(*ch).to_string(),
                addr: if *ext {
                    format!("{id:X} ext")
                } else {
                    format!("{id:X}")
                },
                name: name.to_string(),
                kind: *kind,
                declared: l.declared,
                measured: l.measured,
                count: l.count,
                first_t_us: l.first_t_us,
                last_t_us: l.last_t_us,
            });
        }
        for (((bus, slot, frame), kind), l) in &self.snap.spec.fr_rows {
            // The frame the schedule resolved -- or the absence of one, which is
            // itself the finding. The name comes from the same description the
            // verdict was judged against.
            let name = match frame {
                Some(ix) => self
                    .fr_db(*bus)
                    .and_then(|db| db.frame_index(*ix))
                    .map(|f| {
                        if f.name.is_empty() {
                            format!("frame {ix} in slot {slot}")
                        } else {
                            f.name.clone()
                        }
                    })
                    .unwrap_or_else(|| "frame no longer in the current description".to_string()),
                None => "not in the schedule".to_string(),
            };
            out.push(crate::spec::SpecRow {
                bus: self.fr_bus_name(*bus),
                addr: format!("slot {slot}"),
                name,
                kind: *kind,
                declared: l.declared,
                measured: l.measured,
                count: l.count,
                first_t_us: l.first_t_us,
                last_t_us: l.last_t_us,
            });
        }
        out
    }

    /// What one static slot of this cluster occupies of the medium, in
    /// microseconds -- the unit the occupancy figure is built from. `None` when
    /// the bus has no description or its carries no slot timing, which the
    /// Statistics row shows as "-" rather than as a 0 % nobody measured.
    #[cfg(test)]
    pub(crate) fn fr_slot_wire_us(&self, bus: u8) -> Option<f64> {
        self.fr_db(bus)
            .and_then(|db| crate::load::fr_slot_wire_us(&db.params))
    }

    /// Refreshes Messages window `i`'s throttled text snapshot: the header
    /// count and one pre-formatted struct per row, including the decoded
    /// signal pairs the expanded tree shows. Same gate as
    /// [`App::sync_data_text`].
    #[cfg(test)]
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
                    let name = self
                        .message_name(a.channel, a.id, a.extended)
                        .unwrap_or_else(|| "-".to_string());
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
        if self.msg_windows[i].text_keys == keys
            && !self.text_fresh
            && self.snap.eth_aggs.is_empty()
        {
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
            let declared = self.message_name(agg.channel, agg.id, agg.extended);
            let name = declared.as_deref().unwrap_or("-");
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
            let signals: Vec<(String, String)> = self
                .model_can_all_values(&frame)
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
            // A message the DBC does not declare and one that declares no
            // signals are different facts.
            let empty_note = if !signals.is_empty() {
                None
            } else if declared.is_none() {
                Some("(not in DBC)".to_string())
            } else {
                Some("(message declares no signals)".to_string())
            };
            rows.push(MsgRowText {
                label: format!("{id_str}  {name}"),
                id_key: format!(
                    "c{}:{:08X}{}",
                    agg.channel,
                    agg.id,
                    if agg.extended { "x" } else { "" }
                ),
                bus: self.channel_name(agg.channel),
                count: agg.count.to_string(),
                empty_note,
            });
        }
        // FlexRay rows ride the same table, after the CAN rows: one row per
        // frame the run carried, not per slot. The same scope/text rules apply
        // as in the Trace window -- a CAN channel scope drops them, a cluster
        // scope keeps just that cluster, and the filter text also matches a slot
        // number. "仅 DBC" is decided per row below, not for the section: a frame
        // the cluster description schedules is described by a database exactly
        // as a DBC message is, which is the question the checkbox asks; what it
        // must not paper over is the other two cases -- no description on this
        // bus, or one that does not schedule this phase's frame.
        {
            for agg in &self.snap.fr_aggs {
                if !App::scope_match_fr(scope, &manual, agg.bus, agg.slot) {
                    continue;
                }
                // The description of the bus this frame arrived on decodes its
                // signals -- through the very index this row was keyed by, so
                // the name shown and the layout decoded are the same frame's.
                let db = self.fr_db(agg.bus);
                let frame_ix = self.fr_described_frame(agg);
                if dbc_only && frame_ix.is_none() {
                    continue;
                }
                let name = Self::fr_frame_name(agg);
                if !filter.is_empty()
                    && !format!("slot {}", agg.slot).contains(&filter)
                    && !agg.slot.to_string().contains(&filter)
                    && !name.to_lowercase().contains(&filter)
                {
                    continue;
                }
                // No "FR" prefix here: the Bus column already says which
                // cluster the slot belongs to, and with two clusters watched
                // at once a bare "FR" would leave two identical-looking rows
                // for the same slot number.
                let label = [
                    format!("slot {}", agg.slot),
                    match agg.ab {
                        0 => "A".to_string(),
                        1 => "B".to_string(),
                        _ => String::new(),
                    },
                    name.to_string(),
                ]
                .into_iter()
                .filter(|s| !s.is_empty())
                .collect::<Vec<_>>()
                .join("  ");
                let signals = frame_ix
                    .and_then(|frame_ix| {
                        let row = crate::trace::FrRow {
                            bus: agg.bus,
                            t_us: agg.last_t_us,
                            ab: agg.ab,
                            slot: agg.slot,
                            cycle: 0,
                            payload: agg.payload.clone(),
                            header_crc: 0,
                            flags: 0,
                            name: agg.name.clone(),
                        };
                        self.model_fr_all_values(&row, frame_ix)
                    })
                    .unwrap_or_default();
                // Which of the three reasons it is: no description for this
                // cluster, a description that does not schedule this slot's
                // frame, or a frame that declares no signals.
                let empty_note = if !signals.is_empty() {
                    None
                } else if db.is_none() {
                    Some(format!(
                        "({} has no cluster description)",
                        self.fr_bus_name(agg.bus)
                    ))
                } else if frame_ix.is_none() {
                    Some(format!(
                        "({} description schedules no frame in slot {})",
                        self.fr_bus_name(agg.bus),
                        agg.slot
                    ))
                } else {
                    Some("(frame declares no signals)".to_string())
                };
                rows.push(MsgRowText {
                    label,
                    id_key: format!(
                        "f{}:{}:{}",
                        agg.bus,
                        agg.slot,
                        match &agg.occupant {
                            crate::aggregate::FrOccupant::Frame(i) => format!("f{i}"),
                            crate::aggregate::FrOccupant::Logged(n) => format!("l{n}"),
                            crate::aggregate::FrOccupant::Unknown => "u".to_string(),
                        }
                    ),
                    bus: self.fr_bus_name(agg.bus),
                    count: agg.count.to_string(),
                    empty_note,
                });
            }
        }
        // Ethernet flows follow the window's scope and text filter like the
        // other two streams. The ARXML match names the row when it is unique.
        for agg in &self.snap.eth_aggs {
            if !App::scope_match_eth(scope, &manual, &agg.key) {
                continue;
            }
            let flow = crate::eth::db::EthFlow {
                vlan: agg.key.vlan,
                dst: agg.key.dst,
                src: agg.key.src,
                ethertype: agg.key.ethertype,
                payload_len: agg.key.len,
            };
            let db = self.eth_db(agg.key.bus);
            let matched =
                db.and_then(
                    |db| match db.match_flow(&flow, self.eth_length_includes_header) {
                        crate::eth::db::EthMatch::Unique(i) => Some(db.frames[i].name.clone()),
                        crate::eth::db::EthMatch::Ambiguous(n) => Some(format!("<{n} candidates>")),
                        crate::eth::db::EthMatch::None => None,
                    },
                );
            let macs = format!(
                "{} > {}",
                crate::eth::db::mac_text(&agg.key.src),
                crate::eth::db::mac_text(&agg.key.dst)
            );
            let name = matched.clone().unwrap_or_else(|| macs.clone());
            if dbc_only && matched.is_none() {
                continue;
            }
            if !filter.is_empty() {
                let hay = format!(
                    "eth{} {} {} vlan {:?} 0x{:04X}",
                    agg.key.bus, macs, name, agg.key.vlan, agg.key.ethertype
                )
                .to_lowercase();
                if !hay.contains(&filter) {
                    continue;
                }
            }
            let signals: Vec<(String, String)> = matched
                .as_deref()
                .and_then(|name| self.model_eth_all_values(agg.key.bus, name, &agg.payload))
                .map(|decoded| {
                    decoded
                        .into_iter()
                        .map(|s| {
                            let text = match &s.label {
                                Some(label) => format!("{} {} ({})", s.phys, s.unit, label),
                                None => format!("{} {}", s.phys, s.unit),
                            };
                            (s.name, text.trim().to_string())
                        })
                        .collect()
                })
                .unwrap_or_default();
            let empty_note = if !signals.is_empty() {
                None
            } else if db.is_none() {
                Some(format!("(ETH{} has no ARXML loaded)", agg.key.bus))
            } else if matched.is_none() {
                Some("(multiple or no ARXML frames match this flow)".to_string())
            } else {
                Some("(frame declares no signals)".to_string())
            };
            rows.push(MsgRowText {
                label: format!("ETH{}  {name}", agg.key.bus),
                id_key: format!(
                    "e{}:{:?}:{:?}:{:?}:{:04X}:{}",
                    agg.key.bus,
                    agg.key.vlan,
                    agg.key.src,
                    agg.key.dst,
                    agg.key.ethertype,
                    agg.key.len
                ),
                bus: format!("ETH{}", agg.key.bus),
                // The flow tally merges both capture directions, so it cannot
                // name one; the Trace row carries the per-frame direction.
                count: agg.count.to_string(),
                empty_note,
            });
        }
        let win = &mut self.msg_windows[i];
        win.text_keys = keys;
        win.text_rows = rows;
        // The rows, not the CAN keys: FlexRay slots are in this table too, and
        // a count that ignored them did not match what is on screen.
        win.text_header = format!("{} messages", win.text_rows.len());
    }

    /// Refreshes Trace window `i`'s filtered row cache on the text gate.
    ///
    /// A steady run only *prepends*: when the filter, the expansion setting, the
    /// cluster/matrix descriptions and all three rings are unchanged since the
    /// last build and no column sort has reordered the cache, the walk covers
    /// just the rows that arrived after the build point. Walking the whole ring
    /// every gate tick cost 7 ms on a 50 000-row FlexRay replay and 70 ms with the
    /// FR signal expansion on (measured, `--release`, see
    /// [`crate::headless_tests::perf_flexray_readouts_under_load`]) against a 10
    /// Hz text gate.
    ///
    /// Anything else -- an edited filter, a sorted column, a description swapped
    /// onto a FlexRay bus, a cleared or restarted run -- drops the cache and
    /// walks the whole revealed ring once, exactly as before. Expanding FlexRay
    /// rows into their decoded signals costs that walk the count of child rows
    /// per frame row, nothing else: the children are not entries of this list
    /// (see [`TraceRow::Fr`]).
    #[cfg(test)]
    pub(crate) fn sync_trace_rows(&mut self, i: usize) {
        if !self.text_fresh {
            return;
        }
        let newest = self
            .snap
            .trace
            .last()
            .map(|f| f.t_us)
            .into_iter()
            .chain(self.snap.fr_trace.last().map(|r| r.t_us))
            .chain(self.snap.eth_trace.last().map(|f| f.t_us))
            .max()
            .unwrap_or(u64::MAX);
        let (can_len, fr_len, eth_len) = (
            self.snap.trace.len(),
            self.snap.fr_trace.len(),
            self.snap.eth_trace.len(),
        );
        // The point a new walk starts above: the newest row of any ring at
        // the last build. A CAN-empty run never advances `newest`, which is
        // why the FlexRay and Ethernet rings count here too.
        let top = self
            .snap
            .trace
            .last()
            .map(|f| f.t_us)
            .unwrap_or(0)
            .max(self.snap.fr_trace.last().map(|r| r.t_us).unwrap_or(0))
            .max(self.snap.eth_trace.last().map(|f| f.t_us).unwrap_or(0));
        self.trace_windows[i].shown_t_us = newest;
        self.trace_windows[i].shown_count = can_len;
        let flt = self.trace_windows[i].filter_lens();
        let fr_expand = self.trace_windows[i].fr_expand;
        let eth_len_includes_header = self.eth_length_includes_header;
        // Which description each FlexRay bus resolved its rows against: an
        // expanded child row is an index into it, so a swap means a rebuild.
        let fr_dbs: Vec<(u8, usize)> = self
            .fr_buses
            .iter()
            .map(|(b, c)| (*b, std::sync::Arc::as_ptr(c) as usize))
            .collect();
        // And the same swap check for the Ethernet matrices behind names and
        // decoded signal conditions.
        let eth_dbs: Vec<(u8, usize)> = self
            .eth_buses
            .iter()
            .map(|(b, c)| (*b, std::sync::Arc::as_ptr(c) as usize))
            .collect();
        let since = match (
            &self.trace_windows[i].rows_build,
            self.trace_windows[i].rows_sorted,
        ) {
            (Some(b), false)
                if b.extends_to(
                    &flt,
                    fr_expand,
                    can_len,
                    fr_len,
                    eth_len,
                    top,
                    &fr_dbs,
                    &eth_dbs,
                    eth_len_includes_header,
                ) =>
            {
                Some(b.through())
            }
            _ => None,
        };
        let mut rows = std::mem::take(&mut self.trace_windows[i].rows);
        if since.is_none() {
            // Keep the allocation; only the contents are stale.
            rows.clear();
        }
        self.walk_rows(i, &flt, fr_expand, since, &mut rows);
        let w = &mut self.trace_windows[i];
        w.rows_build = Some(RowsBuild::new(
            &flt,
            fr_expand,
            top,
            can_len,
            fr_len,
            eth_len,
            &fr_dbs,
            &eth_dbs,
            eth_len_includes_header,
        ));
        // The walk produced this list; the cache is it, in the same order.
        w.rows_sorted = false;
        w.shown_count = rows.len();
        w.rows = rows;
    }

    /// Merges the three rings into the window's row list, newest first,
    /// CANoe's Trace shape: walk the newest-first lists taking whichever frame
    /// is the more recent, so the merge is linear in the kept rows. `since`
    /// bounds the walk to rows strictly newer than that timestamp (see
    /// [`App::sync_trace_rows`]); the result is prepended to `rows` in that
    /// case, and becomes the whole list otherwise.
    #[cfg(test)]
    fn walk_rows(
        &self,
        i: usize,
        flt: &TraceFilter,
        fr_expand: bool,
        since: Option<u64>,
        rows: &mut std::collections::VecDeque<TraceRow>,
    ) {
        // All lists newest first, cut at the window's reveal watermark and at
        // `since`, so an extending walk looks at the rows that arrived since
        // the last build and nothing else. `since` is that build's newest
        // timestamp; a full walk has no lower bound.
        let above = |t_us: u64| match since {
            Some(t) => t_us > t,
            None => true,
        };
        let can: Vec<_> = self
            .trace_revealed(&self.trace_windows[i])
            .take_while(|f| above(f.t_us))
            .filter(|f| self.trace_match_lens(flt, f))
            .collect();
        let fr: Vec<_> = self
            .snap
            .fr_trace
            .iter()
            .rev()
            .take_while(|r| above(r.t_us))
            .filter(|r| self.trace_fr_match(flt, r))
            .collect();
        let eth: Vec<_> = self
            .snap
            .eth_trace
            .iter_rev()
            .take_while(|f| above(f.t_us))
            .filter(|f| self.trace_eth_match(flt, f))
            .collect();
        let mut merged: Vec<TraceRow> = Vec::with_capacity(1_024);
        let (mut ci, mut fi, mut ei) = (0usize, 0usize, 0usize);
        while merged.len() < MAX_CACHED_ROWS {
            let tc = can.get(ci).map(|f| f.t_us);
            let tf = fr.get(fi).map(|r| r.t_us);
            let te = eth.get(ei).map(|f| f.t_us);
            let Some(top) = tc.into_iter().chain(tf).chain(te).max() else {
                break;
            };
            // Ties keep the historical CAN-first, then-FlexRay order, with
            // Ethernet last; all three are ordered streams either way.
            if tc == Some(top) {
                merged.push(TraceRow::Can(*can[ci]));
                ci += 1;
                continue;
            }
            if tf == Some(top) {
                let row = (*fr[fi]).clone();
                fi += 1;
                // A FlexRay frame row expands into its decoded signal child
                // rows, the CANoe trace shape, when the window asks and that
                // bus's description database is loaded. Only the count of them
                // travels in the list -- see [`TraceRow::Fr`].
                let kids = if fr_expand {
                    self.fr_child_count(&row)
                } else {
                    0
                };
                merged.push(TraceRow::Fr(row, kids));
                continue;
            }
            merged.push(TraceRow::Eth((*eth[ei]).clone()));
            ei += 1;
        }
        if since.is_some() {
            // Newest first on both sides: prepend the fresh batch by walking it
            // back to front, so the older cached rows stay where they are.
            for r in merged.into_iter().rev() {
                rows.push_front(r);
            }
            if rows.len() > MAX_CACHED_ROWS {
                rows.truncate(MAX_CACHED_ROWS);
            }
        } else {
            rows.extend(merged);
        }
    }

    /// How many decoded signal child rows one FlexRay frame row owns: the
    /// signals of the frame its slot and cycle resolve to that this payload
    /// actually carries. Zero when the bus has no description, or its schedule
    /// has no frame in this slot at this cycle.
    #[cfg(test)]
    fn fr_child_count(&self, row: &crate::trace::FrRow) -> u32 {
        let Some(db) = self.fr_db(row.bus) else {
            return 0;
        };
        let Some(frame_ix) = db.frame_ix_at(row.slot, row.cycle, row.ab) else {
            return 0;
        };
        db.child_count(frame_ix, row.payload.len()) as u32
    }

    /// The `(signal, value)` pair of child `ordinal` under one FlexRay frame
    /// row, decoded from that row's own payload as it is drawn.
    ///
    /// The same walk as [`Self::fr_child_count`], so an ordinal the list
    /// reserved a table row for is one this resolves -- until the description
    /// behind the count is swapped out, which is the case `RowsBuild::fr_dbs`
    /// rebuilds the cache over instead of letting the rows print blank.
    #[cfg(test)]
    pub(crate) fn fr_child_cell(
        &self,
        row: &crate::trace::FrRow,
        ordinal: u32,
    ) -> Option<(String, String)> {
        let db = self.fr_db(row.bus)?;
        let frame_ix = db.frame_ix_at(row.slot, row.cycle, row.ab)?;
        let (child_ix, raw, phys) = db
            .frame_values(frame_ix, &row.payload)
            .nth(ordinal as usize)?;
        db.child_text(frame_ix, child_ix, raw, phys)
    }

    /// Trace window `w`'s revealed frames, newest first: the whole buffer
    /// minus the not-yet-revealed tail beyond the watermark.
    #[cfg(test)]
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
    /// change rarely or must track the replay position.
    #[cfg(test)]
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

    /// Polls Windows PnP for Vector USB devices once a second. A transition
    /// through "no Vector USB device present" (unplug, then replug) reopens
    /// the XL driver automatically, which is what restores CAN FD on the
    /// returning adapter. Other transitions only invalidate the Buses
    /// channel cache, so adding a second adapter never cuts an active port.
    fn watch_hardware_hotplug(&mut self) {
        if !matches!(self.drive, CoreDrive::Threaded) {
            return;
        }
        if self.last_pnp_poll.elapsed() < std::time::Duration::from_secs(1) {
            return;
        }
        self.last_pnp_poll = Instant::now();
        let Some(sig) = crate::hw::vector::present_usb_signature() else {
            return;
        };
        match &self.pnp_signature {
            None => self.pnp_signature = Some(sig),
            Some(prev) if *prev != sig => {
                let crosses_empty = prev.is_empty() || sig.is_empty();
                self.pnp_signature = Some(sig);
                if crosses_empty {
                    self.send(crate::bus::BusCommand::RefreshHardware);
                }
            }
            _ => {}
        }
    }

    pub fn update(&mut self) {
        self.watch_hardware_hotplug();
        // The text-throttle gate only feeds the headless tests; the web DTOs
        // read live values.
        #[cfg(test)]
        {
            self.text_fresh = self.text_rate_hz == 0
                || self.last_text_refresh.elapsed()
                    >= std::time::Duration::from_millis(1000 / self.text_rate_hz.max(1) as u64);
            if self.text_fresh {
                self.last_text_refresh = std::time::Instant::now();
            }
        }
        // The frontend's stepping policy, retuned every frame; the core
        // thread reads the knobs on each of its laps.
        self.knobs
            .set(self.wanted_stride_us(), self.spec_tol_pct, self.spec_grace);
        // One mailbox read per frame, before anything can early-return:
        // every frontend read of the bus goes through the snapshot, so the
        // rendering code cannot tell a hand-cranked bus from the threaded
        // one.
        self.read_snapshot();
        if !self.snap.measuring {
            return;
        }
        if self.snap.trace_paused {
            // Pause/resume bookkeeping lives with whoever runs the bus:
            // the manual drive stamps here, the core thread stamps on its
            // own pause edge.
            let stamp = self.now_us();
            if let CoreDrive::Manual(lane) = &mut self.drive
                && lane.core.paused_at_us.is_none()
            {
                lane.core.paused_at_us = Some(stamp);
            }
            return;
        }
        let now = self.now_us();
        // The f/s figure comes from snapshot deltas, so it reads the same
        // whether the frames were stepped on this thread or the core's.
        let frames = self.snap.frame_counter;
        if let Some((prev_frames, prev_t)) = self.ema_prev {
            let dt_us = now.saturating_sub(prev_t);
            if dt_us > 0 {
                let inst = (frames.saturating_sub(prev_frames)) as f64 * 1e6 / dt_us as f64;
                self.frame_rate = if self.frame_rate == 0.0 {
                    inst
                } else {
                    self.frame_rate * 0.9 + inst * 0.1
                };
            }
        }
        self.ema_prev = Some((frames, now));
        self.last_tick_us = now;
        // Only the manual drive is stepped from here; the threaded core
        // steps itself, on its own clock, inside its own laps.
        if matches!(self.drive, CoreDrive::Manual(_)) {
            self.advance_clock(now);
            self.tick(now);
        }
    }

    /// One step of the measurement loop, polled against wall clock `now_us`.
    /// Split out of [`Self::update`] so a test can run a single step at a time
    /// of its choosing; the generator reads `sim_t_us`, which `update` maintains.
    /// This is one lap of the loop body -- drain the inbox, step the bus,
    /// publish -- hand-cranked on the manual drive; the threaded core runs
    /// the same shape on its own thread. What stays frontend-side is
    /// policy only: the sampling stride, the spec check, and the status
    /// line.
    pub fn tick(&mut self, now_us: u64) {
        let stride = self.wanted_stride_us();
        let (tol, grace) = (self.spec_tol_pct, self.spec_grace);
        let CoreDrive::Manual(lane) = &mut self.drive else {
            return;
        };
        lane.step_lap(now_us, stride, tol, grace);
        self.read_snapshot();
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
