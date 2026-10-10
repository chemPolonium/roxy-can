use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Instant;

use crate::can::frame::{CanFrame, FrameFlags};
// The tests module reaches `Direction` through this module's namespace.
#[allow(unused_imports)]
use crate::can::frame::Direction;
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
/// Row cache cap of one Trace window (R1 virtual scrolling): the newest
/// matching frames the clipper draws from. Memory ≈ 88 B per frame, so
/// 200 k rows ≈ 18 MB per window in the worst case.
pub(crate) const MAX_CACHED_ROWS: usize = 200_000;
/// Synthetic id prefix for derived signals: `emit_value` streams are
/// keyed `(ch, EMITTED_ID_BASE | node_id, false, name)`. The prefix sits
/// far above every real identifier (max 0x1FFFFFFF), so a derived stream
/// can never collide with a frame's own signals.
pub const EMITTED_ID_BASE: u32 = 0x8000_0000;
/// Legacy: the synthetic id prefix FlexRay signals used when they shared the
/// CAN subscription tuple (`(0, FR_SIG_BASE | slot, false, name)`). Nothing
/// writes it any more -- [`crate::observe::SigKey::Fr`] says so outright -- but
/// projects saved before the change still carry it, and loading them has to
/// turn those keys into real FlexRay ones rather than drop the curves.
pub const FR_SIG_BASE: u32 = 0x4000_0000;

/// The subscription key for a FlexRay signal named `name` in `slot` on `bus`.
/// Both the selection tree and the live decode build it from the resolved
/// frame's slot, so a curve and its decoded values share one identity.
pub fn fr_signal_key(bus: u8, slot: u16, name: &str) -> crate::observe::SigKey {
    crate::observe::SigKey::fr(bus, slot, name)
}

/// One FlexRay bus the tool knows about: the cluster description file it was
/// configured from and the database parsed out of it. Kept together so a bus
/// can never end up displaying one file's frames and configuring the port from
/// another's.
pub struct FrBusCfg {
    pub path: String,
    pub db: std::sync::Arc<crate::fr_db::FrDb>,
}
/// Speed ladder shared by the toolbar combo and the slower/faster buttons.
/// The replay speed ladder. The last entry is "as fast as possible":
/// infinity makes the replay clock jump to the end immediately while the
/// UI keeps stepping between polls, so every observer updates in chunks.
pub const REPLAY_SPEEDS: [f64; 5] = [0.5, 1.0, 2.0, 4.0, f64::INFINITY];
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
pub use crate::channel::{Channel, NodeRole};
pub use crate::generator::{TX_CYCLE_MAX_MS, cycle_from_ms_text};
pub use crate::observe::{
    DataWindow, GfxSignal, GraphicsWindow, PickTarget, SampleCache, StateRule, StateWin, YMode,
};
pub use crate::project::PendingAction;
pub use crate::workspace::{
    Desktop, MsgWin, PopupTarget, RowsBuild, SigScope, StatsWin, TraceFilter, TracePick, TraceRow,
    TraceWin, WindowKind,
};

/// "{:.2}" milliseconds, the Min/Avg/Max cell format shared by the snapshot
/// builders below.
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
    /// The row's own identity, for the tree node's ImGui ID. Not derived from
    /// `label`: the same `(id, name)` legitimately appears on two buses -- a
    /// bench where both channels load one DBC does exactly that -- and ImGui
    /// tables do not seed item IDs per cell, so a label-derived ID puts two
    /// visible items on one ID and the conflict check fires.
    pub id_key: String,
    pub bus: String,
    pub dir: &'static str,
    pub count: String,
    pub cycle: String,
    pub flags: FrameFlags,
    pub data: String,
    pub signals: Vec<(String, String)>,
    /// Why the expanded row has no signals to show. Naming the actual reason
    /// matters on a FlexRay table: "no description loaded" and "the
    /// description has no frame for this slot" look identical in the window
    /// and call for opposite fixes.
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

impl Default for MonitorRow {
    fn default() -> Self {
        Self {
            key: crate::observe::SigKey::can(0, 0, false, String::new()),
            label: String::new(),
            digits: 1,
            rule_on: false,
            rising: true,
            threshold: 0.0,
            color: 0,
        }
    }
}

/// Which generator row a dialog is drafting. The Interactive Generator holds
/// two lists keyed differently -- CAN entries by `(channel, id)`, FlexRay
/// entries by `(bus, slot)` -- and an index alone cannot say which one it came
/// from, so the dialogs that serve both carry one of these.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum GenRow {
    /// Index into `Snapshot::tx`.
    Can(usize),
    /// Index into `Snapshot::fr_tx`.
    Fr(usize),
}

pub struct App {
    /// Where the bus actually lives. `Threaded` -- the production drive:
    /// the core runs on its own thread and only commands and snapshots
    /// cross. `Manual` -- tests and the future headless CLI keep the core
    /// here and crank its loop by hand, with `send` staying synchronous.
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
    /// The Triggers window's editor popup, if open: the row it edits plus
    /// the not-yet-applied shape. `None` while the popup is closed.
    pub(crate) trig_draft: Option<crate::ui::triggers::TrigDraft>,
    /// The System Variables manager's add/edit popup, if open. Session
    /// state, like the trigger editor's.
    pub(crate) sysvar_draft: Option<crate::ui::sysvars::SysVarDraft>,
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
    pub show_sysvars: bool,
    pub show_write: bool,
    pub show_monitor: bool,
    /// The recording file format: 0 = ASC, 1 = BLF. The toolbar combo
    /// selects this; the recorder picks the backend by extension.
    pub record_format: usize,
    pub show_shortcuts: bool,
    pub show_about: bool,
    /// The Monitor window's rows: label + value + optional coloring rule
    /// per selected signal. The lightweight big-screen panel (v1).
    pub monitor_rows: Vec<MonitorRow>,
    pub id_filter_search: String,
    pub gen_search: String,
    pub popup_target: Option<PopupTarget>,
    pub focus_title: Option<String>,
    /// The State Tracker row whose state editor is open: window index plus
    /// signal key. Session state only.
    pub state_rule_edit: Option<(usize, crate::observe::SigKey)>,
    /// The swatch currently picking a color in that editor: window index,
    /// signal key, and the target -- a custom band, or a default-mode
    /// state value (normalized bits). Session state only.
    pub state_rule_pick: Option<(usize, crate::observe::SigKey, PickTarget)>,
    /// CTE text editors, keyed by the node's stable id. They bind to the
    /// ImGui context, which `App::new` does not have: rendering a script
    /// editor queues the id in `pending_editors`, and main creates the
    /// editor before the next frame's `Ui` borrow starts. Session state.
    pub editors: HashMap<u64, dear_imgui_cte::TextEditor>,
    /// Node ids whose editor the UI asked for; main drains this queue.
    pub pending_editors: Vec<u64>,
    /// The `node.source` each editor was last seeded with. An external
    /// change (project load, Apply) re-seeds the editor; without this
    /// bookkeeping the editor and the model would fight over the text.
    pub(crate) editor_synced: HashMap<u64, String>,
    /// Text a Replay Blocks editor is typing but has not applied yet,
    /// keyed by the block's stable id: name, log path, id filter text.
    /// Session state only, like the node source drafts.
    pub block_drafts: HashMap<u64, crate::ui::BlockDraft>,
    /// Script-node editors currently open, by node id in open order.
    /// Session state; an id whose node is gone closes its editor.
    pub open_editors: Vec<u64>,
    /// Per-editor static facts (outline, send/receive/sysvar sets),
    /// keyed by node id and re-derived from the editor text when its hash
    /// changes. Session state.
    pub(crate) editor_facts: HashMap<u64, crate::ui::script_editor::EditorFacts>,
    /// Write window's per-kind visibility, ordered
    /// `[Script, Info, Warning, Error]`. Session state.
    pub write_filter: [bool; 4],
    /// Hardware channels discovered on this machine across every
    /// supported driver, enumerated once on first need. `Err` = no
    /// driver is available at all.
    pub hw_channels: Option<Result<Vec<crate::hw::AnyChannelInfo>, String>>,
    /// FlexRay-capable Vector channels, enumerated once on first need
    /// (a real vxlapi call). `Err` = the Vector driver is unavailable.
    pub fr_channels: Option<Result<Vec<crate::hw::vector::ChannelInfo>, String>>,
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
    pub fr_buses: std::collections::BTreeMap<u8, FrBusCfg>,
    /// What the user calls each FlexRay 路. Apart from [`Self::fr_buses`] on
    /// purpose: a name is not a description, so a cluster seen only in a
    /// replayed log -- which is exactly the case where `FR0`/`FR1` say nothing
    /// -- can still be named. Unnamed 路 fall back to `FR{n}`.
    pub fr_names: std::collections::BTreeMap<u8, String>,
    /// FlexRay name draft: the 路 whose Name box has focus, plus its text.
    /// Mirrors `bus_name_edit` for the CAN rows.
    pub fr_name_edit: Option<(u8, String)>,
    /// Profile names from the project's `profiles/` directory, listed
    /// once on first need (`Err` = driver/unavailable? no — parse or IO
    /// failure of the directory scan). Session cache; refresh via
    /// [`App::refresh_profiles`].
    pub profile_names: Option<Result<Vec<String>, String>>,
    /// The selected index in the Network-window profile dropdown.
    pub profile_pick: usize,
    /// The new-profile name being typed in the Network Profile row;
    /// committed by "存当前" into `profiles/<名>.toml`.
    pub profile_draft_name: String,
    /// Delete confirmation for the Profile row: the first click arms it,
    /// the second click within the session deletes the file.
    pub profile_delete_arm: bool,
    pub net_selected: usize,
    /// The FlexRay ECU the Network detail pane is showing: `(cluster index,
    /// ECU name)`. Kept apart from `net_selected` on purpose -- a DBC node index
    /// and a cluster's ECU are different things in different numbering spaces,
    /// and only one of them owns the detail pane at a time.
    pub net_fr_sel: Option<(u8, String)>,
    pub tx_pick: usize,
    /// Which slot the FlexRay add line's combo is pointing at, per 路. Session
    /// state, like `tx_pick`.
    pub fr_tx_pick: std::collections::BTreeMap<u8, usize>,
    /// The generator overview's add-by-id draft: the bus row being typed
    /// in plus its hex text, committed to the model on Add. Session state.
    pub gen_add_buf: Option<(u8, String)>,
    /// Generator row whose value-source parameters the modal is editing:
    /// index into `tx_list` plus the DBC signal name.
    pub src_edit: Option<(GenRow, String)>,
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
    pub tx_cycle_edit: Option<GenRow>,
    /// Draft period in whole milliseconds for that row.
    pub tx_cycle_buf: String,
    /// The generator's editable data box while it has focus: row index plus
    /// the text typed so far. The live buffer is frontend draft state -- the
    /// bus only sees the parsed payload once the edit commits (via
    /// `SetEntryHex`).
    pub tx_data_edit: Option<(usize, String)>,
    /// The same draft box for a FlexRay row's payload, kept apart so a CAN row
    /// and a FlexRay row with the same index never read each other's text.
    pub fr_data_edit: Option<(usize, String)>,
    /// Buses-window rename draft: `(row, text)` while the field has
    /// focus, committed as a `SetChannelConfig` command on focus loss.
    pub bus_name_edit: Option<(usize, String)>,
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
    pub state_trackers: Vec<StateWin>,
    pub(crate) trace_counter: usize,
    pub(crate) msg_counter: usize,
    pub(crate) stats_counter: usize,
    pub(crate) graphics_counter: usize,
    pub(crate) data_counter: usize,
    pub(crate) state_counter: usize,
}

/// Which side of the stage-3 split the bus is on.
pub(crate) enum CoreDrive {
    /// The core runs on its own thread (`App::new`); this App holds only
    /// the command sender and the mailbox reader.
    Threaded,
    /// The core is a plain value here (`App::headless`): tests and the
    /// future headless CLI crank the same loop laps by hand, with
    /// synchronous `send`.
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
        core.channels = vec![
            Channel {
                name: "CAN1".to_string(),
                dbc: None,
                dbc_paths: vec!["assets/sample.dbc".to_string()],
                dbc_sums: Vec::new(),
                node_roles: std::collections::BTreeMap::new(),
                bitrate_kbps: Channel::DEFAULT_BITRATE_KBPS,
                fd_data_kbps: Channel::DEFAULT_FD_DATA_KBPS,
            },
            Channel {
                name: "CAN2".to_string(),
                dbc: None,
                dbc_paths: vec!["assets/motbus.dbc".to_string()],
                dbc_sums: Vec::new(),
                node_roles: std::collections::BTreeMap::new(),
                bitrate_kbps: Channel::DEFAULT_BITRATE_KBPS,
                fd_data_kbps: Channel::DEFAULT_FD_DATA_KBPS,
            },
        ];
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
        let mut app = App {
            drive,
            inbox_tx,
            mail,
            knobs,
            snap: std::sync::Arc::new(crate::bus::Snapshot::default()),
            ema_prev: None,
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
            trig_draft: None,
            sysvar_draft: None,
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
            show_sysvars: false,
            show_write: true,
            show_monitor: false,
            show_shortcuts: false,
            show_about: false,
            monitor_rows: Vec::new(),
            id_filter_search: String::new(),
            gen_search: String::new(),
            popup_target: None,
            focus_title: None,
            state_rule_edit: None,
            state_rule_pick: None,
            editors: HashMap::new(),
        pending_editors: Vec::new(),
        editor_synced: HashMap::new(),
            block_drafts: HashMap::new(),
            open_editors: Vec::new(),
        editor_facts: HashMap::new(),
            write_filter: [true; 4],
            hw_channels: None,
            profile_names: None,
            profile_pick: 0,
            profile_draft_name: String::new(),
            profile_delete_arm: false,
            net_selected: 0,
            net_fr_sel: None,
            tx_pick: 0,
            fr_tx_pick: Default::default(),
            gen_add_buf: None,
            src_edit: None,
            src_seq_buf: String::new(),
            src_draft: None,
            num_draft: crate::ui::Draft::default(),
            tx_cycle_edit: None,
            tx_cycle_buf: String::new(),
            tx_data_edit: None,
            fr_data_edit: None,
            // Buses-window rename draft: (row, text) while the field is
            // being typed in, committed as a command on focus loss.
            bus_name_edit: None,
            // The record draft: a file name, shown as-is in the toolbar
            // input. The directory is implicit (the open project's
            // Record/ folder, resolved when recording arms); the
            // recorder receives the full path via `SetRecordPath`.
            record_path_buf: String::new(),
            // Toolbar combo: 0 = ASC, 1 = BLF. Decides the extension
            // `toggle_record` stamps onto the draft stem.
            record_format: 0,
            // FlexRay watch drafts: the cached FR channel enumeration (the
            // Vector probe is a real driver call). The combo pick is not
            // cached -- each row's 硬件 column picks its own port, per frame.
            fr_channels: None,
            hw_pick: std::collections::BTreeMap::new(),
            fr_pick: std::collections::BTreeMap::new(),
            fr_buses: Default::default(),
            fr_names: Default::default(),
            fr_name_edit: None,
            record_filter_text: String::new(),
            trace_limit: TRACE_LIMIT,
            limits: Default::default(),
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
        let mut first = app.desktop_snapshot();
        first.name = "Desktop 1".to_string();
        app.desktops = vec![first];
        app.active_desktop = 0;
        app.baseline = app.config_snapshot();
        app
    }

    /// The observer windows a session boots with: **Trace and Messages**, the two
    /// an operator looks at first. Statistics / Graphics / Data / State Tracker
    /// are one `+` in Measurement Setup away and are no longer handed to you
    /// unasked -- they used to be six windows in every project, and every window
    /// the workspace holds is a window the project file has to carry.
    ///
    /// `every_kind` is what the manual drive (tests, the headless CLI) passes to
    /// keep one of each: those suites index each kind by position, and dock state
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
        let core = Self::build_core();
        let (inbox_tx, inbox_rx) = std::sync::mpsc::channel();
        let mail = crate::bus::new_mailbox();
        let knobs = std::sync::Arc::new(crate::core_loop::BusKnobs::default());
        let mut lane = crate::core_loop::CoreLoop::new(core, inbox_rx, mail.clone());
        lane.publish();
        Self::shell(CoreDrive::Manual(Box::new(lane)), inbox_tx, mail, knobs, true)
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
            "{} · {} 项未能载入（Write 窗口有明细）",
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
        let ext = if self.record_format == 1 { "blf" } else { "asc" };
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
        match self
            .project_path
            .as_ref()
            .and_then(|p| p.parent())
        {
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
    /// clock does not, so a paused window used to drift its own curve away.
    pub fn plot_now_s(&self) -> f64 {
        if matches!(self.snap.mode, Mode::Replay)
            && let Some((pos, _)) = self.snap.replay
        {
            return pos as f64 / 1e6;
        }
        self.snap.sim_t_us as f64 / 1e6
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
        let mut rows = Vec::with_capacity(aggs.len() + fr_aggs.len());
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
                flags: crate::can::frame::FrameFlags::NONE,
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
    /// carried -- so the name is stored, not re-resolved: re-resolving is what
    /// used to let a slot's row change frames as the cycles ran by.
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
    pub(crate) fn fr_described_frame(
        &self,
        agg: &crate::aggregate::FrFrameAgg,
    ) -> Option<usize> {
        self.fr_db(agg.bus)
            .and_then(|db| agg.occupant.frame_ix().filter(|&i| db.frame_index(i).is_some()))
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
        self.fr_buses.get(&bus).map(|c| &*c.db)
    }

    /// Every latched spec violation, CAN and FlexRay alike: what the report
    /// window lists and what its CSV writes. The two buses keep separate
    /// storage because their identities do not share a key (see
    /// [`crate::spec::Spec`]); a report reads both.
    pub(crate) fn spec_rows(&self) -> Vec<crate::spec::SpecRow> {
        let mut out = Vec::new();
        for ((ch, id, ext, kind), l) in &self.snap.spec.rows {
            let name = self
                .channel_dbc(*ch)
                .and_then(|db| db.message_name_of((*id, *ext)))
                .unwrap_or("not in database");
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
                            format!("slot {slot} 的帧 {ix}")
                        } else {
                            f.name.clone()
                        }
                    })
                    .unwrap_or_else(|| "当前描述已无此帧".to_string()),
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
    pub(crate) fn fr_slot_wire_us(&self, bus: u8) -> Option<f64> {
        self.fr_db(bus)
            .and_then(|db| crate::load::fr_slot_wire_us(&db.params))
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
            let declared = self.message_name(agg.channel, agg.id);
            let name = declared.unwrap_or("-");
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
            // A message the DBC does not declare and one that declares no
            // signals are different facts.
            let empty_note = if !signals.is_empty() {
                None
            } else if declared.is_none() {
                Some("(not in DBC)".to_string())
            } else {
                Some("（该报文不声明信号）".to_string())
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
                dir: match (agg.rx > 0, agg.tx > 0) {
                    (true, true) => "Rx+Tx",
                    (false, true) => "Tx",
                    _ => "Rx",
                },
                count: agg.count.to_string(),
                cycle: if agg.count > 1 {
                    format!("{:.1} ±{:.1}", agg.cycle_us / 1000.0, agg.jitter_us / 1000.0)
                } else {
                    "-".to_string()
                },
                flags: agg.flags,
                data: agg.payload().iter().map(|b| format!("{b:02X} ")).collect(),
                signals,
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
                let signals = match (db, frame_ix) {
                    (Some(db), Some(frame_ix)) => db.decode(frame_ix, &agg.payload),
                    _ => Vec::new(),
                };
                // Which of the three reasons it is: no description for this
                // cluster, a description that does not schedule this slot's
                // frame, or a frame that declares no signals.
                let empty_note = if !signals.is_empty() {
                    None
                } else if db.is_none() {
                    Some(format!("（{} 未加载集群描述）", self.fr_bus_name(agg.bus)))
                } else if frame_ix.is_none() {
                    Some(format!(
                        "（{} 的描述未在 slot {} 调度此帧）",
                        self.fr_bus_name(agg.bus),
                        agg.slot
                    ))
                } else {
                    Some("（该帧不声明信号）".to_string())
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
                    dir: "Rx",
                    count: agg.count.to_string(),
                    cycle: if agg.count > 1 {
                        format!(
                            "{:.1} ±{:.1}",
                            agg.cycle_us / 1000.0,
                            agg.jitter_us / 1000.0
                        )
                    } else {
                        "-".to_string()
                    },
                    flags: crate::can::frame::FrameFlags::NONE,
                    data: agg.payload.iter().map(|b| format!("{b:02X} ")).collect(),
                    signals,
                    empty_note,
                });
            }
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
    /// cluster descriptions and both rings are unchanged since the last build
    /// and no column sort has reordered the cache, the walk covers just the rows
    /// that arrived after the build point. Walking the whole ring every gate tick
    /// cost 7 ms on a 50 000-row FlexRay replay and 70 ms with the FR signal
    /// expansion on (measured, `--release`, see
    /// [`crate::headless_tests::perf_flexray_readouts_under_load`]) against a 10
    /// Hz text gate.
    ///
    /// Anything else -- an edited filter, a sorted column, a description swapped
    /// onto a FlexRay bus, a cleared or restarted run -- drops the cache and
    /// walks the whole revealed ring once, exactly as before. Expanding FlexRay
    /// rows into their decoded signals costs that walk the count of child rows
    /// per frame row, nothing else: the children are not entries of this list
    /// (see [`TraceRow::Fr`]).
    pub(crate) fn sync_trace_rows(&mut self, i: usize) {
        if !self.text_fresh {
            return;
        }
        let newest = self.snap.trace.last().map(|f| f.t_us).unwrap_or(u64::MAX);
        let (can_len, fr_len) = (self.snap.trace.len(), self.snap.fr_trace.len());
        // The point a new walk starts above: the newest row of either ring at
        // the last build. A CAN-empty run never advances `newest`, which is why
        // the FlexRay ring counts here too.
        let top = self
            .snap
            .trace
            .last()
            .map(|f| f.t_us)
            .unwrap_or(0)
            .max(self.snap.fr_trace.last().map(|r| r.t_us).unwrap_or(0));
        self.trace_windows[i].shown_t_us = newest;
        self.trace_windows[i].shown_count = can_len;
        let flt = self.trace_windows[i].filter_lens();
        let fr_expand = self.trace_windows[i].fr_expand;
        // Which description each FlexRay bus resolved its rows against: an
        // expanded child row is an index into it, so a swap means a rebuild.
        let fr_dbs: Vec<(u8, usize)> = self
            .fr_buses
            .iter()
            .map(|(b, c)| (*b, std::sync::Arc::as_ptr(&c.db) as usize))
            .collect();
        let since = match (&self.trace_windows[i].rows_build, self.trace_windows[i].rows_sorted) {
            (Some(b), false) if b.extends_to(&flt, fr_expand, can_len, fr_len, top, &fr_dbs) => {
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
        w.rows_build = Some(RowsBuild::new(&flt, fr_expand, top, can_len, fr_len, &fr_dbs));
        // The walk produced this list; the cache is it, in the same order.
        w.rows_sorted = false;
        w.shown_count = rows.len();
        w.rows = rows;
    }

    /// Merges the two rings into the window's row list, newest first, CANoe's
    /// Trace shape: walk both newest-first lists taking whichever frame is the
    /// more recent, so the merge is linear in the kept rows. `since` bounds the
    /// walk to rows strictly newer than that timestamp (see
    /// [`App::sync_trace_rows`]); the result is prepended to `rows` in that
    /// case, and becomes the whole list otherwise.
    fn walk_rows(
        &self,
        i: usize,
        flt: &TraceFilter,
        fr_expand: bool,
        since: Option<u64>,
        rows: &mut std::collections::VecDeque<TraceRow>,
    ) {
        // Both lists newest first, cut at the window's reveal watermark and at
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
        let mut merged: Vec<TraceRow> = Vec::with_capacity(1_024);
        let (mut ci, mut fi) = (0usize, 0usize);
        while merged.len() < MAX_CACHED_ROWS {
            // Whichever ring still has a row, and of the two the more recent --
            // both lists are newest first, so this merge is linear in the kept
            // rows.
            let take_can = match (can.get(ci), fr.get(fi)) {
                (None, None) => break,
                (Some(_), None) => true,
                (None, Some(_)) => false,
                (Some(c), Some(r)) => c.t_us >= r.t_us,
            };
            if take_can {
                merged.push(TraceRow::Can(*can[ci]));
                ci += 1;
                continue;
            }
            let row = (*fr[fi]).clone();
            fi += 1;
            // A FlexRay frame row expands into its decoded signal child rows,
            // the CANoe trace shape, when the window asks and that bus's
            // description database is loaded. Only the count of them travels in
            // the list -- see [`TraceRow::Fr`].
            let kids = if fr_expand {
                self.fr_child_count(&row)
            } else {
                0
            };
            merged.push(TraceRow::Fr(row, kids));
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

impl App {
    /// Opens the node's script editor (no-op when already open).
    pub fn open_script_editor(&mut self, id: u64) {
        if !self.open_editors.contains(&id) {
            self.open_editors.push(id);
        }
    }

    /// Closes the node's script editor and drops its CTE editor with all
    /// cached state; the next open starts from the node's saved source.
    pub fn close_script_editor(&mut self, id: u64) {
        self.open_editors.retain(|&x| x != id);
        self.editors.remove(&id);
        self.editor_synced.remove(&id);
        self.editor_facts.remove(&id);
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
