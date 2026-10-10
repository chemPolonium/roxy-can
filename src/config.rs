//! Workspace persistence as project files (.rxproj): buses,
//! analysis windows, signals, filters, the generator and the imgui window
//! layout are bundled in one JSON file. A small `roxy-can.meta.json`
//! remembers the last opened project. The legacy `roxy-can.json` (no
//! project path) is still read once as a migration fallback.
use std::collections::HashMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::app::{
    App, DataWindow, Desktop, GfxSignal, GraphicsWindow, MsgWin, SigScope, StatsWin, TraceWin,
    WindowKind, YMode,
};
use crate::observe::SigKey;
use crate::sim::{SrcKind, ValueSrc};
use crate::trigger::{TriggerAction, TriggerCond};

pub const CONFIG_PATH: &str = "roxy-can.json";
pub const META_PATH: &str = "roxy-can.meta.json";
pub const AUTOSAVE_PATH: &str = "roxy-can.autosave.rxproj";
pub const PROJECT_EXT: &str = "rxproj";

/// The directory where roxy-can stores its state files (meta, autosave).
/// Uses `%APPDATA%\roxy-can\` on Windows; created on first access.
/// Falls back to the current directory when `APPDATA` is not set.
pub fn state_dir() -> std::path::PathBuf {
    let base = std::env::var("APPDATA")
        .map(|d| std::path::PathBuf::from(d).join("roxy-can"))
        .unwrap_or_else(|_| std::path::PathBuf::from("."));
    std::fs::create_dir_all(&base).ok();
    base
}

/// Full path for a state file: the state directory joined with the name.
pub fn state_path(name: &str) -> std::path::PathBuf {
    state_dir().join(name)
}

/// Stores a DBC path relative to the project directory when possible, so a
/// project folder can be moved or shared; paths outside the project are
/// stored absolute.
pub fn relativize(p: &str, base: &Path) -> String {
    let path = Path::new(p);
    let abs = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf())
    };
    if let Ok(rel) = abs.strip_prefix(base) {
        return rel.to_string_lossy().to_string();
    }
    abs.to_string_lossy().to_string()
}

/// Resolves a possibly-relative DBC path against the project directory,
/// but only when the file really exists there; otherwise the path is kept
/// as-is (it may be relative to the working directory).
pub fn resolve_dbc(p: &str, base: Option<&Path>) -> String {
    if Path::new(p).is_relative()
        && let Some(b) = base
    {
        let joined = b.join(p);
        if joined.exists() {
            return joined.to_string_lossy().to_string();
        }
    }
    p.to_string()
}

/// One project file: semantic workspace state plus the imgui layout text.
#[derive(Serialize, Deserialize)]
pub struct ProjectFile {
    #[serde(default = "one_default_u32")]
    pub version: u32,
    #[serde(default)]
    pub layout: String,
    /// Only set in autosaves: the project the workspace belonged to.
    #[serde(default)]
    pub project: Option<String>,
    pub config: Config,
}

fn one_default_u32() -> u32 {
    1
}

/// Startup driver written on every exit.
#[derive(Serialize, Deserialize, Default)]
pub struct Meta {
    #[serde(default)]
    pub last_project: Option<String>,
    #[serde(default)]
    pub recent_projects: Vec<String>,
}

fn true_default() -> bool {
    true
}
fn one_default() -> f64 {
    1.0
}
fn ten_default() -> u32 {
    10
}

fn trace_limit_default() -> usize {
    crate::app::TRACE_LIMIT
}
fn sixty_default() -> f64 {
    60.0
}

#[derive(Serialize, Deserialize)]
pub struct ChannelCfg {
    pub name: String,
    /// The bus's primary database. Kept as its own field so older
    /// versions can still read new projects (they see the primary).
    pub dbc_path: String,
    /// Any further databases attached to the bus, in attach order.
    /// Absent from projects saved before multi-database support.
    #[serde(default)]
    pub dbc_paths_extra: Vec<String>,
    /// Per-node roles declared on this bus, in the labels' project-file
    /// spelling. A node without an entry is `Absent` -- the restbus
    /// default. Unknown role names are dropped on load rather than
    /// guessed. Absent from projects saved before the role model.
    #[serde(default)]
    pub node_roles: std::collections::BTreeMap<String, String>,
    /// Legacy v0.5-v0.10 field: names of nodes ticked as simulated.
    /// Merged in as `Simulated` roles on load; no longer written. Absent
    /// from projects saved before v0.5, which then load with nothing
    /// simulated.
    #[serde(default, skip_serializing)]
    pub sim_nodes: Vec<String>,
    /// Arbitration and CAN FD data-phase bitrates in kbit/s, for the load
    /// view. Absent from projects saved before v0.8, which load with the
    /// defaults.
    #[serde(default = "default_bitrate")]
    pub bitrate_kbps: u32,
    #[serde(default = "default_fd_data_bitrate")]
    pub fd_data_kbps: u32,
}

fn default_bitrate() -> u32 {
    crate::app::Channel::DEFAULT_BITRATE_KBPS
}

fn default_fd_data_bitrate() -> u32 {
    crate::app::Channel::DEFAULT_FD_DATA_KBPS
}

/// One window's hand-picked subjects as the file stores them: the two buses
/// each get their own key, so each half is its own list of tuples.
type CanPicks = Vec<(u8, u32)>;
type FrPicks = Vec<(u8, u16)>;

/// A window's hand-picked subjects, split for the project file: CAN ids in
/// `manual`, FlexRay slots in `fr_manual`. Sorted so re-saving an unchanged
/// project does not shuffle the lists (a `HashSet` has no order of its own).
fn split_picks(
    picks: &std::collections::HashSet<crate::workspace::Pick>,
) -> (CanPicks, FrPicks) {
    let mut can: Vec<_> = picks
        .iter()
        .filter_map(|p| match *p {
            crate::workspace::Pick::Can { ch, id } => Some((ch, id)),
            crate::workspace::Pick::Fr { .. } => None,
        })
        .collect();
    let mut fr: Vec<_> = picks
        .iter()
        .filter_map(|p| match *p {
            crate::workspace::Pick::Fr { bus, slot } => Some((bus, slot)),
            crate::workspace::Pick::Can { .. } => None,
        })
        .collect();
    can.sort_unstable();
    fr.sort_unstable();
    (can, fr)
}

/// ...and merged back on load.
fn merge_picks(can: CanPicks, fr: FrPicks) -> std::collections::HashSet<crate::workspace::Pick> {
    can.into_iter()
        .map(|(ch, id)| crate::workspace::Pick::Can { ch, id })
        .chain(
            fr.into_iter()
                .map(|(bus, slot)| crate::workspace::Pick::Fr { bus, slot }),
        )
        .collect()
}

#[derive(Serialize, Deserialize)]
pub struct TraceCfg {
    pub name: String,
    pub opened: bool,
    pub scope: SigScope,
    /// The window's hand-picked CAN ids. Kept as two lists (rather than one
    /// list of a `Pick` enum) so an existing project file reads unchanged: a
    /// FlexRay slot pick goes in `fr_manual` beside it, and a file written by
    /// this version still loads in an older one -- it just ignores the new key.
    #[serde(default)]
    pub manual: CanPicks,
    #[serde(default)]
    pub fr_manual: FrPicks,
    #[serde(default)]
    pub filter: String,
    #[serde(default)]
    pub dir: usize,
    #[serde(default)]
    pub dbc_only: bool,
    #[serde(default)]
    pub payload: String,
    #[serde(default)]
    pub flags_kind: usize,
    /// The time-range bounds and the row that holds them. They travel with the
    /// rest of the filter because a saved window that drops rows outside a range
    /// the user cannot see reads as a broken table, not as a remembered filter.
    #[serde(default)]
    pub time_from: String,
    #[serde(default)]
    pub time_to: String,
    #[serde(default)]
    pub filters_open: bool,
    /// Whether the FlexRay rows are expanded into their decoded signals.
    #[serde(default)]
    pub fr_expand: bool,
}

#[derive(Serialize, Deserialize)]
pub struct MsgCfg {
    pub name: String,
    pub opened: bool,
    pub scope: SigScope,
    #[serde(default)]
    pub manual: CanPicks,
    #[serde(default)]
    pub fr_manual: FrPicks,
    #[serde(default)]
    pub filter: String,
    #[serde(default)]
    pub dbc_only: bool,
}

#[derive(Serialize, Deserialize)]
pub struct StatsCfg {
    pub name: String,
    pub opened: bool,
    pub scope: SigScope,
    #[serde(default)]
    pub manual: CanPicks,
    #[serde(default)]
    pub fr_manual: FrPicks,
}

#[derive(Serialize, Deserialize)]
pub struct SignalCfg {
    pub ch: u8,
    pub id: u32,
    /// The message's frame class. Absent in older projects, which can
    /// only have selected standard-class signals.
    #[serde(default)]
    pub ext: bool,
    /// The FlexRay slot carrying the signal, with `ch` naming the FlexRay
    /// bus. Absent for a CAN signal; a project saved before it existed puts
    /// its FlexRay signals in `id` behind `FR_SIG_BASE`, which
    /// [`sig_key`] migrates.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fr_slot: Option<u16>,
    pub signal: String,
    #[serde(default = "true_default")]
    pub visible: bool,
    /// [`crate::observe::YMode`] code; unknown values load as Auto.
    #[serde(default)]
    pub y_mode: u8,
    /// State Tracker only: the signal's custom state bands (ascending
    /// cuts with per-band names and colors). Present means custom mode.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state_rule: Option<RuleCfg>,
    /// State Tracker only: default-mode color overrides, one per state
    /// value (its physical value, matched back through the same
    /// quantization the bands render with).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state_overrides: Option<Vec<StateOverrideCfg>>,
}

/// One default-mode color override: the state's physical value and its
/// pinned color.
#[derive(Serialize, Deserialize)]
pub struct StateOverrideCfg {
    pub value: f64,
    pub color: [f32; 3],
}

/// One signal's custom state bands, CANoe's Value Definition rows. A
/// `None` color means automatic.
#[derive(Serialize, Deserialize)]
pub struct RuleCfg {
    pub cuts: Vec<f64>,
    pub names: Vec<String>,
    pub colors: Vec<Option<[f32; 3]>>,
}

#[derive(Serialize, Deserialize)]
pub struct GfxCfg {
    pub name: String,
    pub opened: bool,
    #[serde(default)]
    pub signals: Vec<SignalCfg>,
    #[serde(default = "sixty_default")]
    pub time_window_s: f64,
    #[serde(default)]
    pub stacked: bool,
    #[serde(default)]
    pub show_cursor: bool,
    #[serde(default = "true_default")]
    pub zoom_enabled: bool,
    #[serde(default = "true_default")]
    pub show_markers: bool,
}

#[derive(Serialize, Deserialize)]
pub struct DataCfg {
    pub name: String,
    pub opened: bool,
    #[serde(default)]
    pub signals: Vec<SignalCfg>,
}

/// One Monitor window row: the watched signal plus its display settings.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct MonitorRowCfg {
    pub ch: u8,
    pub id: u32,
    #[serde(default)]
    pub ext: bool,
    /// See [`SignalCfg::fr_slot`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fr_slot: Option<u16>,
    pub signal: String,
    #[serde(default)]
    pub label: String,
    #[serde(default = "digits_default")]
    pub digits: usize,
    #[serde(default)]
    pub rule_on: bool,
    #[serde(default)]
    pub rising: bool,
    #[serde(default)]
    pub threshold: f64,
    #[serde(default)]
    pub color: usize,
}

fn digits_default() -> usize {
    1
}

/// One simulation node: identity-free (ids are minted fresh on restore),
/// bound to a channel by index.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct NodeCfg {
    pub name: String,
    pub channel: u8,
    #[serde(default)]
    pub source: String,
    #[serde(default = "true_default")]
    pub enabled: bool,
    /// 绑定的 DBC 节点 (总线, 节点名)：绑定脚本的发帧受该节点角色开关
    /// 控制。旧工程缺省为 None（独立脚本）。
    #[serde(default)]
    pub attached: Option<(u8, String)>,
}

/// One replay block: a recorded log, filtered, injected onto one bus when
/// a simulation runs. Absent from projects saved before replay blocks.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct BlockCfg {
    pub name: String,
    pub channel: u8,
    pub path: String,
    #[serde(default)]
    pub node_filter: Option<String>,
    /// The DBC node the block belongs to `(bus, node)`; absent on old
    /// projects and on free, bus-level blocks.
    #[serde(default)]
    pub attached: Option<(u8, String)>,
    #[serde(default)]
    pub ids: Vec<(u32, bool)>,
    /// Disabled blocks are kept so the declaration survives; nothing
    /// transmits while disabled, matching the runtime semantics.
    #[serde(default)]
    pub enabled: bool,
}

#[derive(Serialize, Deserialize)]
pub struct StateCfg {
    pub name: String,
    pub opened: bool,
    #[serde(default)]
    pub signals: Vec<SignalCfg>,
    #[serde(default = "twenty_default")]
    pub time_window_s: f64,
    /// Minimum display duration in ms for a state band; shorter bands are
    /// absorbed into their neighbour. Absent from older projects (0).
    #[serde(default)]
    pub min_shown_ms: u64,
}

fn twenty_default() -> f64 {
    20.0
}

/// One driven signal's parameters. `kind` is [`crate::sim::SrcKind::to_u8`];
/// an unknown code drops the whole entry on load rather than guessing a shape.
#[derive(Serialize, Deserialize)]
pub struct SrcCfg {
    pub name: String,
    #[serde(default)]
    pub kind: u8,
    #[serde(default)]
    pub lo: f64,
    #[serde(default)]
    pub hi: f64,
    #[serde(default)]
    pub period_us: u64,
    #[serde(default)]
    pub phase_us: u64,
    #[serde(default)]
    pub seq: Vec<f64>,
    #[serde(default)]
    pub seed: u64,
    #[serde(default)]
    pub redraw_us: u64,
}

#[derive(Serialize, Deserialize)]
pub struct TxCfg {
    pub channel: u8,
    pub id: u32,
    #[serde(default)]
    pub active: bool,
    #[serde(default)]
    pub data_text: String,
    #[serde(default)]
    pub data: Vec<u8>,
    #[serde(default = "cycle_default")]
    pub cycle_us: u64,
    #[serde(default)]
    pub fd: bool,
    /// Value sources layered over `data` at emit time. Absent in projects
    /// saved before the stimulus engine, which then behave exactly as before.
    #[serde(default)]
    pub srcs: Vec<SrcCfg>,
}

/// One FlexRay generator entry: which slot of which 路 it fills, and what it
/// puts there. The payload travels as the hex text the row's box shows, since a
/// FlexRay frame is however long the description says it is.
#[derive(Serialize, Deserialize)]
pub struct FrTxCfg {
    pub bus: u8,
    pub slot: u16,
    #[serde(default)]
    pub active: bool,
    #[serde(default)]
    pub data_text: String,
    #[serde(default = "cycle_default")]
    pub cycle_us: u64,
    #[serde(default)]
    pub srcs: Vec<SrcCfg>,
}

#[derive(Serialize, Deserialize)]
pub struct DesktopCfg {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub layout: String,
    #[serde(default)]
    pub open: Vec<(u8, String)>,
    #[serde(default = "true_default")]
    pub show_tx: bool,
    #[serde(default = "true_default")]
    pub show_network: bool,
    #[serde(default = "true_default")]
    pub show_measurement: bool,
    #[serde(default)]
    pub show_buses: bool,
    #[serde(default)]
    pub show_triggers: bool,
    #[serde(default)]
    pub show_bus_stats: bool,
    #[serde(default)]
    pub show_spec: bool,
    #[serde(default)]
    pub show_id_filter: bool,
    #[serde(default)]
    pub show_sysvars: bool,
    #[serde(default = "true_default")]
    pub show_write: bool,
}

fn cycle_default() -> u64 {
    100_000
}

#[derive(Serialize, Deserialize, Default)]
pub struct Counters {
    #[serde(default)]
    pub trace: usize,
    #[serde(default)]
    pub msg: usize,
    #[serde(default)]
    pub stats: usize,
    #[serde(default)]
    pub graphics: usize,
    #[serde(default)]
    pub data: usize,
    #[serde(default)]
    pub state: usize,
}

fn tolerance_default() -> u64 {
    crate::spec::TOLERANCE_PERCENT
}

fn grace_default() -> u64 {
    crate::spec::GRACE_CYCLES
}

/// The persisted form of one trigger: the condition flattened to a kind
/// code plus its fields, the action as a code, and the enabled flag.
/// Runtime edge state (`level`, fire counts) deliberately does not round
/// trip -- a reloaded workspace is a fresh measurement.
#[derive(Serialize, Deserialize)]
pub struct TriggerCfg {
    pub kind: u8,
    pub ch: u8,
    pub id: u32,
    #[serde(default)]
    pub signal: String,
    #[serde(default)]
    pub threshold: f64,
    #[serde(default)]
    pub rising: bool,
    /// SignalCross only: the watched message's frame class. Absent in
    /// older projects, which could only watch standard messages.
    #[serde(default)]
    pub ext: bool,
    pub action: u8,
    /// Target generator entry for `action` code 2 (Send); ignored otherwise.
    #[serde(default)]
    pub send_ch: u8,
    #[serde(default)]
    pub send_id: u32,
    #[serde(default = "true_default")]
    pub enabled: bool,
}

/// How strictly the monitor reads the database's promises. Projects saved
/// before the monitor existed get both defaults.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct SpecCfg {
    #[serde(default = "tolerance_default")]
    pub tolerance_percent: u64,
    #[serde(default = "grace_default")]
    pub grace_cycles: u64,
}

impl Default for SpecCfg {
    fn default() -> Self {
        Self {
            tolerance_percent: tolerance_default(),
            grace_cycles: grace_default(),
        }
    }
}

/// The project-file format version [`Config`] writes. Reader logic keys
/// migrations off this, not off the crate version: the format moves only
/// when its meaning does. Projects older than the field load as version 0
/// (the pre-role-model format) and read fine -- every field has its own
/// default, per the missing-key convention.
pub const SCHEMA_VERSION: u32 = 1;

/// One FlexRay bus in a project file: which bus the description belongs to
/// and the file it was parsed from. Slot numbers repeat across clusters, so
/// a path without its bus index would not say what it describes.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct FrBusFile {
    #[serde(default)]
    pub bus: u8,
    pub path: String,
}

/// Trigger-recording context sizes: pre-trigger frames kept for
/// trigger-started recordings, post-roll frames after a trigger-stopped
/// edge, and the marker list cap.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct LimitsCfg {
    #[serde(default = "pre_frames_default")]
    pub pre_frames: usize,
    #[serde(default = "post_frames_default")]
    pub post_frames: u32,
    #[serde(default = "marker_cap_default")]
    pub marker_cap: usize,
}

impl Default for LimitsCfg {
    fn default() -> Self {
        Self {
            pre_frames: crate::bus::PRE_BUFFER_FRAMES,
            post_frames: crate::bus::POST_ROLL_FRAMES,
            marker_cap: crate::bus::MARKER_CAP,
        }
    }
}

fn pre_frames_default() -> usize {
    crate::bus::PRE_BUFFER_FRAMES
}

fn post_frames_default() -> u32 {
    crate::bus::POST_ROLL_FRAMES
}

fn marker_cap_default() -> usize {
    crate::bus::MARKER_CAP
}

#[derive(Serialize, Deserialize)]
pub struct Config {
    /// The format version the file was written with. Absent from the
    /// earliest projects, which therefore read as 0.
    #[serde(default)]
    pub schema_version: u32,
    /// Trigger-recording context sizes (pre-trigger, post-roll, marker
    /// cap). Absent fields load at the defaults the constants carried.
    #[serde(default)]
    pub limits: LimitsCfg,
    #[serde(default)]
    pub channels: Vec<ChannelCfg>,
    #[serde(default)]
    pub bus_counter: usize,
    /// The FlexRay descriptions, one entry per bus, behind the FR
    /// watch/replay decoding, stored like the DBC paths.
    #[serde(default)]
    pub fr_buses: Vec<FrBusFile>,
    /// What the user named each FlexRay 路, as `(bus, name)` pairs. Separate
    /// from `fr_buses` because a name is not a description: a cluster that only
    /// a replayed log carries can be named too. Absent means the `FR{n}`
    /// default, so an older project file reads unchanged.
    #[serde(default)]
    pub fr_names: Vec<(u8, String)>,
    /// Legacy: the single description file from before FlexRay got a bus
    /// index. It loads as bus 0, which is the only bus a project of that
    /// shape could have watched; new files write `fr_buses` and drop this.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fr_fibex: Option<String>,
    #[serde(default = "true_default")]
    pub show_tx: bool,
    #[serde(default = "true_default")]
    pub show_network: bool,
    #[serde(default = "true_default")]
    pub show_measurement: bool,
    #[serde(default)]
    pub show_buses: bool,
    #[serde(default)]
    pub show_triggers: bool,
    #[serde(default)]
    pub show_bus_stats: bool,
    #[serde(default)]
    pub show_spec: bool,
    #[serde(default)]
    pub show_id_filter: bool,
    #[serde(default)]
    pub show_sysvars: bool,
    #[serde(default = "true_default")]
    pub show_write: bool,
    #[serde(default = "one_default")]
    pub replay_speed: f64,
    /// Throttled text refresh for number readouts, in Hz; 0 follows the
    /// frame rate.
    #[serde(default = "ten_default")]
    pub text_rate_hz: u32,
    /// How many frames the trace ring retains. Absent from older
    /// projects, which load with the default.
    #[serde(default = "trace_limit_default")]
    pub trace_limit: usize,
    #[serde(default)]
    pub trace_windows: Vec<TraceCfg>,
    #[serde(default)]
    pub msg_windows: Vec<MsgCfg>,
    #[serde(default)]
    pub stats_windows: Vec<StatsCfg>,
    #[serde(default)]
    pub graphics: Vec<GfxCfg>,
    #[serde(default)]
    pub data_windows: Vec<DataCfg>,
    /// The Monitor window's rows (single global panel; the window's
    /// open/close stays session state).
    #[serde(default)]
    pub monitor_rows: Vec<MonitorRowCfg>,
    #[serde(default)]
    pub state_trackers: Vec<StateCfg>,
    #[serde(default)]
    pub nodes: Vec<NodeCfg>,
    #[serde(default)]
    pub blocks: Vec<BlockCfg>,
    /// System variable definitions. The live values stay runtime-only;
    /// only the declarations persist.
    #[serde(default)]
    pub sysvars: Vec<crate::bus::SysVarDef>,
    #[serde(default)]
    pub tx: Vec<TxCfg>,
    /// The FlexRay generator entries. Absent in projects saved before they
    /// existed, which then load with none.
    #[serde(default)]
    pub fr_tx: Vec<FrTxCfg>,
    #[serde(default)]
    pub counters: Counters,
    #[serde(default)]
    pub recent_dbc: Vec<String>,
    #[serde(default, alias = "recent_asc")]
    pub recent_log: Vec<String>,
    #[serde(default)]
    pub desktops: Vec<DesktopCfg>,
    #[serde(default)]
    pub active_desktop: usize,
    #[serde(default)]
    pub spec: SpecCfg,
    #[serde(default)]
    pub triggers: Vec<TriggerCfg>,
}

/// The project-file fields a signal key writes: the CAN triple, or the
/// FlexRay bus in `ch` with its slot in `fr_slot`. A FlexRay signal no longer
/// borrows an arbitration id, so nothing here can be mistaken for one.
fn sig_cfg_of(key: &SigKey) -> (u8, u32, bool, Option<u16>) {
    match key {
        SigKey::Can { ch, id, ext, .. } => (*ch, *id, *ext, None),
        SigKey::Fr { bus, slot, .. } => (*bus, 0, false, Some(*slot)),
    }
}

/// The key a project-file field set names, migrating what older projects
/// wrote: they had no `fr_slot` and squeezed FlexRay into `id` behind
/// `FR_SIG_BASE`, with the slot in the low bits and the bus fixed to the only
/// one a FlexRay watch could have. Loading those as FlexRay keys is what keeps
/// their curves.
fn sig_key(ch: u8, id: u32, ext: bool, fr_slot: Option<u16>, signal: String) -> SigKey {
    match fr_slot {
        Some(slot) => SigKey::fr(ch, slot, signal),
        None if id & crate::app::FR_SIG_BASE != 0 => {
            SigKey::fr(0, (id & 0x3F_FF) as u16, signal)
        }
        None => SigKey::can(ch, id, ext, signal),
    }
}

fn sig_cfgs(signals: &[GfxSignal]) -> Vec<SignalCfg> {
    signals
        .iter()
        .map(|s| {
            let (ch, id, ext, fr_slot) = sig_cfg_of(&s.key);
            SignalCfg {
                ch,
                id,
                ext,
                fr_slot,
                signal: s.key.name().to_string(),
                visible: s.visible,
                y_mode: s.y_mode.to_u8(),
                state_rule: None,
                state_overrides: None,
            }
        })
        .collect()
}

fn src_cfg(s: &ValueSrc) -> SrcCfg {
    SrcCfg {
        name: s.name.clone(),
        kind: s.kind.to_u8(),
        lo: s.lo,
        hi: s.hi,
        period_us: s.period_us,
        phase_us: s.phase_us,
        seq: s.seq.clone(),
        seed: s.seed,
        redraw_us: s.redraw_us,
    }
}

fn value_src(c: SrcCfg) -> Option<ValueSrc> {
    let kind = SrcKind::from_u8(c.kind)?;
    Some(ValueSrc {
        name: c.name,
        kind,
        lo: c.lo,
        hi: c.hi,
        period_us: c.period_us,
        phase_us: c.phase_us,
        seq: c.seq,
        seed: c.seed,
        redraw_us: c.redraw_us,
    })
}

fn desktop_cfg(d: &Desktop) -> DesktopCfg {
    DesktopCfg {
        name: d.name.clone(),
        layout: d.layout.clone(),
        open: d
            .open_windows
            .iter()
            .map(|(k, n)| (k.to_u8(), n.clone()))
            .collect(),
        show_tx: d.show_tx,
        show_network: d.show_network,
        show_measurement: d.show_measurement,
        show_buses: d.show_buses,
        show_triggers: d.show_triggers,
        show_bus_stats: d.show_bus_stats,
        show_spec: d.show_spec,
        show_id_filter: d.show_id_filter,
        show_sysvars: d.show_sysvars,
        show_write: d.show_write,
    }
}

fn sig_keys(signals: &[SignalCfg]) -> Vec<GfxSignal> {
    signals
        .iter()
        .map(|s| GfxSignal {
            key: sig_key(s.ch, s.id, s.ext, s.fr_slot, s.signal.clone()),
            visible: s.visible,
            y_mode: YMode::from_u8(s.y_mode),
        })
        .collect()
}

impl Config {
    pub fn from_app(app: &App, base: Option<&Path>) -> Self {
        // Bus-side reads go through the snapshot, like every other
        // frontend read of the bus.
        Config {
            schema_version: SCHEMA_VERSION,
            channels: app
                .snap
                .channels
                .iter()
                .map(|c| {
                    // Primary first, then the extras; every path is stored
                    // relative to the project directory when there is one.
                    let rel = |p: &String| match base {
                        Some(b) => relativize(p, b),
                        None => p.clone(),
                    };
                    let (first, rest) = match c.dbc_paths.split_first() {
                        Some((f, r)) => (rel(f), r.iter().map(rel).collect()),
                        None => (String::new(), Vec::new()),
                    };
                    ChannelCfg {
                        name: c.name.clone(),
                        dbc_path: first,
                        dbc_paths_extra: rest,
                        node_roles: c
                            .node_roles
                            .iter()
                            .map(|(n, r)| (n.clone(), r.tag().to_string()))
                            .collect(),
                        sim_nodes: Vec::new(),
                        bitrate_kbps: c.bitrate_kbps,
                        fd_data_kbps: c.fd_data_kbps,
                    }
                })
                .collect(),
            bus_counter: app.snap.bus_counter,
            fr_buses: app
                .fr_buses
                .iter()
                .map(|(bus, c)| FrBusFile {
                    bus: *bus,
                    path: c.path.clone(),
                })
                .collect(),
            fr_names: app
                .fr_names
                .iter()
                .map(|(bus, name)| (*bus, name.clone()))
                .collect(),
            // Written only by the legacy loader; a save drops it in favour of
            // `fr_buses` above.
            fr_fibex: None,
            show_tx: app.show_tx,
            show_network: app.show_network,
            show_measurement: app.show_measurement,
            show_buses: app.show_buses,
            show_triggers: app.show_triggers,
            show_bus_stats: app.show_bus_stats,
            show_spec: app.show_spec,
            show_id_filter: app.show_id_filter,
            show_sysvars: app.show_sysvars,
            show_write: app.show_write,
            // "As fast as possible" (infinity) cannot serialize; it
            // persists as the ladder's fastest finite notch.
            replay_speed: if app.replay_speed.is_finite() {
                app.replay_speed
            } else {
                100.0
            },
            text_rate_hz: app.text_rate_hz,
            trace_limit: app.trace_limit,
            limits: app.limits,
            trace_windows: app
                .trace_windows
                .iter()
                .map(|w| {
                    let (manual, fr_manual) = split_picks(&w.manual);
                    TraceCfg {
                        name: w.name.clone(),
                        opened: w.opened,
                        scope: w.scope,
                        manual,
                        fr_manual,
                        filter: w.filter.clone(),
                        dir: w.dir,
                        dbc_only: w.dbc_only,
                        payload: w.payload.clone(),
                        flags_kind: w.flags_kind,
                        time_from: w.time_from.clone(),
                        time_to: w.time_to.clone(),
                        filters_open: w.filters_open,
                        fr_expand: w.fr_expand,
                    }
                })
                .collect(),
            msg_windows: app
                .msg_windows
                .iter()
                .map(|w| {
                    let (manual, fr_manual) = split_picks(&w.manual);
                    MsgCfg {
                        name: w.name.clone(),
                        opened: w.opened,
                        scope: w.scope,
                        manual,
                        fr_manual,
                        filter: w.filter.clone(),
                        dbc_only: w.dbc_only,
                    }
                })
                .collect(),
            stats_windows: app
                .stats_windows
                .iter()
                .map(|w| {
                    let (manual, fr_manual) = split_picks(&w.manual);
                    StatsCfg {
                        name: w.name.clone(),
                        opened: w.opened,
                        scope: w.scope,
                        manual,
                        fr_manual,
                    }
                })
                .collect(),
            graphics: app
                .graphics
                .iter()
                .map(|g| GfxCfg {
                    name: g.name.clone(),
                    opened: g.opened,
                    signals: sig_cfgs(&g.signals),
                    time_window_s: g.time_window_s,
                    stacked: g.stacked,
                    show_cursor: g.show_cursor,
                    zoom_enabled: g.zoom_enabled,
                    show_markers: g.show_markers,
                })
                .collect(),
            data_windows: app
                .data_windows
                .iter()
                .map(|d| DataCfg {
                    name: d.name.clone(),
                    opened: d.opened,
                    signals: sig_cfgs(&d.signals),
                })
                .collect(),
            monitor_rows: app
                .monitor_rows
                .iter()
                .map(|r| {
                    let (ch, id, ext, fr_slot) = sig_cfg_of(&r.key);
                    MonitorRowCfg {
                        ch,
                        id,
                        ext,
                        fr_slot,
                        signal: r.key.name().to_string(),
                        label: r.label.clone(),
                        digits: r.digits,
                        rule_on: r.rule_on,
                        rising: r.rising,
                        threshold: r.threshold,
                        color: r.color,
                    }
                })
                .collect(),
            state_trackers: app
                .state_trackers
                .iter()
                .map(|w| StateCfg {
                    name: w.name.clone(),
                    opened: w.opened,
                    time_window_s: w.time_window_s,
                    min_shown_ms: w.min_shown_ms,
                    signals: w
                        .signals
                        .iter()
                        .map(|s| {
                            let (ch, id, ext, fr_slot) = sig_cfg_of(&s.key);
                            SignalCfg {
                                ch,
                                id,
                                ext,
                                fr_slot,
                                signal: s.key.name().to_string(),
                                visible: s.visible,
                                y_mode: s.y_mode.to_u8(),
                                state_rule: w.rules.get(&s.key).map(|r| RuleCfg {
                                    cuts: r.cuts.clone(),
                                    names: r.names.clone(),
                                    colors: r.colors.clone(),
                                }),
                                state_overrides: w.overrides.get(&s.key).map(|m| {
                                    m.iter()
                                        .map(|(bits, c)| StateOverrideCfg {
                                            value: f64::from_bits(*bits),
                                            color: *c,
                                        })
                                        .collect()
                                }),
                            }
                        })
                        .collect(),
                })
                .collect(),
            nodes: app
                .snap
                .nodes
                .iter()
                .map(|n| NodeCfg {
                    name: n.name.clone(),
                    channel: n.channel,
                    source: n.source.clone(),
                    enabled: n.enabled,
                    attached: n.attached.clone(),
                })
                .collect(),
            blocks: app
                .snap
                .blocks
                .iter()
                .map(|b| BlockCfg {
                    name: b.name.clone(),
                    channel: b.channel,
                    path: b.path.clone(),
                    node_filter: b.node_filter.clone(),
                    attached: b.attached.clone(),
                    ids: b.ids.clone(),
                    enabled: b.enabled,
                })
                .collect(),
            sysvars: app.snap.sysvars.iter().map(|v| v.def.clone()).collect(),
            tx: app
                .snap
                .tx
                .iter()
                .map(|t| TxCfg {
                    channel: t.channel,
                    id: t.id,
                    active: t.active,
                    data_text: t.data_text.clone(),
                    data: crate::generator::parse_hex_bytes(&t.data_text).unwrap_or_default(),
                    cycle_us: t.cycle_us,
                    fd: t.fd,
                    srcs: t.srcs.iter().map(src_cfg).collect(),
                })
                .collect(),
            fr_tx: app
                .snap
                .fr_tx
                .iter()
                .map(|t| FrTxCfg {
                    bus: t.bus,
                    slot: t.slot,
                    active: t.active,
                    data_text: t.data_text.clone(),
                    cycle_us: t.cycle_us,
                    srcs: t.srcs.iter().map(src_cfg).collect(),
                })
                .collect(),
            counters: app.window_counters(),
            recent_dbc: app.recent_dbc.clone(),
            recent_log: app.recent_log.clone(),
            desktops: {
                let mut ds: Vec<DesktopCfg> = app.desktops.iter().map(desktop_cfg).collect();
                // The active desktop's stored state can lag behind the live
                // windows; persist the live snapshot instead.
                if let Some(d) = ds.get_mut(app.active_desktop) {
                    let mut live = desktop_cfg(&app.desktop_snapshot());
                    live.name = d.name.clone();
                    *d = live;
                }
                ds
            },
            active_desktop: app.active_desktop,
            spec: SpecCfg {
                tolerance_percent: app.spec_tol_pct,
                grace_cycles: app.spec_grace,
            },
            triggers: app
                .snap
                .triggers
                .iter()
                .map(|t| {
                    let mut send_ch = 0u8;
                    let mut send_id = 0u32;
                    let (kind, ch, id, ext, signal, threshold, rising) = match &t.cond {
                        TriggerCond::SignalCross {
                            ch,
                            id,
                            ext,
                            signal,
                            threshold,
                            rising,
                        } => (0, *ch, *id, *ext, signal.clone(), *threshold, *rising),
                        TriggerCond::IdPresent { ch, id } => {
                            (1, *ch, *id, false, String::new(), 0.0, false)
                        }
                        TriggerCond::ErrorFrame { ch } => {
                            (2, *ch, 0, false, String::new(), 0.0, false)
                        }
                        TriggerCond::CycleTimeout { ch, id } => {
                            (3, *ch, *id, false, String::new(), 0.0, false)
                        }
                        // The sysvar key rides the `signal` string field;
                        // ch/id carry nothing for it.
                        TriggerCond::SysVar {
                            key,
                            threshold,
                            rising,
                        } => (4, 0, 0, false, key.clone(), *threshold, *rising),
                        // A FlexRay rule's `ch` is a FlexRay bus index and its
                        // `id` the slot: `kind` 5/6 is what says so, and no
                        // arbitration id is invented for a schedule position.
                        TriggerCond::FrFramePresent { bus, slot } => {
                            (5, *bus, u32::from(*slot), false, String::new(), 0.0, false)
                        }
                        TriggerCond::FrSignalCross {
                            bus,
                            slot,
                            signal,
                            threshold,
                            rising,
                        } => (
                            6,
                            *bus,
                            u32::from(*slot),
                            false,
                            signal.clone(),
                            *threshold,
                            *rising,
                        ),
                    };
                    TriggerCfg {
                        kind,
                        ch,
                        id,
                        signal,
                        threshold,
                        rising,
                        ext,
                        action: match t.action {
                            TriggerAction::StartRecording => 0,
                            TriggerAction::StopRecording => 1,
                            TriggerAction::Send { ch, id } => {
                                send_ch = ch;
                                send_id = id;
                                2
                            }
                            TriggerAction::ClearTrace => 3,
                            TriggerAction::InsertMarker => 4,
                        },
                        send_ch,
                        send_id,
                        enabled: t.enabled,
                    }
                })
                .collect(),
        }
    }

    /// Resolves relative DBC paths against the project directory they were
    /// saved relative to; call before `apply`.
    pub fn resolve_paths(&mut self, base: Option<&Path>) {
        for c in &mut self.channels {
            c.dbc_path = resolve_dbc(&c.dbc_path, base);
            for p in &mut c.dbc_paths_extra {
                *p = resolve_dbc(p, base);
            }
        }
        if let Some(p) = &mut self.fr_fibex {
            *p = resolve_dbc(p, base);
        }
        for f in &mut self.fr_buses {
            f.path = resolve_dbc(&f.path, base);
        }
    }

    /// Overwrites the freshly built defaults with the saved workspace.
    /// Bus-side state moves entirely through commands (so the same code
    /// works once the core runs on its own thread); the handful of
    /// mid-restore reads go through the snapshot, settling first on the
    /// threaded drive so the reads see applied commands.
    /// Restores a saved workspace into `app`. The returned strings are the
    /// parts it could not restore -- a saved file that moved or broke -- which
    /// the caller has to report: `apply` cannot say it itself, because the load
    /// path writes its own "project loaded" over the bar the moment this
    /// returns.
    pub fn apply(self, app: &mut App) -> Vec<String> {
        let mut problems: Vec<String> = Vec::new();
        if !self.channels.is_empty() {
            // Grow or shrink the fresh app's two default buses to the
            // saved count, then overlay the saved declarations on top.
            // The count is stepped locally -- on the threaded drive the
            // snapshot lags the commands by a lap.
            let mut count = app.snap.channel_count;
            while count > self.channels.len() {
                app.remove_channel(count - 1);
                count -= 1;
            }
            while count < self.channels.len() {
                app.add_channel();
                count += 1;
            }
            for (i, c) in self.channels.iter().enumerate() {
                // Role declarations: drop unknown spellings (the 未知码丢条
                // convention), then fold the legacy simulated-name list in
                // -- old projects keep their nodes simulating.
                let mut roles: std::collections::BTreeMap<String, crate::app::NodeRole> = c
                    .node_roles
                    .iter()
                    .filter_map(|(n, r)| crate::app::NodeRole::parse(r).map(|r| (n.clone(), r)))
                    .collect();
                for n in &c.sim_nodes {
                    roles
                        .entry(n.clone())
                        .or_insert(crate::app::NodeRole::Simulated);
                }
                app.send(crate::bus::BusCommand::SetChannelConfig {
                    ch: i as u8,
                    name: Some(c.name.clone()),
                    dbc_path: Some(c.dbc_path.clone()),
                    bitrate_kbps: Some(c.bitrate_kbps),
                    fd_data_kbps: Some(c.fd_data_kbps),
                    node_roles: Some(roles),
                });
                // The full attach list (primary + extras) travels as one
                // command; SetChannelConfig only pins the primary.
                let mut paths = vec![c.dbc_path.clone()];
                paths.extend(c.dbc_paths_extra.iter().cloned());
                app.send(crate::bus::BusCommand::LoadDbc { ch: i as u8, paths });
            }
            // Intent only. What transmits is decided by each TxCfg's
            // `active` below, so a restored project never starts
            // traffic that was stopped when it was saved.
            app.set_bus_counter(self.bus_counter.max(self.channels.len()));
            app.settle();
            // Databases were already loaded with their full attach lists
            // above; no second pass needed.
        }
        // Whatever the fresh app or the loaded databases put in the generator
        // goes, and the project's own entries come back in its place.
        let stale: Vec<(u8, u32)> = app.snap.tx.iter().map(|t| (t.channel, t.id)).collect();
        for (ch, id) in stale {
            app.send(crate::bus::BusCommand::RemoveEntry { ch, id });
        }
        // The saved entries, and only those. This used to add a row for **every
        // message in every loaded database** ("rebuild from the DBCs, then
        // overlay"), because `set_entry_config` only edits an entry that already
        // exists -- so opening a project filled the panel with the whole
        // database, including the entries the user had deleted before saving.
        // A message the databases no longer declare keeps its row on purpose:
        // the generator has always accepted an entry the database does not know.
        for t in &self.tx {
            app.add_tx(t.channel, t.id);
        }
        app.settle();
        for t in self.tx {
            // 0 is a real state now: a DBC-declared event-triggered
            // message. Everything else keeps the anti-typo floor.
            let cycle_us = if t.cycle_us == 0 {
                0
            } else {
                t.cycle_us.max(1_000)
            };
            let data_text = if t.data_text.is_empty() {
                None
            } else {
                Some(t.data_text)
            };
            app.send(crate::bus::BusCommand::SetEntryConfig {
                ch: t.channel,
                id: t.id,
                active: t.active,
                cycle_us,
                fd: t.fd,
                flags: None,
                data_text,
                srcs: t.srcs.into_iter().filter_map(value_src).collect(),
            });
        }
        // Every window kind is restored as the file states it -- an empty list
        // means the project has none of that kind. These used to be skipped
        // when the saved list was empty, which let the six windows a fresh
        // session boots with (app.rs's `shell`) leak back into a project whose
        // user had deleted them.
        app.trace_windows = self
            .trace_windows
            .into_iter()
            .map(|w| TraceWin {
                name: w.name,
                opened: w.opened,
                scope: w.scope,
                manual: merge_picks(w.manual, w.fr_manual),
                filter: w.filter,
                dir: w.dir.min(2),
                dbc_only: w.dbc_only,
                payload: w.payload,
                flags_kind: w.flags_kind,
                time_from: w.time_from,
                time_to: w.time_to,
                filters_open: w.filters_open,
                fr_expand: w.fr_expand,
                mark_us: [None, None],
                pick: None,
                rows: std::collections::VecDeque::new(),
                rows_build: None,
                row_ends: Vec::new(),
                rows_sorted: false,
                shown_t_us: u64::MAX,
                shown_count: 0,
            })
            .collect();
        app.msg_windows = self
            .msg_windows
            .into_iter()
            .map(|w| MsgWin {
                name: w.name,
                opened: w.opened,
                scope: w.scope,
                manual: merge_picks(w.manual, w.fr_manual),
                filter: w.filter,
                dbc_only: w.dbc_only,
                text_keys: Vec::new(),
                text_header: String::new(),
                text_rows: Vec::new(),
            })
            .collect();
        app.stats_windows = self
            .stats_windows
            .into_iter()
            .map(|w| StatsWin {
                name: w.name,
                opened: w.opened,
                scope: w.scope,
                manual: merge_picks(w.manual, w.fr_manual),
                text_keys: Vec::new(),
                text_header: String::new(),
                text_rows: Vec::new(),
            })
            .collect();
        app.graphics = self
            .graphics
            .into_iter()
            .map(|g| GraphicsWindow {
                name: g.name,
                signals: sig_keys(&g.signals),
                time_window_s: g.time_window_s.clamp(0.1, 3600.0),
                stacked: g.stacked,
                opened: g.opened,
                t_offset_s: 0.0,
                show_cursor: g.show_cursor,
                cursor_s: [None, None],
                cursor_drag: None,
                zoom_enabled: g.zoom_enabled,
                show_markers: g.show_markers,
                y_locks: HashMap::new(),
                legend_keys: Vec::new(),
                legend: Vec::new(),
            })
            .collect();
        app.data_windows = self
            .data_windows
            .into_iter()
            .map(|d| DataWindow {
                name: d.name,
                signals: sig_keys(&d.signals),
                opened: d.opened,
                text_keys: Vec::new(),
                text_cache: Vec::new(),
            })
            .collect();
        if !self.monitor_rows.is_empty() {
            app.monitor_rows = self
                .monitor_rows
                .into_iter()
                .map(|m| crate::app::MonitorRow {
                    key: sig_key(m.ch, m.id, m.ext, m.fr_slot, m.signal),
                    label: m.label,
                    digits: m.digits,
                    rule_on: m.rule_on,
                    rising: m.rising,
                    threshold: m.threshold,
                    color: m.color,
                })
                .collect();
        }
        app.state_trackers = self
            .state_trackers
            .into_iter()
            .map(|w| {
                let signals = sig_keys(&w.signals);
                let mut rules = HashMap::new();
                let mut overrides = HashMap::new();
                for (s, cfg) in signals.iter().zip(&w.signals) {
                    if let Some(r) = &cfg.state_rule {
                        rules.insert(
                            s.key.clone(),
                            crate::observe::StateRule {
                                cuts: r.cuts.clone(),
                                names: r.names.clone(),
                                colors: r.colors.clone(),
                            },
                        );
                    }
                    if let Some(list) = &cfg.state_overrides {
                        // The map keys by the value's normalized bits,
                        // the same key the band view classifies with.
                        let mut m = HashMap::new();
                        for o in list {
                            let q = o.value;
                            let q = if q == 0.0 { 0.0 } else { q };
                            m.insert(q.to_bits(), o.color);
                        }
                        overrides.insert(s.key.clone(), m);
                    }
                }
                crate::observe::StateWin {
                    name: w.name,
                    opened: w.opened,
                    signals,
                    time_window_s: w.time_window_s,
                    min_shown_ms: w.min_shown_ms,
                    color_slots: HashMap::new(),
                    rules,
                    overrides,
                }
            })
            .collect();
        app.show_tx = self.show_tx;
        app.show_network = self.show_network;
        app.show_measurement = self.show_measurement;
        app.show_buses = self.show_buses;
        app.show_triggers = self.show_triggers;
        app.show_bus_stats = self.show_bus_stats;
        app.show_spec = self.show_spec;
        app.show_id_filter = self.show_id_filter;
        app.show_sysvars = self.show_sysvars;
        app.show_write = self.show_write;
        app.text_rate_hz = self.text_rate_hz;
        app.trace_limit = self.trace_limit;
        app.set_trace_limit(self.trace_limit);
        app.limits = self.limits;
        app.send(crate::bus::BusCommand::SetRunLimits {
            pre_frames: Some(self.limits.pre_frames),
            post_frames: Some(self.limits.post_frames),
            marker_cap: Some(self.limits.marker_cap),
        });
        // Nodes cross as one wholesale command: the core mints fresh ids
        // and (when a measurement is running) starts every enabled node.
        // If measuring, the node source recompiles now -- a restore into a
        // running bus behaves like a live edit.
        app.send(crate::bus::BusCommand::SetNodes {
            nodes: self.nodes.clone(),
        });
        // System variables restore as one Define per entry: existing
        // definitions with the same key are replaced, values start at
        // their declared init.
        for def in &self.sysvars {
            app.send(crate::bus::BusCommand::DefineSysVar(def.clone()));
        }
        // Replay blocks cross wholesale like the nodes: ids are minted
        // fresh, and enabled blocks load their queues right away so a
        // broken log path surfaces at restore, not at the first start.
        app.send(crate::bus::BusCommand::SetReplayBlocks {
            blocks: self.blocks.clone(),
        });
        // An unknown kind code (a project from a future version) drops
        // only that trigger; the rest load. The list belongs to the bus,
        // so it crosses as a command; restore reads the list back below
        // (the project's clean baseline), hence the settle.
        let triggers: Vec<crate::trigger::Trigger> = self
            .triggers
            .iter()
            .filter_map(|c| {
                let cond = match c.kind {
                    0 => TriggerCond::SignalCross {
                        ch: c.ch,
                        id: c.id,
                        ext: c.ext,
                        signal: c.signal.clone(),
                        threshold: c.threshold,
                        rising: c.rising,
                    },
                    1 => TriggerCond::IdPresent { ch: c.ch, id: c.id },
                    2 => TriggerCond::ErrorFrame { ch: c.ch },
                    3 => TriggerCond::CycleTimeout { ch: c.ch, id: c.id },
                    // The sysvar key rides the `signal` string field.
                    4 => TriggerCond::SysVar {
                        key: c.signal.clone(),
                        threshold: c.threshold,
                        rising: c.rising,
                    },
                    // A FlexRay rule's `ch` is a FlexRay bus index and its `id`
                    // the slot; kind 5/6 is what says so.
                    5 => TriggerCond::FrFramePresent {
                        bus: c.ch,
                        slot: c.id.min(u32::from(u16::MAX)) as u16,
                    },
                    6 => TriggerCond::FrSignalCross {
                        bus: c.ch,
                        slot: c.id.min(u32::from(u16::MAX)) as u16,
                        signal: c.signal.clone(),
                        threshold: c.threshold,
                        rising: c.rising,
                    },
                    _ => return None,
                };
                let action = match c.action {
                    1 => TriggerAction::StopRecording,
                    2 => TriggerAction::Send {
                        ch: c.send_ch,
                        id: c.send_id,
                    },
                    3 => TriggerAction::ClearTrace,
                    4 => TriggerAction::InsertMarker,
                    _ => TriggerAction::StartRecording,
                };
                let mut t = crate::trigger::Trigger::new(cond, action);
                t.enabled = c.enabled;
                Some(t)
            })
            .collect();
        app.send(crate::bus::BusCommand::SetTriggers(triggers));
        app.settle();
        app.trig_draft = None;
        app.spec_tol_pct = self.spec.tolerance_percent;
        app.spec_grace = self.spec.grace_cycles.max(1);
        app.replay_speed = self.replay_speed.clamp(0.01, 100.0);
        // The FlexRay descriptions: each bus's file is parsed back into that
        // bus's database so a FlexRay replay decodes against the same network.
        // A file that moved or broke is reported, not fatal.
        // `fr_buses` is per-bus; the older single path describes bus 0, the
        // only bus that session could have watched.
        let legacy = (self.fr_buses.is_empty() && self.fr_fibex.is_some())
            .then(|| FrBusFile {
                bus: 0,
                path: self.fr_fibex.clone().unwrap_or_default(),
            });
        for file in self.fr_buses.iter().chain(legacy.iter()) {
            let path = &file.path;
            match std::fs::read(path)
                .map_err(|e| e.to_string())
                .map(crate::dbc::text_from_bytes)
                .and_then(|t| crate::fr_db::FrDb::parse(&t))
            {
                Ok(db) => {
                    app.fr_buses.insert(
                        file.bus,
                        crate::app::FrBusCfg {
                            path: path.clone(),
                            db: std::sync::Arc::new(db),
                        },
                    );
                }
                Err(e) => problems.push(format!("FlexRay 描述加载失败: {e}")),
            }
        }
        // Names come back before anything that labels a bus, and they survive a
        // description that failed to load: a name is the user's, not the file's.
        for (bus, name) in &self.fr_names {
            app.set_flexray_name(*bus, name);
        }
        // The descriptions move to the core with the rest of the restored bus
        // state, so a replay or a watch attach finds them where the user left
        // them -- no lazy re-push at the moment of use.
        app.push_fr_db_to_core();
        // The FlexRay generator entries are rebuilt *after* the descriptions
        // land, because their name, payload length and send period are read out
        // of the description: restored earlier, every entry would come back as a
        // raw 8-byte filler.
        let stale_fr: Vec<(u8, u16)> = app
            .snap
            .fr_tx
            .iter()
            .map(|t| (t.bus, t.slot))
            .collect();
        for (bus, slot) in stale_fr {
            app.send(crate::bus::BusCommand::RemoveFrEntry { bus, slot });
        }
        let fr_slots: Vec<(u8, u16)> = self
            .fr_tx
            .iter()
            .map(|t| (t.bus, t.slot))
            .collect();
        for (bus, slot) in fr_slots {
            app.add_fr_tx(bus, slot);
        }
        app.settle();
        for t in self.fr_tx {
            let cycle_us = if t.cycle_us == 0 { 0 } else { t.cycle_us.max(1_000) };
            app.send(crate::bus::BusCommand::SetFrEntryCycle {
                bus: t.bus,
                slot: t.slot,
                cycle_us,
            });
            if !t.data_text.is_empty() {
                app.send(crate::bus::BusCommand::SetFrEntryHex {
                    bus: t.bus,
                    slot: t.slot,
                    text: t.data_text,
                });
            }
            for src in t.srcs.into_iter().filter_map(value_src) {
                app.send(crate::bus::BusCommand::SetFrEntrySource {
                    bus: t.bus,
                    slot: t.slot,
                    src,
                });
            }
            app.send(crate::bus::BusCommand::SetFrEntryActive {
                bus: t.bus,
                slot: t.slot,
                on: t.active,
            });
        }
        app.set_window_counters(self.counters);
        app.recent_dbc = self.recent_dbc;
        app.recent_log = self.recent_log;
        // Restored signal lists need their subscriptions recreated,
        // otherwise they render grey and never receive values.
        let keys: Vec<crate::observe::SigKey> = app
            .graphics
            .iter()
            .flat_map(|g| g.signals.iter().map(|s| s.key.clone()))
            .chain(
                app.data_windows
                    .iter()
                    .flat_map(|d| d.signals.iter().map(|s| s.key.clone())),
            )
            .chain(
                app.state_trackers
                    .iter()
                    .flat_map(|w| w.signals.iter().map(|s| s.key.clone())),
            )
            .chain(app.monitor_rows.iter().map(|r| r.key.clone()))
            .collect();
        for key in keys {
            app.subscribe(key);
        }
        if !self.desktops.is_empty() {
            app.desktops = self
                .desktops
                .into_iter()
                .enumerate()
                .map(|(i, d)| Desktop {
                    name: if d.name.trim().is_empty() {
                        format!("Desktop {}", i + 1)
                    } else {
                        d.name
                    },
                    layout: d.layout,
                    open_windows: d
                        .open
                        .into_iter()
                        .filter_map(|(k, n)| WindowKind::from_u8(k).map(|k| (k, n)))
                        .collect(),
                    show_tx: d.show_tx,
                    show_network: d.show_network,
                    show_measurement: d.show_measurement,
                    show_buses: d.show_buses,
                    show_triggers: d.show_triggers,
                    show_bus_stats: d.show_bus_stats,
                    show_spec: d.show_spec,
                    show_id_filter: d.show_id_filter,
                    show_sysvars: d.show_sysvars,
                    show_write: d.show_write,
                })
                .collect();
            app.active_desktop = self.active_desktop.min(app.desktops.len() - 1);
        } else {
            // Legacy config without desktops: fold the restored window state
            // into a single default desktop.
            let mut snap = app.desktop_snapshot();
            snap.name = "Desktop 1".to_string();
            app.desktops = vec![snap];
            app.active_desktop = 0;
        }
        let target = app.desktops[app.active_desktop].clone();
        app.apply_desktop(&target);
        problems
    }
}

impl App {
    /// Restores the legacy `roxy-can.json` workspace if one exists; used
    /// only as a one-time migration when no project meta file is found.
    pub fn load_config(&mut self) {
        let Ok(text) = std::fs::read_to_string(state_path(CONFIG_PATH)) else {
            return;
        };
        match serde_json::from_str::<Config>(&text) {
            Ok(cfg) => {
                let problems = cfg.apply(self);
                self.mark_clean();
                self.report_load_problems(problems);
            }
            Err(e) => self.fail(format!("config ignored: {e}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::NodeRole;

    #[test]
    fn config_round_trips_through_json() {
        let mut app = App::headless();
        app.show_id_filter = true;
        app.replay_speed = 2.0;
        app.trace_windows[0].filter = "Motor".to_string();
        app.trace_windows[0].scope = SigScope::Bus(1);
        app.trace_windows[0]
            .manual
            .insert(crate::workspace::Pick::Can { ch: 1, id: 0x123 });
        // A hand-picked FlexRay slot rides along too, and lands in its own
        // list in the file (`fr_manual`) so an older reader keeps the CAN half.
        app.trace_windows[0]
            .manual
            .insert(crate::workspace::Pick::Fr { bus: 2, slot: 71 });
        // A FlexRay cluster scope is its own numbering space and its own
        // variant, so a project can hold one of each without either moving.
        app.stats_windows[0].scope = SigScope::FrBus(2);
        app.tx_list[0].active = true;
        app.tx_list[0].cycle_us = 50_000;
        app.refresh_snapshot();

        let json = serde_json::to_string(&Config::from_app(&app, None)).unwrap();
        let mut restored = App::headless();
        serde_json::from_str::<Config>(&json)
            .unwrap()
            .apply(&mut restored);

        assert!(restored.show_id_filter);
        assert_eq!(restored.replay_speed, 2.0);
        assert_eq!(restored.trace_windows[0].filter, "Motor");
        assert_eq!(restored.trace_windows[0].scope, SigScope::Bus(1));
        assert_eq!(restored.stats_windows[0].scope, SigScope::FrBus(2));
        assert!(restored.trace_windows[0].manual.contains(&crate::workspace::Pick::Can {
            ch: 1,
            id: 0x123
        }));
        assert!(restored.trace_windows[0].manual.contains(&crate::workspace::Pick::Fr {
            bus: 2,
            slot: 71
        }));
        assert!(restored.tx_list[0].active);
        assert_eq!(restored.tx_list[0].cycle_us, 50_000);
        assert_eq!(restored.channels.len(), app.channels.len());
    }

    /// A project with no analysis windows gets none back.
    ///
    /// The restore used to skip a saved list that was empty, so the six windows
    /// a fresh session boots with reappeared in a project whose user had deleted
    /// them -- "I kept one Trace, and Graphics came back too". `new_project`
    /// also used to miss the State Tracker kind, which is why the empty
    /// workspace here is built through that path.
    #[test]
    fn a_project_without_windows_restores_without_windows() {
        let mut app = App::headless();
        app.new_project();
        assert!(
            app.trace_windows.is_empty()
                && app.msg_windows.is_empty()
                && app.stats_windows.is_empty()
                && app.graphics.is_empty()
                && app.data_windows.is_empty()
                && app.state_trackers.is_empty(),
            "the new-project path empties every kind"
        );

        let json = serde_json::to_string(&Config::from_app(&app, None)).unwrap();
        let mut restored = App::headless();
        serde_json::from_str::<Config>(&json)
            .unwrap()
            .apply(&mut restored);

        assert!(restored.trace_windows.is_empty(), "no Trace came back");
        assert!(restored.msg_windows.is_empty(), "no Messages came back");
        assert!(restored.stats_windows.is_empty(), "no Statistics came back");
        assert!(restored.graphics.is_empty(), "no Graphics came back");
        assert!(restored.data_windows.is_empty(), "no Data came back");
        assert!(
            restored.state_trackers.is_empty(),
            "no State Tracker came back"
        );
    }

    /// A restored project gets back the generator entries it saved -- and only
    /// those.
    ///
    /// The restore used to rebuild the generator from the databases' own message
    /// list, so opening a project added a row for every message in every attached
    /// DBC: delete entries, save, reopen, and the node's whole transmit list is
    /// back as inactive rows nobody asked for.
    #[test]
    fn a_restored_project_keeps_just_its_own_generator_entries() {
        let mut app = App::headless();
        let all: Vec<(u8, u32)> = app.snap.tx.iter().map(|t| (t.channel, t.id)).collect();
        assert!(
            all.len() > 2,
            "the default project ships several entries to delete from"
        );
        for (ch, id) in all.iter().skip(2) {
            app.send(crate::bus::BusCommand::RemoveEntry { ch: *ch, id: *id });
        }
        app.settle();
        let saved: Vec<(u8, u32)> = app.snap.tx.iter().map(|t| (t.channel, t.id)).collect();
        assert_eq!(saved.len(), 2, "the two kept entries");

        let json = serde_json::to_string(&Config::from_app(&app, None)).unwrap();
        let mut restored = App::headless();
        serde_json::from_str::<Config>(&json)
            .unwrap()
            .apply(&mut restored);

        let back: Vec<(u8, u32)> = restored.snap.tx.iter().map(|t| (t.channel, t.id)).collect();
        assert_eq!(
            back, saved,
            "the project's own entries came back, nothing else was added"
        );
    }

    /// A FlexRay 路 name is the user's opinion, so it survives a save the way a
    /// CAN channel name does -- and it survives a bus that has no description
    /// at all, which is the case for a cluster only a replayed log carries.
    /// The cluster index stays readable beside it because the places a *number*
    /// is typed (the record filter's `FR0:13`, a script's `fr_sig(0, ..)`) are
    /// numbered by index, not by name.
    #[test]
    fn a_flexray_bus_name_round_trips_and_blank_clears_it() {
        let mut app = App::headless();
        app.set_flexray_name(1, "动力总成");
        assert_eq!(app.fr_bus_name(1), "动力总成");
        assert_eq!(app.fr_bus_label(1), "动力总成 (FR1)");
        assert_eq!(app.fr_bus_name(0), "FR0", "another 路 is untouched");
        assert_eq!(
            app.sig_bus_label(&crate::observe::SigKey::Fr {
                bus: 1,
                slot: 13,
                name: "DriveTorque".to_string(),
            }),
            "动力总成",
            "the legends a FlexRay signal appears in follow the name"
        );

        let json = serde_json::to_string(&Config::from_app(&app, None)).unwrap();
        let mut restored = App::headless();
        serde_json::from_str::<Config>(&json)
            .expect("a file with the new key loads")
            .apply(&mut restored);
        assert_eq!(restored.fr_bus_name(1), "动力总成");
        assert_eq!(restored.fr_bus_label(1), "动力总成 (FR1)");

        // Typing the index back, or blanking the box, is "no name": nothing is
        // stored, so a project whose buses were never renamed stays as clean as
        // it was before the field existed.
        app.set_flexray_name(1, "FR1");
        assert_eq!(app.fr_bus_name(1), "FR1");
        assert!(
            unnamed(&app),
            "a 路 called by its own index stores no name"
        );
        app.set_flexray_name(1, "   ");
        assert_eq!(app.fr_bus_name(1), "FR1");
        assert!(unnamed(&app), "a blank box stores no name");
    }

    /// True when `app` would write no FlexRay names into its project file.
    fn unnamed(app: &App) -> bool {
        serde_json::to_value(Config::from_app(app, None)).expect("serialises")["fr_names"]
            .as_array()
            .unwrap()
            .is_empty()
    }

    /// A FlexRay generator entry is a stimulus setup the user built: slot, ECU,
    /// period, bytes and driven signals all come back through the project file.
    /// And a file written before the key existed still loads -- with no entries,
    /// rather than failing or inventing one.
    #[test]
    fn a_flexray_generator_entry_round_trips_through_a_project() {
        use crate::app::GenRow;
        use crate::sim::{SrcKind, ValueSrc};
        let mut app = App::headless();
        // An entry can only exist for a slot the description binds to a sending
        // ECU, so the project under test needs its description on disk too --
        // which is also why the restore loads descriptions before entries.
        let arxml = "assets/arxml/PowerTrain.arxml";
        let text = crate::dbc::text_from_bytes(
            std::fs::read(arxml).expect("the asset is committed"),
        );
        let db = crate::fr_db::FrDb::parse(&text).expect("parses");
        let slot = (0..db.frames.len())
            .find_map(|ix| {
                db.frame_sender(ix)?;
                Some(db.frame_index(ix)?.triggering.slot_id as u16)
            })
            .expect("the asset binds a slot to a sending ECU");
        app.fr_buses.insert(
            0,
            crate::app::FrBusCfg {
                path: arxml.into(),
                db: std::sync::Arc::new(db),
            },
        );
        app.push_fr_db_to_core();
        app.add_fr_tx(0, slot);
        // The width the schedule gives this slot, read before anything is typed
        // into it: the payload must still be that wide after the save.
        let wide = app.fr_tx_list[0].len;
        app.set_fr_tx_hex(0, slot, "AA 55");
        app.set_fr_tx_cycle(0, slot, 20_000);
        app.set_gen_source(
            GenRow::Fr(0),
            ValueSrc::new("Torque", SrcKind::Ramp, 0.0, 100.0),
        );
        app.set_fr_tx_active(0, slot, true);
        app.settle();
        assert_eq!(app.fr_tx_list.len(), 1, "the entry exists to be saved");
        let srcs = app.fr_tx_list[0].srcs.clone();
        let node = app.fr_tx_list[0].node.clone();
        assert!(!node.is_empty(), "the entry carries its declared sender");

        let json = serde_json::to_string(&Config::from_app(&app, None)).unwrap();
        let mut restored = App::headless();
        serde_json::from_str::<Config>(&json)
            .unwrap()
            .apply(&mut restored);
        assert_eq!(restored.fr_tx_list.len(), 1);
        let tx = &restored.fr_tx_list[0];
        assert_eq!((tx.bus, tx.slot), (0, slot), "the slot keeps its 路");
        assert_eq!(tx.node, node, "and its ECU row");
        assert_eq!(tx.cycle_us, 20_000, "and its period");
        assert!(tx.active, "and the On state");
        assert!(
            tx.data_text.starts_with("AA 55"),
            "the typed bytes lead the payload: {}",
            tx.data_text
        );
        assert_eq!(
            tx.len, wide,
            "the slot's own width survived the save -- a FlexRay payload is not a \
             DLC the operator can shorten"
        );
        assert_eq!(tx.data_text.split_whitespace().count(), wide);
        assert_eq!(tx.srcs, srcs, "and the driven signal");
        assert!(
            !restored.snap.fr_tx[0].undescribed,
            "the description came back too, so the slot still decodes"
        );

        // The same file with the key removed is the older project: it loads.
        let mut doc: serde_json::Value = serde_json::from_str(&json).unwrap();
        doc.as_object_mut().unwrap().remove("fr_tx");
        assert!(!doc.to_string().contains("fr_tx"), "the file really is older");
        let mut legacy = App::headless();
        serde_json::from_value::<Config>(doc)
            .expect("a file without the new key loads")
            .apply(&mut legacy);
        assert!(
            legacy.fr_tx_list.is_empty(),
            "and carries no FlexRay entries"
        );
    }

    /// A project written before FlexRay picks existed has no `fr_manual` key at
    /// all. It reads as the CAN-only set it always was -- the new list is
    /// additive, so nothing is renumbered, reinterpreted or dropped.
    #[test]
    fn a_project_file_without_flexray_picks_reads_as_can_only() {
        let mut app = App::headless();
        app.trace_windows[0]
            .manual
            .insert(crate::workspace::Pick::Can { ch: 1, id: 0x123 });
        app.trace_windows[0]
            .manual
            .insert(crate::workspace::Pick::Fr { bus: 0, slot: 13 });
        app.refresh_snapshot();
        let json = serde_json::to_string(&Config::from_app(&app, None)).unwrap();
        let mut doc: serde_json::Value = serde_json::from_str(&json).unwrap();
        // All three window kinds gained the key, so an old file lacks it in all
        // three -- stripping one and leaving the others would not be the file
        // this test claims to read.
        for kind in ["trace_windows", "msg_windows", "stats_windows"] {
            for w in doc[kind].as_array_mut().unwrap() {
                w.as_object_mut().unwrap().remove("fr_manual");
            }
        }
        assert!(
            !doc.to_string().contains("fr_manual"),
            "the edited file really looks like an old one"
        );

        let mut restored = App::headless();
        serde_json::from_value::<Config>(doc)
            .expect("a file without the new key loads")
            .apply(&mut restored);
        assert!(
            restored.trace_windows[0]
                .manual
                .contains(&crate::workspace::Pick::Can { ch: 1, id: 0x123 })
        );
        assert_eq!(
            restored.trace_windows[0].manual.len(),
            1,
            "only the CAN pick an old file could have meant"
        );
    }

    /// A waveform that does not survive a save comes back flat, which is the
    /// kind of loss a project file exists to prevent.
    #[test]
    fn tx_value_sources_round_trip() {
        let mut app = App::headless();
        app.set_source(
            0,
            ValueSrc {
                period_us: 2_000_000,
                phase_us: 250_000,
                ..ValueSrc::new("EngineSpeed", SrcKind::Sine, 0.0, 8000.0)
            },
        );
        app.set_source(
            0,
            ValueSrc {
                seq: vec![1.0, 2.5, 4.0],
                ..ValueSrc::new("GearPosition", SrcKind::Step, 0.0, 6.0)
            },
        );
        app.set_source(
            0,
            ValueSrc {
                seed: 99,
                redraw_us: 5_000,
                ..ValueSrc::new("ThrottlePos", SrcKind::Random, 0.0, 100.0)
            },
        );

        let json = serde_json::to_string(&Config::from_app(&app, None)).unwrap();
        assert!(json.contains(r#""srcs""#), "the key is written");
        let mut restored = App::headless();
        serde_json::from_str::<Config>(&json)
            .unwrap()
            .apply(&mut restored);
        assert_eq!(restored.tx_list[0].srcs, app.tx_list[0].srcs);
    }

    /// v0.3 project files have no `srcs` key at all: they must load with
    /// nothing driven, which is exactly how those generators behave today.
    #[test]
    fn legacy_tx_entry_without_srcs_stays_flat() {
        let mut restored = App::headless();
        let legacy = r#"{"tx":[{"channel":0,"id":256,"active":true,"cycle_us":20000,
                               "data_text":"01 02","data":[1,2]}]}"#;
        serde_json::from_str::<Config>(legacy)
            .unwrap()
            .apply(&mut restored);
        let tx = restored
            .tx_list
            .iter()
            .find(|t| t.id == 0x100)
            .expect("EngineStatus entry");
        assert!(tx.srcs.is_empty(), "an absent key means no driven signal");
        assert!(tx.active, "the rest of the entry still applied");
        assert_eq!(tx.len, 2);
    }

    /// A source written by a newer build must vanish on its own rather than
    /// come back as some other shape.
    #[test]
    fn an_unknown_kind_code_drops_only_that_source() {
        let mut restored = App::headless();
        let json = r#"{"tx":[{"channel":0,"id":256,"srcs":[{"name":"EngineSpeed","kind":1,"lo":0.0,"hi":8000.0,"period_us":2000000},{"name":"GearPosition","kind":200}]}]}"#;
        serde_json::from_str::<Config>(json)
            .unwrap()
            .apply(&mut restored);
        let tx = restored
            .tx_list
            .iter()
            .find(|t| t.id == 0x100)
            .expect("EngineStatus entry");
        assert_eq!(tx.srcs.len(), 1, "only the unreadable entry goes");
        assert_eq!(tx.srcs[0].name, "EngineSpeed");
        assert_eq!(tx.srcs[0].kind, SrcKind::Sine, "code 1 is Sine");
        assert_eq!(tx.srcs[0].period_us, 2_000_000);
    }

    /// A stored cycle of 0 means event-triggered, so the load-time floor must
    /// not resurrect it as a 1 ms cyclic sender -- but it must still catch
    /// genuinely bogus small values.
    #[test]
    fn an_event_triggered_cycle_survives_a_project_round_trip() {
        let mut app = App::headless();
        let i = app
            .tx_list
            .iter()
            .position(|t| t.channel == 0 && t.id == 0x100)
            .expect("EngineStatus entry");
        app.tx_list[i].cycle_us = 0;
        let j = app
            .tx_list
            .iter()
            .position(|t| t.channel == 0 && t.id == 0x200)
            .expect("VehicleState entry");
        app.tx_list[j].cycle_us = 500;
        app.refresh_snapshot();

        let json = serde_json::to_string(&Config::from_app(&app, None)).unwrap();
        let mut restored = App::headless();
        serde_json::from_str::<Config>(&json)
            .unwrap()
            .apply(&mut restored);
        let find = |id: u32| {
            restored
                .tx_list
                .iter()
                .find(|t| t.channel == 0 && t.id == id)
                .map(|t| t.cycle_us)
        };
        assert_eq!(find(0x100), Some(0), "0 must not come back as 1ms");
        assert_eq!(
            find(0x200),
            Some(1_000),
            "the anti-typo floor still applies"
        );
    }

    /// The Monitor window's rows persist with the project: the watched
    /// keys, labels, digits, and coloring rules all round-trip.
    #[test]
    fn monitor_rows_round_trip_with_the_project() {
        let mut app = App::headless();
        app.set_win_signal(
            crate::workspace::PopupTarget::Monitor,
            SigKey::can(0, 0x100, false, "EngineStatus"),
            true,
        );
        assert_eq!(app.monitor_rows.len(), 1, "the row was added");
        app.monitor_rows[0].digits = 2;
        app.monitor_rows[0].rule_on = true;
        app.monitor_rows[0].rising = false;
        app.monitor_rows[0].threshold = 87.5;
        app.monitor_rows[0].color = 3;

        let json = serde_json::to_string(&Config::from_app(&app, None)).unwrap();
        let mut restored = App::headless();
        serde_json::from_str::<Config>(&json)
            .unwrap()
            .apply(&mut restored);
        assert_eq!(restored.monitor_rows.len(), 1);
        let r = &restored.monitor_rows[0];
        assert!(
            matches!(
                r.key,
                crate::observe::SigKey::Can {
                    ch: 0,
                    id: 0x100,
                    ext: false,
                    ..
                }
            ),
            "the round trip keeps a standard-class CAN key"
        );
        assert_eq!(r.key.name(), "EngineStatus");
        assert_eq!((r.digits, r.rule_on, r.rising, r.threshold, r.color),
            (2, true, false, 87.5, 3));
    }

    /// Which roles a bus declares has to outlive the session, or the bus
    /// composition a user set up is lost on every reload. Restoring it must
    /// not, however, start any traffic by itself.
    #[test]
    fn node_roles_round_trip_without_starting_traffic() {
        let mut app = App::headless();
        app.channels[1].set_node_role("ABS", NodeRole::Absent);
        app.channels[1].set_node_role("GearBox", NodeRole::Simulated);
        app.refresh_snapshot();
        let json = serde_json::to_string(&Config::from_app(&app, None)).unwrap();
        let mut restored = App::headless();
        serde_json::from_str::<Config>(&json)
            .unwrap()
            .apply(&mut restored);
        assert_eq!(
            restored.channels[1].role_of("ABS"),
            NodeRole::Absent,
            "an offline node stays offline"
        );
        assert_eq!(
            restored.channels[1].role_of("GearBox"),
            NodeRole::Simulated,
            "a simulated node keeps its declaration"
        );
        assert_eq!(
            restored.channels[1].role_of("DashBoard"),
            NodeRole::Absent,
            "an undeclared node is absent, not an error"
        );
        assert!(
            restored.channels[0].node_roles.is_empty(),
            "the other bus keeps its own map"
        );
        assert!(
            restored.tx_list.iter().all(|t| !t.active),
            "a read-only look at a saved project must not begin transmitting"
        );
    }

    /// Old projects carry the simulated names as a plain list; they load
    /// as `Simulated` role declarations. Role names the program does not
    /// know are dropped, per the unknown-code convention -- including the
    /// retired `Monitor` word, which the two-state model folds into
    /// `Absent`.
    #[test]
    fn legacy_sim_nodes_become_roles_and_unknown_roles_are_dropped() {
        let cfg: Config = serde_json::from_str(
            r#"{"channels":[{"name":"A","dbc_path":"assets/sample.dbc",
                 "sim_nodes":["ABS"],
                 "node_roles":{"GearBox":"Monitor","DashBoard":"Bogus"}}]}"#,
        )
        .unwrap();
        assert_eq!(cfg.channels[0].sim_nodes, ["ABS"]);
        assert_eq!(
            cfg.channels[0]
                .node_roles
                .get("DashBoard")
                .map(String::as_str),
            Some("Bogus"),
            "the raw file keeps unknown names; the restore filters them"
        );
        let mut app = App::headless();
        cfg.apply(&mut app);
        assert_eq!(
            app.channels[0].role_of("ABS"),
            NodeRole::Simulated,
            "the legacy simulated list migrated"
        );
        assert_eq!(
            app.channels[0].role_of("GearBox"),
            NodeRole::Absent,
            "the retired Monitor word lands as Absent, the same behaviour"
        );
        assert_eq!(
            app.channels[0].role_of("DashBoard"),
            NodeRole::Absent,
            "an unknown role name is dropped, not guessed"
        );
    }

    /// Projects saved before simulated nodes existed carry no `sim_nodes` key.
    /// They must load with nothing simulated -- and `name`/`dbc_path` must stay
    /// required, since accepting a nameless bus would hide real corruption.
    #[test]
    fn legacy_channel_config_without_sim_nodes_still_loads() {
        let cfg: Config =
            serde_json::from_str(r#"{"channels":[{"name":"A","dbc_path":"assets/sample.dbc"}]}"#)
                .unwrap();
        assert_eq!(cfg.channels.len(), 1);
        assert!(cfg.channels[0].sim_nodes.is_empty());
        assert!(cfg.channels[0].node_roles.is_empty());
        assert!(
            serde_json::from_str::<Config>(r#"{"channels":[{"dbc_path":"x.dbc"}]}"#).is_err(),
            "name is still required"
        );
    }

    /// Projects saved while FlexRay signals borrowed a CAN id (bit 30 set, the
    /// slot in the low bits) must come back as real FlexRay keys, and saving
    /// them again must state the slot outright -- a key that round-trips
    /// through a fake arbitration id would break the moment a second cluster
    /// or a real id range touches it.
    #[test]
    fn legacy_flexray_keys_migrate_and_stop_being_synthetic() {
        let cfg: Config = serde_json::from_str(
            r#"{"graphics":[{"name":"G","opened":true,"signals":[
                 {"ch":0,"id":1073741837,"signal":"CarSpeed"}]}]}"#,
        )
        .unwrap();
        let mut restored = App::headless();
        cfg.apply(&mut restored);
        let key = &restored.graphics[0].signals[0].key;
        assert!(
            matches!(key, crate::observe::SigKey::Fr { bus: 0, slot: 13, .. }),
            "the synthetic id loads as a FlexRay key, not a CAN message: {key:?}"
        );
        assert_eq!(key.name(), "CarSpeed");

        let json = serde_json::to_string(&Config::from_app(&restored, None)).unwrap();
        assert!(json.contains("\"fr_slot\":13"), "the slot says so: {json}");
        assert!(
            !json.contains("1073741837"),
            "no synthetic arbitration id survives a save: {json}"
        );
        let mut again = App::headless();
        serde_json::from_str::<Config>(&json)
            .unwrap()
            .apply(&mut again);
        assert_eq!(
            again.graphics[0].signals[0].key, *key,
            "the key is stable across saves"
        );
    }

    /// The FR watch of a project saved before FlexRay had a bus index states
    /// one description file: it describes bus 0, the only bus such a session
    /// could have watched. Saving again must state the bus outright, and a
    /// second cluster has to survive the trip.
    #[test]
    fn a_legacy_single_fibex_path_becomes_a_bus_entry() {
        let arxml = "assets/arxml/PowerTrain.arxml";
        let fibex = "assets/fibex/PowerTrain_v2.xml";
        if !std::path::Path::new(arxml).exists() || !std::path::Path::new(fibex).exists() {
            panic!("{arxml} or {fibex} missing -- it is tracked in the repository, so a broken checkout must fail, not skip");
        }
        let cfg: Config =
            serde_json::from_str(&format!(r#"{{"fr_fibex":"{arxml}"}}"#)).unwrap();
        let mut restored = App::headless();
        cfg.apply(&mut restored);
        assert_eq!(
            restored.fr_buses.keys().copied().collect::<Vec<_>>(),
            vec![0],
            "the legacy path lands on bus 0"
        );
        assert!(restored.fr_db(0).is_some(), "and parses into a description");
        assert_eq!(
            restored.fr_dbs.keys().copied().collect::<Vec<_>>(),
            vec![0],
            "the core is handed what the project loaded, so a replay decodes \
             without pushing again at the moment of use"
        );

        // A second cluster on a second bus, as the Buses window's picker
        // would have left it.
        let text = crate::dbc::text_from_bytes(std::fs::read(fibex).expect("fibex"));
        let db = crate::fr_db::FrDb::parse(&text).expect("the bundled FIBEX parses");
        restored.fr_buses.insert(
            1,
            crate::app::FrBusCfg {
                path: fibex.to_string(),
                db: std::sync::Arc::new(db),
            },
        );

        let json = serde_json::to_string(&Config::from_app(&restored, None)).unwrap();
        assert!(
            json.contains(r#""fr_buses":[{"bus":0"#) && json.contains(r#"},{"bus":1,"#),
            "both buses are stated: {json}"
        );
        assert!(
            !json.contains("\"fr_fibex\""),
            "the legacy field is not written again: {json}"
        );

        let mut again = App::headless();
        serde_json::from_str::<Config>(&json)
            .unwrap()
            .apply(&mut again);
        assert_eq!(
            again.fr_buses.keys().copied().collect::<Vec<_>>(),
            vec![0, 1],
            "each bus keeps its own description"
        );
        assert_eq!(
            again.fr_dbs.keys().copied().collect::<Vec<_>>(),
            vec![0, 1],
            "and the core is told about both"
        );
        assert_ne!(
            again.fr_db(0).expect("bus 0").frames[0].name,
            again.fr_db(1).expect("bus 1").frames[0].name,
            "the two entries are the two files, not one copied twice"
        );
    }

    /// Deleting a 路 leaves a hole, and the hole is exactly what the project
    /// file states: every description carries its own cluster index, so the
    /// survivor comes back as FR1 rather than shifted into FR0's place -- which
    /// would point a recording's `clusterNo 1` rows, a script's `fr_sig(1, ..)`
    /// and the filter's `FR1:5` at the wrong schedule.
    #[test]
    fn a_removed_flexray_bus_does_not_renumber_the_survivor_through_a_save() {
        let arxml = "assets/arxml/PowerTrain.arxml";
        let fibex = "assets/fibex/PowerTrain_v2.xml";
        if !std::path::Path::new(arxml).exists() || !std::path::Path::new(fibex).exists() {
            panic!("{arxml} or {fibex} missing -- it is tracked in the repository, so a broken checkout must fail, not skip");
        }
        let mut app = App::headless();
        assert_eq!(app.load_cluster_description(arxml, Some(0)), Some(0));
        assert_eq!(app.load_cluster_description(fibex, Some(1)), Some(1));
        app.set_flexray_name(0, "动力总成");
        app.set_flexray_name(1, "底盘");
        app.remove_fr_bus(0);

        let json = serde_json::to_string(&Config::from_app(&app, None)).expect("serialises");
        assert!(
            !json.contains("动力总成"),
            "the deleted 路's name went with it: {json}"
        );
        let mut again = App::headless();
        serde_json::from_str::<Config>(&json)
            .expect("parses")
            .apply(&mut again);
        assert_eq!(
            again.fr_buses.keys().copied().collect::<Vec<_>>(),
            [1],
            "FR1 stayed FR1"
        );
        assert_eq!(again.fr_bus_name(1), "底盘", "with the name it was given");
        assert_eq!(
            again.next_flexray_bus(),
            Some(0),
            "and the freed index is what the next 路 added takes"
        );
    }

    /// A FlexRay rule persists as kind 5/6, which is what states that `ch` names
    /// a FlexRay bus and `id` a schedule slot. Reloaded as any other kind it
    /// would become a CAN watch on a made-up arbitration id.
    #[test]
    fn a_flexray_trigger_keeps_its_kind_and_slot() {
        use crate::trigger::{Trigger, TriggerAction, TriggerCond};
        let mut app = App::headless();
        app.triggers.push(Trigger::new(
            TriggerCond::FrFramePresent { bus: 1, slot: 13 },
            TriggerAction::StartRecording,
        ));
        app.triggers.push(Trigger::new(
            TriggerCond::FrSignalCross {
                bus: 0,
                slot: 24,
                signal: "CarSpeed".into(),
                threshold: 118.0,
                rising: false,
            },
            TriggerAction::InsertMarker,
        ));
        app.refresh_snapshot();
        let json = serde_json::to_string(&Config::from_app(&app, None)).unwrap();
        assert!(
            json.contains(r#""kind":5"#) && json.contains(r#""id":13"#) && json.contains(r#""ch":1"#),
            "the slot is stored as a slot: {json}"
        );
        assert!(
            json.contains(r#""kind":6"#)
                && json.contains(r#""id":24"#)
                && json.contains("CarSpeed")
                && json.contains(r#""rising":false"#),
            "so is the signal, its threshold and its edge: {json}"
        );

        let mut restored = App::headless();
        serde_json::from_str::<Config>(&json)
            .unwrap()
            .apply(&mut restored);
        assert_eq!(
            restored.snap.triggers[0].cond,
            TriggerCond::FrFramePresent { bus: 1, slot: 13 },
            "and comes back as the same cluster's slot"
        );
        assert_eq!(
            restored.snap.triggers[1].cond,
            TriggerCond::FrSignalCross {
                bus: 0,
                slot: 24,
                signal: "CarSpeed".into(),
                threshold: 118.0,
                rising: false,
            },
            "the FlexRay crossing condition survives whole"
        );
        assert_eq!(restored.snap.triggers[1].action, TriggerAction::InsertMarker);
    }

    /// The bitrates feed the load view's arithmetic, so a saved opinion about
    /// them must come back exactly.
    #[test]
    fn bitrates_round_trip() {
        let mut app = App::headless();
        app.channels[0].bitrate_kbps = 1_000;
        app.channels[0].fd_data_kbps = 5_000;
        app.refresh_snapshot();
        let json = serde_json::to_string(&Config::from_app(&app, None)).unwrap();
        let mut restored = App::headless();
        serde_json::from_str::<Config>(&json)
            .unwrap()
            .apply(&mut restored);
        assert_eq!(restored.channels[0].bitrate_kbps, 1_000);
        assert_eq!(restored.channels[0].fd_data_kbps, 5_000);
        assert_eq!(
            restored.channels[1].bitrate_kbps,
            crate::app::Channel::DEFAULT_BITRATE_KBPS,
            "the other bus keeps its own rate"
        );
    }

    /// Projects saved before the load view existed carry no bitrate keys;
    /// they load at the defaults rather than failing or reading zero.
    #[test]
    fn legacy_channel_config_without_bitrates_loads_at_defaults() {
        let cfg: Config =
            serde_json::from_str(r#"{"channels":[{"name":"A","dbc_path":"assets/sample.dbc"}]}"#)
                .unwrap();
        assert_eq!(
            cfg.channels[0].bitrate_kbps,
            crate::app::Channel::DEFAULT_BITRATE_KBPS
        );
        assert_eq!(
            cfg.channels[0].fd_data_kbps,
            crate::app::Channel::DEFAULT_FD_DATA_KBPS
        );
    }

    /// The trace ring's retention is a project opinion about how much
    /// history to keep, so it survives a save.
    #[test]
    fn the_trace_capacity_round_trips() {
        let mut app = App::headless();
        app.trace_limit = 500_000;
        app.set_trace_limit(500_000);
        let json = serde_json::to_string(&Config::from_app(&app, None)).unwrap();
        let mut restored = App::headless();
        serde_json::from_str::<Config>(&json)
            .unwrap()
            .apply(&mut restored);
        assert_eq!(restored.trace_limit, 500_000);
        let default = App::headless();
        let plain = serde_json::to_string(&Config::from_app(&default, None)).unwrap();
        assert!(
            plain.contains(&format!(r#""trace_limit":{}"#, crate::app::TRACE_LIMIT)),
            "the default capacity is written too"
        );
    }

    /// The time-range bounds, the expanded-filter row and the FlexRay signal
    /// expansion are filter state, so they survive a save like the rest of the
    /// row does: a window that hides everything outside 1.0–2.5 s has to show
    /// those boxes filled, or it reads as a broken table rather than as a
    /// remembered filter.
    #[test]
    fn the_trace_time_range_and_expansion_round_trip() {
        let mut app = App::headless();
        let w = &mut app.trace_windows[0];
        w.time_from = "1.0".to_string();
        w.time_to = "2.5".to_string();
        w.filters_open = true;
        w.fr_expand = true;
        let json = serde_json::to_string(&Config::from_app(&app, None)).unwrap();

        let mut restored = App::headless();
        serde_json::from_str::<Config>(&json)
            .unwrap()
            .apply(&mut restored);
        let w = &restored.trace_windows[0];
        assert_eq!(
            (w.time_from.as_str(), w.time_to.as_str()),
            ("1.0", "2.5"),
            "the bounds come back"
        );
        assert!(
            w.filters_open && w.fr_expand,
            "and the rows that hold them are open"
        );
        let flt = w.filter_lens();
        assert_eq!(
            (flt.from_s, flt.to_s),
            (Some(1.0), Some(2.5)),
            "and they still filter"
        );

        // A file written before these keys existed reads as it always did.
        let mut doc: serde_json::Value = serde_json::from_str(&json).unwrap();
        for w in doc["trace_windows"].as_array_mut().unwrap() {
            let obj = w.as_object_mut().unwrap();
            for key in ["time_from", "time_to", "filters_open", "fr_expand"] {
                obj.remove(key);
            }
        }
        let mut legacy = App::headless();
        serde_json::from_value::<Config>(doc)
            .expect("an older file still loads")
            .apply(&mut legacy);
        let w = &legacy.trace_windows[0];
        assert!(
            w.time_from.is_empty() && w.time_to.is_empty() && !w.fr_expand,
            "and carries no time bounds"
        );
    }

    /// Trigger-recording context sizes are project opinions too: they
    /// round-trip, and the defaults are what the constants carried.
    #[test]
    fn run_limits_round_trip() {
        let mut app = App::headless();
        assert_eq!(app.limits.pre_frames, 256, "the constant was the default");
        app.limits.pre_frames = 1_024;
        app.limits.post_frames = 128;
        app.limits.marker_cap = 2_048;
        let json = serde_json::to_string(&Config::from_app(&app, None)).unwrap();
        let mut restored = App::headless();
        serde_json::from_str::<Config>(&json)
            .unwrap()
            .apply(&mut restored);
        assert_eq!(restored.limits.pre_frames, 1_024);
        assert_eq!(restored.limits.post_frames, 128);
        assert_eq!(restored.limits.marker_cap, 2_048);
    }

    /// The format version travels with the file: new files carry the
    /// current version, files from before the field existed read as 0.
    /// Both load -- the number is for future migrations to branch on.
    #[test]
    fn the_project_file_carries_its_schema_version() {
        let app = App::headless();
        let json = serde_json::to_string(&Config::from_app(&app, None)).unwrap();
        assert!(
            json.contains(r#""schema_version":1"#),
            "new files are stamped with the current version"
        );
        let cfg: Config =
            serde_json::from_str(r#"{"channels":[{"name":"A","dbc_path":"assets/sample.dbc"}]}"#)
                .unwrap();
        assert_eq!(cfg.schema_version, 0, "pre-versioning files read as 0");
    }

    /// The two monitor thresholds are a project-level opinion about how
    /// strictly to read the database, so they must survive a save.
    #[test]
    fn spec_settings_survive_a_project_round_trip() {
        let mut app = App::headless();
        app.spec_tol_pct = 25;
        app.spec_grace = 8;
        let json = serde_json::to_string(&Config::from_app(&app, None)).unwrap();
        let mut restored = App::headless();
        serde_json::from_str::<Config>(&json)
            .unwrap()
            .apply(&mut restored);
        assert_eq!(restored.spec_tol_pct, 25);
        assert_eq!(restored.spec_grace, 8);
    }

    /// Projects saved before the monitor existed carry no `spec` block, and a
    /// hand-edited block may name only one of the two keys: each defaults on
    /// its own. A grace of zero would condemn every message not received in
    /// the current step, so the floor is applied on the way in.
    #[test]
    fn an_old_project_without_a_spec_block_loads_the_defaults() {
        let mut restored = App::headless();
        serde_json::from_str::<Config>(r#"{"channels":[]}"#)
            .unwrap()
            .apply(&mut restored);
        assert_eq!(restored.spec_tol_pct, crate::spec::TOLERANCE_PERCENT);
        assert_eq!(restored.spec_grace, crate::spec::GRACE_CYCLES);

        let partial: Config = serde_json::from_str(r#"{"spec":{"grace_cycles":5}}"#).unwrap();
        assert_eq!(
            partial.spec.tolerance_percent,
            crate::spec::TOLERANCE_PERCENT,
            "the key that was not written keeps its own default"
        );

        let mut zeroed = App::headless();
        serde_json::from_str::<Config>(r#"{"spec":{"grace_cycles":0}}"#)
            .unwrap()
            .apply(&mut zeroed);
        assert_eq!(
            zeroed.spec_grace, 1,
            "a grace of zero cycles is not a thing"
        );
    }

    #[test]
    fn missing_fields_fall_back_to_defaults() {
        let cfg: Config = serde_json::from_str("{}").unwrap();
        assert!(cfg.show_tx);
        assert_eq!(cfg.replay_speed, 1.0);
        assert!(cfg.channels.is_empty());
    }

    /// v0.2.x persisted `recent_asc`; the field became `recent_log` when BLF
    /// joined. The alias keeps an old `roxy-can.json` loadable, and the next
    /// save must migrate it forward rather than keep emitting the old key.
    #[test]
    fn legacy_recent_asc_key_migrates_to_recent_log() {
        let mut restored = App::headless();
        serde_json::from_str::<Config>(r#"{"recent_asc":["old1.asc","old2.asc"]}"#)
            .unwrap()
            .apply(&mut restored);
        assert_eq!(restored.recent_log, ["old1.asc", "old2.asc"]);

        let json = serde_json::to_string(&Config::from_app(&restored, None)).unwrap();
        assert!(json.contains(r#""recent_log""#), "writes the new key");
        assert!(!json.contains("recent_asc"), "stops writing the legacy key");
    }

    /// Project files saved before the Dots toggle existed must keep the
    /// behaviour they already had: markers default to on.
    #[test]
    fn graphics_config_defaults_markers_on_for_older_projects() {
        let g: GfxCfg = serde_json::from_str(r#"{"name":"G1","opened":true}"#).unwrap();
        assert!(g.show_markers, "absent key keeps markers on");
        let off: GfxCfg =
            serde_json::from_str(r#"{"name":"G1","opened":true,"show_markers":false}"#).unwrap();
        assert!(!off.show_markers, "an explicit choice is honoured");
    }

    #[test]
    fn dbc_paths_relativize_and_resolve_round_the_project_dir() {
        let base = Path::new("C:/work/myproj");
        let inside = "C:/work/myproj/dbc/motor.dbc";
        assert_eq!(relativize(inside, base), "dbc/motor.dbc");
        assert_eq!(
            relativize("C:/elsewhere/other.dbc", base),
            "C:/elsewhere/other.dbc",
            "paths outside the project stay absolute"
        );
        let rel = relativize("assets/sample.dbc", base);
        assert!(
            Path::new(&rel).is_absolute(),
            "CWD-relative paths are stored absolute when outside the project"
        );
        assert!(
            Path::new(&rel).ends_with("assets/sample.dbc"),
            "absolutized path keeps its tail"
        );

        assert_eq!(
            resolve_dbc("missing.dbc", Some(base)),
            "missing.dbc",
            "project-relative path without a real file falls back to the CWD form"
        );
        assert_eq!(
            resolve_dbc("C:/abs/x.dbc", Some(base)),
            "C:/abs/x.dbc",
            "absolute paths are untouched"
        );
        // A file that really exists next to the project resolves there.
        let tmp = std::env::temp_dir().join("roxy_can_resolve_test");
        std::fs::create_dir_all(&tmp).unwrap();
        let f = tmp.join("motor.dbc");
        std::fs::write(&f, "x").unwrap();
        assert_eq!(
            resolve_dbc("motor.dbc", Some(tmp.as_path())),
            f.to_string_lossy()
        );
        std::fs::remove_dir_all(&tmp).ok();
    }

    #[test]
    fn project_file_round_trips_layout_and_config() {
        let app = App::headless();
        let proj = ProjectFile {
            version: 1,
            layout: "[Window][Trace 1]\nPos=10,20\n[Docking][Data]\n".to_string(),
            project: None,
            config: Config::from_app(&app, None),
        };
        let text = serde_json::to_string(&proj).unwrap();
        let back: ProjectFile = serde_json::from_str(&text).unwrap();
        assert_eq!(back.version, 1);
        assert!(back.layout.contains("[Docking][Data]"));
        assert_eq!(back.project, None);
        assert_eq!(back.config.channels.len(), app.channels.len());
    }
}
