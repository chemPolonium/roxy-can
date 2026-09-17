//! The bus core: everything that must keep running at the bus's own rate,
//! independent of the frontend's frame loop.
//!
//! Phase 1 of the split (TODO.md, 主线): the struct exists and `App` derefs
//! to it, so bus fields and methods migrate slice by slice while every
//! existing access site keeps compiling. Everything is still driven from the
//! UI thread; the command/snapshot boundary and the dedicated thread are
//! stages 2 and 3.

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;

use crate::aggregate::MessageAgg;
use crate::app::{Mode, SAMPLE_INTERVAL_US, TRACE_LIMIT};
use crate::can::frame::{CanFrame, Direction};
use crate::channel::{Channel, NodeRole};
use crate::dbc::DecodedSignal;
use crate::generator::TxMsg;
use crate::observe::{SigKey, Subscription};
use crate::script::HostInput;
use crate::source::FrameSource;
use crate::spec::{Kind, cycle_offender, dlc_offender, missing_offender};
use crate::trigger::{TriggerAction, TriggerCond};

/// Slots one generator entry may backfill per step after the clock jumped
/// (a frozen UI, a seek). Bounds the burst; anything longer streams over the
/// following steps. At 100 Hz one step covers ~10 s of missed timeline.
pub(crate) const MAX_TX_CATCHUP: u32 = 1024;

/// Oldest bus-event markers fall off once the list outgrows this.
pub(crate) const MARKER_CAP: usize = 512;

/// FNV-1a content checksum for a DBC file, used by the auto-reload sweep
/// to detect external edits without full re-parses.
pub(crate) fn content_sum(bytes: &[u8]) -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for &b in bytes {
        h ^= b as u64;
        h = h.wrapping_mul(0x100_0000_01b3);
    }
    h
}

/// How many frames before a trigger-initiated recording start are written
/// into the file (the pre-trigger context).
pub(crate) const PRE_BUFFER_FRAMES: usize = 256;

/// Frames a trigger-initiated recording keeps writing after a
/// `StopRecording` edge (the post-trigger context).
pub(crate) const POST_ROLL_FRAMES: u32 = 32;

/// How many distinct derived-signal streams (`emit_value` names) may
/// exist at once. Beyond it, emissions under further NEW names are
/// ignored for the rest of the session -- a guard against a script
/// minting names in a loop, not a real limit.
pub(crate) const MAX_EMITTED_STREAMS: usize = 256;

/// One replay block as the frontend sees it this frame: the declaration
/// plus what the runtime made of it (queue size, load failure).
#[derive(Clone, Debug)]
pub struct ReplayBlockView {
    pub id: u64,
    pub name: String,
    pub channel: u8,
    pub path: String,
    pub node_filter: Option<String>,
    /// The DBC node this block belongs to (tree placement, node detail).
    pub attached: Option<(u8, String)>,
    pub ids: Vec<(u32, bool)>,
    pub enabled: bool,
    /// Frames in the loaded queue; 0 unless the block loaded successfully.
    pub frames: usize,
    pub last_error: Option<String>,
}

/// One bus's hardware attachment as the frontend sees it.
#[derive(Clone, Debug)]
pub struct HwBusView {
    pub bus: u8,
    /// The adapter identity (driver-specific channel index).
    pub adapter: i32,
    /// Which vendor driver this attachment talks through.
    pub driver: crate::hw::HwDriver,
    pub kbps: u32,
    /// Whether the attachment can transmit (holds init access).
    pub can_tx: bool,
    /// FD data-phase params were applied; FD frames can leave.
    pub fd: bool,
}

/// The FlexRay RX-only watch as the frontend sees it: identity only --
/// the port itself stays core-side.
#[derive(Clone, Debug, PartialEq)]
pub struct FrWatchView {
    pub channel_index: i32,
    pub fibex_path: String,
}

/// What the frontend may ask the bus to do. One variant per transport
/// action, deliberately carrying no UI state -- no picked file paths, no
/// window selections; the frontend resolves those before asking. That is
/// what lets the same enum cross a thread unchanged in stage 3.
#[derive(Clone, Debug, PartialEq)]
pub enum BusCommand {
    /// Fresh virtual run: swap in a new virtual source, blank every
    /// run-scored counter, measure against the wall clock.
    StartVirtual,
    /// Stop measuring and close the recorder.
    Stop,
    /// Freeze or unfreeze the trace views; frozen arrivals stamp at the
    /// freeze instant.
    SetTracePaused(bool),
    /// Toggle ASC recording. While stopped, the file is created by the
    /// next start -- checking Record must not leave an empty file behind.
    ToggleRecord,
    /// Clears the level latch of every appearance-type condition (ID
    /// present / error frame) so they fire again on their next
    /// occurrence. Real-level conditions are untouched.
    RearmTriggers,
    /// Blanks the trace view: the ring and its disk archive are dropped,
    /// everything else (recording, aggregates, triggers) keeps running.
    ClearTrace,
    /// Resets the per-message counters the Messages / Statistics windows
    /// read. Load, spec memory and the trace are untouched.
    ClearAggregates,
    /// Select the bus kind the next start will run (Simulation / Replay).
    SetRunMode(crate::app::Mode),
    /// Add a bus with the sample DBC path, load it, and pre-populate its
    /// generator.
    AddChannel,
    /// Remove bus `ch` and remap every channel-indexed reference one step
    /// down. The frontend remaps its own window state in the same stroke.
    RemoveChannel {
        ch: usize,
    },
    /// Declare the role of a DBC node: `Simulated` transmits its DBC
    /// traffic through the generator; `Monitor`/`Absent` do not.
    SetNodeRole {
        ch: u8,
        node: String,
        role: crate::app::NodeRole,
    },
    /// Replay-speed multiplier applied to the log source. (The remembered
    /// choice for the next run and the combo's display stay frontend.)
    SetReplaySpeed(f64),
    /// Move the replay playhead to `t_s` seconds, clamped to the log's
    /// duration. Works while running, paused, or stopped after the log
    /// ran out; a scrub past the right edge lands on the last frame.
    SeekReplay(f64),
    /// Generator-entry edits, keyed by `(channel, id)` -- `add_tx`
    /// dedupes on that pair, so the key is stable where an index would
    /// shift under the sender's feet.
    SetEntryActive {
        ch: u8,
        id: u32,
        on: bool,
    },
    /// Toggle the entry's CAN FD flag.
    SetEntryFd {
        ch: u8,
        id: u32,
        fd: bool,
    },
    /// Set the send period in µs (0 = event-triggered); the schedule
    /// restarts now rather than at the end of the old one.
    SetEntryCycle {
        ch: u8,
        id: u32,
        cycle_us: u64,
    },
    /// Replace the base payload from hex text. Active sources deliberately
    /// survive: correcting one byte must not throw away a stimulus setup.
    SetEntryHex {
        ch: u8,
        id: u32,
        text: String,
    },
    /// Drop the entry, payload, sources and schedule with it.
    RemoveEntry {
        ch: u8,
        id: u32,
    },
    /// Add the entry `(ch, id)` unless it exists. Name, node, length and
    /// period come from the bus's database when it knows the message.
    AddEntry {
        ch: u8,
        id: u32,
    },
    /// Add or replace the source driving one signal on the entry.
    SetEntrySource {
        ch: u8,
        id: u32,
        src: crate::sim::ValueSrc,
    },

    /// Stop driving one signal; the base bytes take over again.
    ClearEntrySource {
        ch: u8,
        id: u32,
        name: String,
    },
    /// Write a physical value into the base payload and pin that signal
    /// by dropping only its source: grabbing a moving slider means
    /// "hold here".
    PinEntrySignal {
        ch: u8,
        id: u32,
        name: String,
        phys: f64,
    },
    /// Start caching one signal: a fresh subscription gets the next
    /// palette color and the database's display type. An existing
    /// subscription for the key is left untouched.
    Subscribe {
        key: SigKey,
    },
    /// Drop the subscription. The frontend only asks after none of its
    /// windows references the signal anymore.
    Unsubscribe {
        key: SigKey,
    },
    /// Open `path` and replay it: generators stand down for the ids the
    /// log carries, a fresh replay source replaces the old input at
    /// `speed`, and run-scored state resets. `speed` is the frontend's
    /// remembered multiplier, passed at start like the other policy
    /// knobs.
    StartReplay {
        path: String,
        speed: f64,
    },
    /// Resume a scrubbed replay in place: measurement restarts with the
    /// trace unfrozen, captured history untouched, so playback continues
    /// from the playhead. `speed` is the frontend's remembered
    /// multiplier, for the status line.
    ResumeReplay {
        speed: f64,
    },
    /// Static per-bus declarations from the Buses window / project
    /// restore: rename, bitrate figures, the DBC path (without loading
    /// it -- pair with `LoadDbc`), or the node-role map.
    SetChannelConfig {
        ch: u8,
        name: Option<String>,
        dbc_path: Option<String>,
        bitrate_kbps: Option<u32>,
        fd_data_kbps: Option<u32>,
        node_roles: Option<std::collections::BTreeMap<String, crate::app::NodeRole>>,
    },
    /// Attach the listed DBC files to the bus (primary first; on a
    /// duplicate message id the earlier database wins) and parse them all
    /// into one merged table. An empty list drops the databases. A file
    /// that fails to parse is skipped and reported: what the snapshot
    /// then shows is what the bus will actually use.
    LoadDbc {
        ch: u8,
        paths: Vec<String>,
    },
    /// Rewind the bus-name counter used to mint "CAN{n}" for new buses;
    /// project restore pins it so the next added bus keeps counting from
    /// where the project left off.
    SetBusCounter(usize),
    /// The dated ASC file the recorder derives its name from. Only read
    /// when recording actually arms.
    SetRecordPath(String),
    /// Id whitelist for recorded files: only these `(id, extended)` frames
    /// land in the ASC (empty list = record everything). Gates the file,
    /// the pre-trigger context, and post-roll counting; never the trace,
    /// aggregates, or the spec.
    SetRecordFilter {
        ids: Vec<(u32, bool)>,
    },
    /// The trace ring's retention in frames, clamped core-side. Takes
    /// effect on the next ingest; already-dropped frames stay gone.
    SetTraceLimit {
        frames: usize,
    },
    /// Attach (or re-attach) a hardware adapter to a bus: its received
    /// frames are ingested, and per-node switches direct generator
    /// traffic onto the wire. `None` detaches. `fd_data_kbps` opts the
    /// channel into CAN FD when a data-phase preset exists for it.
    SetHardwareChannel {
        bus: u8,
        driver: crate::hw::HwDriver,
        adapter: i32,
        kbps: u32,
        fd_data_kbps: Option<u32>,
    },
    DetachHardware {
        bus: u8,
    },
    /// Attach (or move) the FlexRay RX-only watch to a Vector channel,
    /// configured from the FIBEX description at `fibex_path`; `None`
    /// detaches. RX-only and read-only: it never transmits, and its
    /// frames feed the Trace window's FR section alone.
    SetFrWatch {
        channel_index: Option<i32>,
        fibex_path: String,
    },
    /// Trigger-recording context, clamped core-side: pre-trigger frames,
    /// post-roll frames, and the marker list cap. Any subset may be set.
    SetRunLimits {
        pre_frames: Option<usize>,
        post_frames: Option<u32>,
        marker_cap: Option<usize>,
    },
    /// Add a script node bound to one channel; it starts with the next
    /// measurement (or immediately, if one is running).
    AddNode {
        name: String,
        channel: u8,
        /// 绑定的 DBC 节点 (总线, 节点名)：绑定脚本的发帧受该节点
        /// 角色闸控制。
        attached: Option<(u8, String)>,
    },
    /// Remove a node wholesale: source, runtime, log.
    RemoveNode {
        id: u64,
    },
    SetNodeName {
        id: u64,
        name: String,
    },
    /// Replace a node's source. While measuring the node recompiles and
    /// restarts in place; while stopped the edit simply waits.
    SetNodeSource {
        id: u64,
        source: String,
    },
    /// Enabled nodes run with the measurement; disabled ones sit idle.
    SetNodeEnabled {
        id: u64,
        on: bool,
    },
    /// Replace the node list wholesale -- the project-restore shape. Ids
    /// are minted fresh; the frontend's drafts are keyed by id and simply
    /// detach, matching the removed sources.
    SetNodes {
        nodes: Vec<crate::config::NodeCfg>,
    },
    /// Add a replay block: recorded traffic from `path`, filtered, injected
    /// onto bus `ch` when a simulation runs. New blocks start disabled --
    /// nothing transmits until the user says so. `attached` names the DBC
    /// node the block belongs to (its management home); `None` is a free,
    /// bus-level block.
    AddReplayBlock {
        name: String,
        channel: u8,
        path: String,
        node_filter: Option<String>,
        attached: Option<(u8, String)>,
        ids: Vec<(u32, bool)>,
    },
    /// Edit a replay block's declaration wholesale. An enabled block
    /// reloads its queue from the log, so the edit takes effect at once.
    SetReplayBlock {
        id: u64,
        name: String,
        channel: u8,
        path: String,
        node_filter: Option<String>,
        attached: Option<(u8, String)>,
        ids: Vec<(u32, bool)>,
    },
    /// Remove a replay block wholesale.
    RemoveReplayBlock {
        id: u64,
    },
    /// Toggle a replay block. Enabling loads its queue from the log right
    /// away, so a broken path surfaces as a status line, not as silence.
    SetReplayBlockEnabled {
        id: u64,
        on: bool,
    },
    /// Replace the replay-block list wholesale -- the project-restore
    /// shape. Ids are minted fresh; enabled blocks load their queues.
    SetReplayBlocks {
        blocks: Vec<crate::config::BlockCfg>,
    },
    /// Offline analysis (R1): ingest the whole loaded log once -- no
    /// playback clock, no trigger actions, no node dispatch, no
    /// recording -- so every observer (aggregates, Statistics, the load
    /// history, spec verdicts, Graphics/Tracker caches, Trace) holds the
    /// full file for browsing. The source is opened fresh and parked
    /// after the scan; Play afterwards restarts the replay the normal way.
    ScanLog {
        path: String,
        tol_pct: u64,
        grace: u64,
    },
    /// Restore one generator row wholesale from a saved project. `None`
    /// data_text keeps the row's current base payload. Rows the bus does
    /// not know are ignored -- the database decides which messages exist.
    /// `Some` flags overrides the FD-derived ones wholesale (a trace row
    /// added to the generator keeps the exact flags it was seen with).
    SetEntryConfig {
        ch: u8,
        id: u32,
        active: bool,
        cycle_us: u64,
        fd: bool,
        flags: Option<crate::can::frame::FrameFlags>,
        data_text: Option<String>,
        srcs: Vec<crate::sim::ValueSrc>,
    },
    /// Decode the frames covering `[from_us, to_us]` into the signal
    /// caches at `stride_us`. This is core work (it scans the log
    /// source), requested by a plot window that wants to draw ground the
    /// playhead has not reached yet. The request is idempotent: covered
    /// spans are skipped on the bus side.
    Backfill {
        from_us: u64,
        to_us: u64,
        stride_us: u64,
    },
    /// Blank every bus's database and drop the generator and the signal
    /// subscriptions with it -- the bus half of "new project".
    ClearDatabases,
    /// Latch-free the specification report: every row forgets it fired.
    /// The next run replaces the rows anyway; this is the in-run reset.
    ClearSpec,
    /// Transmit a single frame from the generator entry `(ch, id)` right
    /// now: its own base bytes and waveform values, its schedule and
    /// active flag untouched. Delivered on the next step of a running bus;
    /// a request made while stopped is dropped.
    SendNow {
        ch: u8,
        id: u32,
    },
    /// Replace the trigger rule list wholesale -- the project-restore
    /// shape of trigger editing.
    SetTriggers(Vec<crate::trigger::Trigger>),
    /// Append a trigger rule (the Triggers window's "+ <shape>" buttons).
    AddTrigger {
        cond: crate::trigger::TriggerCond,
        action: crate::trigger::TriggerAction,
    },
    /// Drop trigger `index`.
    RemoveTrigger {
        index: usize,
    },
    /// Flip one trigger's enable checkbox.
    SetTriggerEnabled {
        index: usize,
        on: bool,
    },
    /// Replace trigger `index`'s condition and action -- the editor's
    /// Apply. The edge level resets: it belonged to the old condition,
    /// and a reshaped trigger starts from a clean edge.
    EditTrigger {
        index: usize,
        cond: crate::trigger::TriggerCond,
        action: crate::trigger::TriggerAction,
    },
    /// Define a system variable, or replace an existing definition with
    /// the same `namespace::name` (the live value resets to `init`).
    DefineSysVar(SysVarDef),
    /// Drop the system variable. Script reads of it fall back to 0 and
    /// writes are reported at the writing node.
    DeleteSysVar {
        namespace: String,
        name: String,
    },
    /// Set a system variable's live value (manager edit or script write
    /// relay), clamped against the definition's bounds.
    SetSysVar {
        namespace: String,
        name: String,
        value: f64,
    },
    /// Empties the Write window's ring.
    ClearWrite,
    /// CANoe-style bus mode: `real` connects every attachment to its
    /// adapter (RX feeds the bus, directed TX goes to the wire);
    /// `!real` (Simulated) parks them while keeping the configuration.
    SetBusMode { real: bool },
}

/// The synthetic stream id system variables publish under: an id no node
/// can ever mint (`node_counter` counts up from 1), so a sysvar stream
/// cannot collide with a script's derived signals.
pub(crate) const SYSVAR_STREAM_ID: u64 = u64::MAX;

/// Oldest Write-window lines fall off once the list outgrows this.
pub(crate) const WRITE_LOG_CAP: usize = 1000;

/// One line of the Write window: a system event stamped on the bus
/// timeline. `kind` only drives the color.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WriteKind {
    /// Script `print` output.
    Script,
    /// Everything informational the bus itself reports.
    Info,
    /// Checks and recoverable misbehaviour.
    Warning,
    /// A handler failed, a command failed outright.
    Error,
}

#[derive(Clone, Debug)]
pub struct WriteLine {
    /// Local wall-clock time of the line, as microseconds since local
    /// midnight -- the Write window displays this as `HH:MM:SS.mmm`.
    pub wall_us: u64,
    pub kind: WriteKind,
    pub text: String,
}

/// Local wall-clock microseconds since midnight, for absolute timestamps.
pub fn wall_us_now() -> u64 {
    use chrono::{Local, Timelike};
    let now = Local::now();
    let t = now.time();
    (t.num_seconds_from_midnight() as u64) * 1_000_000 + (t.nanosecond() / 1_000) as u64
}

/// One system variable definition: CANoe-style namespaced value with
/// optional clamping bounds and display metadata. Also the persisted
/// form -- the live value stays core-side, the definition is the config.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct SysVarDef {
    pub namespace: String,
    pub name: String,
    /// Value applied at every measurement start and at definition time.
    pub init: f64,
    /// Inclusive clamp bounds for numeric writes; `None` = unbounded.
    #[serde(default)]
    pub min: Option<f64>,
    #[serde(default)]
    pub max: Option<f64>,
    #[serde(default)]
    pub unit: String,
    #[serde(default)]
    pub comment: String,
}

impl SysVarDef {
    /// The full lookup key scripts and commands use.
    pub fn key(&self) -> String {
        format!("{}::{}", self.namespace, self.name)
    }

    /// Clamps `v` into the declared bounds.
    pub fn clamp(&self, v: f64) -> f64 {
        let v = self.min.map(|lo| v.max(lo)).unwrap_or(v);
        self.max.map(|hi| v.min(hi)).unwrap_or(v)
    }
}

/// One system variable as of the last publish: the definition plus its
/// live value. The manager window's data source.
#[derive(Clone, Debug)]
pub struct SysVarView {
    pub def: SysVarDef,
    pub value: f64,
}

/// One simulation node as of this frame: identity, source, runtime state
/// and the log ring. The Nodes window's data source and the project
/// save; edits go through commands keyed by `id`.
#[derive(Clone, Debug)]
pub struct NodeView {
    pub id: u64,
    pub name: String,
    pub channel: u8,
    pub source: String,
    pub enabled: bool,
    /// Optional file path if the node source was saved to or loaded from a
    /// standalone `.rxcan` file.
    #[allow(dead_code)]
    pub file_path: Option<String>,
    /// 绑定的 DBC 节点 (总线, 节点名)（节点窗口按此聚合脚本）。
    pub attached: Option<(u8, String)>,
    /// Runtime present and error-free (while measuring).
    pub running: bool,
    /// A handler failed: the node is stopped until restart or edit.
    pub errored: bool,
    /// Oldest-first log lines (print output and errors).
    pub log: Vec<String>,
}

/// What the frontend may see of the bus: one immutable, frame-shaped
/// bundle of the read-only facts. Single-threaded it is a plain copy
/// taken once per UI frame; stage 3 publishes it behind an Arc swap
/// instead. Frontend reads go through the snapshot, never the live
/// state, so the same rendering code works across the coming thread
/// boundary.
#[derive(Clone, Debug, Default)]
pub struct Snapshot {
    /// Frames received this run.
    pub frame_counter: u64,
    /// Current trace-ring length.
    pub trace_len: usize,
    /// Live signal subscriptions.
    pub sub_count: usize,
    /// Replay playhead and log length in µs, when a log with a known
    /// position is loaded.
    pub replay: Option<(u64, u64)>,
    /// How many buses exist (frontends key window state off this).
    pub channel_count: usize,
    /// Commands the core has processed; a threaded frontend watches it to
    /// tell whether its sent commands have been applied yet.
    pub laps: u64,
    /// The bus-name counter used to mint "CAN{n}" for new buses; the
    /// frontend reads it for saves and pins it back on project restore.
    pub bus_counter: usize,
    /// The simulation clock, for frontends that anchor plots on bus time.
    pub sim_t_us: u64,
    /// Contiguous log-time span whose frames are already decoded into the
    /// signal caches; a plot asks for scans outside it.
    pub sample_cover: Option<(u64, u64)>,
    /// The violation monitor's latched rows, for the report export.
    pub spec: crate::spec::Spec,
    /// Per-bus load rollups as of the last publish; the Bus Statistics
    /// window's whole data source.
    pub bus_loads: Arc<Vec<crate::load::BusLoad>>,
    /// Simulation nodes as of the last publish: identity, source, state
    /// and log. The Nodes window's data source and the project save.
    pub nodes: Arc<Vec<NodeView>>,
    /// One view per replay block, as of the last publish.
    pub blocks: Vec<ReplayBlockView>,
    /// Derived-signal streams opened by `emit_value` so far: key plus the
    /// owning node's display name.
    pub emitted: Vec<(SigKey, String)>,
    /// The system variables as of this frame: definition plus live value.
    /// The manager window's data source and the project save.
    pub sysvars: Vec<SysVarView>,
    /// The Write window's ring as of the last publish, oldest first.
    pub write: Arc<Vec<WriteLine>>,
    /// Hardware attachments: one per wired-up bus.
    pub hw: Vec<HwBusView>,
    /// The FlexRay RX-only watch, when attached.
    pub fr_watch: Option<FrWatchView>,
    /// Whether the attachments are wire-connected (Real bus) or parked
    /// (Simulated): the toolbar bus-mode switch reads this.
    pub real_bus: bool,
    /// The user's trigger rules, judged on the bus; the frontend saves
    /// them with the project.
    pub triggers: Vec<crate::trigger::Trigger>,
    /// The file the recorder derived its last output from; the replay
    /// start button falls back to it.
    pub last_record: String,
    /// One entry per bus: identity, timing declarations and the shared
    /// (immutable) database. Static configuration as of this frame.
    pub channels: Vec<ChannelView>,
    /// One record per (bus, id) seen this run, behind the Messages /
    /// Statistics views and their exports.
    pub aggs: Vec<MessageAgg>,
    /// One record per FlexRay slot seen this run, slot-sorted -- the
    /// Messages window's FR rows.
    pub fr_aggs: Vec<crate::aggregate::FrSlotAgg>,
    /// One entry per live subscription: the scalar stats the Data window
    /// draws and the sampled history the curves read.
    pub subs: Vec<SubView>,
    /// Bus-event markers (timestamps in µs), newest last, capped oldest-out.
    /// Trigger actions and future bus events drop these; Graphics draws
    /// them as vertical lines.
    pub markers: Vec<u64>,
    /// One entry per generator row: display state plus the bytes that
    /// actually go out at this frame's sim time.
    pub tx: Vec<TxView>,
    /// The trace ring, published once per step that ingested frames and
    /// shared by Arc: cloning the snapshot copies a pointer, not 50k
    /// frames. Read-only for the frontend.
    pub trace: std::sync::Arc<crate::trace::TraceView>,
    /// FlexRay frames as of the last publish, oldest first -- the Trace
    /// window's FR section. Shared like `trace`.
    pub fr_trace: std::sync::Arc<Vec<crate::trace::FrRow>>,
    /// FR rows the ring trimmed from its head since the run began.
    pub fr_dropped: u64,
    /// Which timeline the bus runs on this frame.
    pub mode: Mode,
    /// The bus kind selected for the next start (Simulation / Replay).
    pub run_mode: Mode,
    /// Measurement running / trace views frozen / recorder armed.
    pub measuring: bool,
    pub trace_paused: bool,
    pub recording: bool,
    /// One-shot text from the commands drained since the last publish;
    /// `None` on routine frame publishes. Status is news, not state: the
    /// frontend surfaces it once and the next publish clears it.
    pub status: Option<String>,
}

/// The hand-off between core and frontend: the latest published snapshot
/// behind a mutex. The core overwrites it after every drain/step lap; the
/// frontend `try_lock`s the newest copy out and keeps last frame's copy
/// when the writer is mid-publish -- reading the bus must never wait on
/// the bus.
pub(crate) type SnapshotMailbox = std::sync::Arc<std::sync::Mutex<std::sync::Arc<Snapshot>>>;

pub(crate) fn new_mailbox() -> SnapshotMailbox {
    std::sync::Arc::new(std::sync::Mutex::new(std::sync::Arc::new(
        Snapshot::default(),
    )))
}

// The mailbox plan: commands cross threads, snapshots are shared by Arc.
// Asserting here means a future non-Send/Sync field fails at the struct
// that caused it, not at the `thread::spawn` of the core-thread slice.
const _: () = {
    const fn assert_send<T: Send>() {}
    const fn assert_send_sync<T: Send + Sync>() {}
    assert_send::<BusCommand>();
    assert_send_sync::<Snapshot>();
};

/// The frontend's view of one bus. The database travels as an `Arc` the
/// bus and frontend share; loads are rare, reads are per-frame.
#[derive(Clone)]
pub struct ChannelView {
    pub name: String,
    /// Every attached database path, primary first.
    pub dbc_paths: Vec<String>,
    pub bitrate_kbps: u32,
    pub fd_data_kbps: u32,
    pub node_roles: std::collections::BTreeMap<String, crate::app::NodeRole>,
    pub dbc: Option<std::sync::Arc<crate::dbc::SymbolTable>>,
}

impl ChannelView {
    /// The role declared for `node`; every node without an entry is
    /// `Absent` -- the restbus default. Mirrors `Channel::role_of` so
    /// frontend reads never touch live bus state.
    pub fn role_of(&self, node: &str) -> NodeRole {
        self.node_roles
            .get(node)
            .copied()
            .unwrap_or(NodeRole::Absent)
    }
}

impl std::fmt::Debug for ChannelView {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // The shared table has no Debug of its own; its presence is what
        // matters here.
        f.debug_struct("ChannelView")
            .field("name", &self.name)
            .field("dbc_paths", &self.dbc_paths)
            .field("bitrate_kbps", &self.bitrate_kbps)
            .field("fd_data_kbps", &self.fd_data_kbps)
            .field("node_roles", &self.node_roles)
            .field("dbc_loaded", &self.dbc.is_some())
            .finish()
    }
}

/// One generator entry as the frontend sees it this frame. `muted` folds
/// the replay-silencing decision in -- the frontend renders the chip, the
/// bus owns the policy. The `sent_*` fields are computed by the bus so
/// "what you see is what the wire sees" holds without the frontend
/// touching the databases or the clock.
#[derive(Clone, Debug)]
pub struct TxView {
    pub channel: u8,
    pub id: u32,
    pub name: String,
    /// The DBC transmitter stamped on the entry ("" when unassigned) --
    /// what the generator groups its rows by.
    pub node: String,
    pub active: bool,
    /// 角色闸是否放行（绑定节点的角色为「模拟」，或条目无节点归属）。
    /// 闸关时即便 active 也不发车——UI 用它区分「条目关」与「总关」。
    pub gate_open: bool,
    pub fd: bool,
    pub cycle_us: u64,
    /// Base payload as hex text -- also the no-DBC message's editable box.
    pub data_text: String,
    /// The payload that goes out this frame: base with every driven
    /// source laid over it.
    pub sent_data: [u8; crate::can::frame::MAX_CAN_FD_LEN],
    pub sent_text: String,
    pub srcs: Vec<crate::sim::ValueSrc>,
    pub muted: bool,
}

/// One subscribed signal as the frontend sees it this frame. The
/// run-internal sampler bookkeeping (`sum`/`n`/`last_sample_us`) stays
/// on the bus; this is the display-facing projection.
#[derive(Clone, Debug)]
pub struct SubView {
    pub key: SigKey,
    pub latest: f64,
    pub last_raw: i64,
    pub unit: String,
    pub label: Option<String>,
    pub type_tag: String,
    pub min: f64,
    pub max: f64,
    /// Running average over the samples taken this run (the Data window's
    /// Avg column).
    pub avg: f64,
    /// Sampled history at the frame's stride, for the curve windows --
    /// shared by Arc and rebuilt only when the cache actually changed,
    /// never deep-copied per frame.
    pub history: std::sync::Arc<crate::observe::SampleCache>,
    /// Palette row assigned at subscribe time.
    pub color: usize,
}

/// Every (channel, id) the log file carries -- the twin-silencing set for
/// replay. A plain full read of a temporary stream: parsing is the cost of
/// one open, paid once per replay, never per frame.
fn scan_log_ids(path: &std::path::Path) -> Option<std::collections::HashSet<(u8, u32)>> {
    let mut stream = crate::log::open_stream(path).ok()?;
    let mut ids = std::collections::HashSet::new();
    while let Some(f) = stream.next_frame() {
        ids.insert((f.channel, f.id));
    }
    Some(ids)
}

/// Status-line note about the FD state of a fresh hardware attach: active
/// data-phase bitrate, classic-mode degradation, or nothing for classic buses.
fn hw_fd_note(fd_data_kbps: Option<u32>, fd_active: bool) -> String {
    match (fd_data_kbps, fd_active) {
        (Some(k), true) => format!("，FD 数据段 {k} kbit/s"),
        (Some(_), false) => "，经典模式（FD 预设不匹配或硬件不支持）".to_string(),
        (None, _) => String::new(),
    }
}

/// The simulation half of the application. Fields move here from `App` in
/// slices; during the migration `App` derefs to this, so existing field
/// access keeps compiling. Crate-wide visibility is migration scaffolding --
/// the command/snapshot boundary (stage 2) replaces it.
pub struct BusCore {
    /// Which timeline the bus runs on: Virtual generates against the sim
    /// clock, Replay follows a loaded log's own stamps.
    pub(crate) mode: Mode,
    /// The mode the current run started in; switching buses restores it.
    pub(crate) run_mode: Mode,
    /// The bus's frame input: the virtual idle bus, or the log being
    /// replayed / scanned for backfill.
    pub(crate) source: Box<dyn crate::source::FrameSource>,
    /// Every (channel, id) the loaded log carries. While replaying,
    /// generator entries carrying one of these ids stand down -- replaying
    /// a recording of this very simulation used to interleave two senders
    /// of one signal. Filled by [`App::replay`], consulted only in Replay
    /// mode, never persisted.
    pub(crate) replay_ids: std::collections::HashSet<(u8, u32)>,
    pub(crate) channels: Vec<Channel>,
    /// Counter for naming new buses (CAN3, CAN4, ...).
    pub(crate) bus_counter: usize,
    /// The interactive generator's entries: what this tool transmits as.
    pub(crate) tx_list: Vec<TxMsg>,
    /// Simulation clock: accumulates only while measuring and unpaused.
    /// Generator frames are stamped on it and their signal values are
    /// evaluated from it, so a pause freezes the bus in place instead of
    /// letting it jump phase. Replay polling still uses the wall clock
    /// (`last_tick_us`, still on `App` during the migration).
    pub(crate) sim_t_us: u64,
    /// `now_us()` at the previous accepted tick, the reference for `sim_t_us`.
    pub(crate) sim_prev_us: u64,
    /// Next palette index handed to a new subscription.
    pub(crate) color_counter: usize,
    /// Frames received this run, capped at [`TRACE_LIMIT`] as a ring.
    pub(crate) trace: crate::trace::TraceRing,
    /// The ring as of the last publish, shared with snapshots. Rebuilt
    /// only on steps that ingested frames -- an idle bus copies nothing,
    /// and even then only refcounts plus one sealed tail.
    pub(crate) published_trace: Arc<crate::trace::TraceView>,
    /// FlexRay frames received this run, in their own ring: they never
    /// enter the CAN aggregates or subscriptions, the Trace window is
    /// their only consumer.
    pub(crate) fr_trace: crate::trace::FrRing,
    /// The FR ring as of the last publish, shared with snapshots.
    pub(crate) published_fr: Arc<Vec<crate::trace::FrRow>>,
    /// Per-slot FlexRay tallies behind the Messages window's FR rows.
    pub(crate) fr_aggs: HashMap<u16, crate::aggregate::FrSlotAgg>,
    /// Trace-window freeze: arrivals keep coming but are stamped at the
    /// pause instant instead of their own time, so the view stays still and
    /// resuming does not dump a burst of backdated rows.
    pub(crate) trace_paused: bool,
    /// Wall-clock instant the freeze started; None while not frozen.
    pub(crate) paused_at_us: Option<u64>,
    /// Frames received this run, for the run-total counter in the status bar.
    pub(crate) frame_counter: u64,
    /// Per-(bus, id) aggregates behind the Messages / Statistics views.
    pub(crate) aggs: HashMap<(u8, u32, bool), MessageAgg>,
    /// Per-bus load / frame-rate / error rolling state, one entry per channel.
    pub(crate) bus_loads: Vec<crate::load::BusLoad>,
    /// Send-now requests recorded by commands, built into frames at the
    /// next step. Kept apart from `buf` because `step` clears `buf` before
    /// polling its source.
    pub(crate) injected: Vec<(u8, u32)>,
    /// The load rollups as of the last publish. Rebuilt only when a step
    /// (or a channel add/remove) touched them; the Bus Statistics window
    /// reads this, never the live state.
    pub(crate) published_loads: Arc<Vec<crate::load::BusLoad>>,
    /// True when `bus_loads` changed since `published_loads` was built.
    pub(crate) loads_dirty: bool,
    /// Subscribed signals: latest value, min/max/avg, sampled history.
    pub(crate) subs: HashMap<SigKey, Subscription>,
    /// Bus-event markers, oldest first, capped oldest-out. Dropped by
    /// trigger actions and drawn by Graphics as vertical lines.
    pub(crate) markers: Vec<u64>,
    /// The last [`PRE_BUFFER_FRAMES`] frames, written into the file when a
    /// trigger starts a recording: the context before the event.
    pub(crate) pre_buffer: std::collections::VecDeque<CanFrame>,
    /// While `Some(n)`, a trigger-initiated stop is rolling: the recorder
    /// stays open for `n` more frames, then closes.
    pub(crate) post_roll: Option<u32>,
    /// Contiguous log-time span whose frames have already been decoded into
    /// the signal caches. A Graphics window asking for a range outside it
    /// triggers a backfill scan.
    pub(crate) sample_cover: Option<(u64, u64)>,
    /// The sampling stride currently applied to the signal caches. When the
    /// smallest Graphics window shrinks, the stride gets finer -- and spans
    /// already scanned at the coarse stride must rescan, so `sample_cover`
    /// is invalidated here too.
    pub(crate) applied_stride_us: u64,
    /// True while the measurement runs: the clock advances, generators
    /// emit, replay polls. A pause stops all of it in place.
    pub(crate) measuring: bool,
    /// Frames polled this step, plus frames pushed by Send reactions;
    /// the tick loop walks it by index so late arrivals are processed.
    pub(crate) buf: Vec<CanFrame>,
    /// ASC recording state: the checkbox intent plus the open file.
    pub(crate) recorder: crate::recorder::Recorder,
    /// User rules judged per frame (edges) and per step (aggregate sweeps).
    pub(crate) triggers: Vec<crate::trigger::Trigger>,
    /// Observed-versus-declared violations, recomputed on every step from
    /// `aggs` and the loaded databases.
    pub(crate) spec: crate::spec::Spec,
    /// Commands processed so far; rides the snapshot so a threaded
    /// frontend can tell whether the core has caught up with what it
    /// sent. Never resets -- it is progress, not state.
    pub(crate) laps: u64,
    /// Simulation nodes: scripts that react to bus events and emit
    /// frames. Runtime state lives here in the core; the frontend sees
    /// [`NodeView`]s through the snapshot.
    pub(crate) nodes: Vec<crate::node::ScriptNode>,
    /// Counter minting stable node ids; never resets.
    pub(crate) node_counter: u64,
    /// The node views as of the last publish. Rebuilt only when a command
    /// or a script print changed something.
    pub(crate) published_nodes: Arc<Vec<NodeView>>,
    /// Replay-block views as of the last publish.
    pub(crate) published_blocks: Arc<Vec<ReplayBlockView>>,
    /// True when `nodes` changed since `published_nodes` was built.
    pub(crate) nodes_dirty: bool,
    /// Replay blocks: recorded logs, filtered, streamed back onto the
    /// simulated timeline as independently toggleable traffic drivers.
    pub(crate) replay_blocks: Vec<crate::block::ReplayBlock>,
    /// Counter minting stable replay-block ids; never resets.
    pub(crate) block_counter: u64,
    /// An offline scan already ingested the whole log this run. A second
    /// scan would double every figure; cleared by any run restart.
    pub(crate) scan_done: bool,
    /// Derived-signal streams a node has opened with `emit_value`:
    /// synthetic key plus the owning node's name, for the selection tree.
    pub(crate) emitted_streams: Vec<(SigKey, String)>,
    /// System variables: namespaced, typed values defined in the manager,
    /// read/written by scripts (`sys_get` / `sys_set`), and published to
    /// the observers as one synthetic stream per variable. The live value
    /// rides next to the definition; resets to `init` at every run start.
    pub(crate) sysvars: Vec<(SysVarDef, f64)>,
    /// The Write window's line ring, oldest first: node prints, checks,
    /// warnings and bus events, in one system-wide stream.
    pub(crate) write_log: VecDeque<WriteLine>,
    /// The write ring as of the last publish.
    pub(crate) published_write: Arc<Vec<WriteLine>>,
    /// True when `write_log` changed since `published_write` was built.
    pub(crate) write_dirty: bool,
    /// Hardware attachments (Kvaser today): bus → adapter, plus the
    /// per-node wire-egress switches. Session state, per the hardware
    /// mapping overlay model.
    pub(crate) hw: crate::hw::Hardware,
    /// How many frames the trace ring retains. Default
    /// [`TRACE_LIMIT`]; user-settable (project-persisted) because the
    /// right capacity depends on bus load and how long a capture runs.
    pub(crate) trace_limit: usize,
    /// Trigger-recording context: pre-trigger frames kept for
    /// trigger-started recordings, post-roll frames after a
    /// trigger-stopped edge, and the marker list cap. Defaults
    /// [`PRE_BUFFER_FRAMES`] / [`POST_ROLL_FRAMES`] / [`MARKER_CAP`];
    /// all three are project-persisted settings now.
    pub(crate) pre_frames: usize,
    pub(crate) post_frames: u32,
    pub(crate) marker_cap: usize,
}

impl BusCore {
    /// Empty bus state; `App::new` layers the sample channel configuration
    /// and the measurement start on top.
    pub(crate) fn new(bus_loads: Vec<crate::load::BusLoad>) -> Self {
        let published_loads = Arc::new(bus_loads.clone());
        BusCore {
            mode: Mode::Virtual,
            run_mode: Mode::Virtual,
            source: Box::new(crate::source::virtual_source::VirtualSource::new()),
            replay_ids: std::collections::HashSet::new(),
            channels: Vec::new(),
            bus_counter: 2,
            tx_list: Vec::new(),
            sim_t_us: 0,
            sim_prev_us: 0,
            color_counter: 0,
            trace: crate::trace::TraceRing::default(),
            published_trace: Arc::new(crate::trace::TraceView::default()),
            fr_trace: crate::trace::FrRing::default(),
            published_fr: Arc::new(Vec::new()),
            fr_aggs: HashMap::new(),
            trace_paused: false,
            paused_at_us: None,
            frame_counter: 0,
            aggs: HashMap::new(),
            bus_loads,
            injected: Vec::new(),
            published_loads,
            loads_dirty: false,
            subs: HashMap::new(),
            markers: Vec::new(),
            pre_buffer: std::collections::VecDeque::new(),
            post_roll: None,
            sample_cover: None,
            applied_stride_us: SAMPLE_INTERVAL_US,
            measuring: false,
            buf: Vec::new(),
            recorder: crate::recorder::Recorder::new(),
            triggers: Vec::new(),
            spec: crate::spec::Spec::default(),
            laps: 0,
            nodes: Vec::new(),
            node_counter: 0,
            published_nodes: Arc::new(Vec::new()),
            published_blocks: Arc::new(Vec::new()),
            nodes_dirty: false,
            replay_blocks: Vec::new(),
            block_counter: 0,
            scan_done: false,
            emitted_streams: Vec::new(),
            sysvars: Vec::new(),
            write_log: VecDeque::new(),
            published_write: Arc::new(Vec::new()),
            write_dirty: false,
            hw: crate::hw::Hardware::new(),
            trace_limit: TRACE_LIMIT,
            pre_frames: PRE_BUFFER_FRAMES,
            post_frames: POST_ROLL_FRAMES,
            marker_cap: MARKER_CAP,
        }
    }

    /// Executes a command from the frontend. Single-threaded this is a
    /// plain call (`App::send`); stage 3 sends the same enum over a
    /// channel instead. Status text an action produces goes into `status`
    /// -- the bus has no display of its own. Every command bumps the lap
    /// counter so a caller can see progress through the snapshot.
    pub(crate) fn handle(&mut self, cmd: BusCommand, status: &mut String) {
        self.laps += 1;
        match cmd {
            BusCommand::StartVirtual => self.start_virtual(status),
            BusCommand::Stop => self.stop_bus(status),
            BusCommand::SetTracePaused(on) => self.trace_paused = on,
            BusCommand::ToggleRecord => self.toggle_record(status),
            BusCommand::RearmTriggers => self.rearm_latched_triggers(status),
            BusCommand::ClearTrace => self.clear_trace_view(status),
            BusCommand::ClearAggregates => self.clear_aggregates(status),
            BusCommand::SetRunMode(mode) => self.run_mode = mode,
            BusCommand::AddChannel => self.add_channel(status),
            BusCommand::RemoveChannel { ch } => self.remove_channel(ch, status),
            BusCommand::SetNodeRole { ch, node, role } => {
                self.set_node_role(ch, &node, role, status)
            }
            BusCommand::AddNode {
                name,
                channel,
                attached,
            } => {
                self.node_counter += 1;
                let id = self.node_counter;
                self.nodes
                    .push(crate::node::ScriptNode::new(id, name, channel));
                let last = self.nodes.last_mut().expect("just added");
                last.attached = attached.clone();
                // A bound script lives on its node's bus: the binding, not
                // the passed-in channel, decides the wire (and the gate).
                if let Some((ach, _)) = attached {
                    last.channel = ach;
                }
                if self.measuring {
                    let dbc = self
                        .channels
                        .get(channel as usize)
                        .and_then(|c| c.dbc.clone());
                    let keys = self.sysvar_keys();
                    let last = self.nodes.last_mut().expect("just added");
                    last.sysvar_keys = keys;
                    last.start(dbc);
                }
                self.nodes_dirty = true;
            }
            BusCommand::RemoveNode { id } => {
                self.nodes.retain(|n| n.id != id);
                // The removed node's derived-signal streams have no future
                // emitter; drop them from the selection tree. Existing
                // window selections keep working like after a DBC swap.
                self.emitted_streams
                    .retain(|(k, _)| k.1 & !crate::app::EMITTED_ID_BASE != id as u32);
                self.nodes_dirty = true;
            }
            BusCommand::SetNodeName { id, name } => {
                if let Some(n) = self.nodes.iter_mut().find(|n| n.id == id) {
                    n.name = name.clone();
                    // The selection tree lists streams under the owner's
                    // name: a rename rewrites the owner, not the key.
                    for (k, owner) in &mut self.emitted_streams {
                        if k.1 & !crate::app::EMITTED_ID_BASE == id as u32 {
                            *owner = name.clone();
                        }
                    }
                    self.nodes_dirty = true;
                }
            }
            BusCommand::SetNodeSource { id, source } => {
                let keys = self.sysvar_keys();
                if let Some(n) = self.nodes.iter_mut().find(|n| n.id == id) {
                    let dbc = self
                        .channels
                        .get(n.channel as usize)
                        .and_then(|c| c.dbc.clone());
                    n.sysvar_keys = keys;
                    n.set_source(source, dbc, self.measuring);
                    self.nodes_dirty = true;
                }
            }
            BusCommand::SetNodeEnabled { id, on } => {
                let keys = self.sysvar_keys();
                if let Some(n) = self.nodes.iter_mut().find(|n| n.id == id) {
                    let dbc = self
                        .channels
                        .get(n.channel as usize)
                        .and_then(|c| c.dbc.clone());
                    n.sysvar_keys = keys;
                    n.set_enabled(on, dbc, self.measuring);
                    self.nodes_dirty = true;
                }
            }
            BusCommand::SetNodes { nodes } => {
                let keys = self.sysvar_keys();
                self.nodes = nodes
                    .into_iter()
                    .map(|cfg| {
                        self.node_counter += 1;
                        let mut n =
                            crate::node::ScriptNode::new(self.node_counter, cfg.name, cfg.channel);
                        n.source = cfg.source;
                        n.enabled = cfg.enabled;
                        // The saved binding is the whole point: without it
                        // every restored script would look like an orphan
                        // and get adopted by an arbitrary node.
                        n.attached = cfg.attached.clone();
                        if let Some((ach, _)) = &n.attached {
                            n.channel = *ach;
                        }
                        if self.measuring && n.enabled {
                            let dbc = self
                                .channels
                                .get(n.channel as usize)
                                .and_then(|c| c.dbc.clone());
                            n.sysvar_keys = keys.clone();
                            n.start(dbc);
                        }
                        n
                    })
                    .collect();
                // Only genuinely orphan scripts (legacy saves with no
                // binding) are adopted; bound ones keep their node.
                self.adopt_orphan_scripts_all();
                // Wholesale replacement mints new node ids: every existing
                // derived-signal stream loses its owner.
                self.emitted_streams.clear();
                // Sysvar streams are not node-owned; recreate them right
                // away so observer selections survive the replacement.
                let live: Vec<(String, String, f64)> = self
                    .sysvars
                    .iter()
                    .map(|(d, v)| (d.namespace.clone(), d.name.clone(), *v))
                    .collect();
                for (ns, name, v) in live {
                    self.ingest_sysvar(&ns, &name, v);
                }
                self.nodes_dirty = true;
            }
            BusCommand::DefineSysVar(def) => {
                let key = def.key();
                match self.sysvars.iter().position(|(d, _)| d.key() == key) {
                    Some(i) => {
                        let v = def.init;
                        self.sysvars[i] = (def, v);
                    }
                    None => {
                        let v = def.init;
                        self.sysvars.push((def, v));
                    }
                }
                self.refresh_node_sysvar_keys();
                if let Some((d, v)) = self.sysvars.iter().find(|(d, _)| d.key() == key) {
                    let (ns, name, val) = (d.namespace.clone(), d.name.clone(), *v);
                    self.ingest_sysvar(&ns, &name, val);
                }
                self.nodes_dirty = true;
            }
            BusCommand::DeleteSysVar { namespace, name } => {
                let before = self.sysvars.len();
                self.sysvars
                    .retain(|(d, _)| d.namespace != namespace || d.name != name);
                if self.sysvars.len() != before {
                    self.refresh_node_sysvar_keys();
                    // The dead variable's stream has no future writer.
                    let dead = crate::app::EMITTED_ID_BASE | (SYSVAR_STREAM_ID as u32);
                    self.emitted_streams
                        .retain(|(k, owner)| !(k.1 == dead && k.3 == name && *owner == namespace));
                    self.nodes_dirty = true;
                }
            }
            BusCommand::SetSysVar {
                namespace,
                name,
                value,
            } => {
                let key = format!("{namespace}::{name}");
                if !self.sysvar_write(&key, value) {
                    *status = format!("[sysvar] 未定义: \"{key}\"");
                }
            }
            BusCommand::ClearWrite => {
                self.write_log.clear();
                self.write_dirty = true;
            }
            BusCommand::SetBusMode { real } => {
                self.hw.live = real;
                *status = if real {
                    "总线模式：Real bus——挂接的硬件已上线（RX 进总线，定向 TX 出线）".into()
                } else {
                    "总线模式：Simulated——硬件挂接保留但已下线（纯仿真）".into()
                };
            }
            BusCommand::AddReplayBlock {
                name,
                channel,
                path,
                node_filter,
                attached,
                ids,
            } => self.add_replay_block(name, channel, path, node_filter, attached, ids, status),
            BusCommand::SetReplayBlock {
                id,
                name,
                channel,
                path,
                node_filter,
                attached,
                ids,
            } => self.set_replay_block(id, name, channel, path, node_filter, attached, ids, status),
            BusCommand::RemoveReplayBlock { id } => {
                self.replay_blocks.retain(|b| b.id != id);
                self.nodes_dirty = true;
            }
            BusCommand::SetReplayBlockEnabled { id, on } => {
                self.set_replay_block_enabled(id, on, status)
            }
            BusCommand::SetReplayBlocks { blocks } => {
                self.replay_blocks = blocks
                    .into_iter()
                    .map(|cfg| {
                        self.block_counter += 1;
                        let mut b = crate::block::ReplayBlock::new(
                            self.block_counter,
                            cfg.name,
                            cfg.channel,
                            cfg.path,
                            cfg.node_filter,
                            cfg.attached,
                            cfg.ids,
                            cfg.enabled,
                        );
                        if b.enabled {
                            let dbc = self
                                .channels
                                .get(b.channel as usize)
                                .and_then(|c| c.dbc.clone());
                            b.load_queue(dbc.as_deref(), self.sim_t_us);
                        }
                        b
                    })
                    .collect();
                self.nodes_dirty = true;
            }
            BusCommand::ScanLog {
                path,
                tol_pct,
                grace,
            } => self.scan_log(&path, tol_pct, grace, status),
            BusCommand::SetReplaySpeed(speed) => self.source.set_speed(speed),
            BusCommand::SeekReplay(t_s) => self.seek_replay(t_s, status),
            BusCommand::SetEntryActive { ch, id, on } => {
                // Activating anchors the schedule at the current clock:
                // `next_t_us` still sits at the last slot before the entry
                // was switched off, and letting the catch-up loop run from
                // there would re-emit frames dated across the whole off
                // period. A re-enabled entry starts sending from now.
                // Deactivating touches only the flag, so payload, waveforms
                // and schedule survive the pause.
                let sim = self.sim_t_us;
                if let Some(tx) = self.entry_mut(ch, id) {
                    tx.active = on;
                    if on {
                        tx.next_t_us = sim;
                    }
                }
            }
            BusCommand::SetEntryFd { ch, id, fd } => {
                if let Some(tx) = self.entry_mut(ch, id) {
                    tx.flags = if fd {
                        crate::can::frame::FrameFlags::FD
                    } else {
                        crate::can::frame::FrameFlags::NONE
                    };
                }
            }
            BusCommand::SetEntryCycle { ch, id, cycle_us } => {
                if let Some(tx) = self.entry_mut(ch, id) {
                    tx.cycle_us = cycle_us;
                    // Same convention as every other way of switching a
                    // message on: the new schedule starts now rather than
                    // at the end of the old one.
                    tx.next_t_us = 0;
                }
            }
            BusCommand::SetEntryHex { ch, id, text } => {
                self.set_entry_hex(ch, id, &text);
            }
            BusCommand::RemoveEntry { ch, id } => {
                self.tx_list.retain(|t| !(t.channel == ch && t.id == id));
            }
            BusCommand::AddEntry { ch, id } => self.add_entry(ch, id),
            BusCommand::SetEntrySource { ch, id, src } => {
                if let Some(tx) = self.entry_mut(ch, id) {
                    match tx.srcs.iter_mut().find(|s| s.name == src.name) {
                        Some(held) => *held = src,
                        None => tx.srcs.push(src),
                    }
                }
            }
            BusCommand::ClearEntrySource { ch, id, name } => {
                if let Some(tx) = self.entry_mut(ch, id) {
                    tx.srcs.retain(|s| s.name != name);
                }
            }
            BusCommand::PinEntrySignal { ch, id, name, phys } => {
                self.pin_entry_signal(ch, id, &name, phys);
            }
            BusCommand::Subscribe { key } => self.subscribe_signal(key),
            BusCommand::Unsubscribe { key } => {
                self.subs.remove(&key);
            }
            BusCommand::StartReplay { path, speed } => self.start_replay(&path, speed, status),
            BusCommand::ResumeReplay { speed } => {
                // The poll clock uses `saturating_sub`, so rewinding
                // `sim_prev_us` to zero is harmless; the frontend restarts
                // its own wall clock (`t0`) alongside this command.
                self.sim_prev_us = 0;
                self.trace_paused = false;
                self.paused_at_us = None;
                self.measuring = true;
                *status = format!("resumed at {speed}x");
            }
            BusCommand::SetChannelConfig {
                ch,
                name,
                dbc_path,
                bitrate_kbps,
                fd_data_kbps,
                node_roles,
            } => {
                self.set_channel_config(ch, name, dbc_path, bitrate_kbps, fd_data_kbps, node_roles)
            }
            BusCommand::LoadDbc { ch, paths } => self.load_dbc(ch, paths, status),
            BusCommand::SetBusCounter(n) => self.bus_counter = n,
            BusCommand::SetRecordPath(path) => self.recorder.record_path = path,
            BusCommand::SetRecordFilter { ids } => self.recorder.ids = ids,
            BusCommand::SetTraceLimit { frames } => {
                self.trace_limit = frames.clamp(1_000, 5_000_000)
            }
            BusCommand::SetHardwareChannel {
                bus,
                driver,
                adapter,
                kbps,
                fd_data_kbps,
            } => {
                // Prefer init access (the wire-egress switches can then
                // direct traffic onto the wire). When another program
                // holds the channel, fall back to receive-only. Vector
                // first cut is classic CAN (fd_data_kbps is accepted but
                // not applied; the attach status reports fd=false).
                let opened = match driver {
                    crate::hw::HwDriver::Kvaser => {
                        crate::hw::kvaser::KvaserChannel::open(adapter, kbps, fd_data_kbps, true)
                            .map(|p| {
                                let fd = p.fd;
                                (crate::hw::HwPort::Kvaser(p), fd)
                            })
                            .map_err(|e| (e, {
                                crate::hw::kvaser::KvaserChannel::open(
                                    adapter,
                                    kbps,
                                    fd_data_kbps,
                                    false,
                                )
                                .map(|p| {
                                    let fd = p.fd;
                                    (crate::hw::HwPort::Kvaser(p), fd)
                                })
                            }))
                    }
                    crate::hw::HwDriver::Vector => {
                        // Vector classic CAN first cut: init access only
                        // (rx-only open is provided the same way).
                        crate::hw::vector::VectorChannel::open(adapter, kbps, fd_data_kbps, true)
                            .map(|p| {
                                let fd = p.fd;
                                (crate::hw::HwPort::Vector(p), fd)
                            })
                            .map_err(|e| (e, {
                                crate::hw::vector::VectorChannel::open(
                                    adapter,
                                    kbps,
                                    fd_data_kbps,
                                    false,
                                )
                                .map(|p| {
                                    let fd = p.fd;
                                    (crate::hw::HwPort::Vector(p), fd)
                                })
                            }))
                    }
                };
                match opened {
                    Ok((port, fd)) => {
                        let driver_name = match driver {
                            crate::hw::HwDriver::Kvaser => "Kvaser",
                            crate::hw::HwDriver::Vector => "Vector",
                        };
                        self.hw
                            .attach(bus, driver, adapter, kbps, true, port);
                        *status = format!(
                            "hardware attached to {bus}: {driver_name} ch{adapter} @ {kbps} kbit/s (收发){}",
                            hw_fd_note(fd_data_kbps, fd)
                        );
                    }
                    Err((init_err, retry)) => match retry {
                        Ok((port, fd)) => {
                            self.hw
                                .attach(bus, driver, adapter, kbps, false, port);
                            *status = format!(
                                "hardware attached to {bus}: ch{adapter} @ {kbps} kbit/s（只收——通道被其他程序占用）{}",
                                hw_fd_note(fd_data_kbps, fd)
                            );
                        }
                        Err(e) => {
                            *status = format!("hardware attach failed: {init_err} / {e}")
                        }
                    },
                }
            }
            BusCommand::DetachHardware { bus } => {
                self.hw.detach(bus);
                *status = format!("hardware detached from bus {}", bus + 1);
            }
            BusCommand::SetFrWatch {
                channel_index,
                fibex_path,
            } => match channel_index {
                Some(idx) => match self.hw.attach_fr(idx, &fibex_path) {
                    Ok(()) => {
                        *status = format!("FlexRay 监听已挂接: Vector ch{idx}（只收）");
                    }
                    Err(e) => {
                        *status = format!("FlexRay 监听挂接失败: {e}");
                    }
                },
                None => {
                    self.hw.detach_fr();
                    *status = "FlexRay 监听已断开".to_string();
                }
            },
            BusCommand::SetRunLimits {
                pre_frames,
                post_frames,
                marker_cap,
            } => {
                if let Some(n) = pre_frames {
                    self.pre_frames = n.clamp(0, 32_768);
                }
                if let Some(n) = post_frames {
                    self.post_frames = n.clamp(0, 32_768);
                }
                if let Some(n) = marker_cap {
                    self.marker_cap = n.clamp(8, 65_536);
                }
            }
            BusCommand::SetEntryConfig {
                ch,
                id,
                active,
                cycle_us,
                fd,
                flags,
                data_text,
                srcs,
            } => self.set_entry_config(ch, id, active, cycle_us, fd, flags, data_text, srcs),
            BusCommand::Backfill {
                from_us,
                to_us,
                stride_us,
            } => self.backfill(from_us, to_us, stride_us, status),
            BusCommand::ClearDatabases => {
                for c in &mut self.channels {
                    c.dbc_paths.clear();
                    c.dbc = None;
                }
                self.tx_list.clear();
                self.subs.clear();
            }
            BusCommand::ClearSpec => self.spec.clear(),
            BusCommand::SendNow { ch, id } => {
                // Only a running bus delivers: a stopped one would hold the
                // request until some future run, which is never what the
                // button means.
                if self.measuring {
                    self.injected.push((ch, id));
                }
            }
            BusCommand::SetTriggers(triggers) => self.triggers = triggers,
            BusCommand::AddTrigger { cond, action } => {
                self.triggers
                    .push(crate::trigger::Trigger::new(cond, action));
            }
            BusCommand::RemoveTrigger { index } => {
                if index < self.triggers.len() {
                    self.triggers.remove(index);
                }
            }
            BusCommand::SetTriggerEnabled { index, on } => {
                if let Some(t) = self.triggers.get_mut(index) {
                    t.enabled = on;
                }
            }
            BusCommand::EditTrigger {
                index,
                cond,
                action,
            } => {
                if let Some(t) = self.triggers.get_mut(index) {
                    t.cond = cond;
                    t.action = action;
                    t.level = false;
                }
            }
        }
    }

    /// Republishes changed signal caches for the next snapshot; pointer
    /// clones otherwise. One call per step (and after backfills and
    /// resets), never per frame.
    fn refresh_sub_histories(&mut self) {
        for sub in self.subs.values_mut() {
            sub.refresh_published_history();
        }
    }

    /// Republishes the shared ring view. Called on steps that ingested
    /// frames (and on resets); the copy is one sealed tail plus chunk
    /// refcounts -- the ring itself is shared, never deep-copied.
    fn publish_trace(&mut self) {
        self.published_trace = self.trace.publish();
        self.published_fr = self.fr_trace.publish();
    }

    /// One FlexRay frame lands: stamped against the sim clock when the
    /// source had no time of its own, then into the FR ring and the
    /// per-slot aggregates. Kept apart from [`BusCore::ingest`] -- FR
    /// rows deliberately skip the CAN aggregates, load rollups and
    /// subscriptions.
    pub(crate) fn ingest_fr_row(&mut self, mut row: crate::trace::FrRow) {
        if row.t_us == 0 {
            row.t_us = self.sim_t_us;
        }
        let agg = self.fr_aggs.entry(row.slot).or_default();
        // Only a strictly later timestamp marks a real cycle (seek /
        // out-of-order rows), and the smoothing mirrors the CAN path.
        if agg.count > 0 && row.t_us > agg.last_t_us {
            let dt = (row.t_us - agg.last_t_us) as f64;
            let prev = agg.cycle_us;
            agg.cycle_us = if agg.count == 1 {
                dt
            } else {
                agg.cycle_us * 0.9 + dt * 0.1
            };
            let dev = (dt - prev).abs();
            agg.jitter_us = if agg.count == 1 {
                dev
            } else {
                agg.jitter_us * 0.9 + dev * 0.1
            };
        }
        agg.slot = row.slot;
        agg.count += 1;
        agg.last_t_us = row.t_us;
        agg.ab = row.ab;
        agg.last_cycle = row.cycle;
        agg.payload = row.payload.clone();
        self.fr_trace.push(row, self.trace_limit);
    }

    /// Republishes the load rollups after a step (or a channel add/remove)
    /// touched them. The rollup state itself is a deep copy -- bounded by
    /// one window of frames per bus -- so it only re-clones when changed.
    pub(crate) fn publish_loads(&mut self) {
        if self.loads_dirty {
            self.published_loads = Arc::new(self.bus_loads.clone());
            self.loads_dirty = false;
        }
    }

    /// Republishes the node views after a command or a script print
    /// changed something. Alongside the script nodes, one role card per
    /// DBC node per bus (not only transmitters) lands in `group_cards` —
    /// every declared node is visible, which is what lets the user assign
    /// a role to a node that sends nothing yet.
    pub(crate) fn publish_nodes(&mut self) {
        if !self.nodes_dirty {
            return;
        }
        let views: Vec<NodeView> = self
            .nodes
            .iter()
            .map(|n| NodeView {
                id: n.id,
                name: n.name.clone(),
                channel: n.channel,
                source: n.source.clone(),
                enabled: n.enabled,
                file_path: n.file_path.clone(),
                attached: n.attached.clone(),
                running: n.running(),
                errored: n.errored(),
                log: n.log_snapshot(),
            })
            .collect();
        self.published_nodes = Arc::new(views);
        self.published_blocks = Arc::new(
            self.replay_blocks
                .iter()
                .map(|b| ReplayBlockView {
                    id: b.id,
                    name: b.name.clone(),
                    channel: b.channel,
                    path: b.path.clone(),
                    node_filter: b.node_filter.clone(),
                    attached: b.attached.clone(),
                    ids: b.ids.clone(),
                    enabled: b.enabled,
                    frames: b.queue_len(),
                    last_error: b.last_error.clone(),
                })
                .collect(),
        );
        self.nodes_dirty = false;
    }

    /// Appends one line to the Write ring, stamped with the bus clock and
    /// the local wall clock (the Write window displays the wall time).
    pub(crate) fn write_push(&mut self, kind: WriteKind, text: String) {
        let wall_us = wall_us_now();
        self.write_log.push_back(WriteLine { wall_us, kind, text });
        if self.write_log.len() > WRITE_LOG_CAP {
            self.write_log.pop_front();
        }
        self.write_dirty = true;
    }

    /// Classifies a node log line and mirrors it into the Write ring
    /// under the node's name.
    fn write_node_line(&mut self, node_name: &str, line: &str) {
        let kind = if line.starts_with("[error]") {
            WriteKind::Error
        } else if line.starts_with("[check]")
            || line.starts_with("[timer]")
            || line.starts_with("[sysvar]")
            || line.starts_with("[compile]")
            || line.starts_with("[start]")
        {
            WriteKind::Warning
        } else {
            WriteKind::Script
        };
        self.write_push(kind, format!("[{node_name}] {line}"));
    }

    /// Rebuilds the published Write ring after a change.
    pub(crate) fn publish_write(&mut self) {
        if !self.write_dirty {
            return;
        }
        self.published_write = Arc::new(self.write_log.iter().cloned().collect());
        self.write_dirty = false;
    }

    /// Arms every enabled node for a measurement: recompile from source,
    /// run the main chunk, fire `on start`.
    fn nodes_start(&mut self) {        let keys = self.sysvar_keys();
        for node in &mut self.nodes {
            if node.enabled {
                let dbc = self
                    .channels
                    .get(node.channel as usize)
                    .and_then(|c| c.dbc.clone());
                node.sysvar_keys = keys.clone();
                node.start(dbc);
            }
        }
        self.nodes_dirty = true;
    }

    /// The defined system variable keys ("ns::name").
    fn sysvar_keys(&self) -> Vec<String> {
        self.sysvars.iter().map(|(d, _)| d.key()).collect()
    }

    /// Pushes the current key set to every node: the start check reports
    /// a script's references outside this set.
    fn refresh_node_sysvar_keys(&mut self) {
        let keys = self.sysvar_keys();
        for n in &mut self.nodes {
            n.sysvar_keys = keys.clone();
        }
    }

    /// Node timer handlers that are due at wall clock `now_us`; their
    /// queued frames join this step's buffer. `inputs` carries what
    /// `now()`/`sig()` read, per channel.
    fn run_node_timers(&mut self, now_us: u64, inputs: &HashMap<u8, HostInput>) {
        let mut derived: Vec<(u8, u64, String, String, f64)> = Vec::new();
        let mut sys_writes: Vec<(u64, String, f64)> = Vec::new();
        let mut node_lines: Vec<(String, String)> = Vec::new();
        for node in &mut self.nodes {
            let input = inputs.get(&node.channel).cloned().unwrap_or_default();
            // 绑定脚本的发帧受所属 DBC 节点的角色闸：节点离线时脚本同样
            // 不发车（模拟 = 闸门放行）。
            let script_allowed = node.attached.as_ref().is_none_or(|(ach, anode)| {
                self.channels
                    .get(*ach as usize)
                    .is_some_and(|c| c.role_of(anode) == NodeRole::Simulated)
            });
            for (id, ext, data) in node.run_timers(now_us, &input) {
                let frame = Self::node_frame(node.channel, id, ext, &data, self.sim_t_us);
                // Real bus 模式下脚本帧与生成器帧一样上挂接通道。
                if script_allowed {
                    self.hw.write_if_live(node.channel, &frame);
                    self.buf.push(frame);
                }
            }
            for (name, v) in node.take_emitted() {
                derived.push((node.channel, node.id, node.name.clone(), name, v));
            }
            for (key, v) in node.take_sys_sets() {
                sys_writes.push((node.id, key, v));
            }
            let node_name = node.name.clone();
            for line in node.take_new_lines() {
                node_lines.push((node_name.clone(), line));
            }
            if node.take_log_if_dirty().is_some() {
                self.nodes_dirty = true;
            }
        }
        for (ch, node_id, node_name, name, v) in derived {
            self.ingest_emitted(ch, node_id, &node_name, &name, v);
        }
        self.apply_sys_writes(sys_writes);
        for (node_name, line) in node_lines {
            self.write_node_line(&node_name, &line);
        }
    }

    /// Delivers one frame to the matching node handlers; the frames they
    /// queue are returned for the step loop to append to the buffer.
    /// Decodes the current frame's DBC signals and merges them into the
    /// host input so `sig()` in node handlers reads fresh values.
    fn dispatch_node_frame(
        &mut self,
        f: &CanFrame,
        input: &HostInput,
    ) -> Vec<(u32, bool, Vec<u8>)> {
        let mut out = Vec::new();
        let mut derived: Vec<(u8, u64, String, String, f64)> = Vec::new();
        let mut sys_writes: Vec<(u64, String, f64)> = Vec::new();
        let mut node_lines: Vec<(String, String)> = Vec::new();
        let data = &f.data[..f.len as usize];
        for node in &mut self.nodes {
            // 绑定脚本的发帧受所属 DBC 节点的角色闸（见 run_node_timers）。
            let script_allowed = node.attached.as_ref().is_none_or(|(ach, anode)| {
                self.channels
                    .get(*ach as usize)
                    .is_some_and(|c| c.role_of(anode) == NodeRole::Simulated)
            });
            let node_out =
                node.dispatch_frame(f.channel, f.id, f.extended, f.is_error(), data, input);
            // The node's own wire egress: Real bus 模式下反应帧与生成器帧
            // 一样上挂接通道。
            if script_allowed {
                for (id, ext, data) in &node_out {
                    let frame = Self::node_frame(node.channel, *id, *ext, data, f.t_us);
                    self.hw.write_if_live(node.channel, &frame);
                }
            }
            out.extend(node_out);
            for (name, v) in node.take_emitted() {
                derived.push((node.channel, node.id, node.name.clone(), name, v));
            }
            for (key, v) in node.take_sys_sets() {
                sys_writes.push((node.id, key, v));
            }
            let node_name = node.name.clone();
            for line in node.take_new_lines() {
                node_lines.push((node_name.clone(), line));
            }
            if node.take_log_if_dirty().is_some() {
                self.nodes_dirty = true;
            }
        }
        for (ch, node_id, node_name, name, v) in derived {
            self.ingest_emitted(ch, node_id, &node_name, &name, v);
        }
        self.apply_sys_writes(sys_writes);
        for (node_name, line) in node_lines {
            self.write_node_line(&node_name, &line);
        }
        out
    }

    /// Applies the system variable writes a handler run queued: clamp
    /// against the definition, publish to the observers, and report an
    /// undefined key to the writing node's log.
    fn apply_sys_writes(&mut self, writes: Vec<(u64, String, f64)>) {
        for (node_id, key, v) in writes {
            if !self.sysvar_write(&key, v)
                && let Some(n) = self.nodes.iter_mut().find(|n| n.id == node_id)
            {
                n.note(format!("[sysvar] 未定义，写入被丢弃: \"{key}\""));
            }
        }
    }

    /// Stores one system variable write, clamped to its bounds, and
    /// publishes the sample to the observers under the sysvar stream id.
    /// Returns false when no such variable is defined.
    fn sysvar_write(&mut self, key: &str, value: f64) -> bool {
        let Some(i) = self.sysvars.iter().position(|(d, _)| d.key() == key) else {
            return false;
        };
        let (def, v) = &mut self.sysvars[i];
        *v = def.clamp(value);
        let (ns, name, val) = (def.namespace.clone(), def.name.clone(), *v);
        self.ingest_sysvar(&ns, &name, val);
        true
    }

    /// Publishes one system variable value to the observers. Each
    /// variable is a synthetic stream grouped under its namespace, the
    /// same tree Data and Graphics browse.
    fn ingest_sysvar(&mut self, namespace: &str, name: &str, v: f64) {
        self.ingest_emitted(0, SYSVAR_STREAM_ID, namespace, name, v);
    }

    /// Run start: every system variable returns to its declared init
    /// value and republishes, so observers never plot a stale tail.
    fn init_sysvars(&mut self) {
        let defs: Vec<(String, String, f64)> = self
            .sysvars
            .iter()
            .map(|(d, _)| (d.namespace.clone(), d.name.clone(), d.init))
            .collect();
        for (d, v) in &mut self.sysvars {
            *v = d.init;
        }
        for (ns, name, v) in defs {
            self.ingest_sysvar(&ns, &name, v);
        }
    }

    /// Folds one `emit_value` sample into its synthetic subscription,
    /// creating the stream on first emission. The key carries
    /// [`crate::app::EMITTED_ID_BASE`] in its id, so a derived signal can
    /// never collide with a real frame's signals; the selection tree lists
    /// these streams under the owning node's name.
    fn ingest_emitted(&mut self, ch: u8, node_id: u64, node_name: &str, name: &str, v: f64) {
        let key: SigKey = (
            ch,
            crate::app::EMITTED_ID_BASE | node_id as u32,
            false,
            name.to_string(),
        );
        if !self.emitted_streams.iter().any(|(k, _)| k == &key) {
            // A script computing names in a loop would otherwise balloon
            // the registry, the selection tree, and the subscription map
            // with one-shot streams. The cap is generous for real derived
            // signals and keeps a runaway script cheap.
            if self.emitted_streams.len() >= MAX_EMITTED_STREAMS {
                return;
            }
            self.emitted_streams
                .push((key.clone(), node_name.to_string()));
            self.nodes_dirty = true;
        }
        self.subscribe_signal(key.clone());
        let stride = self.applied_stride_us;
        let t_us = self.sim_t_us;
        let Some(entry) = self.subs.get_mut(&key) else {
            return;
        };
        entry.latest = v;
        entry.last_raw = v as i64;
        entry.last_update_us = t_us;
        if t_us >= entry.last_sample_us + stride || entry.history.is_empty() {
            entry.push_sample(t_us, v, stride);
        }
    }

    /// Builds one [`HostInput`] per channel that has nodes: the bus clock
    /// in seconds plus every decoded signal value seen on that channel
    /// (from the aggregates, so it covers replay and live alike).
    fn build_node_inputs(&self, now_us: u64) -> HashMap<u8, HostInput> {
        let mut inputs: HashMap<u8, HostInput> = HashMap::new();
        for node in &self.nodes {
            inputs.entry(node.channel).or_insert_with(|| HostInput {
                now_s: now_us as f64 / 1e6,
                signals: HashMap::new(),
                sysvars: HashMap::new(),
            });
        }
        // System variables are global: every channel's nodes read the
        // same live set.
        let mut sysmap: HashMap<String, f64> = HashMap::with_capacity(self.sysvars.len());
        for (d, v) in &self.sysvars {
            sysmap.insert(d.key(), *v);
        }
        for input in inputs.values_mut() {
            input.sysvars = sysmap.clone();
        }
        for (&(ch, id, extended), agg) in &self.aggs {
            let Some(input) = inputs.get_mut(&ch) else {
                continue;
            };
            let Some(db) = self.channel_dbc(ch) else {
                continue;
            };
            let f = CanFrame {
                t_us: agg.last_t_us,
                channel: ch,
                id,
                extended,
                len: agg.len,
                data: agg.data,
                dir: agg.dir,
                flags: agg.flags,
            };
            for d in db.decode_signals(&f) {
                input.signals.insert((id, d.name), d.phys);
            }
        }
        inputs
    }

    /// Frames the script queued: `dir` Tx, stamped on the bus's own
    /// timeline; an id above 0x7FF travels extended unless the script
    /// forced the class the other way (`send_ext`). A payload past 8
    /// bytes becomes a CAN FD frame, snapped to a valid FD length exactly
    /// like the generator's own emit path.
    fn node_frame(channel: u8, id: u32, force_extended: bool, data: &[u8], t_us: u64) -> CanFrame {
        use crate::can::frame::{FrameFlags, dlc2len, len2dlc};
        let data_len = data.len().min(crate::can::frame::MAX_CAN_FD_LEN);
        let (len, flags) = if data_len > 8 {
            (dlc2len(len2dlc(data_len as u8)), FrameFlags::FD)
        } else {
            (data_len as u8, FrameFlags::NONE)
        };
        let mut buf = [0u8; crate::can::frame::MAX_CAN_FD_LEN];
        buf[..data_len].copy_from_slice(&data[..data_len]);
        CanFrame {
            t_us,
            channel,
            id,
            extended: force_extended || id > 0x7FF,
            len,
            data: buf,
            dir: Direction::Tx,
            flags,
        }
    }

    /// The frame read plus the text the just-drained commands produced.
    /// This is the form the publisher hands to the mailbox.
    pub(crate) fn snapshot_with_status(&self, status: Option<String>) -> Snapshot {
        let mut snap = self.snapshot();
        snap.status = status;
        snap
    }

    /// One frame-shaped read of the bus for the frontend.
    pub(crate) fn snapshot(&self) -> Snapshot {
        let replay = match (self.source.position(), self.source.duration()) {
            (Some(p), Some(d)) => Some((p, d)),
            _ => None,
        };
        Snapshot {
            frame_counter: self.frame_counter,
            trace_len: self.trace.len(),
            trace: Arc::clone(&self.published_trace),
            fr_trace: Arc::clone(&self.published_fr),
            fr_dropped: self.fr_trace.dropped(),
            fr_watch: self.hw.fr_watch.as_ref().map(|w| FrWatchView {
                channel_index: w.channel_index,
                fibex_path: w.fibex_path.clone(),
            }),
            sub_count: self.subs.len(),
            replay,
            channel_count: self.channels.len(),
            laps: self.laps,
            bus_counter: self.bus_counter,
            sim_t_us: self.sim_t_us,
            sample_cover: self.sample_cover,
            spec: self.spec.clone(),
            bus_loads: Arc::clone(&self.published_loads),
            nodes: Arc::clone(&self.published_nodes),
            blocks: (*self.published_blocks).clone(),
            emitted: self.emitted_streams.clone(),
            sysvars: self
                .sysvars
                .iter()
                .map(|(d, v)| SysVarView {
                    def: d.clone(),
                    value: *v,
                })
                .collect(),
            write: Arc::clone(&self.published_write),
            hw: self
                .hw
                .buses
                .iter()
                .map(|(&bus, bh)| HwBusView {
                    bus,
                    adapter: bh.adapter,
                    driver: bh.driver,
                    kbps: bh.kbps,
                    can_tx: bh.can_tx,
                    fd: self.hw.fd(bus),
                })
                .collect(),
            real_bus: self.hw.live,
            triggers: self.triggers.clone(),
            last_record: self.recorder.last_record.clone(),
            channels: self
                .channels
                .iter()
                .map(|c| ChannelView {
                    name: c.name.clone(),
                    dbc_paths: c.dbc_paths.clone(),
                    bitrate_kbps: c.bitrate_kbps,
                    fd_data_kbps: c.fd_data_kbps,
                    node_roles: c.node_roles.clone(),
                    dbc: c.dbc.clone(),
                })
                .collect(),
            mode: self.mode,
            run_mode: self.run_mode,
            measuring: self.measuring,
            trace_paused: self.trace_paused,
            recording: self.recorder.recording,
            status: None,
            aggs: self.aggs.values().copied().collect(),
            fr_aggs: {
                let mut rows: Vec<crate::aggregate::FrSlotAgg> =
                    self.fr_aggs.values().cloned().collect();
                rows.sort_by_key(|a| a.slot);
                rows
            },
            subs: self
                .subs
                .iter()
                .map(|(key, s)| SubView {
                    key: key.clone(),
                    latest: s.latest,
                    last_raw: s.last_raw,
                    unit: s.unit.clone(),
                    label: s.label.clone(),
                    type_tag: s.type_tag.clone(),
                    min: s.min,
                    max: s.max,
                    avg: s.avg,
                    history: s.published.clone(),
                    color: s.color,
                })
                .collect(),
            markers: self.markers.clone(),
            tx: self
                .tx_list
                .iter()
                .map(|t| {
                    // 显示值读发射时存储的载荷（随消息周期跳变）；
                    // 从未发射过的条目回退显示 base 字节。
                    let (sent_data, sent_len) = if t.last_len > 0 {
                        (t.last_sent, t.last_len)
                    } else {
                        (t.data, t.len)
                    };
                    TxView {
                        channel: t.channel,
                        id: t.id,
                        name: t.name.clone(),
                        node: t.node.clone(),
                        active: t.active,
                        gate_open: t.node.is_empty()
                            || self.channels[t.channel as usize].role_of(&t.node)
                                == crate::app::NodeRole::Simulated,
                        fd: t.flags.contains(crate::can::frame::FrameFlags::FD),
                        cycle_us: t.cycle_us,
                        data_text: t.data_text.clone(),
                        sent_data,
                        sent_text: crate::generator::hex_text(&sent_data, sent_len),
                        srcs: t.srcs.clone(),
                        muted: matches!(self.mode, Mode::Replay)
                            && self.replay_ids.contains(&(t.channel, t.id)),
                    }
                })
                .collect(),
        }
    }

    /// Opens `path` and starts replaying it. File open and the silence-set
    /// scan are the two deliberate blocking costs of a replay start; they
    /// run once per start, never per frame.
    fn start_replay(&mut self, path: &str, speed: f64, status: &mut String) {
        let stream = match crate::log::open_stream(std::path::Path::new(path)) {
            Ok(stream) => stream,
            Err(e) => {
                *status = format!("replay failed [{path}]: {e}");
                return;
            }
        };
        // Collect the log's ids once at open, from a temporary second
        // stream, so the generators can stand down for the ids the replay
        // itself covers. Draining here, never per frame.
        self.replay_ids = scan_log_ids(std::path::Path::new(path)).unwrap_or_default();
        let info = stream.describe();
        self.recorder.close();
        // Replay just re-emits an existing log; recording it would only
        // duplicate the file, so drop the Record state.
        self.recorder.recording = false;
        let mut source = crate::source::replay::ReplaySource::new(stream);
        source.set_speed(speed);
        self.source = Box::new(source);
        self.mode = Mode::Replay;
        self.run_mode = Mode::Replay;
        self.reset_run();
        self.measuring = true;
        self.nodes_start();
        let tag = if info.is_empty() {
            String::new()
        } else {
            format!(" [{info}]")
        };
        *status = format!("replaying{tag} at {speed}x");
    }

    /// Starts caching one signal: a fresh [`Subscription`] gets the next
    /// palette color and the database's display type. An existing
    /// subscription for the key is left untouched.
    pub(crate) fn subscribe_signal(&mut self, key: SigKey) {
        if !self.subs.contains_key(&key) {
            let color = self.color_counter;
            self.color_counter += 1;
            let type_tag = self.signal_meta(&key);
            self.subs.insert(
                key,
                Subscription {
                    latest: 0.0,
                    last_raw: 0,
                    unit: String::new(),
                    label: None,
                    type_tag,
                    min: f64::INFINITY,
                    max: f64::NEG_INFINITY,
                    avg: 0.0,
                    sum: 0.0,
                    n: 0,
                    last_update_us: 0,
                    last_sample_us: 0,
                    history: crate::observe::SampleCache::default(),
                    published: std::sync::Arc::new(crate::observe::SampleCache::default()),
                    history_dirty: false,
                    color,
                },
            );
        }
    }

    /// The display type the database declares for a subscribed signal, used
    /// until the first frame refreshes it -- and forever when no database
    /// names it.
    fn signal_meta(&self, key: &SigKey) -> String {
        let msg = self
            .channels
            .get(key.0 as usize)
            .and_then(|c| c.dbc.as_ref())
            .and_then(|db| db.messages.get(&(key.1, key.2)));
        msg.and_then(|m| m.signals.iter().find(|s| s.name == key.3))
            .map(|s| s.type_tag.clone())
            .unwrap_or_default()
    }

    /// Adds a bus with the sample DBC path, loads it and pre-populates its
    /// generator from the database.
    fn add_channel(&mut self, status: &mut String) {
        self.bus_counter += 1;
        self.channels.push(Channel {
            name: format!("CAN{}", self.bus_counter),
            dbc: None,
            dbc_paths: vec!["assets/sample.dbc".to_string()],
            dbc_sums: Vec::new(),
            node_roles: std::collections::BTreeMap::new(),
            bitrate_kbps: Channel::DEFAULT_BITRATE_KBPS,
            fd_data_kbps: Channel::DEFAULT_FD_DATA_KBPS,
        });
        self.bus_loads.push(crate::load::BusLoad::new());
        self.loads_dirty = true;
        let ch = self.channels.len() - 1;
        self.load_channel(ch, status);
        let ids: Vec<u32> = self.channels[ch]
            .dbc
            .as_ref()
            .map(|db| db.order.iter().map(|&(id, _)| id).collect())
            .unwrap_or_default();
        for id in ids {
            self.add_entry(ch as u8, id);
        }
    }

    /// (Re)loads every database attached to the bus (primary first, then
    /// extras) and merges them into one lookup table: on a duplicate
    /// message id the earlier database wins. Files that fail to parse are
    /// skipped and reported; a bus whose files all fail ends up without a
    /// database. An empty path list is silent: a bus deliberately
    /// configured without a database is not an error.
    fn load_channel(&mut self, ch: usize, status: &mut String) -> bool {
        let Some(channel) = self.channels.get_mut(ch) else {
            return false;
        };
        if channel.dbc_paths.iter().all(|p| p.trim().is_empty()) {
            channel.dbc = None;
            channel.dbc_sums.clear();
            return false;
        }
        let name = channel.name.clone();
        let mut tables: Vec<crate::dbc::SymbolTable> = Vec::new();
        let mut sums: Vec<u64> = Vec::new();
        let mut first_error: Option<String> = None;
        for path in &channel.dbc_paths {
            let path = path.trim();
            if path.is_empty() {
                continue;
            }
            match crate::dbc::read_file(std::path::Path::new(path)) {
                Ok(content) => match crate::dbc::load_dbc_str(&content) {
                    Ok(table) => {
                        sums.push(content_sum(content.as_bytes()));
                        tables.push(table);
                    }
                    Err(e) => {
                        first_error.get_or_insert_with(|| format!("{path}: {e}"));
                    }
                },
                Err(e) => {
                    first_error.get_or_insert_with(|| format!("{path}: {e}"));
                }
            }
        }
        channel.dbc_sums = sums;
        let loaded = tables.len();
        let total: usize = tables.iter().map(|t| t.order.len()).sum();
        if tables.is_empty() {
            channel.dbc = None;
            *status = match first_error {
                Some(e) => format!("{name} DBC error: {e}"),
                None => format!("{name} DBC error: no database file"),
            };
            return false;
        }
        let mut merged = tables.remove(0);
        for extra in tables {
            crate::dbc::absorb(&mut merged, extra);
        }
        let merged = std::sync::Arc::new(merged);
        // Transmitter lists feed the role cards.
        self.nodes_dirty = true;
        channel.dbc = Some(merged.clone());
        // `$报文::信号` 的名字→id 映射随库刷新：报文换了 id 或名字没了，
        // 节点的映射原地更新，变化进节点日志（静默错位由此可解释）。
        for node in &mut self.nodes {
            if node.channel as usize == ch {
                node.refresh_named_signals(&merged);
            }
        }
        // The role declarations are already in place on a reload, so the
        // fresh table has to meet them: Simulated nodes get entries for
        // messages the database newly declares.
        self.seed_entries_for_simulated(ch);
        // Free scripts are legacy: the moment a database exists on their
        // bus, they get a node to hang under.
        self.adopt_orphan_scripts(ch);
        *status = match first_error {
            Some(e) => format!("{name} DBC partial: {loaded} file(s), {total} messages ({e})"),
            None => format!("{name} DBC loaded: {total} messages"),
        };
        true
    }

    /// Gives every `Simulated` node on the bus the generator entries it is
    /// missing, so a database that gained messages makes them show up.
    /// New entries arrive **inactive** -- neither a DBC edit nor a project
    /// restore may start traffic by itself; switching the role or the TX
    /// switch is the explicit act that does.
    fn seed_entries_for_simulated(&mut self, ch: usize) {
        let simulated: Vec<String> = self.channels[ch]
            .node_roles
            .iter()
            .filter(|(_, r)| **r == NodeRole::Simulated)
            .map(|(n, _)| n.clone())
            .collect();
        for node in simulated {
            let ids = self
                .channel_dbc(ch as u8)
                .map(|db| db.node_tx_ids(&node))
                .unwrap_or_default();
            for id in ids {
                self.add_entry(ch as u8, id);
            }
        }
    }

    /// Binds every free script node on the bus to the database's first
    /// node. Projects from before script bindings existed carry scripts
    /// with no node; the free script is gone from the model, so an orphan
    /// gets a home as soon as a database can host one.
    fn adopt_orphan_scripts(&mut self, ch: usize) {
        let first = self.channels[ch]
            .dbc
            .as_ref()
            .and_then(|db| db.nodes.first().cloned());
        let Some(first) = first else {
            return;
        };
        let mut changed = false;
        for node in &mut self.nodes {
            if node.channel as usize == ch && node.attached.is_none() {
                node.attached = Some((ch as u8, first.clone()));
                changed = true;
            }
        }
        if changed {
            self.nodes_dirty = true;
        }
    }

    /// Adoption pass across every bus -- the project-restore shape.
    fn adopt_orphan_scripts_all(&mut self) {
        for ch in 0..self.channels.len() {
            self.adopt_orphan_scripts(ch);
        }
    }

    /// Re-reads every attached DBC file and reloads the merged table when
    /// any content checksum changed. Returns the reload status text when
    /// something was reloaded. Cheap: one small read per file per sweep,
    /// rate-limited by the caller. Keeps external DBC edits flowing into
    /// decode without a manual reload.
    pub(crate) fn maybe_reload_changed_dbcs(&mut self) -> Option<String> {
        // First pass: find buses whose checksum changed (read-only scan).
        let mut to_reload: Vec<usize> = Vec::new();
        for (ch, channel) in self.channels.iter().enumerate() {
            if channel.dbc_paths.len() != channel.dbc_sums.len() {
                continue; // lists out of sync until the next explicit load
            }
            for (i, path) in channel.dbc_paths.iter().enumerate() {
                let path = path.trim();
                if path.is_empty() {
                    continue;
                }
                let Ok(content) = crate::dbc::read_file(std::path::Path::new(path)) else {
                    continue;
                };
                if content_sum(content.as_bytes()) != channel.dbc_sums[i] {
                    to_reload.push(ch);
                    break; // one reload per bus per sweep
                }
            }
        }
        // Second pass: reload and collect status text.
        let mut reloaded: Option<String> = None;
        for ch in to_reload {
            let mut status = String::new();
            if self.load_channel(ch, &mut status) {
                reloaded = Some(status);
            }
        }
        reloaded
    }

    /// Stores the bus's database path list (primary first, extras after)
    /// and parses everything in the same stroke -- the path is not really
    /// set until its file has been accepted.
    fn load_dbc(&mut self, ch: u8, paths: Vec<String>, status: &mut String) {
        let Some(channel) = self.channels.get_mut(ch as usize) else {
            return;
        };
        channel.dbc_paths = paths;
        self.load_channel(ch as usize, status);
    }

    /// Static per-bus declarations: rename, bitrate figures, the DBC
    /// path (without loading it -- that is [`Self::load_dbc`]'s job), or
    /// the node-role map.
    fn set_channel_config(
        &mut self,
        ch: u8,
        name: Option<String>,
        dbc_path: Option<String>,
        bitrate_kbps: Option<u32>,
        fd_data_kbps: Option<u32>,
        node_roles: Option<std::collections::BTreeMap<String, crate::app::NodeRole>>,
    ) {
        let Some(c) = self.channels.get_mut(ch as usize) else {
            return;
        };
        if let Some(name) = name {
            c.name = name;
        }
        if let Some(p) = dbc_path {
            // Replaces the primary database; any extra attached databases
            // stay on the bus.
            if c.dbc_paths.is_empty() {
                c.dbc_paths.push(p);
            } else {
                c.dbc_paths[0] = p;
            }
        }
        if let Some(k) = bitrate_kbps {
            c.bitrate_kbps = k.max(1);
        }
        if let Some(k) = fd_data_kbps {
            c.fd_data_kbps = k.max(1);
        }
        if let Some(roles) = node_roles {
            // Project restore pins the declarations wholesale. Whether
            // anything transmits is decided by the restored generator
            // rows, so this only records intent -- no entry activation
            // here.
            c.node_roles = roles;
            self.nodes_dirty = true;
        }
    }

    /// Restores one generator row wholesale from a saved project, with
    /// every field's own convention applied: FD flag, the anti-typo
    /// period floor is the sender's to apply first, `None` payload keeps
    /// the row's current base bytes, and the schedule restarts now.
    // The signature mirrors the `SetEntryConfig` command one-to-one; a
    // payload struct would just relocate the field list.
    #[allow(clippy::too_many_arguments)]
    fn set_entry_config(
        &mut self,
        ch: u8,
        id: u32,
        active: bool,
        cycle_us: u64,
        fd: bool,
        flags: Option<crate::can::frame::FrameFlags>,
        data_text: Option<String>,
        srcs: Vec<crate::sim::ValueSrc>,
    ) {
        let Some(tx) = self.entry_mut(ch, id) else {
            return;
        };
        tx.active = active;
        tx.cycle_us = cycle_us;
        tx.next_t_us = 0;
        tx.flags = flags.unwrap_or(if fd {
            crate::can::frame::FrameFlags::FD
        } else {
            crate::can::frame::FrameFlags::NONE
        });
        if let Some(text) = data_text
            && let Some(bytes) = crate::generator::parse_hex_bytes(&text)
        {
            let mut data = [0u8; crate::can::frame::MAX_CAN_FD_LEN];
            data[..bytes.len()].copy_from_slice(&bytes);
            crate::generator::set_tx_base(tx, data, bytes.len() as u8);
        }
        tx.srcs = srcs;
    }

    /// One scan + merge over a span known to be uncovered: the plot's
    /// history beyond the playhead. Lives on the bus because the scan
    /// owns the log source and the caches it fills.
    fn backfill(&mut self, t_from_us: u64, t_to_us: u64, stride: u64, status: &mut String) {
        let mut frames = Vec::new();
        let capped =
            !self
                .source
                .scan_range(t_from_us, t_to_us, crate::app::MAX_SCAN_FRAMES, &mut frames);
        // A scan-local stride: the shared per-signal baseline sits at the
        // playhead and would reject every point that lies behind it.
        let mut stride_map: HashMap<SigKey, u64> = HashMap::new();
        let mut batches: HashMap<SigKey, Vec<(u64, f64)>> = HashMap::new();
        for f in &frames {
            for (key, d) in self.subscribed_values(f) {
                if stride_map.get(&key).is_some_and(|&lt| f.t_us < lt + stride) {
                    continue;
                }
                stride_map.insert(key.clone(), f.t_us);
                batches.entry(key).or_default().push((f.t_us, d.phys));
            }
        }
        for (key, pts) in batches {
            if let Some(sub) = self.subs.get_mut(&key) {
                let taken = sub.history.merge(&pts, stride);
                for v in taken {
                    sub.observe(v);
                }
                sub.history_dirty = true;
            }
        }
        self.refresh_sub_histories();
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
            *status = format!(
                "plot: window too dense to decode fully ({})",
                crate::app::MAX_SCAN_FRAMES
            );
        }
    }

    /// Loads every bus's declared database in one go. This is bootstrap
    /// work, run before the first publish (and before the core thread
    /// ever starts), not a command: there is no frontend yet to receive
    /// the status lines.
    pub(crate) fn bootstrap_dbcs(&mut self) {
        for ch in 0..self.channels.len() {
            let mut status = String::new();
            self.load_channel(ch, &mut status);
        }
    }

    /// Pre-populates the generator with one entry per message the loaded
    /// databases declare. Bootstrap work, like [`Self::bootstrap_dbcs`]:
    /// it runs before the first publish, so a threaded frontend's very
    /// first snapshot already shows the full send table.
    pub(crate) fn populate_generator(&mut self) {
        let ids: Vec<(u8, u32)> = self
            .channels
            .iter()
            .enumerate()
            .flat_map(|(ch, c)| {
                c.dbc
                    .as_ref()
                    .map(|db| db.order.iter().map(move |&(id, _)| (ch as u8, id)))
                    .into_iter()
                    .flatten()
            })
            .collect();
        for (ch, id) in ids {
            self.add_entry(ch, id);
        }
    }

    /// Removes bus `ch` and remaps every bus-side, channel-indexed
    /// reference one step down. Window state is the frontend's to remap.
    fn remove_channel(&mut self, ch: usize, status: &mut String) {
        if self.channels.len() <= 1 {
            *status = "at least one bus is required".to_string();
            return;
        }
        if ch >= self.channels.len() {
            return;
        }
        let name = self.channels[ch].name.clone();
        self.channels.remove(ch);
        self.bus_loads.remove(ch);
        self.loads_dirty = true;
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
            .filter_map(|((c, id, ext), mut a)| {
                remap(c).map(|nc| {
                    a.channel = nc;
                    ((nc, id, ext), a)
                })
            })
            .collect();
        self.subs = self
            .subs
            .drain()
            .filter_map(|((c, id, ext, sig), s)| remap(c).map(|nc| ((nc, id, ext, sig), s)))
            .collect();
        self.spec.drop_channel(ch as u8);
        self.tx_list.retain(|t| t.channel as usize != ch);
        for t in &mut self.tx_list {
            if t.channel as usize > ch {
                t.channel -= 1;
            }
        }
        // The hardware attachment of the removed bus closes with it; the
        // other buses' attachments shift down with their bus.
        self.hw.remove_bus(ch);
        // Script nodes and replay blocks bind to a bus by index: the ones
        // on the removed bus lose their meaning and go with it, the rest
        // shift down -- the same policy the tx entries follow. Derived
        // streams follow their owning node's bus.
        self.nodes.retain(|n| n.channel as usize != ch);
        for n in &mut self.nodes {
            if n.channel as usize > ch {
                n.channel -= 1;
            }
        }
        self.replay_blocks.retain(|b| b.channel as usize != ch);
        for b in &mut self.replay_blocks {
            if b.channel as usize > ch {
                b.channel -= 1;
            }
        }
        // Triggers bind to a bus by index as well: a rule watching the
        // removed bus is meaningless, and a Send reaction aimed at it has
        // no target left. Everything else shifts down.
        let removed = ch as u8;
        self.triggers.retain(|t| {
            let cond_gone = t.cond.bus() == removed;
            let action_gone = matches!(&t.action, TriggerAction::Send { ch, .. } if *ch == removed);
            !cond_gone && !action_gone
        });
        for t in &mut self.triggers {
            let cond_ch = match &mut t.cond {
                TriggerCond::SignalCross { ch, .. }
                | TriggerCond::IdPresent { ch, .. }
                | TriggerCond::ErrorFrame { ch }
                | TriggerCond::CycleTimeout { ch, .. } => ch,
                // System variables are global: no bus to shift.
                TriggerCond::SysVar { .. } => continue,
            };
            if *cond_ch > removed {
                *cond_ch -= 1;
            }
            if let TriggerAction::Send { ch, .. } = &mut t.action
                && *ch > removed
            {
                *ch -= 1;
            }
        }
        self.emitted_streams = self
            .emitted_streams
            .drain(..)
            .filter_map(|(mut k, owner)| {
                remap(k.0).map(|nc| {
                    k.0 = nc;
                    (k, owner)
                })
            })
            .collect();
        self.nodes_dirty = true;
        self.trace.rewrite(|f| {
            if f.channel as usize == ch {
                return false;
            }
            if f.channel as usize > ch {
                f.channel -= 1;
            }
            true
        });
        self.publish_trace();
        *status = format!("{name} removed");
    }

    /// Declares a DBC node's role.
    ///
    /// `Simulated` adds whatever generator entry the node is missing and
    /// switches them on. The period of an entry that already exists is
    /// never rewritten, so a value tuned by hand outlives the click.
    /// Leaving `Simulated` only stops sending: entries keep their payload
    /// and waveforms, so simulating the node again restores it exactly as
    /// it was. `Monitor` and `Absent` are behaviourally the same here --
    /// neither transmits -- and differ in what they declare: a monitor is
    /// present and listening, an absent node is not on the simulated bus.
    fn set_node_role(&mut self, channel: u8, node: &str, role: NodeRole, status: &mut String) {
        if self.channels.get(channel as usize).is_none() {
            return;
        }
        // The role is recorded first and unconditionally: a node that sends
        // nothing still has to remember what we declared it to be.
        self.channels[channel as usize].set_node_role(node, role);

        // Membership comes from the live database, not from each entry's
        // stamped `node`: loading another DBC does not rebuild the generator,
        // so a stamp can name a message this node no longer owns.
        let ids = self
            .channel_dbc(channel)
            .map(|db| db.node_tx_ids(node))
            .unwrap_or_default();
        if role == NodeRole::Simulated {
            for id in &ids {
                self.add_entry(channel, *id);
            }
            // 闸门刚开：把该节点已启用条目的排程锚定到当前时钟。闸关
            // 期间发射循环不跑、next_t_us 冻结，不锚定会在开闸瞬间把
            // 陈旧排程倾泻成突发。只动排程，绝不改写条目的开/关——
            // 那是用户的逐条自定义，角色切换无权触碰。
            let sim = self.sim_t_us;
            for t in &mut self.tx_list {
                if t.channel == channel && (ids.contains(&t.id) || t.node == node) && t.active {
                    t.next_t_us = sim;
                }
            }
        }
        // 切出「模拟」（监听/离线）：条目开/关原样保留，闸门关闭即停发。
        let bus = self
            .channels
            .get(channel as usize)
            .map(|c| c.name.clone())
            .unwrap_or_else(|| format!("CAN{}", channel + 1));
        *status = match role {
            NodeRole::Simulated => format!(
                "simulating {node} on {bus} ({} message(s), 条目按各自开关发车)",
                ids.len()
            ),
            NodeRole::Absent => format!("{node} offline from {bus}"),
        };
        // The role cards read the role map: republish.
        self.nodes_dirty = true;
    }

    /// Adds a replay block. New blocks start disabled: nothing transmits
    /// until the user says so, whichever way the toggle flips later.
    #[allow(clippy::too_many_arguments)]
    fn add_replay_block(
        &mut self,
        name: String,
        channel: u8,
        path: String,
        node_filter: Option<String>,
        attached: Option<(u8, String)>,
        ids: Vec<(u32, bool)>,
        status: &mut String,
    ) {
        self.block_counter += 1;
        let mut block = crate::block::ReplayBlock::new(
            self.block_counter,
            name,
            channel,
            path,
            node_filter,
            attached,
            ids,
            false,
        );
        if block.enabled {
            let dbc = self
                .channels
                .get(block.channel as usize)
                .and_then(|c| c.dbc.clone());
            block.load_queue(dbc.as_deref(), self.sim_t_us);
        }
        *status = format!("replay block `{}` added (disabled)", block.name);
        self.replay_blocks.push(block);
        self.nodes_dirty = true;
    }

    /// Rewrites one block's declaration. An enabled block reloads its
    /// queue against the current database, so the edit lands immediately;
    /// a disabled one just carries the new text until it is enabled.
    // The signature mirrors the `SetReplayBlock` command one-to-one; a
    // payload struct would just relocate the field list.
    #[allow(clippy::too_many_arguments)]
    fn set_replay_block(
        &mut self,
        id: u64,
        name: String,
        channel: u8,
        path: String,
        node_filter: Option<String>,
        attached: Option<(u8, String)>,
        ids: Vec<(u32, bool)>,
        status: &mut String,
    ) {
        let Some(block) = self.replay_blocks.iter_mut().find(|b| b.id == id) else {
            return;
        };
        block.name = name;
        block.channel = channel;
        block.path = path;
        block.node_filter = node_filter;
        block.attached = attached;
        block.ids = ids;
        if block.enabled {
            let dbc = self
                .channels
                .get(block.channel as usize)
                .and_then(|c| c.dbc.clone());
            block.load_queue(dbc.as_deref(), self.sim_t_us);
        }
        *status = format!("replay block `{}` updated", block.name);
        self.nodes_dirty = true;
    }

    /// Toggles a block. Enabling loads the queue right away regardless of
    /// the measurement state, so a broken path surfaces now, not silently
    /// at the next start.
    fn set_replay_block_enabled(&mut self, id: u64, on: bool, status: &mut String) {
        let Some(block) = self.replay_blocks.iter_mut().find(|b| b.id == id) else {
            return;
        };
        block.enabled = on;
        if on {
            let dbc = self
                .channels
                .get(block.channel as usize)
                .and_then(|c| c.dbc.clone());
            block.load_queue(dbc.as_deref(), self.sim_t_us);
            *status = match (&block.last_error, block.enabled) {
                (Some(e), _) => format!("replay block `{}`: {e}", block.name),
                (None, true) => format!(
                    "replay block `{}` on ({} frame(s))",
                    block.name,
                    block.queue_len()
                ),
                (None, false) => format!("replay block `{}` off", block.name),
            };
        } else {
            *status = format!("replay block `{}` off", block.name);
        }
        self.nodes_dirty = true;
    }

    /// Adds the generator entry `(ch, id)` unless it already exists.
    fn add_entry(&mut self, ch: u8, id: u32) {
        if self.tx_list.iter().any(|t| t.channel == ch && t.id == id) {
            return;
        }
        let (name, node, len, cycle_us) = self
            .channel_dbc(ch)
            .and_then(|db| db.message_of(id))
            .map(|m| {
                (
                    m.name.clone(),
                    m.transmitter.clone(),
                    m.dlc.min(crate::can::frame::MAX_CAN_FD_LEN as u64) as u8,
                    // A declared 0 is event-triggered, so `unwrap_or` rather
                    // than `unwrap_or_default` on the Option: only "the DBC
                    // said nothing" gets our invented period.
                    m.cycle_us.unwrap_or(crate::app::DEFAULT_TX_CYCLE_US),
                )
            })
            .unwrap_or_else(|| {
                (
                    format!("{id:X}"),
                    String::new(),
                    8,
                    crate::app::DEFAULT_TX_CYCLE_US,
                )
            });
        let data_text = vec!["00"; len as usize].join(" ");
        self.tx_list.push(TxMsg {
            channel: ch,
            id,
            srcs: Vec::new(),
            extended: id > 0x7FF,
            name,
            node,
            len,
            data: [0; crate::can::frame::MAX_CAN_FD_LEN],
            flags: if len > 8 {
                crate::can::frame::FrameFlags::FD
            } else {
                crate::can::frame::FrameFlags::NONE
            },
            data_text,
            cycle_us,
            active: false,
            next_t_us: 0,
            last_sent: [0; crate::can::frame::MAX_CAN_FD_LEN],
            last_len: 0,
        });
    }

    /// Writes a physical value into the base payload and pins that signal
    /// by dropping only its source: grabbing a moving slider means "hold
    /// here". Returns false when the database cannot encode it.
    fn pin_entry_signal(&mut self, ch: u8, id: u32, name: &str, phys: f64) -> bool {
        let mut data = match self.entry_mut(ch, id) {
            Some(tx) => tx.data,
            None => return false,
        };
        let Some(table) = self.channel_dbc(ch) else {
            return false;
        };
        if !table.encode_signal(id, name, phys, &mut data) {
            return false;
        }
        let msg_size = table
            .message_of(id)
            .map(|m| m.dlc.min(crate::can::frame::MAX_CAN_FD_LEN as u64) as u8)
            .unwrap_or(0);
        let tx = self.entry_mut(ch, id).expect("entry checked above");
        tx.srcs.retain(|s| s.name != name);
        let len = tx.len.max(msg_size);
        crate::generator::set_tx_base(tx, data, len);
        true
    }

    /// The generator entry `(ch, id)`, if present.
    fn entry_mut(&mut self, ch: u8, id: u32) -> Option<&mut TxMsg> {
        self.tx_list
            .iter_mut()
            .find(|t| t.channel == ch && t.id == id)
    }

    /// Replaces the base payload from the generator's hex box. Returns
    /// false if the text is not whole hex bytes or no entry carries the
    /// key.
    fn set_entry_hex(&mut self, ch: u8, id: u32, text: &str) -> bool {
        let Some(bytes) = crate::generator::parse_hex_bytes(text) else {
            return false;
        };
        let Some(tx) = self.entry_mut(ch, id) else {
            return false;
        };
        let mut data = [0u8; crate::can::frame::MAX_CAN_FD_LEN];
        data[..bytes.len()].copy_from_slice(&bytes);
        let len = bytes.len() as u8;
        crate::generator::set_tx_base(tx, data, len);
        true
    }

    /// Moves the replay playhead to `t_s` seconds. The log's own duration
    /// bounds the request so a drag past the right edge lands on the last
    /// frame.
    fn seek_replay(&mut self, t_s: f64, status: &mut String) {
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
                *status = match dur {
                    Some(d) => format!("seek {:.2} / {:.2} s", landed as f64 / 1e6, d),
                    None => format!("seek {:.2} s", landed as f64 / 1e6),
                };
            }
            None => *status = "seek: past end of log".to_string(),
        }
    }

    /// Lets every signal's sampler resume at a scrubbed playhead. Retained
    /// samples are left in place; see [`Subscription::resume_sampling_at`].
    pub(crate) fn rewind_samples_to(&mut self, t_us: u64) {
        for sub in self.subs.values_mut() {
            sub.resume_sampling_at(t_us);
        }
    }

    /// Fresh virtual run: new source, blank run state, wall-clock
    /// measuring.
    fn start_virtual(&mut self, status: &mut String) {
        self.recorder.close();
        self.source = Box::new(crate::source::virtual_source::VirtualSource::new());
        self.mode = Mode::Virtual;
        self.run_mode = Mode::Virtual;
        self.reset_run();
        self.measuring = true;
        self.nodes_start();
        // Enabled blocks load their queues now: the log is read against
        // the database as it stands at run start, and a broken path or a
        // filtered-to-nothing log names itself instead of going silent.
        let mut block_notes: Vec<String> = Vec::new();
        for block in &mut self.replay_blocks {
            if block.enabled {
                let dbc = self
                    .channels
                    .get(block.channel as usize)
                    .and_then(|c| c.dbc.clone());
                block.load_queue(dbc.as_deref(), self.sim_t_us);
                if let Some(e) = &block.last_error {
                    block_notes.push(format!("`{}`: {e}", block.name));
                }
            }
        }
        *status = if block_notes.is_empty() {
            "measuring (virtual)".to_string()
        } else {
            format!(
                "measuring (virtual); replay block {}",
                block_notes.join("; ")
            )
        };
        if self.recorder.recording {
            match self.recorder.open() {
                Ok(path) => *status = format!("recording to {path}"),
                Err(e) => *status = format!("record failed: {e}"),
            }
        }
    }

    /// Blanks every run-scored counter: rings, aggregates, loads, spec
    /// memory, generator schedules, subscription measurement state.
    pub(crate) fn reset_run(&mut self) {
        self.sim_t_us = 0;
        self.sim_prev_us = 0;
        for block in &mut self.replay_blocks {
            block.rewind();
        }
        // A fresh start must not inherit the previous run's pause state.
        self.trace_paused = false;
        self.paused_at_us = None;
        self.scan_done = false;
        self.trace.clear();
        self.fr_trace.clear();
        self.publish_trace();
        self.frame_counter = 0;
        self.aggs.clear();
        self.fr_aggs.clear();
        for load in &mut self.bus_loads {
            load.clear();
        }
        // Along with the aggregates it reads: keeping the previous run's
        // interval memory would turn the first step of a new run into one
        // enormous measured period.
        self.spec = crate::spec::Spec::default();
        self.sample_cover = None;
        self.markers.clear();
        self.pre_buffer.clear();
        self.post_roll = None;
        // Send-now intents recorded while the old run was winding down
        // belong to it, not to the fresh one.
        self.injected.clear();
        for tx in &mut self.tx_list {
            tx.next_t_us = 0;
        }
        for sub in self.subs.values_mut() {
            sub.reset_measurement();
        }
        // Every run starts from the declared values, CANoe-style -- and
        // republishes them, so observers never plot the last run's tail.
        self.init_sysvars();
        self.refresh_sub_histories();
    }

    fn stop_bus(&mut self, status: &mut String) {
        self.measuring = false;
        self.recorder.close();
        for node in &mut self.nodes {
            node.stop();
        }
        self.nodes_dirty = true;
        *status = "stopped".to_string();
    }

    fn toggle_record(&mut self, status: &mut String) {
        // A manual stop is immediate: any pending trigger post-roll dies.
        self.post_roll = None;
        if self.recorder.recording {
            self.recorder.close();
            self.recorder.recording = false;
        } else {
            self.recorder.recording = true;
            // While stopped, the file is created by the next start;
            // checking Record must not leave an empty record file behind.
            if self.measuring {
                let opened = self.recorder.open();
                self.recorder.recording = opened.is_ok();
                match opened {
                    Ok(path) => *status = format!("recording to {path}"),
                    Err(e) => *status = format!("record failed: {e}"),
                }
            }
        }
    }

    /// Manual re-arm: clears the level latch of every appearance-type
    /// condition (ID present / error frame) so the next occurrence fires
    /// again. Signal-cross and cycle-timeout levels are real levels and
    /// are never touched.
    fn rearm_latched_triggers(&mut self, status: &mut String) {
        let mut n = 0;
        for t in &mut self.triggers {
            if t.cond.latches_once() && t.level {
                t.rearm();
                n += 1;
            }
        }
        *status = format!("re-armed {n} latched trigger(s)");
    }

    /// Offline analysis (R1): ingests the whole loaded log in one pass --
    /// no playback clock, no trigger actions, no node dispatch, no
    /// recording -- so every observer holds the full file for browsing.
    /// Aggregates, load history, spec verdicts, the Graphics / Tracker
    /// caches and the Trace ring all end up exactly as a replay that ran
    /// to the end would leave them, just instantly. A second scan is
    /// refused (it would double every figure); any run restart clears it.
    fn scan_log(
        &mut self,
        path: &str,
        tol_pct: u64,
        grace: u64,
        status: &mut String,
    ) {
        if self.measuring {
            *status = "scan needs a stopped measurement".to_string();
            return;
        }
        // A fresh replay source and clean tallies: the scan measures the
        // file once, from zero, and a Play afterwards starts over the
        // normal way (which also clears the scan results -- no double
        // counting).
        self.start_replay(path, 1.0, status);
        if !matches!(self.mode, Mode::Replay) {
            return; // start_replay reported the failure
        }
        let stride = self.applied_stride_us;
        let mut scanned = 0u64;
        let mut from = 0u64;
        let mut batch: Vec<CanFrame> = Vec::with_capacity(8_192);
        loop {
            batch.clear();
            // scan_range parks and restores the playhead, so a scan never
            // disturbs where playback would resume.
            let complete = self.source.scan_range(from, u64::MAX, 8_192, &mut batch);
            for f in &batch {
                scanned += 1;
                self.ingest(*f, stride);
            }
            if complete || batch.is_empty() {
                break;
            }
            // Resume past the last ingested stamp: seek lands on the first
            // frame at/after the offset, so nothing is rescanned.
            from = batch.last().map(|f| f.t_us + 1).unwrap_or(from);
        }
        // One spec sweep over the whole-file aggregates: the verdicts are
        // facts about frames already seen. The Missing check stays
        // replay-only anyway (no live claim over a scanned file).
        self.sim_t_us = self
            .aggs
            .values()
            .map(|a| a.last_t_us)
            .max()
            .unwrap_or_default();
        self.check_spec(tol_pct, grace);
        for load in &mut self.bus_loads {
            load.sample();
        }
        self.publish_trace();
        self.refresh_sub_histories();
        // Park: browsing state, not a running replay. Play restarts.
        self.measuring = false;
        self.scan_done = true;
        *status = format!("scan complete: {scanned} frame(s) ingested for browsing");
    }

    /// Blanks the trace the way the Clear button means it: the frames on
    /// display (and their archive) go away; the measurement itself keeps
    /// running -- recording stays open, counts keep counting, arrivals
    /// after the clear show up as usual.
    fn clear_trace_view(&mut self, status: &mut String) {
        self.trace.clear();
        self.fr_trace.clear();
        self.publish_trace();
        *status = "trace cleared".to_string();
    }

    /// Resets the per-message counters behind the Messages and Statistics
    /// rows. Deliberately narrower than `reset_run`: load, spec memory and
    /// the recording are mid-run state the user did not ask to lose.
    fn clear_aggregates(&mut self, status: &mut String) {
        let n = self.aggs.len();
        self.aggs.clear();
        *status = format!("cleared {n} message counter(s)");
    }

    /// The wall-clock instant the bus next has work due: the next replay
    /// frame, or the next generator slot against the sim clock (which runs
    /// 1:1 with the wall in Virtual). `None` means nothing is scheduled --
    /// the event loop may sleep until a command arrives. Paused or stopped
    /// buses are never due.
    pub(crate) fn next_deadline(&self, now_us: u64) -> Option<u64> {
        if !self.measuring || self.trace_paused {
            return None;
        }
        let bus_due = match self.mode {
            Mode::Replay => self.source.next_deadline(now_us),
            Mode::Virtual => {
                let min_next = self
                    .tx_list
                    .iter()
                    // 角色闸同发射循环：闸关的条目不产生死线。
                    .filter(|t| {
                        t.active
                            && t.cycle_us != 0
                            && (t.node.is_empty()
                                || self.channels[t.channel as usize].role_of(&t.node)
                                    == NodeRole::Simulated)
                    })
                    .map(|t| t.next_t_us)
                    .min()?;
                Some(now_us + min_next.saturating_sub(self.sim_t_us))
            }
        };
        // Node timers are absolute wall-clock dues and never delay the
        // bus past its own work, but the loop must wake for them.
        let node_due = self
            .nodes
            .iter()
            .filter(|n| n.running())
            .flat_map(|n| n.timer_dues())
            .min();
        match (bus_due, node_due) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (Some(a), None) => Some(a),
            (None, Some(b)) => Some(b),
            (None, None) => None,
        }
    }

    /// Advance the bus's own clocks to wall-clock `now_us`, before a step.
    /// A span spent frozen is skipped rather than accumulated: replay
    /// resumes in place instead of fast-forwarding through it, and the
    /// generator's schedule and waveform phase do not jump by the paused
    /// span. In Replay the sim clock is owned by [`Self::step`] instead,
    /// which reads it off the log frames; adding wall time here would let
    /// the generator march on while the log is quiet and snap back on the
    /// next frame.
    pub(crate) fn advance_clock(&mut self, now_us: u64) {
        if let Some(t) = self.paused_at_us.take() {
            self.source.shift_time(now_us.saturating_sub(t));
            self.sim_prev_us = now_us;
        }
        if !matches!(self.mode, Mode::Replay) {
            self.sim_t_us += now_us.saturating_sub(self.sim_prev_us);
        }
        self.sim_prev_us = now_us;
    }

    /// The event loop's unit of work: advance the clocks to `now_us`, then
    /// run one full step. `step` alone assumes the caller maintains the
    /// clocks (as the UI loop does); `step_to` is what a deadline-driven
    /// loop calls when it wakes.
    // Wired into the core thread's event loop (阶段 3).
    #[allow(dead_code)]
    pub(crate) fn step_to(
        &mut self,
        now_us: u64,
        stride: u64,
        tol_pct: u64,
        grace: u64,
        status: &mut String,
    ) -> bool {
        self.advance_clock(now_us);
        self.step(now_us, stride, tol_pct, grace, status)
    }

    /// One full step of the bus against wall-clock `now_us`: poll the
    /// source, let generators emit against the sim clock, walk the queue
    /// through triggers / recorder / ingest, sample the load rollups, check
    /// the databases' promises, sweep timeout triggers, and settle a
    /// finished replay. Returns true when a replay finished this step.
    /// `stride` is the frontend's requested sampling period for signal
    /// history; `tol_pct` and `grace` its spec tolerances -- frontend
    /// policy the bus applies but does not own. Status messages from
    /// trigger actions or the finish go into `status` (last write wins;
    /// the status bar has one line).
    pub(crate) fn step(
        &mut self,
        now_us: u64,
        stride: u64,
        tol_pct: u64,
        grace: u64,
        status: &mut String,
    ) -> bool {
        // Every step moves the clock, so the load windows drain and resample
        // whether or not a frame arrived; publish the rollups after it.
        self.loads_dirty = true;
        self.buf.clear();
        self.source.poll(now_us, &mut self.buf);
        // FlexRay rows a BLF replay carries come through the same source;
        // they skip the CAN pipeline and land in the FR ring directly.
        let mut fr_replay: Vec<crate::trace::FrRow> = Vec::new();
        self.source.poll_fr(now_us, &mut fr_replay);
        let fr_replayed = !fr_replay.is_empty();
        // Send-now requests are intents recorded by commands; building the
        // frames here stamps them with this step's clock, so a request that
        // waited out a pause never sends a pre-pause timestamp.
        let injected: Vec<(u8, u32)> = std::mem::take(&mut self.injected);
        for (ch, id) in injected {
            self.send_one_shot(ch, id, now_us, &HashMap::new());
        }
        let source_empty = self.buf.is_empty();
        if stride != self.applied_stride_us {
            self.sample_cover = None;
            self.applied_stride_us = stride;
        }

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
        // "replay a real log and stir a few frames in" possible. One
        // exception: an id the log itself carries stays silent, or replaying
        // a recording of this same simulation interleaves two senders of one
        // signal and every consumer sees their mixed values. Only consulted
        // in Replay mode, so nothing to restore when the run ends.
        let channels = &self.channels;
        let mut emitted: Vec<CanFrame> = Vec::new();
        for tx in &mut self.tx_list {
            if matches!(self.mode, Mode::Replay) && tx.next_t_us == 0 {
                // Log time has no slot zero: an entry picked up mid-log is
                // anchored at the playhead instead of emitting one frame
                // dated the epoch.
                tx.next_t_us = sim;
            }
            let muted =
                matches!(self.mode, Mode::Replay) && self.replay_ids.contains(&(tx.channel, tx.id));
            // 角色闸：绑定到 DBC 节点的条目只有在节点角色为「模拟」时才
            // 发车；未分配条目（无 node 戳）不受闸。闸关时循环不跑、
            // next_t_us 冻结，切回模拟由 set_node_role 锚定排程。
            let gate_open = tx.node.is_empty()
                || channels[tx.channel as usize].role_of(&tx.node) == NodeRole::Simulated;
            // Every slot the clock has passed goes out at its own stamp.
            // Skipping the backlog after a UI stall (the old policy) kept the
            // tick cheap but punched a hole into the bus's own timeline --
            // and at Graphics strides fine enough to show single updates,
            // that hole reads as the curve being eaten while the plot
            // slides on. Frames carry their slot's timestamp, so spacing
            // stays exactly `cycle_us` even in the catch-up burst.
            let mut budget = MAX_TX_CATCHUP;
            while budget > 0
                && tx.active
                && gate_open
                && !muted
                && tx.cycle_us != 0
                && tx.next_t_us <= sim
            {
                budget -= 1;
                // Values are read at the slot, not at `sim`: a frame stamped
                // `slot` must carry the waveform's value at `slot`, or every
                // payload would lead its own timestamp by up to a full cycle.
                let slot = tx.next_t_us;
                tx.next_t_us += tx.cycle_us;
                let (data, len, flags) = crate::generator::tx_payload(channels, tx, slot);
                // 发射时计算并存储：快照直接读存储值，UI 显示随消息周期
                // 跳变，不再逐帧重算。
                tx.last_sent = data;
                tx.last_len = len;
                let frame = CanFrame {
                    t_us: slot,
                    channel: tx.channel,
                    id: tx.id,
                    extended: tx.extended,
                    len,
                    data,
                    dir: Direction::Tx,
                    flags,
                };
                // The wire egress in Real bus mode: simulated frames go
                // onto the attached channel *and* stay on the internal
                // bus so every view sees them.
                self.hw.write_if_live(tx.channel, &frame);
                emitted.push(frame);
            }
        }
        self.buf.extend(emitted);

        // Steps that only received FlexRay frames publish too, or the FR
        // section would freeze until the next CAN arrival.
        let mut fr_landed = fr_replayed;
        for row in fr_replay {
            self.ingest_fr_row(row);
        }

        // Replay blocks stream their recorded traffic onto the same sim
        // clock the generator uses. Replay mode is excluded: the whole log
        // already IS the source there, and doubling its frames up would
        // poison every aggregate, load figure, and spec verdict.
        if !matches!(self.mode, Mode::Replay) {
            let mut block_out: Vec<CanFrame> = Vec::new();
            for block in &mut self.replay_blocks {
                block.poll(sim, &mut block_out, MAX_TX_CATCHUP as usize);
            }
            self.buf.extend(block_out);

            // Hardware RX: whatever an attached adapter received is real
            // bus traffic and ingests like everything else. Replay mode
            // stays wire-free for the same double-delivery reason.
            let mut hw_rx: Vec<CanFrame> = Vec::new();
            self.hw.poll_rx(sim, &mut hw_rx);
            self.buf.extend(hw_rx);

            // The FlexRay watch drains on the same cadence as the CAN
            // adapters. Its rows skip the CAN pipeline: the ingest goes
            // straight to the FR ring.
            let mut fr_rx: Vec<crate::trace::FrRow> = Vec::new();
            self.hw.poll_fr(&mut fr_rx);
            if !fr_rx.is_empty() {
                fr_landed = true;
                for row in fr_rx {
                    self.ingest_fr_row(row);
                }
            }
        }

        // Node timers fire before the ingest walk so the frames they
        // queue join this same step. Their clock is the step's wall
        // clock: periodic node behaviour keeps its cadence in replay too.
        let node_inputs = self.build_node_inputs(now_us);
        self.run_node_timers(now_us, &node_inputs);

        let replay_done =
            matches!(self.mode, Mode::Replay) && source_empty && self.source.is_done();

        // Index walk rather than `for f in &self.buf`: a frame is copied
        // out one at a time, and a `while`, not a `for` over `0..len()`:
        // the range freezes its end before the loop, and a Send reaction
        // pushing onto `buf` mid-loop must be processed by this same step,
        // not wiped by the next one.
        let mut i = 0;
        while i < self.buf.len() {
            let f = self.buf[i];
            i += 1;
            // Triggers judge the frame before anything else consumes it,
            // so a trigger that starts a recording captures the very
            // frame that fired it.
            self.eval_triggers(&f, status);
            // The record filter gates only what lands in the file: direct
            // writes, the pre-trigger context, and post-roll counting.
            // Trace, aggregates, and the spec always see the whole bus.
            let record_this = self.recorder.admits(&f);
            if record_this {
                self.recorder.write(&f);
                // Pre-trigger memory: a trigger that starts a recording drains
                // this ring into the file first, so the event keeps its past.
                self.pre_buffer.push_back(f);
                if self.pre_buffer.len() > self.pre_frames {
                    self.pre_buffer.pop_front();
                }
                // Post-roll: a trigger-initiated stop keeps the file open for
                // this many frames before the recorder actually closes.
                if let Some(left) = self.post_roll.as_mut() {
                    if *left <= 1 {
                        self.post_roll = None;
                        self.recorder.close();
                        self.recorder.recording = false;
                        *status = "trigger post-roll finished".to_string();
                    } else {
                        *left -= 1;
                    }
                }
            }
            // Ingest first, then dispatch to nodes: `sig()` in node
            // handlers reads the freshly-updated aggregates.
            self.ingest(f, stride);
            // Build fresh inputs after ingest so `sig()` reads current
            // values, not stale ones from before the frame arrived.
            let node_inputs = self.build_node_inputs(now_us);
            let input = node_inputs.get(&f.channel).cloned().unwrap_or_default();
            let queued = self.dispatch_node_frame(&f, &input);
            for (id, ext, data) in queued {
                self.buf
                    .push(Self::node_frame(f.channel, id, ext, &data, f.t_us));
            }
        }
        if i > 0 || fr_landed {
            self.publish_trace();
        }
        self.refresh_sub_histories();

        // One sample of the windowed numbers per step feeds the Min/Max/Avg
        // columns of the Bus Statistics window.
        for load in &mut self.bus_loads {
            load.sample();
        }

        self.check_spec(tol_pct, grace);
        self.eval_timeout_triggers(now_us, grace, status);

        if replay_done {
            self.measuring = false;
            self.recorder.close();
            let dur = self.source.duration().unwrap_or(0) as f64 / 1e6;
            *status = format!("replay finished at {dur:.2}s");
        }
        replay_done
    }

    /// Compare what arrived against what the databases promise.
    ///
    /// Once per step, not per frame: every verdict here is a claim about a
    /// *message's* timing or identity, and the loop above has already folded the
    /// frames into one aggregate per `(bus, id)`. Sweeping the aggregates turns a
    /// step of two hundred frames into one verdict each instead of two hundred
    /// latch writes, and it cannot read an aggregate mid-update the way a check
    /// inside that loop would. `tol_pct` and `grace` are the frontend's
    /// configured tolerances.
    pub(crate) fn check_spec(&mut self, tol_pct: u64, grace: u64) {
        let now = self.sim_t_us;
        // "Dropped" is the only verdict that needs a present tense. Replay runs
        // on the log's own timestamps, where "still going" has no meaning, so
        // only live simulation may call a message gone; pausing is covered
        // separately by the step not running at all. The other three are facts
        // about frames already seen and stay on in every mode.
        let live = matches!(self.mode, Mode::Virtual) && !self.trace_paused;
        /// A verdict bound to one message's class, with the declared and
        /// measured quantities for the report row.
        type Hit = ((u8, u32, bool, Kind), f64, f64);
        let mut hits: Vec<Hit> = Vec::new();
        let mut seen: Vec<((u8, u32, bool), u64)> = Vec::with_capacity(self.aggs.len());
        for (&key, agg) in &self.aggs {
            let (ch, id, ext) = key;
            seen.push((key, agg.last_t_us));
            // No database on this bus means no opinion, not a clean bill.
            let Some(db) = self.channel_dbc(ch) else {
                continue;
            };
            let Some(m) = db.messages.get(&(id, ext)) else {
                hits.push(((ch, id, ext, Kind::Unknown), 0.0, 0.0));
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
                    hits.push(((ch, id, ext, Kind::Dlc), m.dlc as f64, f64::from(agg.len)));
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
                    && cycle_offender(interval, d, tol_pct)
                {
                    hits.push(((ch, id, ext, Kind::Cycle), d as f64, interval as f64));
                }
            }
            if let (true, Some(d)) = (live, declared)
                && missing_offender(now, agg.last_t_us, d, grace)
            {
                hits.push((
                    (ch, id, ext, Kind::Missing),
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

    /// The database loaded on bus `ch`, if any.
    pub(crate) fn channel_dbc(&self, ch: u8) -> Option<&crate::dbc::SymbolTable> {
        self.channels
            .get(ch as usize)
            .and_then(|c| c.dbc.as_deref())
    }

    /// What the database declares for this message: `Some(0)` for an
    /// event-triggered one, `None` when it says nothing at all.
    pub(crate) fn dbc_cycle_us(&self, ch: u8, id: u32) -> Option<u64> {
        self.channel_dbc(ch)
            .and_then(|db| db.message_of(id))
            .and_then(|m| m.cycle_us)
    }

    /// Folds one accepted frame into every bus-side consumer: the trace
    /// ring, the run counter, the per-bus load rollups, the per-message
    /// aggregates and the subscribed-signal caches. `stride` is the sampling
    /// period the frontend currently wants for signal history; everything
    /// else this touches is the bus's own state.
    pub(crate) fn ingest(&mut self, f: CanFrame, stride: u64) {
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
            self.trace.push(f);
            self.trace.enforce_limit(self.trace_limit);
            return;
        }
        let agg = self
            .aggs
            .entry((f.channel, f.id, f.extended))
            .or_insert(MessageAgg {
                id: f.id,
                extended: f.extended,
                channel: f.channel,
                dir: f.dir,
                count: 0,
                rx: 0,
                tx: 0,
                last_t_us: 0,
                cycle_us: 0.0,
                min_us: f64::MAX,
                max_us: 0.0,
                jitter_us: 0.0,
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
            let prev_cycle = agg.cycle_us;
            agg.cycle_us = if agg.count == 1 {
                dt
            } else {
                agg.cycle_us * 0.9 + dt * 0.1
            };
            // Jitter: EMA of the intervals' absolute deviation from the
            // running mean cycle, smoothed like the cycle itself.
            let dev = (dt - prev_cycle).abs();
            agg.jitter_us = if agg.count == 1 {
                dev
            } else {
                agg.jitter_us * 0.9 + dev * 0.1
            };
            if dt < agg.min_us {
                agg.min_us = dt;
            }
            if dt > agg.max_us {
                agg.max_us = dt;
            }
        }
        match f.dir {
            Direction::Rx => agg.rx += 1,
            Direction::Tx => agg.tx += 1,
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
            entry.last_raw = d.raw;
            entry.unit = d.unit;
            entry.type_tag = d.type_tag;
            entry.label = d.label;
            entry.last_update_us = f.t_us;
            if f.t_us >= entry.last_sample_us + stride || entry.history.is_empty() {
                entry.push_sample(f.t_us, d.phys, stride);
            }
        }
        self.trace.push(f);
        self.trace.enforce_limit(self.trace_limit);
    }

    /// The frames a frame carries for signals this run subscribes to, looked
    /// up in the sending bus's database.
    pub(crate) fn subscribed_values(&self, f: &CanFrame) -> Vec<(SigKey, DecodedSignal)> {
        let Some(db) = self
            .channels
            .get(f.channel as usize)
            .and_then(|c| c.dbc.as_ref())
        else {
            return Vec::new();
        };
        db.decode_signals(f)
            .into_iter()
            .filter_map(|d| {
                let key = (f.channel, f.id, f.extended, d.name.clone());
                self.subs.contains_key(&key).then_some((key, d))
            })
            .collect()
    }

    /// Folds one frame into every enabled trigger and acts on edges.
    /// Runs as the first thing that happens to a received frame, so a
    /// trigger that starts a recording still captures the frame that
    /// fired it. Status messages an action wants to show go into
    /// `status` (last write wins, as the status bar only has one line).
    pub(crate) fn eval_triggers(&mut self, f: &CanFrame, status: &mut String) {
        if self.triggers.is_empty() {
            return;
        }
        let mut fired: Vec<(TriggerAction, u64)> = Vec::new();
        for i in 0..self.triggers.len() {
            // Observe under shared borrows first: acting on an edge
            // needs `self` exclusively, and the two cannot overlap.
            let now = {
                let t = &self.triggers[i];
                if !t.enabled {
                    continue;
                }
                match &t.cond {
                    TriggerCond::SignalCross {
                        ch,
                        id,
                        ext,
                        signal,
                        threshold,
                        rising,
                    } => {
                        if f.channel != *ch || f.id != *id || f.extended != *ext || f.is_error() {
                            // Not this message's frame: the level holds.
                            continue;
                        }
                        let Some(db) = self.channel_dbc(*ch) else {
                            continue; // no database on the bus, no opinion
                        };
                        match db.decode_signals(f).into_iter().find(|d| d.name == *signal) {
                            Some(d) => {
                                if *rising {
                                    d.phys >= *threshold
                                } else {
                                    d.phys <= *threshold
                                }
                            }
                            // The condition names a signal the database
                            // lacks -- same courtesy as a missing db.
                            None => continue,
                        }
                    }
                    TriggerCond::IdPresent { ch, id } => {
                        if f.channel != *ch || f.id != *id || f.is_error() {
                            continue;
                        }
                        true // latch: once seen, it stays seen
                    }
                    TriggerCond::ErrorFrame { ch } => {
                        if f.channel != *ch || !f.is_error() {
                            continue;
                        }
                        true
                    }
                    // Not a frame condition: swept once per step against
                    // the aggregates in `eval_timeout_triggers`.
                    // Not frame conditions: swept once per step in
                    // `eval_timeout_triggers` (cycle silence, system vars).
                    TriggerCond::CycleTimeout { .. } | TriggerCond::SysVar { .. } => continue,
                }
            };
            let t = &mut self.triggers[i];
            let was = t.level;
            t.level = now;
            if now && !was {
                t.fired += 1;
                t.last_fire_t_us = f.t_us;
                fired.push((t.action, f.t_us));
            }
        }
        // A reaction send mirrors the triggering frame: decode it once, and
        // every same-named signal of the target entry's message is encoded
        // over the sent payload.
        let mut mirror: HashMap<String, f64> = HashMap::new();
        if fired
            .iter()
            .any(|(a, _)| matches!(a, TriggerAction::Send { .. }))
            && let Some(db) = self.channel_dbc(f.channel)
        {
            for d in db.decode_signals(f) {
                mirror.insert(d.name, d.phys);
            }
        }
        self.run_actions(fired, &mirror, status);
    }

    /// Sweep conditions: evaluated once per measurement step against the
    /// aggregates, not per frame. A message only convicts after it has
    /// been seen once (the Missing verdict takes the same stance), and
    /// the level clears when traffic resumes, so every new dropout is a
    /// fresh edge. `grace` is the spec's missed-cycles tolerance, a
    /// frontend setting the bus does not own.
    pub(crate) fn eval_timeout_triggers(&mut self, now_us: u64, grace: u64, status: &mut String) {
        if self.triggers.is_empty() {
            return;
        }
        let mut fired: Vec<(TriggerAction, u64)> = Vec::new();
        for i in 0..self.triggers.len() {
            // Level conditions swept once per step: cycle silence against
            // the aggregates, system variables against the live registry.
            let level = match &self.triggers[i].cond {
                TriggerCond::CycleTimeout { ch, id } => {
                    if !self.triggers[i].enabled {
                        continue;
                    }
                    Some(self.timeout_silent(*ch, *id, now_us, grace))
                }
                TriggerCond::SysVar {
                    key,
                    threshold,
                    rising,
                } => {
                    if !self.triggers[i].enabled {
                        continue;
                    }
                    let v = self
                        .sysvars
                        .iter()
                        .find(|(d, _)| d.key() == *key)
                        .map(|(_, v)| *v);
                    Some(match v {
                        Some(v) => {
                            if *rising {
                                v >= *threshold
                            } else {
                                v <= *threshold
                            }
                        }
                        // An undefined key reads 0.0 at runtime (see
                        // sys_get): the sweep stays consistent with that.
                        None => {
                            if *rising {
                                0.0 >= *threshold
                            } else {
                                0.0 <= *threshold
                            }
                        }
                    })
                }
                _ => continue,
            };
            let Some(level) = level else {
                continue;
            };
            let t = &mut self.triggers[i];
            let was = t.level;
            t.level = level;
            if level && !was {
                t.fired += 1;
                t.last_fire_t_us = now_us;
                fired.push((t.action, now_us));
            }
        }
        // A timeout has no triggering frame, so a reaction send it fires
        // carries nothing to mirror.
        self.run_actions(fired, &HashMap::new(), status);
    }

    /// The spec's own grace comparison decides silence, so a trigger and
    /// the Dropped verdict can never disagree about the same message.
    fn timeout_silent(&self, ch: u8, id: u32, now_us: u64, grace: u64) -> bool {
        // Trigger conditions carry no frame class yet: either class's
        // aggregate counts.
        let Some(agg) = self.aggs.values().find(|a| a.channel == ch && a.id == id) else {
            return false; // never seen: no opinion, not a dropout
        };
        let Some(declared) = self.dbc_cycle_us(ch, id) else {
            return false; // no database, message, or declared period
        };
        crate::spec::missing_offender(now_us, agg.last_t_us, declared, grace)
    }

    fn run_actions(
        &mut self,
        fired: Vec<(TriggerAction, u64)>,
        mirror: &HashMap<String, f64>,
        status: &mut String,
    ) {
        for (action, at_us) in fired {
            match action {
                TriggerAction::StartRecording => {
                    if self.measuring && !self.recorder.recording {
                        self.recorder.recording = true;
                        let opened_path = self.recorder.open();
                        self.recorder.recording = opened_path.is_ok();
                        *status = match &opened_path {
                            Ok(path) => format!("trigger started recording to {path}"),
                            Err(e) => format!("trigger record failed: {e}"),
                        };
                        if opened_path.is_ok() {
                            // A fresh recording cancels any pending roll.
                            self.post_roll = None;
                            // Oldest first: the file opens with the frames
                            // that came before the trigger fired.
                            for pre in self.pre_buffer.drain(..) {
                                self.recorder.write(&pre);
                            }
                        }
                    }
                }
                TriggerAction::StopRecording => {
                    if self.recorder.recording {
                        // Roll on: the file stays open for the post-roll,
                        // then the step loop closes it once the countdown
                        // is spent.
                        self.post_roll = Some(self.post_frames);
                        *status = "trigger stopping after post-roll".to_string();
                    }
                }
                TriggerAction::Send { ch, id } => {
                    self.send_one_shot(ch, id, at_us, mirror);
                }
                TriggerAction::InsertMarker => {
                    self.markers.push(at_us);
                    if self.markers.len() > self.marker_cap {
                        self.markers.remove(0);
                    }
                }
                TriggerAction::ClearTrace => {
                    self.trace.clear();
                    self.fr_trace.clear();
                    self.publish_trace();
                    // Clearing the view is also the re-arm: appearance
                    // watches fire again on their next occurrence.
                    for t in &mut self.triggers {
                        t.rearm();
                    }
                    *status = "trigger cleared the trace".to_string();
                }
            }
        }
    }

    /// Transmits one frame from the generator entry `(ch, id)`, stamped
    /// `at_us`. The frame goes onto `buf`, so the running tick processes
    /// it exactly like received traffic -- trace, aggregates, load,
    /// recording -- and the trigger evaluator sees it too. That is safe:
    /// every frame-driven condition latches on the frame it matched, so
    /// a rule reacting to its own output cannot loop.
    ///
    /// `mirror` carries the triggering frame's decoded signal values: every
    /// entry whose name matches a signal of the target message is encoded
    /// over the payload, turning a reaction send into a gateway hop. A
    /// periodic send of the same entry passes an empty map and keeps its
    /// own base bytes and waveforms.
    fn send_one_shot(&mut self, ch: u8, id: u32, at_us: u64, mirror: &HashMap<String, f64>) {
        let Some(i) = self
            .tx_list
            .iter()
            .position(|t| t.channel == ch && t.id == id)
        else {
            return; // the generator row is gone: the rule idles
        };
        let (mut data, mut len, mut flags) =
            crate::generator::tx_payload(&self.channels, &self.tx_list[i], at_us);
        let (tch, tid, ext) = (
            self.tx_list[i].channel,
            self.tx_list[i].id,
            self.tx_list[i].extended,
        );
        if !mirror.is_empty()
            && let Some(table) = self.channels.get(tch as usize).and_then(|c| c.dbc.as_ref())
        {
            for (name, value) in mirror {
                crate::generator::encode_mirror(
                    table, tid, name, *value, &mut data, &mut len, &mut flags,
                );
            }
        }
        // 发送反应帧同样更新条目的最近发射载荷，显示与发送一致。
        self.tx_list[i].last_sent = data;
        self.tx_list[i].last_len = len;
        self.buf.push(CanFrame {
            t_us: at_us,
            channel: tch,
            id: tid,
            extended: ext,
            len,
            data,
            dir: crate::can::frame::Direction::Tx,
            flags,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fr_row(t_us: u64, slot: u16) -> crate::trace::FrRow {
        crate::trace::FrRow {
            t_us,
            ab: 2,
            slot,
            cycle: 3,
            payload: vec![0x11; 8],
            header_crc: 0xBEEF,
            flags: 0,
        }
    }

    /// Per-slot tallies behind the Messages FR rows: count, the EMA
    /// cycle from real arrivals, and the last frame's cycle number (so
    /// slots shared across repetitions resolve their frame name).
    #[test]
    fn fr_slot_aggregates_track_arrivals() {
        let mut core = BusCore::new(Vec::new());
        let mut a = fr_row(1_000_000, 10);
        a.cycle = 0;
        let mut b = fr_row(1_005_000, 10);
        b.cycle = 4; // rep-4 frame's next occurrence
        core.ingest_fr_row(a);
        core.ingest_fr_row(b);

        core.publish_trace();
        let snap = core.snapshot();
        assert_eq!(snap.fr_aggs.len(), 1);
        let agg = &snap.fr_aggs[0];
        assert_eq!(agg.slot, 10);
        assert_eq!(agg.count, 2);
        assert_eq!(agg.last_cycle, 4, "the name lookup cycle follows arrivals");
        assert_eq!(agg.cycle_us, 5000.0, "first interval seeds the EMA");
    }

    /// The FR ring publishes through the same lifecycle as the CAN ring:
    /// rows stamp against the sim clock when the source had no time of
    /// its own, and the snapshot carries the published view.
    #[test]
    fn fr_rows_ingest_publish_and_stamp() {
        let mut core = BusCore::new(Vec::new());
        core.sim_t_us = 1_500;
        core.ingest_fr_row(fr_row(0, 10));
        core.ingest_fr_row(fr_row(2_000, 11));
        core.publish_trace();

        let snap = core.snapshot();
        assert_eq!(snap.fr_trace.len(), 2);
        assert_eq!(
            snap.fr_trace[0].t_us, 1_500,
            "a zero time stamps against the sim clock"
        );
        assert_eq!(snap.fr_trace[1].t_us, 2_000, "a real time survives");
        assert_eq!(snap.fr_trace[1].slot, 11);
        assert_eq!(snap.fr_dropped, 0);

        // A clear resets both ring and view.
        core.fr_trace.clear();
        core.publish_trace();
        assert!(core.snapshot().fr_trace.is_empty());
    }

    /// The FR ring shares the trace limit: rows past it trim head-first
    /// and the loss is counted for the window's note.
    #[test]
    fn fr_rows_honor_the_trace_limit() {
        let mut core = BusCore::new(Vec::new());
        core.trace_limit = 3;
        for slot in 1..=6u16 {
            core.ingest_fr_row(fr_row(0, slot));
        }
        core.publish_trace();
        let snap = core.snapshot();
        assert_eq!(snap.fr_trace.len(), 3);
        assert_eq!(snap.fr_trace[0].slot, 4, "the newest survive");
        assert_eq!(snap.fr_dropped, 3);
    }

    /// A watch attach with a missing description file fails cleanly and
    /// leaves nothing attached; detaching without a watch is a no-op.
    #[test]
    fn fr_watch_attach_failures_leave_no_partial_state() {
        let mut core = BusCore::new(Vec::new());
        let mut status = String::new();
        core.handle(
            BusCommand::SetFrWatch {
                channel_index: Some(0),
                fibex_path: "target/definitely-missing.fibex".into(),
            },
            &mut status,
        );
        assert!(status.contains("失败"), "attach failure reports: {status}");
        assert!(core.hw.fr_watch.is_none());

        core.handle(
            BusCommand::SetFrWatch {
                channel_index: None,
                fibex_path: String::new(),
            },
            &mut status,
        );
        assert!(status.contains("断开"));
        assert!(core.hw.fr_watch.is_none());
    }

    /// A description without FlexRay cluster parameters is refused
    /// before any driver call: a DBC parses fine as text but is no
    /// cluster spec.
    #[test]
    fn fr_watch_refuses_a_non_fibex_description() {
        let mut core = BusCore::new(Vec::new());
        let mut status = String::new();
        core.handle(
            BusCommand::SetFrWatch {
                channel_index: Some(0),
                fibex_path: "assets/sample.dbc".into(),
            },
            &mut status,
        );
        assert!(status.contains("失败"), "refusal reports: {status}");
        assert!(core.hw.fr_watch.is_none());
    }
}
