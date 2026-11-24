//! The bus core: everything that must keep running at the bus's own rate,
//! independent of the frontend's frame loop. Commands arrive from the
//! frontend, snapshots go back; the core lives on its own thread.

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;

use crate::aggregate::MessageAgg;
use crate::app::{Mode, SAMPLE_INTERVAL_US, TRACE_LIMIT};
use crate::can::frame::{CanFrame, Direction};
use crate::channel::NodeRole;
use crate::dbc::DecodedSignal;
use crate::generator::{FrTxMsg, TxMsg};
use crate::observe::{SigKey, Subscription};
use crate::script::HostInput;
use crate::source::FrameSource;
use crate::spec::{Kind, cycle_offender, dlc_offender, missing_offender};
use crate::trigger::{TriggerAction, TriggerCond};

mod core_channels;
mod core_command;
mod core_eth;
mod core_models;
mod core_publish;
mod core_replay;
mod core_run;
mod core_send;
mod core_someip;
mod core_triggers;
mod core_view;

#[cfg(test)]
mod tests;

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
/// Frames a node may inject into the current step's walk. A handler that
/// sends the frame it receives re-enters itself; without a bound that is an
/// infinite same-step loop that freezes the core. 1024 keeps legitimate
/// reaction chains intact and turns a runaway script into one warning.
pub(crate) const MAX_NODE_REACTIONS_PER_STEP: usize = 1024;

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
    /// Why the wire last refused this bus's transmit, and how many writes it
    /// has refused. `None` while every write has been taken.
    pub tx_fail: Option<(String, u64)>,
}

/// What the run's frames do about the wire, in the toolbar switch's own words:
/// `simulation` with nothing attached, `real bus · N` when N buses ride the
/// wire, `simulation · hardware parked` when the attachments are held back.
/// The status bar and the start-of-run line say it together. "(virtual)" used
/// to stand in for the first of these -- it named the clock's source -- and on
/// a bench with a channel attached it read as "none of this is real hardware".
pub fn wire_note(attached: usize, real_bus: bool) -> String {
    match (attached, real_bus) {
        (0, _) => "simulation".to_string(),
        (n, true) => format!("real bus · {n}"),
        (_, false) => "simulation · hardware parked".to_string(),
    }
}

/// The FlexRay watch as the frontend sees it: identity only --
/// the port itself stays core-side.
#[derive(Clone, Debug, PartialEq)]
pub struct FrWatchView {
    /// The FlexRay bus the watch feeds; rows carry it so several ports stay
    /// separable.
    pub bus: u8,
    pub channel_index: i32,
    pub fibex_path: String,
}

/// What the frontend may ask the bus to do. One variant per transport
/// action, deliberately carrying no UI state -- no picked file paths, no
/// window selections; the frontend resolves those before asking.
pub enum BusCommand {
    /// Fresh virtual run: swap in a new virtual source, blank every
    /// run-scored counter, measure against the wall clock.
    StartVirtual,
    /// Stop measuring and close the recorder.
    Stop,
    /// Freeze or unfreeze the trace views; frozen arrivals stamp at the
    /// freeze instant.
    SetTracePaused(bool),
    /// Advance a replay by this much log time in one immediate step.
    /// Forward only; the observers see the frames through the normal step.
    ReplayStep {
        delta_us: u64,
    },
    /// Play forward until the playhead reaches this absolute log time,
    /// then pause. A target behind the playhead is refused.
    ReplayPlayTo {
        target_us: u64,
    },
    /// Play forward for this much log time from the current playhead, then
    /// pause.
    ReplayPlayFor {
        duration_us: u64,
    },
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
    /// Fires `on panel_action("name")` handlers on the next step. Queued so
    /// the handler's sends join a normal step instead of the command path.
    /// `value` is the control's argument (buttons send theirs, a Numeric
    /// sends its new setpoint).
    PanelAction {
        name: String,
        value: Option<f64>,
    },
    /// Writes one signal from a Panel setpoint, addressed by the observer key
    /// so every protocol shares the command. CAN updates/creates the generator
    /// entry and injects one frame; FlexRay encodes the slot's payload and
    /// injects one frame (writing it to a live FR port first); Ethernet
    /// encodes the frame's PDU payload and injects one session TX frame.
    PanelWriteSignal {
        key: SigKey,
        value: f64,
    },
    /// A Button preset that sends one CAN frame once: raw bytes on the wire
    /// (and into this session), not through a generator entry. `physical`
    /// encodes `signals` against the catalog message and ignores `data`.
    PanelSendFrame {
        channel: u8,
        id: u32,
        extended: bool,
        data: Vec<u8>,
        physical: bool,
        signals: std::collections::BTreeMap<String, f64>,
    },
    /// One-shot FlexRay frame into this session (and onto a live FR port).
    PanelSendFrFrame {
        bus: u8,
        slot: u16,
        data: Vec<u8>,
        physical: bool,
        signals: std::collections::BTreeMap<String, f64>,
    },
    /// One-shot Ethernet frame into this session's flow ring (no port).
    PanelSendEthFrame {
        bus: u8,
        frame: String,
        data: Vec<u8>,
        physical: bool,
        signals: std::collections::BTreeMap<String, f64>,
    },
    PanelSendSomeIp {
        bus: u8,
        service: u16,
        method: u16,
        client: u16,
        session: u16,
        message_type: u8,
        data: Vec<u8>,
        values: std::collections::BTreeMap<String, f64>,
        texts: std::collections::BTreeMap<String, String>,
        transport: String,
        host: String,
        port: u16,
    },
    /// Resets the per-message counters the Messages / Statistics windows
    /// read. Load, spec memory and the trace are untouched.
    /// Replace the SOME/IP service catalog used to decode service fields.
    SetSomeIpServices(crate::eth::arxml_service::ServiceCatalog),
    ClearAggregates,
    /// Select the bus kind the next start will run (Simulation / Replay).
    SetRunMode(crate::app::Mode),
    /// Add a bus with the sample DBC path, load it, and pre-populate its
    /// generator.
    AddChannel,
    /// Add a bus with no description: raw monitoring until files are bound.
    AddEmptyChannel,
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
    /// Replaces the FlexRay descriptions the core decodes arriving frames
    /// against, one per bus, with the source file each came from. The whole
    /// map moves at once so the core's view can never hold a bus the frontend
    /// dropped.
    SetFrDbs {
        dbs: std::collections::BTreeMap<u8, Arc<crate::fr_db::FrDb>>,
        sources: std::collections::BTreeMap<u8, Vec<crate::model::ClusterSource>>,
    },
    /// Replaces the Ethernet matrices the core decodes captured frames
    /// against, one per Ethernet bus, with their source files.
    SetEthDbs {
        dbs: std::collections::BTreeMap<u8, Arc<crate::eth::db::EthDb>>,
        sources: std::collections::BTreeMap<u8, Vec<crate::model::ClusterSource>>,
    },
    /// Whether an ARXML `FRAME-LENGTH` counts the 14-byte L2 header. The
    /// capture's payload never does, so this decides how a definition's
    /// stated length compares with an arriving flow.
    SetEthLengthIncludesHeader(bool),
    /// Replaces one CAN channel's unified model with a matrix read straight
    /// from ARXML. The DBC table remains the merge base, but container/mux/E2E
    /// structure exists only in the unified model, so the reader's output has
    /// to reach the core instead of being re-derived from the table.
    SetChannelModel {
        ch: u8,
        comm: Option<Arc<crate::model::Comm>>,
        sources: Vec<crate::model::ClusterSource>,
    },
    /// Restores the persisted global cluster ids from a project file.
    SetClusterIds {
        next: u32,
        can: Vec<u32>,
        fr: std::collections::BTreeMap<u8, u32>,
        eth: std::collections::BTreeMap<u8, u32>,
    },
    /// Message rows the frontend has expanded, so the snapshot can carry
    /// their decoded signal children without the frontend decoding again.
    SetExpandedMessages(Vec<String>),
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
    /// Run the entry's FD data phase at the bus's data rate. Only
    /// meaningful while the entry is FD; dropping FD clears it.
    SetEntryBrs {
        ch: u8,
        id: u32,
        brs: bool,
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
    /// The FlexRay twin of the entry commands above, keyed by `(bus, slot)`:
    /// one entry per slot per cluster, the same identity a window's hand-picked
    /// set and `fr_sig` use. A slot is addressed, not identified by a message
    /// id, so the two keying spaces stay apart. `node` is the ECU the frontend
    /// resolved from the description as this frame's sender -- the grouping the
    /// Network tree edits under.
    AddFrEntry {
        bus: u8,
        slot: u16,
        node: String,
    },
    /// Drop the entry, payload, sources and schedule with it.
    RemoveFrEntry {
        bus: u8,
        slot: u16,
    },
    SetFrEntryActive {
        bus: u8,
        slot: u16,
        on: bool,
    },
    /// Send period in µs; 0 = only on `SendFrNow`.
    SetFrEntryCycle {
        bus: u8,
        slot: u16,
        cycle_us: u64,
    },
    SetFrEntryHex {
        bus: u8,
        slot: u16,
        text: String,
    },
    SetFrEntrySource {
        bus: u8,
        slot: u16,
        src: crate::sim::ValueSrc,
    },
    /// Adds one line to the Write ring without touching the status bar. The
    /// frontend's own news -- a failed export, a refused operation, a project
    /// that half-loaded -- has no other way into the log, and one line on the
    /// bar is the only place it currently appears: whatever writes next erases
    /// it, and a message produced during a synchronous load is never drawn at
    /// all.
    LogStatus {
        kind: WriteKind,
        text: String,
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
    /// Replace the derived-channel definitions. The core owns their sampled
    /// subscriptions from here on; unchanged definitions keep their history.
    SetDerived(Vec<crate::derived::DerivedDef>),
    /// Open `path` and replay it: generators stand down for the ids the
    /// log carries, a fresh replay source replaces the old input at
    /// `speed`, and run-scored state resets. `speed` is the frontend's
    /// remembered multiplier, passed at start like the other policy
    /// knobs.
    StartReplay {
        path: String,
        speed: f64,
    },
    /// 整张回放通道映射：日志里的通道编号 → 本工程的第几路（`None` = 那一路
    /// 不回放）。一次给全表而不是给一行，是为了让"工程文件说什么就恢复什么"
    /// 这条规则成立——一个没有映射的工程必须能把上一份工程留下的映射清掉。
    /// 正在回放时下一条命令被抽干就生效，已经进环的帧不动。
    SetReplayMap(crate::replay_map::ReplayMap),
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
    /// Sets the bus-name counter so the next added bus keeps counting from
    /// where a restored project left off.
    SetBusCounter(usize),
    /// The dated ASC file the recorder derives its name from. Only read
    /// when recording actually arms.
    SetRecordPath(String),
    /// Whitelist for recorded files: the CAN `(id, extended)` frames and the
    /// FlexRay `(bus, slot)` arrivals that land in the log (both halves empty =
    /// record everything). Gates the file, the pre-trigger context, and post-roll
    /// counting; never the trace, aggregates, or the spec.
    SetRecordFilter {
        filter: crate::recorder::RecordFilter,
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
    /// Attach/detach a live Npcap Ethernet port.
    AttachEthPort {
        bus: u8,
        device: String,
    },
    DetachEthPort {
        bus: u8,
    },
    /// Recovery after a USB replug: close Vector ports, reopen the XL
    /// driver so it rescans the adapter, and let the frontend re-enumerate.
    /// Other drivers' attachments are untouched.
    RefreshHardware,
    /// Attach (or move) the FlexRay watch to a Vector channel,
    /// configured from the FIBEX description at `fibex_path`; `None`
    /// detaches. The watch carries RX and TX; a passive fallback cannot
    /// transmit.
    SetFrWatch {
        /// The FlexRay bus this watch feeds. One bus today; the index is
        /// threaded through now so several ports can be watched at once.
        bus: u8,
        channel_index: Option<i32>,
        fibex_path: String,
    },
    /// Deletes one FlexRay 路 outright: its watch, its cluster description,
    /// the frames it received, its tallies and its send entries. Unlike CAN's
    /// `RemoveChannel` this **does not renumber the survivors** -- see
    /// [`BusCore::remove_fr_bus`].
    RemoveFrBus {
        bus: u8,
        /// The 路 as the operator named it, resolved by the frontend: the bar's
        /// confirmation has to say which one went, and only the frontend holds
        /// user names for clusters.
        label: String,
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
        /// 角色开关控制。
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
    /// Clone replay block `id` as a new disabled block: same log, node
    /// filter and id filter, name suffixed with " copy". Editing the copy
    /// is the cheap path to id-filter variants.
    DuplicateReplayBlock {
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
    /// Blank every bus's database and drop the generator and the signal
    /// subscriptions with it -- the bus half of "new project".
    #[cfg(test)]
    ClearDatabases,
    /// Latch-free the specification report: every row forgets it fired.
    /// The next run replaces the rows anyway; this is the in-run reset.
    ClearSpec,
    /// Transmit a single frame from the generator entry `(ch, id)` right
    /// now: its own base bytes and waveform values, its schedule and
    /// active flag untouched. Delivered on the next step of a running bus;
    /// a request made while stopped is dropped.
    #[cfg(test)]
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
        /// New rules start disarmed so a half-edited condition cannot act.
        enabled: bool,
    },
    /// Drop trigger `index`.
    RemoveTrigger {
        index: usize,
    },
    /// Clone trigger `index` onto the end of the list: condition, action
    /// and limits all copy; the copy starts disarmed with a clean latch
    /// and zeroed fire count.
    DuplicateTrigger {
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
        cooldown_us: u64,
        max_fires: u32,
        pre_frames: Option<usize>,
        post_frames: Option<u32>,
    },
    /// Re-arm one rule: clear its latch and fire count without touching
    /// the other rules.
    RearmTrigger {
        index: usize,
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
    /// Change only a SysVar's default BandSet binding; the live value and
    /// definition stay untouched.
    SetSysVarBandSet {
        namespace: String,
        name: String,
        band_set_id: Option<String>,
    },
    /// Empties the Write window's ring.
    #[cfg(test)]
    ClearWrite,
    /// CANoe-style bus mode: `real` connects every attachment to its
    /// adapter (RX feeds the bus, directed TX goes to the wire);
    /// `!real` (Simulated) parks them while keeping the configuration.
    SetBusMode {
        real: bool,
    },
}

/// The synthetic stream id system variables publish under: an id no node
/// can ever mint (`node_counter` counts up from 1), so a sysvar stream
/// cannot collide with a script's derived signals.
pub(crate) const SYSVAR_STREAM_ID: u64 = u64::MAX;

/// Oldest Write-window lines fall off once the list outgrows this.
pub(crate) const WRITE_LOG_CAP: usize = 1000;

/// How many trigger edges the replay bar can walk back. Runtime only.
pub(crate) const TRIGGER_FIRE_CAP: usize = 64;

/// One edge that passed the cooldown/limit gate and is about to run.
struct FiredTrigger {
    index: usize,
    action: TriggerAction,
    at_us: u64,
}

/// Upper bound on one Button preset's signal writes.
/// CAN FD payload lengths a one-shot frame may carry: classic sizes plus the
/// FD DLC ladder. A length in between is refused rather than silently padded.
fn panel_frame_length_ok(len: usize) -> bool {
    matches!(len, 0..=8 | 12 | 16 | 20 | 24 | 32 | 48 | 64)
}

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

/// One display label for a numeric system-variable value. Labels are
/// presentation metadata only: scripts, triggers and observers still use the
/// numeric value.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct SysVarLabel {
    pub value: f64,
    pub label: String,
}

/// A named view over a numeric system variable. Widgets reference the stable
/// `id`; the `is_default` set is used when a widget has no explicit choice.
/// Bands are ordered by upper bound and the last band is open-ended.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SysVarLabelSet {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub is_default: bool,
    pub bands: Vec<crate::band::Band>,
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
    /// Named display-label sets; legacy, migrated into `App::band_sets`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub label_sets: Vec<SysVarLabelSet>,
    /// Project BandSet used by default for this variable.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub band_set_id: Option<String>,
    /// Pre-multi-set text labels, read for old projects only.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub labels: Vec<SysVarLabel>,
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

    /// Multi-set labels for the frontend, with the old single `labels`
    /// field folded into a default text-only set.
    pub fn display_label_sets(&self) -> Vec<SysVarLabelSet> {
        if !self.label_sets.is_empty() {
            return self.label_sets.clone();
        }
        if self.labels.is_empty() {
            return Vec::new();
        }
        vec![SysVarLabelSet {
            id: "default".to_string(),
            name: "Default".to_string(),
            is_default: true,
            bands: self
                .labels
                .iter()
                .map(|label| crate::band::Band {
                    upper: Some(label.value),
                    inclusive: true,
                    text: label.label.clone(),
                    color: None,
                })
                .collect(),
        }]
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
    pub file_path: Option<String>,
    /// 绑定的 DBC 节点 (总线, 节点名)（节点窗口按此聚合脚本）。
    pub attached: Option<(u8, String)>,
    /// Runtime present and error-free (while measuring).
    pub running: bool,
    /// A handler failed: the node is stopped until restart or edit.
    pub errored: bool,
    /// Last compile failure, with its source position.
    pub issue: Option<crate::node::ScriptIssue>,
    /// `test(...)` results from the last stopped run.
    pub tests: Vec<crate::script::TestResult>,
    /// `on panel_action("…")` names the compiled script declares.
    pub panel_actions: Vec<String>,
    /// Oldest-first log lines (print output and errors).
    pub log: Vec<String>,
}

#[allow(clippy::too_many_arguments)]
fn send_someip_tcp(
    host: &str,
    port: u16,
    service: u16,
    method: u16,
    client: u16,
    session: u16,
    message_type: u8,
    payload: &[u8],
) -> Result<(), String> {
    send_someip_tcp_rc(
        host,
        port,
        service,
        method,
        client,
        session,
        message_type,
        0,
        payload,
    )
}

#[allow(clippy::too_many_arguments)]
fn send_someip_tcp_rc(
    host: &str,
    port: u16,
    service: u16,
    method: u16,
    client: u16,
    session: u16,
    message_type: u8,
    return_code: u8,
    payload: &[u8],
) -> Result<(), String> {
    use std::io::Write;
    let address = format!("{host}:{port}");
    let mut stream = std::net::TcpStream::connect(&address)
        .map_err(|error| format!("SOME/IP TCP {address}: {error}"))?;
    let _ = stream.set_write_timeout(Some(std::time::Duration::from_millis(500)));
    stream
        .write_all(&crate::eth::someip::build_message_rc(
            service,
            method,
            client,
            session,
            message_type,
            return_code,
            payload,
        ))
        .map_err(|error| format!("SOME/IP TCP {address}: {error}"))
}

/// What the frontend may see of the bus: one immutable, frame-shaped
/// bundle of the read-only facts, published behind an Arc swap.
#[derive(Clone, Debug, Default)]
pub struct Snapshot {
    /// Frames received this run.
    pub frame_counter: u64,
    /// Current trace-ring length.
    #[cfg(test)]
    pub trace_len: usize,
    /// Live signal subscriptions.
    #[cfg(test)]
    pub sub_count: usize,
    /// Replay playhead and log length in µs, when a log with a known
    /// position is loaded.
    pub replay: Option<(u64, u64)>,
    /// 当前生效的回放通道映射：日志里的通道编号 → 本工程的第几路。
    pub replay_map: crate::replay_map::ReplayMap,
    /// 本次回放见过的文件 CAN 通道与各自的条数，按**文件里的**编号升序。
    /// Buses 窗口的映射表用它说出"这个文件里有这几路"。
    pub replay_census_can: Vec<(u8, u64)>,
    /// 同上，FlexRay 的 cluster 号。
    pub replay_census_fr: Vec<(u8, u64)>,
    /// 同上，以太网抓包里的 channel。
    pub replay_census_eth: Vec<(u8, u64)>,
    /// How many buses exist (frontends key window state off this).
    pub channel_count: usize,
    /// Global cluster ids, written into schema v2 project files.
    pub cluster_ids: WebClusterIds,
    /// The unified model's own frame names and signal counts, keyed like a
    /// CAN Messages row. The tables prefer these over the DBC table: the
    /// model keeps standard and extended ids apart and counts container
    /// children the table never held.
    pub model_frames: std::collections::BTreeMap<(u8, u32, bool), ModelFrameView>,
    /// E2E counter continuity per CAN message; absent when the message is
    /// not protected or its profile layout is not checked.
    pub e2e: std::collections::BTreeMap<(u8, u32, bool), E2eView>,
    /// The same for FlexRay slots and Ethernet frames.
    pub fr_e2e: std::collections::BTreeMap<(u8, u16), E2eView>,
    pub eth_e2e: std::collections::BTreeMap<String, E2eView>,
    /// The same for FlexRay slots, keyed by `(bus, slot)`.
    pub fr_model_frames: std::collections::BTreeMap<(u8, u16), ModelFrameView>,
    /// The same for Ethernet frames, keyed by frame name.
    pub eth_model_frames: std::collections::BTreeMap<String, ModelFrameView>,
    /// Decoded children for the Messages rows the frontend has expanded,
    /// keyed by the row's stable message key. CAN only for now; FR/ETH rows
    /// still decode in the frontend.
    pub expanded_signals: std::collections::BTreeMap<String, Vec<ExpandedSignal>>,
    /// Commands the core has processed; a threaded frontend watches it to
    /// tell whether its sent commands have been applied yet.
    pub laps: u64,
    /// Generation of the last Vector driver reopen. The Buses dropdown
    /// clears its cache when this changes, not when Refresh is clicked.
    pub hw_refresh: u64,
    /// The counter that mints "CAN{n}" for new buses; the frontend reads it
    /// for saves and pins it back on project restore.
    pub bus_counter: usize,
    /// The simulation clock, for frontends that anchor plots on bus time.
    pub sim_t_us: u64,
    /// The violation monitor's latched rows, for the report export.
    pub spec: crate::spec::Spec,
    /// Per-bus load rollups as of the last publish; the Bus Statistics
    /// window's whole data source.
    pub bus_loads: Arc<Vec<crate::load::BusLoad>>,
    /// Per-FlexRay-cluster occupancy rollups as of the last publish, keyed by
    /// cluster index (the number the tables print as `FR{n}`). The same window
    /// the CAN rows use, with a static slot's wire time as the unit.
    #[cfg(test)]
    pub fr_loads: Arc<std::collections::BTreeMap<u8, crate::load::FrLoad>>,
    /// Simulation nodes as of the last publish: identity, source, state
    /// and log. The Nodes window's data source and the project save.
    pub nodes: Arc<Vec<NodeView>>,
    /// One view per replay block, as of the last publish.
    pub blocks: Vec<ReplayBlockView>,
    /// Derived-signal streams opened by `emit_value` so far: key plus the
    /// owning node's display name.
    #[cfg(test)]
    pub emitted: Vec<(SigKey, String)>,
    /// The system variables as of this frame: definition plus live value.
    /// The manager window's data source and the project save.
    pub sysvars: Vec<SysVarView>,
    /// The Write window's ring as of the last publish, oldest first.
    pub write: Arc<Vec<WriteLine>>,
    /// Hardware attachments: one per wired-up bus.
    pub hw: Vec<HwBusView>,
    /// The FlexRay watches, one per watched bus, in bus order.
    pub fr_watches: Vec<FrWatchView>,
    /// Whether the attachments are wire-connected (Real bus) or parked
    /// (Simulated): the toolbar bus-mode switch reads this.
    pub real_bus: bool,
    /// The user's trigger rules, judged on the bus; the frontend saves
    /// them with the project.
    pub triggers: Vec<crate::trigger::Trigger>,
    /// Recent trigger edges (runtime only, oldest first). The replay bar's
    /// previous/next navigation walks these; each also became one Write
    /// line.
    pub trigger_fires: Vec<crate::trigger::TriggerFire>,
    /// Derived lanes whose expression could not be sampled, by definition
    /// id. The editor shows the core's reason instead of leaving an empty
    /// curve unexplained.
    pub derived_errors: std::collections::BTreeMap<String, String>,
    /// The file the recorder derived its last output from; the replay
    /// start button falls back to it.
    pub last_record: String,
    /// Matching Ethernet PCAP written alongside the main recording (or the
    /// main file itself when the recording format is PCAP).
    pub last_eth_record: String,
    /// One entry per bus: identity, timing declarations and the shared
    /// (immutable) database. Static configuration as of this frame.
    pub channels: Vec<ChannelView>,
    /// One record per (bus, id) seen this run, behind the Messages /
    /// Statistics views and their exports.
    pub aggs: Vec<MessageAgg>,
    /// One record per FlexRay slot seen this run, slot-sorted -- the
    /// Messages window's FR rows.
    pub fr_aggs: Vec<crate::aggregate::FrFrameAgg>,
    /// One record per captured Ethernet flow.
    pub eth_aggs: Vec<crate::aggregate::EthAgg>,
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
    /// The same for the FlexRay slots this tool fills.
    pub fr_tx: Vec<FrTxView>,
    /// The trace ring, published once per step that ingested frames and
    /// shared by Arc: cloning the snapshot copies a pointer, not 50k
    /// frames. Read-only for the frontend.
    pub trace: std::sync::Arc<crate::trace::TraceView>,
    /// FlexRay frames as of the last publish, oldest first -- the Trace
    /// window's FR section. Shared like `trace`.
    pub fr_trace: std::sync::Arc<crate::trace::FrTraceView>,
    /// Ethernet frames as of the last publish, oldest first, byte-capped.
    pub eth_trace: std::sync::Arc<crate::eth::EthTraceView>,
    /// Decoded SOME/IP service events, oldest first.
    pub someip: Vec<crate::eth::someip::SomeIpEvent>,
    /// Remote eventgroup subscriptions accepted by the local IL.
    pub someip_subscriptions: Vec<SomeIpSubscriptionView>,
    /// Live Npcap device names by Ethernet bus.
    pub eth_attached: std::collections::BTreeMap<u8, String>,
    /// FR rows the ring trimmed from its head since the run began.
    #[cfg(test)]
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

/// Persisted identity of every cluster, exposed so a project save can write
/// the same ids back.
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WebClusterIds {
    pub next: u32,
    pub can: Vec<u32>,
    pub fr: std::collections::BTreeMap<u8, u32>,
    pub eth: std::collections::BTreeMap<u8, u32>,
}

/// One decoded child row of an expanded Messages entry.
#[derive(Clone, Debug, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExpandedSignal {
    pub name: String,
    pub value: String,
    pub raw: i64,
    pub unit: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

/// E2E counter continuity for one CAN message. The CRC is not evaluated;
/// the counter is the part a reader can check without keys.
#[derive(Clone, Debug, Default)]
pub struct E2eView {
    pub profile: u32,
    pub data_id: Option<u32>,
    pub counter: Option<u8>,
    pub crc: Option<u64>,
    pub crc_ok: Option<bool>,
    pub crc_checked: u64,
    pub crc_fail: u64,
    pub frames: u64,
    pub repeats: u64,
    pub gaps: u64,
    pub jumps: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum E2eChange {
    First,
    Ok,
    Repeat,
    Gap,
    Jump,
}

fn note_e2e(entry: &mut E2eView, reading: crate::model::E2eReading) -> E2eChange {
    entry.profile = reading.profile;
    entry.data_id = reading.data_id;
    if let Some(crc) = reading.crc {
        entry.crc = Some(crc);
        entry.crc_checked += 1;
        entry.crc_ok = reading.crc_ok;
        if reading.crc_ok == Some(false) {
            entry.crc_fail += 1;
        }
    }
    entry.frames += 1;
    let change = if let Some(last) = entry.counter {
        let delta = (reading.counter + 16 - last) % 16;
        match delta {
            0 => {
                entry.repeats += 1;
                E2eChange::Repeat
            }
            1 => E2eChange::Ok,
            2 | 3 => {
                entry.gaps += 1;
                E2eChange::Gap
            }
            _ => {
                entry.jumps += 1;
                E2eChange::Jump
            }
        }
    } else {
        E2eChange::First
    };
    entry.counter = Some(reading.counter);
    change
}

/// What the unified model says about one CAN frame, for the Messages and
/// Statistics tables.
#[derive(Clone, Debug, Default)]
pub struct ModelFrameView {
    pub name: String,
    pub signal_count: usize,
}

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
    /// DBC node name -> unified `EcuId` when the ARXML model names the same
    /// ECU. The role map stays the simulation truth; this is the identity
    /// bridge so DBC nodes and ARXML ECUs meet.
    pub node_ecus: std::collections::BTreeMap<String, u32>,
    pub dbc: Option<std::sync::Arc<crate::dbc::SymbolTable>>,
}

impl ChannelView {
    /// The role declared for `node`; every node without an entry is
    /// `Absent` -- the restbus default. Mirrors `BusCore::role_in` so
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
            .field("node_ecus", &self.node_ecus)
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
    /// 角色开关是否允许发送（绑定节点的角色为「模拟」，或条目无节点归属）。
    /// 开关关闭时即便 active 也不发送——UI 用它区分「条目自己关着」与「所属节点的角色不是模拟」。
    pub gate_open: bool,
    pub fd: bool,
    /// FD data phase runs at the bus's data bitrate.
    pub brs: bool,
    pub cycle_us: u64,
    /// Base payload as hex text -- also the no-DBC message's editable box.
    pub data_text: String,
    /// The payload that goes out this frame: base with every driven
    /// source laid over it.
    pub sent_text: String,
    pub srcs: Vec<crate::sim::ValueSrc>,
    pub muted: bool,
}

/// One FlexRay generator entry as the frontend sees it. The twin of
/// [`TxView`] without the CAN-only parts: a slot has no transmitter role to
/// gate it, no FD flag, and its payload is however long the frame is.
#[derive(Clone, Debug)]
pub struct FrTxView {
    pub bus: u8,
    pub slot: u16,
    pub name: String,
    /// The ECU the description declares as this frame's sender; the Network
    /// window lists the row under it, as a CAN row sits under its transmitter.
    pub node: String,
    pub active: bool,
    pub cycle_us: u64,
    /// Base payload as hex text -- what the row's box edits.
    pub data_text: String,
    /// The payload of the frame that last went out, or the base if none has.
    pub sent_text: String,
    pub srcs: Vec<crate::sim::ValueSrc>,
    /// The replayed log carries this slot itself, so the entry stands down for
    /// the run.
    pub muted: bool,
    /// The bus's cluster description schedules no frame for this slot, so the
    /// payload goes out as typed and nothing decodes it.
    pub undescribed: bool,
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
}

/// What each log carries, so a generator entry whose traffic the replay
/// itself provides stands down for the run. CAN by `(channel, id)`, FlexRay
/// by `(bus, slot)`; the two numbering spaces stay apart, because `(0, 5)`
/// means both a CAN id and a slot.
type LogIds = (
    std::collections::HashSet<(u8, u32)>,
    std::collections::HashSet<(u8, u16)>,
);

fn scan_log_ids(path: &std::path::Path) -> Option<LogIds> {
    let mut stream = crate::log::open_stream(path).ok()?;
    let mut ids = std::collections::HashSet::new();
    let mut fr_ids = std::collections::HashSet::new();
    let mut rows = Vec::new();
    let mut take_rows = |stream: &mut dyn crate::source::FrameStream, rows: &mut Vec<_>| {
        stream.poll_fr_rows(u64::MAX, rows);
        for r in rows.drain(..) {
            fr_ids.insert((r.bus, r.slot));
        }
    };
    while let Some(f) = stream.next_frame() {
        ids.insert((f.channel, f.id));
        // Drained as the walk goes rather than once at the end: a FlexRay-heavy
        // log would otherwise pile every row into memory beside the ring the
        // replay itself is already holding.
        if rows.len() >= 4_096 {
            take_rows(&mut *stream, &mut rows);
        }
    }
    take_rows(&mut *stream, &mut rows);
    Some((ids, fr_ids))
}

/// Status-line note about the FD state of a fresh hardware attach: active
/// data-phase bitrate, classic-mode degradation, or nothing for classic buses.
fn hw_fd_note(fd_data_kbps: Option<u32>, fd_active: bool, fd_error: Option<&str>) -> String {
    match (fd_data_kbps, fd_active) {
        (Some(k), true) => format!(", FD data phase {k} kbit/s"),
        (Some(0), false) => String::new(),
        (Some(_), false) => match fd_error {
            Some(e) => format!(", classic mode ({e})"),
            None => ", classic mode (FD preset mismatch or unsupported)".to_string(),
        },
        (None, _) => String::new(),
    }
}

/// The simulation half of the application: mode, generators, trace, the
/// recorder and the observers. The frontend talks to it through commands
/// and snapshots only.
type ModelHandle = (
    crate::model::ClusterId,
    Arc<crate::model::Comm>,
    Arc<crate::model::PlanCache>,
);

/// The decoded model and the description it was built from, kept together so
/// a table replacement can never leave a stale plan pretending to match.
pub(crate) struct ProtocolRuntime<D> {
    pub comm: Arc<crate::model::Comm>,
    pub plans: Arc<crate::model::PlanCache>,
    pub db: Arc<D>,
}

pub(crate) type FrRuntime = ProtocolRuntime<crate::fr_db::FrDb>;
pub(crate) type EthRuntime = ProtocolRuntime<crate::eth::db::EthDb>;

impl<D> std::ops::Deref for ProtocolRuntime<D> {
    type Target = crate::model::Comm;

    fn deref(&self) -> &Self::Target {
        &self.comm
    }
}

/// The raw wire event shared by recorder/trigger/export boundaries. Decoding
/// still uses the per-protocol plan; this only gives one envelope.
#[derive(Clone, Debug)]
pub enum BusEvent<'a> {
    Can(&'a CanFrame),
    Fr(&'a crate::trace::FrRow),
    Eth(&'a crate::eth::EthFrame),
}

/// One in-flight SOME/IP request. Only the correlation fields are kept; the
/// response payload is reported to Write, not stored.
struct SomeIpPending {
    bus: u8,
    service: u16,
    method: u16,
    client: u16,
    session: u16,
    deadline_us: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum NmState {
    Unknown,
    RepeatMessage,
    NormalOperation,
    ReadySleep,
    PrepareBusSleep,
    BusSleep,
}

impl NmState {
    fn label(self) -> &'static str {
        match self {
            NmState::Unknown => "Unknown",
            NmState::RepeatMessage => "RepeatMessage",
            NmState::NormalOperation => "NormalOperation",
            NmState::ReadySleep => "ReadySleep",
            NmState::PrepareBusSleep => "PrepareBusSleep",
            NmState::BusSleep => "BusSleep",
        }
    }
}

struct NmTrack {
    state: NmState,
    since_us: u64,
    last_seen_us: u64,
}

/// One remote eventgroup subscription accepted by the local IL.
#[derive(Clone)]
struct SomeIpSubscription {
    bus: u8,
    service: u16,
    instance: u16,
    eventgroup: u16,
    ip: String,
    port: u16,
    ttl: u32,
}

#[derive(Clone, Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SomeIpSubscriptionView {
    pub bus: u8,
    pub service: u16,
    pub instance: u16,
    pub eventgroup: u16,
    pub ip: String,
    pub port: u16,
    pub ttl: u32,
}

pub struct BusCore {
    /// Which timeline the bus runs on: Virtual generates against the sim
    /// clock, Replay follows a loaded log's own stamps.
    pub(crate) mode: Mode,
    /// The mode the current run started in; switching buses restores it.
    pub(crate) run_mode: Mode,
    /// The bus's frame input: the virtual idle bus, or the log being replayed.
    pub(crate) source: Box<dyn crate::source::FrameSource>,
    /// Every (channel, id) the loaded log carries. While replaying,
    /// generator entries carrying one of these ids stand down, so a replay
    /// of this very simulation cannot interleave two senders of one signal.
    /// Filled by [`App::replay`], consulted only in Replay mode, never
    /// persisted.
    pub(crate) replay_ids: std::collections::HashSet<(u8, u32)>,
    /// The `(bus, slot)` pairs the loaded log carries, for the same reason
    /// [`Self::replay_ids`] exists: a FlexRay entry filling a slot the replayed
    /// log fills too would put two senders of one signal into every view.
    pub(crate) replay_fr_ids: std::collections::HashSet<(u8, u16)>,
    /// What the loaded log carries in the **log's own** numbering.
    /// `Self::replay_ids` is derived from it plus the current mapping, and the
    /// derivation re-runs whenever either changes -- re-reading the whole file
    /// to recompute a set we already have is not a price worth paying for a
    /// mapping edit.
    pub(crate) log_ids_file: std::collections::HashSet<(u8, u32)>,
    /// The same, for FlexRay slots.
    pub(crate) log_fr_ids_file: std::collections::HashSet<(u8, u16)>,
    /// 日志里的通道编号 → 本工程的第几路。别人的日志记的是他工程的编号，
    /// 而文件里根本没有总线名可对照。
    pub(crate) replay_map: crate::replay_map::ReplayMap,
    /// 本次回放实际见过的文件通道与条数（文件编号）。Buses 窗口的映射表靠它
    /// 列出"这个文件里有这几路"，所以不必为数通道把文件再读一遍。
    pub(crate) log_census: crate::replay_map::LogCensus,
    /// 已经报过"这一路没有对应的总线"的文件通道：一句话只说一次，同一条规矩
    /// 适用于发送被线路拒绝的总线（一秒几千条会把状态行埋掉）。
    pub(crate) drop_news_done: std::collections::HashSet<u8>,
    /// Per-cluster state that is not part of the imported description:
    /// content checksums that detect external DBC edits, and the simulated
    /// node roles. Keyed by the stable cluster id, so removing a bus never
    /// renumbers anything.
    pub(crate) can_sums: std::collections::BTreeMap<crate::model::ClusterId, Vec<u64>>,
    pub(crate) can_node_roles: std::collections::BTreeMap<
        crate::model::ClusterId,
        std::collections::BTreeMap<String, NodeRole>,
    >,
    /// Unified-model view of each CAN channel, aligned with
    /// `can_cluster_ids`. Every channel has one (a channel without a
    /// database is an empty cluster); only a loaded database gives it plans.
    pub(crate) model_comms: Vec<Option<Arc<crate::model::Comm>>>,
    /// Flattened decode plans for `model_comms`, same alignment.
    pub(crate) model_plans: Vec<Option<Arc<crate::model::PlanCache>>>,
    /// The exact database Arc each model was built from. A direct replacement
    /// of the table (tests, or a path that bypasses the load helpers) must
    /// drop the plans rather than decode with a stale model.
    pub(crate) model_dbs: Vec<Option<Arc<crate::dbc::SymbolTable>>>,
    /// ARXML-derived CAN models, merged with the DBC table on rebuild.
    pub(crate) can_xml_models: Vec<Option<Arc<crate::model::Comm>>>,
    /// Global cluster ids for the CAN channels, aligned with `channels`; and
    /// per-bus ids for the FR/ETH maps. Allocated once and never reused.
    pub(crate) cluster_ids: crate::model::ClusterIds,
    pub(crate) can_cluster_ids: Vec<crate::model::ClusterId>,
    pub(crate) fr_cluster_ids: std::collections::BTreeMap<u8, crate::model::ClusterId>,
    pub(crate) eth_cluster_ids: std::collections::BTreeMap<u8, crate::model::ClusterId>,
    /// Unified-model view of each FlexRay bus, keyed by bus index.
    pub(crate) fr_models: std::collections::BTreeMap<u8, Arc<FrRuntime>>,
    /// Unified-model view of each Ethernet bus, keyed by bus index.
    pub(crate) eth_models: std::collections::BTreeMap<u8, Arc<EthRuntime>>,
    /// Model/plan issues already written to the Write log. Rebuilding a
    /// cluster must not repeat yesterday's warnings on every edit.
    pub(crate) model_issues_seen: std::collections::HashSet<String>,
    /// FlexRay buses whose hardware TX refusal was already written. Keeps a
    /// dead wire from producing one Write line per generated frame.
    pub(crate) fr_tx_errors: std::collections::HashSet<u8>,
    /// E2E counter continuity per CAN message, updated on every frame.
    pub(crate) e2e_tracks: std::collections::BTreeMap<(u8, u32, bool), E2eView>,
    pub(crate) fr_e2e_tracks: std::collections::BTreeMap<(u8, u16), E2eView>,
    pub(crate) eth_e2e_tracks: std::collections::BTreeMap<String, E2eView>,
    /// Counter for naming new buses (CAN3, CAN4, ...).
    pub(crate) bus_counter: usize,
    /// The interactive generator's entries: what this tool transmits as.
    pub(crate) tx_list: Vec<TxMsg>,
    /// The same, for the FlexRay slots this tool fills.
    pub(crate) fr_tx_list: Vec<FrTxMsg>,
    /// Simulation clock: accumulates only while measuring and unpaused.
    /// Generator frames are stamped on it and their signal values are
    /// evaluated from it, so a pause freezes the bus in place instead of
    /// letting it jump phase. Replay polling uses the wall clock.
    pub(crate) sim_t_us: u64,
    /// `now_us()` at the previous accepted tick, the reference for `sim_t_us`.
    pub(crate) sim_prev_us: u64,
    /// Frames received this run, capped at [`TRACE_LIMIT`] as a ring.
    pub(crate) trace: crate::trace::TraceRing,
    /// The ring as of the last publish, shared with snapshots. Rebuilt
    /// only on steps that ingested frames -- an idle bus copies nothing,
    /// and even then only refcounts plus one sealed tail.
    pub(crate) published_trace: Arc<crate::trace::TraceView>,
    /// FlexRay frames received this run, in their own ring: they skip the CAN
    /// aggregates and load rollups, and their decoded signals reach the
    /// observers through [`BusCore::ingest_fr_row`]'s fold.
    pub(crate) fr_trace: crate::trace::FrRing,
    /// The FR ring as of the last publish, shared with snapshots.
    pub(crate) published_fr: Arc<crate::trace::FrTraceView>,
    /// Ethernet frames received this run, in their own byte-capped ring.
    pub(crate) eth_trace: crate::eth::EthRing,
    /// Decoded SOME/IP service events from captured Ethernet frames, newest
    /// last. Bounded like the trace; the full payload stays in the trace.
    pub(crate) someip_events: std::collections::VecDeque<crate::eth::someip::SomeIpEvent>,
    /// In-order TCP reassembly feeding the same event ring.
    pub(crate) someip_tcp: crate::eth::someip::SomeIpTcp,
    /// SOME/IP-TP segment reassembly feeding the same event ring.
    pub(crate) someip_tp: crate::eth::someip::SomeIpTp,
    /// IPv4 fragment reassembly feeding the same event ring.
    pub(crate) someip_ip: crate::eth::someip::SomeIpIp,
    /// ARXML service layouts used to decode SOME/IP fields for observers.
    pub(crate) someip_services: crate::eth::arxml_service::ServiceCatalog,
    /// Next session id per (bus, service, method, client) when the caller
    /// leaves session at 0 (auto).
    someip_sessions: std::collections::BTreeMap<(u8, u16, u16, u16), u16>,
    /// Requests waiting for a matching response. The response is reported in
    /// Write; timeout is checked on the core's regular publish tick.
    someip_pending: Vec<SomeIpPending>,
    /// Local IL offers: (bus, service, instance) -> TTL. Subscribe requests
    /// for these are acknowledged automatically.
    someip_offers: std::collections::BTreeMap<(u8, u16, u16), u32>,
    someip_subscriptions: Vec<SomeIpSubscription>,
    someip_replies: std::collections::BTreeMap<(u16, u16), (u8, Vec<u8>)>,
    nm_timing: Option<crate::script::NmTiming>,
    /// Last decoded NM signal summary per (bus, PDU); suppresses repeated
    /// identical Write lines when a cyclic NM frame does not change.
    nm_last: std::collections::BTreeMap<String, String>,
    nm_states: std::collections::BTreeMap<String, NmTrack>,
    /// Set while a SOME/IP action injects its own frame; prevents the
    /// injected message from firing the same rule again recursively.
    someip_send_guard: bool,
    /// The Ethernet ring as of the last publish, shared with snapshots.
    pub(crate) published_eth: Arc<crate::eth::EthTraceView>,
    /// Per-(bus, slot) FlexRay tallies behind the Messages window's FR rows.
    /// Per FlexRay **frame**, not per slot -- see
    /// [`crate::aggregate::FrFrameAgg`] for why a slot is not the unit.
    pub(crate) fr_aggs:
        HashMap<(u8, u16, crate::aggregate::FrOccupant), crate::aggregate::FrFrameAgg>,
    /// Per-flow Ethernet tallies behind the Messages window.
    pub(crate) eth_aggs: HashMap<crate::aggregate::EthFlowKey, crate::aggregate::EthAgg>,
    /// The parsed FlexRay descriptions, one per bus, when the project has
    /// loaded them. Held here (not only in the frontend) so an arriving
    /// FlexRay frame can be decoded against *its own cluster's* description and
    /// folded into subscriptions for the Graphics / Data / State observers,
    /// exactly like a CAN frame's signals.
    pub(crate) fr_dbs: std::collections::BTreeMap<u8, Arc<crate::fr_db::FrDb>>,
    /// Source files per FlexRay bus, in bind order.
    pub(crate) fr_sources: std::collections::BTreeMap<u8, Vec<crate::model::ClusterSource>>,
    /// Ethernet matrices, pushed from the frontend when ARXML files load.
    pub(crate) eth_dbs: std::collections::BTreeMap<u8, Arc<crate::eth::db::EthDb>>,
    /// Source files per Ethernet bus, in bind order.
    pub(crate) eth_sources: std::collections::BTreeMap<u8, Vec<crate::model::ClusterSource>>,
    /// Project-level `FRAME-LENGTH` convention: true when the matrix states
    /// a frame length including the 14-byte L2 header rather than the
    /// captured payload length.
    pub(crate) eth_length_includes_header: bool,
    /// Trace-window freeze: arrivals keep coming but are stamped at the
    /// pause instant instead of their own time, so the view stays still and
    /// resuming does not dump a burst of backdated rows.
    pub(crate) trace_paused: bool,
    /// Wall-clock instant the freeze started; None while not frozen.
    pub(crate) paused_at_us: Option<u64>,
    /// Replay stops here (absolute log time) when the playhead reaches it;
    /// set by Play-for / Play-to, cleared by manual pause or a fresh run.
    pub(crate) replay_stop_at: Option<u64>,
    /// The last sampling/spec policy a normal step ran with, reused when a
    /// step command invokes a step outside the frontend's tick.
    last_stride_us: u64,
    last_tol_pct: u64,
    last_grace: u64,
    /// Frames received this run, for the run-total counter in the status bar.
    pub(crate) frame_counter: u64,
    /// Per-(bus, id) aggregates behind the Messages / Statistics views.
    pub(crate) aggs: HashMap<(crate::model::ClusterId, u32, bool), MessageAgg>,
    /// Messages rows whose signal children the frontend has open.
    pub(crate) expanded_messages: std::collections::HashSet<String>,
    /// Per-bus load / frame-rate / error rolling state, one entry per channel.
    pub(crate) bus_loads: Vec<crate::load::BusLoad>,
    /// The same for each FlexRay cluster being watched, by cluster index.
    pub(crate) fr_loads: std::collections::BTreeMap<u8, crate::load::FrLoad>,
    /// Send-now requests recorded by commands, built into frames at the
    /// next step. Kept apart from `buf` because `step` clears `buf` before
    /// polling its source.
    pub(crate) injected: Vec<(u8, u32)>,
    /// The same "one frame now" intent for a FlexRay slot.
    /// FlexRay frames queued by a script's `fr_send` during this step, with the
    /// node that wrote them so a refusal can reach that node's log. They enter
    /// the session at this step's generator block, or the next one's if the
    /// handler woke after it -- taking the queue there (rather than inside the
    /// dispatch) is what caps a self-answering script at one round per step.
    pub(crate) fr_from_scripts: Vec<(u64, u8, u16, Vec<u8>)>,
    /// Panel setpoint writes for FlexRay slots, encoded at command time and
    /// landed with the same generator block as script frames.
    pub(crate) panel_fr_frames: Vec<(u8, u16, Vec<u8>, u8)>,
    /// One-shot CAN frames from Panel send buttons, built at the next step's
    /// sim clock so a queued click never carries a stale timestamp.
    pub(crate) panel_frames: Vec<(u8, u32, bool, Vec<u8>)>,
    /// The load rollups as of the last publish. Rebuilt only when a step
    /// (or a channel add/remove) touched them; the Bus Statistics window
    /// reads this, never the live state. FlexRay's rollups ride the same
    /// dirty flag: they change on the same frames, and both are read together.
    pub(crate) published_loads: Arc<Vec<crate::load::BusLoad>>,
    pub(crate) published_fr_loads: Arc<std::collections::BTreeMap<u8, crate::load::FrLoad>>,
    /// True when `bus_loads` changed since `published_loads` was built.
    pub(crate) loads_dirty: bool,
    /// Subscribed signals: latest value, min/max/avg, sampled history.
    pub(crate) subs: HashMap<SigKey, Subscription>,
    /// Derived-channel definitions and their compiled expressions. The core
    /// samples them once per step from their source subscriptions, so the
    /// observers, exports and the chart read one series type.
    pub(crate) derived: Vec<DerivedRuntime>,
    /// Bus-event markers, oldest first, capped oldest-out. Dropped by
    /// trigger actions and drawn by Graphics as vertical lines.
    pub(crate) markers: Vec<u64>,
    /// The last [`PRE_BUFFER_FRAMES`] frames, written into the file when a
    /// trigger starts a recording: the context before the event.
    pub(crate) pre_buffer: std::collections::VecDeque<CanFrame>,
    /// While `Some(n)`, a trigger-initiated stop is rolling: the recorder
    /// stays open for `n` more frames, then closes.
    pub(crate) post_roll: Option<u32>,
    /// The sampling stride currently applied to the signal caches, pushed by
    /// the frontend's knob whenever the smallest open Graphics window changes.
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
    /// Recent trigger edges, capped oldest-out. Runtime only.
    pub(crate) trigger_fires: std::collections::VecDeque<crate::trigger::TriggerFire>,
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
    /// Panel button actions waiting for the next step, in click order.
    pub(crate) pending_panel_actions: Vec<(String, Option<f64>)>,
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

/// The bus a log channel replays on, given the map and the three cluster
/// tables. A free function so the replay loop can resolve targets while it
/// holds a mutable borrow of the frame buffer.
fn replay_bus_in(
    map: &crate::replay_map::ReplayMap,
    kind: crate::replay_map::LogBusKind,
    raw: u8,
    can: &[crate::model::ClusterId],
    fr: &std::collections::BTreeMap<u8, crate::model::ClusterId>,
    eth: &std::collections::BTreeMap<u8, crate::model::ClusterId>,
) -> Option<u8> {
    match map.target(kind, raw) {
        crate::replay_map::ReplayTarget::Same => Some(raw),
        crate::replay_map::ReplayTarget::Drop => None,
        crate::replay_map::ReplayTarget::Cluster(cluster) => match kind {
            crate::replay_map::LogBusKind::Can => can
                .iter()
                .position(|id| *id == cluster)
                .map(|index| index as u8),
            crate::replay_map::LogBusKind::Fr => fr
                .iter()
                .find(|(_, id)| **id == cluster)
                .map(|(bus, _)| *bus),
            crate::replay_map::LogBusKind::Eth => eth
                .iter()
                .find(|(_, id)| **id == cluster)
                .map(|(bus, _)| *bus),
        },
    }
}

/// The script node a derived-signal stream key belongs to: `emit_value`
/// folds `EMITTED_ID_BASE | node_id` into the CAN id field, so the stream's
/// owner is readable without a second registry. Only a CAN key can name a
/// stream -- FlexRay signals arrive from the bus, never from a script.
fn emitted_node_id(key: &SigKey) -> Option<u64> {
    match key {
        SigKey::Can { id, .. } => Some((id & !crate::app::EMITTED_ID_BASE) as u64),
        SigKey::Fr { .. } | SigKey::Eth { .. } | SigKey::Derived { .. } => None,
    }
}

/// One derived channel the core evaluates. `refs` keeps the reference text
/// in first-appearance order; resolution against subscriptions happens every
/// step, so adding a source lane later starts the lane without redefining it.
pub(crate) struct DerivedRuntime {
    pub(crate) def: crate::derived::DerivedDef,
    pub(crate) key: SigKey,
    expr: Option<crate::derived::Expr>,
    refs: Vec<String>,
    /// Reference text -> exact key the editor resolved. Empty for definitions
    /// saved before the editor pinned its sources.
    pinned: Vec<(String, SigKey)>,
    pub(crate) error: Option<String>,
    /// Newest source timestamp already sampled; `None` means backfill from
    /// whatever history the sources still hold.
    last_t_us: Option<u64>,
}

/// The label a `[Bus:signal]` reference matches. The bus index comes from the
/// key's cluster, so a renamed cluster still answers to its bus number and a
/// renamed FlexRay bus keeps its default `FRn` label.
fn derived_key_bus(key: &SigKey, core: &BusCore) -> String {
    match key {
        SigKey::Can { .. } => core
            .key_bus(key)
            .map(|ch| core.can_name(ch as usize))
            .unwrap_or_else(|| "CAN".to_string()),
        SigKey::Fr { .. } => core
            .key_bus(key)
            .map(|bus| format!("FR{bus}"))
            .unwrap_or_else(|| "FR".to_string()),
        SigKey::Eth { .. } => core
            .key_bus(key)
            .map(|bus| format!("ETH{bus}"))
            .unwrap_or_else(|| "ETH".to_string()),
        SigKey::Derived { .. } => "Derived".to_string(),
    }
}

/// Resolves one expression reference against the live subscriptions.
/// `Bus:signal` narrows by label; `Bus:Frame:signal` is needed when one bus
/// carries the same signal name in two frames. A name shared by two buses
/// stays unresolved until the user qualifies it, mirroring the editor's rule.
fn resolve_derived_ref(
    text: &str,
    keys: &[SigKey],
    core: &BusCore,
    own: &SigKey,
) -> Option<SigKey> {
    // The editor also accepts a full key (`sig:can:1:256:RPM`), so a verbatim
    // match wins before interpreting the text.
    if let Some(key) = keys.iter().find(|key| key != &own && key.text() == text) {
        return Some(key.clone());
    }
    let parts: Vec<&str> = text.split(':').collect();
    let (bus, frame, name) = match parts.as_slice() {
        [name] if !name.is_empty() => (None, None, *name),
        [bus, name] if !bus.is_empty() && !name.is_empty() => (Some(*bus), None, *name),
        [bus, frame, name] if !bus.is_empty() && !frame.is_empty() && !name.is_empty() => {
            (Some(*bus), Some(*frame), *name)
        }
        _ => return None,
    };
    let mut found: Option<SigKey> = None;
    for key in keys {
        if key == own || key.name() != name {
            continue;
        }
        if let Some(bus) = bus
            && derived_key_bus(key, core) != bus
        {
            continue;
        }
        if let Some(frame) = frame
            && !derived_ref_frame_matches(key, frame, core)
        {
            continue;
        }
        if found.is_some() {
            return None;
        }
        found = Some(key.clone());
    }
    found
}

/// Matches the middle part of `[Bus:Frame:Signal]` against the frame class
/// the series key came from. CAN accepts the hex id (`0x0C7`) or the DBC
/// message name, FlexRay a slot number or `slotN`, Ethernet the frame name.
fn derived_ref_frame_matches(key: &SigKey, qualifier: &str, core: &BusCore) -> bool {
    match key {
        SigKey::Can { id, ext, .. } => {
            let digits = qualifier
                .strip_prefix("0x")
                .or_else(|| qualifier.strip_prefix("0X"))
                .unwrap_or(qualifier);
            if u32::from_str_radix(digits, 16).ok() == Some(*id) {
                return true;
            }
            core.key_bus(key)
                .and_then(|ch| core.model_dbs.get(ch as usize))
                .and_then(|db| db.as_ref())
                .and_then(|db| db.message_name_of((*id, *ext)))
                .is_some_and(|name| name == qualifier)
        }
        SigKey::Fr { slot, .. } => {
            qualifier.eq_ignore_ascii_case(&format!("slot{slot}"))
                || qualifier.parse::<u16>().ok() == Some(*slot)
        }
        SigKey::Eth { frame, .. } => frame == qualifier,
        SigKey::Derived { name, .. } => name == qualifier,
    }
}
