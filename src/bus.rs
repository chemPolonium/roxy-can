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
use crate::channel::Channel;
use crate::dbc::DecodedSignal;
use crate::generator::TxMsg;
use crate::observe::Subscription;
use crate::source::FrameSource;
use crate::spec::{Kind, cycle_offender, dlc_offender, missing_offender};
use crate::trigger::{TriggerAction, TriggerCond};

/// Slots one generator entry may backfill per step after the clock jumped
/// (a frozen UI, a seek). Bounds the burst; anything longer streams over the
/// following steps. At 100 Hz one step covers ~10 s of missed timeline.
pub(crate) const MAX_TX_CATCHUP: u32 = 1024;

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
    /// Select the bus kind the next start will run (Simulation / Replay).
    SetRunMode(crate::app::Mode),
    /// Add a bus with the sample DBC path, load it, and pre-populate its
    /// generator.
    AddChannel,
    /// Remove bus `ch` and remap every channel-indexed reference one step
    /// down. The frontend remaps its own window state in the same stroke.
    RemoveChannel { ch: usize },
    /// Enable/disable every generator entry of one bus; freshly enabled
    /// entries anchor at the current clock.
    SetBusTx { ch: u8, on: bool },
    /// Tick or untick a DBC node as one this tool transmits as.
    SetNodeSim { ch: u8, node: String, on: bool },
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
    SetEntryActive { ch: u8, id: u32, on: bool },
    /// Toggle the entry's CAN FD flag.
    SetEntryFd { ch: u8, id: u32, fd: bool },
    /// Set the send period in µs (0 = event-triggered); the schedule
    /// restarts now rather than at the end of the old one.
    SetEntryCycle { ch: u8, id: u32, cycle_us: u64 },
    /// Replace the base payload from hex text. Active sources deliberately
    /// survive: correcting one byte must not throw away a stimulus setup.
    SetEntryHex { ch: u8, id: u32, text: String },
    /// Drop the entry, payload, sources and schedule with it.
    RemoveEntry { ch: u8, id: u32 },
    /// Add the entry `(ch, id)` unless it exists. Name, node, length and
    /// period come from the bus's database when it knows the message.
    AddEntry { ch: u8, id: u32 },
    /// Add or replace the source driving one signal on the entry.
    SetEntrySource {
        ch: u8,
        id: u32,
        src: crate::sim::ValueSrc,
    },
    /// Stop driving one signal; the base bytes take over again.
    ClearEntrySource { ch: u8, id: u32, name: String },
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
    Subscribe { key: (u8, u32, String) },
    /// Drop the subscription. The frontend only asks after none of its
    /// windows references the signal anymore.
    Unsubscribe { key: (u8, u32, String) },
    /// Open `path` and replay it: generators stand down for the ids the
    /// log carries, a fresh replay source replaces the old input at
    /// `speed`, and run-scored state resets. `speed` is the frontend's
    /// remembered multiplier, passed at start like the other policy
    /// knobs.
    StartReplay { path: String, speed: f64 },
    /// Resume a scrubbed replay in place: measurement restarts with the
    /// trace unfrozen, captured history untouched, so playback continues
    /// from the playhead. `speed` is the frontend's remembered
    /// multiplier, for the status line.
    ResumeReplay { speed: f64 },
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
    /// One entry per bus: identity, timing declarations and the shared
    /// (immutable) database. Static configuration as of this frame.
    pub channels: Vec<ChannelView>,
    /// One record per (bus, id) seen this run, behind the Messages /
    /// Statistics views and their exports.
    pub aggs: Vec<MessageAgg>,
    /// One entry per live subscription: the scalar stats the Data window
    /// draws and the sampled history the curves read.
    pub subs: Vec<SubView>,
    /// One entry per generator row: display state plus the bytes that
    /// actually go out at this frame's sim time.
    pub tx: Vec<TxView>,
    /// The trace ring, published once per step that ingested frames and
    /// shared by Arc: cloning the snapshot copies a pointer, not 50k
    /// frames. Read-only for the frontend.
    pub trace: std::sync::Arc<Vec<CanFrame>>,
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
    pub dbc_path: String,
    pub bitrate_kbps: u32,
    pub fd_data_kbps: u32,
    pub sim_nodes: Vec<String>,
    pub dbc: Option<std::sync::Arc<crate::dbc::SymbolTable>>,
}

impl std::fmt::Debug for ChannelView {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // The shared table has no Debug of its own; its presence is what
        // matters here.
        f.debug_struct("ChannelView")
            .field("name", &self.name)
            .field("dbc_path", &self.dbc_path)
            .field("bitrate_kbps", &self.bitrate_kbps)
            .field("fd_data_kbps", &self.fd_data_kbps)
            .field("sim_nodes", &self.sim_nodes)
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
    pub active: bool,
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
    pub key: (u8, u32, String),
    pub latest: f64,
    pub last_raw: i64,
    pub unit: String,
    pub label: Option<String>,
    pub type_tag: String,
    pub min: f64,
    pub max: f64,
    /// Sampled history at the frame's stride, for the curve windows.
    pub history: crate::observe::SampleCache,
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
    pub(crate) trace: VecDeque<CanFrame>,
    /// The ring as of the last publish, shared with snapshots. Rebuilt
    /// only on steps that ingested frames -- an idle bus copies nothing.
    pub(crate) published_trace: Arc<Vec<CanFrame>>,
    /// Trace-window freeze: arrivals keep coming but are stamped at the
    /// pause instant instead of their own time, so the view stays still and
    /// resuming does not dump a burst of backdated rows.
    pub(crate) trace_paused: bool,
    /// Wall-clock instant the freeze started; None while not frozen.
    pub(crate) paused_at_us: Option<u64>,
    /// Frames received this run, for the run-total counter in the status bar.
    pub(crate) frame_counter: u64,
    /// Per-(bus, id) aggregates behind the Messages / Statistics views.
    pub(crate) aggs: HashMap<(u8, u32), MessageAgg>,
    /// Per-bus load / frame-rate / error rolling state, one entry per channel.
    pub(crate) bus_loads: Vec<crate::load::BusLoad>,
    /// Subscribed signals: latest value, min/max/avg, sampled history.
    pub(crate) subs: HashMap<(u8, u32, String), Subscription>,
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
}

impl BusCore {
    /// Empty bus state; `App::new` layers the sample channel configuration
    /// and the measurement start on top.
    pub(crate) fn new(bus_loads: Vec<crate::load::BusLoad>) -> Self {
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
            trace: VecDeque::with_capacity(TRACE_LIMIT.min(8_192)),
            published_trace: Arc::new(Vec::new()),
            trace_paused: false,
            paused_at_us: None,
            frame_counter: 0,
            aggs: HashMap::new(),
            bus_loads,
            subs: HashMap::new(),
            sample_cover: None,
            applied_stride_us: SAMPLE_INTERVAL_US,
            measuring: false,
            buf: Vec::new(),
            recorder: crate::recorder::Recorder::new(),
            triggers: Vec::new(),
            spec: crate::spec::Spec::default(),
        }
    }

    /// Executes a command from the frontend. Single-threaded this is a
    /// plain call (`App::send`); stage 3 sends the same enum over a
    /// channel instead. Status text an action produces goes into `status`
    /// -- the bus has no display of its own.
    pub(crate) fn handle(&mut self, cmd: BusCommand, status: &mut String) {
        match cmd {
            BusCommand::StartVirtual => self.start_virtual(status),
            BusCommand::Stop => self.stop_bus(status),
            BusCommand::SetTracePaused(on) => self.trace_paused = on,
            BusCommand::ToggleRecord => self.toggle_record(status),
            BusCommand::SetRunMode(mode) => self.run_mode = mode,
            BusCommand::AddChannel => self.add_channel(status),
            BusCommand::RemoveChannel { ch } => self.remove_channel(ch, status),
            BusCommand::SetBusTx { ch, on } => self.set_bus_tx(ch, on),
            BusCommand::SetNodeSim { ch, node, on } => self.set_node_sim(ch, &node, on, status),
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
        }
    }

    /// Republishes the shared ring view. Called on steps that ingested
    /// frames (and on resets); the copy is the price of frontend reads
    /// that never alias the live deque.
    fn publish_trace(&mut self) {
        self.published_trace = Arc::new(self.trace.iter().copied().collect());
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
            sub_count: self.subs.len(),
            replay,
            channel_count: self.channels.len(),
            channels: self
                .channels
                .iter()
                .map(|c| ChannelView {
                    name: c.name.clone(),
                    dbc_path: c.dbc_path.clone(),
                    bitrate_kbps: c.bitrate_kbps,
                    fd_data_kbps: c.fd_data_kbps,
                    sim_nodes: c.sim_nodes.clone(),
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
                    history: s.history.clone(),
                    color: s.color,
                })
                .collect(),
            tx: self
                .tx_list
                .iter()
                .map(|t| {
                    let (sent_data, sent_len, _) =
                        crate::generator::tx_payload(&self.channels, t, self.sim_t_us);
                    TxView {
                        channel: t.channel,
                        id: t.id,
                        name: t.name.clone(),
                        active: t.active,
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
    pub(crate) fn subscribe_signal(&mut self, key: (u8, u32, String)) {
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
                    color,
                },
            );
        }
    }

    /// The display type the database declares for a subscribed signal, used
    /// until the first frame refreshes it -- and forever when no database
    /// names it.
    fn signal_meta(&self, key: &(u8, u32, String)) -> String {
        self.channels
            .get(key.0 as usize)
            .and_then(|c| c.dbc.as_ref())
            .and_then(|db| db.messages.get(&key.1))
            .and_then(|m| m.signals.iter().find(|s| s.name == key.2))
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
            dbc_path: "assets/sample.dbc".to_string(),
            sim_nodes: Vec::new(),
            bitrate_kbps: Channel::DEFAULT_BITRATE_KBPS,
            fd_data_kbps: Channel::DEFAULT_FD_DATA_KBPS,
        });
        self.bus_loads.push(crate::load::BusLoad::new());
        let ch = self.channels.len() - 1;
        self.load_channel(ch, status);
        let ids: Vec<u32> = self.channels[ch]
            .dbc
            .as_ref()
            .map(|db| db.order.clone())
            .unwrap_or_default();
        for id in ids {
            self.add_entry(ch as u8, id);
        }
    }

    /// (Re)loads the database named by the bus's `dbc_path`.
    fn load_channel(&mut self, ch: usize, status: &mut String) -> bool {
        let Some(channel) = self.channels.get_mut(ch) else {
            return false;
        };
        let name = channel.name.clone();
        match std::fs::read_to_string(channel.dbc_path.trim()) {
            Ok(content) => match crate::dbc::load_dbc_str(&content) {
                Ok(table) => {
                    *status = format!("{name} DBC loaded: {} messages", table.order.len());
                    channel.dbc = Some(std::sync::Arc::new(table));
                    true
                }
                Err(e) => {
                    *status = format!("{name} DBC error: {e}");
                    false
                }
            },
            Err(e) => {
                *status = format!("{name} DBC read failed: {e}");
                false
            }
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
        self.spec.drop_channel(ch as u8);
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
        self.publish_trace();
        *status = format!("{name} removed");
    }

    /// Enables or disables every generator message of one bus; freshly
    /// enabled messages restart their cycle immediately.
    fn set_bus_tx(&mut self, ch: u8, on: bool) {
        let sim = self.sim_t_us;
        for t in &mut self.tx_list {
            if t.channel == ch && t.active != on {
                t.active = on;
                if on {
                    t.next_t_us = sim;
                }
            }
        }
    }

    /// Ticks or unticks a DBC node as one this tool transmits as.
    ///
    /// Ticking adds whatever generator entry the node is missing and
    /// switches them on. The period of an entry that already exists is
    /// never rewritten, so a value tuned by hand outlives the click.
    /// Unticking only stops sending: entries keep their payload and
    /// waveforms, so ticking the node again restores it exactly as it was.
    fn set_node_sim(&mut self, channel: u8, node: &str, on: bool, status: &mut String) {
        if self.channels.get(channel as usize).is_none() {
            return;
        }
        // The tick is recorded first and unconditionally: a node that sends
        // nothing still has to remember that we mean to be it.
        let list = &mut self.channels[channel as usize].sim_nodes;
        if on {
            if !list.iter().any(|n| n == node) {
                list.push(node.to_string());
            }
        } else {
            list.retain(|n| n != node);
        }

        // Membership comes from the live database, not from each entry's
        // stamped `node`: loading another DBC does not rebuild the generator,
        // so a stamp can name a message this node no longer owns.
        let ids = self
            .channel_dbc(channel)
            .map(|db| db.node_tx_ids(node))
            .unwrap_or_default();
        if on {
            for id in &ids {
                self.add_entry(channel, *id);
            }
            let sim = self.sim_t_us;
            for t in &mut self.tx_list {
                if t.channel == channel && ids.contains(&t.id) && !t.active {
                    t.active = true;
                    t.next_t_us = sim;
                }
            }
        } else {
            // The stamped name is included on the way out only, so unchecking
            // still silences a node whose database has since been swapped or
            // unloaded. "I unchecked it and it is still transmitting" is the
            // one outcome a user cannot recover from by guessing.
            for t in &mut self.tx_list {
                if t.channel == channel && t.active && (ids.contains(&t.id) || t.node == node) {
                    t.active = false;
                }
            }
        }
        let bus = self
            .channels
            .get(channel as usize)
            .map(|c| c.name.clone())
            .unwrap_or_else(|| format!("CAN{}", channel + 1));
        *status = if on {
            format!("simulating {node} on {bus} ({} message(s))", ids.len())
        } else {
            format!("{node} stopped on {bus}")
        };
    }

    /// Adds the generator entry `(ch, id)` unless it already exists.
    fn add_entry(&mut self, ch: u8, id: u32) {
        if self.tx_list.iter().any(|t| t.channel == ch && t.id == id) {
            return;
        }
        let (name, node, len, cycle_us) = self
            .channel_dbc(ch)
            .and_then(|db| db.messages.get(&id))
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
            .messages
            .get(&id)
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
        *status = "measuring (virtual)".to_string();
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
        // A fresh start must not inherit the previous run's pause state.
        self.trace_paused = false;
        self.paused_at_us = None;
        self.trace.clear();
        self.publish_trace();
        self.frame_counter = 0;
        self.aggs.clear();
        for load in &mut self.bus_loads {
            load.clear();
        }
        // Along with the aggregates it reads: keeping the previous run's
        // interval memory would turn the first step of a new run into one
        // enormous measured period.
        self.spec = crate::spec::Spec::default();
        self.sample_cover = None;
        for tx in &mut self.tx_list {
            tx.next_t_us = 0;
        }
        for sub in self.subs.values_mut() {
            sub.reset_measurement();
        }
    }

    fn stop_bus(&mut self, status: &mut String) {
        self.measuring = false;
        self.recorder.close();
        *status = "stopped".to_string();
    }

    fn toggle_record(&mut self, status: &mut String) {
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

    /// The wall-clock instant the bus next has work due: the next replay
    /// frame, or the next generator slot against the sim clock (which runs
    /// 1:1 with the wall in Virtual). `None` means nothing is scheduled --
    /// the event loop may sleep until a command arrives. Paused or stopped
    /// buses are never due.
    // Wired into the core thread's event loop (阶段 3).
    #[allow(dead_code)]
    pub(crate) fn next_deadline(&self, now_us: u64) -> Option<u64> {
        if !self.measuring || self.trace_paused {
            return None;
        }
        match self.mode {
            Mode::Replay => self.source.next_deadline(now_us),
            Mode::Virtual => {
                let min_next = self
                    .tx_list
                    .iter()
                    .filter(|t| t.active && t.cycle_us != 0)
                    .map(|t| t.next_t_us)
                    .min()?;
                Some(now_us + min_next.saturating_sub(self.sim_t_us))
            }
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
        self.buf.clear();
        self.source.poll(now_us, &mut self.buf);
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
            // Every slot the clock has passed goes out at its own stamp.
            // Skipping the backlog after a UI stall (the old policy) kept the
            // tick cheap but punched a hole into the bus's own timeline --
            // and at Graphics strides fine enough to show single updates,
            // that hole reads as the curve being eaten while the plot
            // slides on. Frames carry their slot's timestamp, so spacing
            // stays exactly `cycle_us` even in the catch-up burst.
            let mut budget = MAX_TX_CATCHUP;
            while budget > 0 && tx.active && !muted && tx.cycle_us != 0 && tx.next_t_us <= sim {
                budget -= 1;
                // Values are read at the slot, not at `sim`: a frame stamped
                // `slot` must carry the waveform's value at `slot`, or every
                // payload would lead its own timestamp by up to a full cycle.
                let slot = tx.next_t_us;
                tx.next_t_us += tx.cycle_us;
                let (data, len, flags) = crate::generator::tx_payload(channels, tx, slot);
                emitted.push(CanFrame {
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
        self.buf.extend(emitted);

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
            self.recorder.write(&f);
            self.ingest(f, stride);
        }
        if i > 0 {
            self.publish_trace();
        }

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
                    && cycle_offender(interval, d, tol_pct)
                {
                    hits.push(((ch, id, Kind::Cycle), d as f64, interval as f64));
                }
            }
            if let (true, Some(d)) = (live, declared)
                && missing_offender(now, agg.last_t_us, d, grace)
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
            .and_then(|db| db.messages.get(&id))
            .and_then(|m| m.cycle_us)
    }

    /// Folds one accepted frame into every bus-side consumer: the trace
    /// ring, the run counter, the per-bus load rollups, the per-message
    /// aggregates and the subscribed-signal caches. `stride` is the sampling
    /// period the frontend currently wants for signal history; everything
    /// else this touches is the bus's own state.
    pub(crate) fn ingest(&mut self, f: CanFrame, stride: u64) {
        if self.trace.len() >= TRACE_LIMIT {
            self.trace.pop_front();
        }
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
            self.trace.push_back(f);
            return;
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
            entry.last_raw = d.raw;
            entry.unit = d.unit;
            entry.type_tag = d.type_tag;
            entry.label = d.label;
            entry.last_update_us = f.t_us;
            if f.t_us >= entry.last_sample_us + stride || entry.history.is_empty() {
                entry.push_sample(f.t_us, d.phys, stride);
            }
        }
        self.trace.push_back(f);
    }

    /// The frames a frame carries for signals this run subscribes to, looked
    /// up in the sending bus's database.
    pub(crate) fn subscribed_values(
        &self,
        f: &CanFrame,
    ) -> Vec<((u8, u32, String), DecodedSignal)> {
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
                let key = (f.channel, f.id, d.name.clone());
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
                        signal,
                        threshold,
                        rising,
                    } => {
                        if f.channel != *ch || f.id != *id || f.is_error() {
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
                    TriggerCond::CycleTimeout { .. } => continue,
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
        self.run_actions(fired, status);
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
            let (ch, id) = match &self.triggers[i].cond {
                TriggerCond::CycleTimeout { ch, id } => (*ch, *id),
                _ => continue,
            };
            if !self.triggers[i].enabled {
                continue;
            }
            let silent = self.timeout_silent(ch, id, now_us, grace);
            let t = &mut self.triggers[i];
            let was = t.level;
            t.level = silent;
            if silent && !was {
                t.fired += 1;
                t.last_fire_t_us = now_us;
                fired.push((t.action, now_us));
            }
        }
        self.run_actions(fired, status);
    }

    /// The spec's own grace comparison decides silence, so a trigger and
    /// the Dropped verdict can never disagree about the same message.
    fn timeout_silent(&self, ch: u8, id: u32, now_us: u64, grace: u64) -> bool {
        let Some(agg) = self.aggs.get(&(ch, id)) else {
            return false; // never seen: no opinion, not a dropout
        };
        let Some(declared) = self.dbc_cycle_us(ch, id) else {
            return false; // no database, message, or declared period
        };
        crate::spec::missing_offender(now_us, agg.last_t_us, declared, grace)
    }

    fn run_actions(&mut self, fired: Vec<(TriggerAction, u64)>, status: &mut String) {
        for (action, at_us) in fired {
            match action {
                TriggerAction::StartRecording => {
                    if self.measuring && !self.recorder.recording {
                        self.recorder.recording = true;
                        let opened = self.recorder.open();
                        self.recorder.recording = opened.is_ok();
                        *status = match opened {
                            Ok(path) => format!("trigger started recording to {path}"),
                            Err(e) => format!("trigger record failed: {e}"),
                        };
                    }
                }
                TriggerAction::StopRecording => {
                    if self.recorder.recording {
                        self.recorder.close();
                        self.recorder.recording = false;
                        *status = "trigger stopped recording".to_string();
                    }
                }
                TriggerAction::Send { ch, id } => {
                    self.send_one_shot(ch, id, at_us);
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
    fn send_one_shot(&mut self, ch: u8, id: u32, at_us: u64) {
        let Some(i) = self
            .tx_list
            .iter()
            .position(|t| t.channel == ch && t.id == id)
        else {
            return; // the generator row is gone: the rule idles
        };
        let (data, len, flags) =
            crate::generator::tx_payload(&self.channels, &self.tx_list[i], at_us);
        let (tch, tid, ext) = (
            self.tx_list[i].channel,
            self.tx_list[i].id,
            self.tx_list[i].extended,
        );
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
