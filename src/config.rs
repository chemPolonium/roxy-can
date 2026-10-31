//! Workspace persistence as project files (.rxproj): buses, observer
//! windows, signals, filters and the generator are bundled in one JSON
//! file. A small `roxy-can.meta.json` remembers the last opened project.
use std::collections::HashMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::app::{
    App, DataWindow, GfxSignal, GraphicsWindow, MsgWin, SigScope, StatsWin, TraceWin, YMode,
};
use crate::observe::SigKey;
use crate::sim::{SrcKind, ValueSrc};

pub const PROJECT_EXT: &str = "rxproj";

/// Stores a DBC path relative to the project directory when possible, so a
/// project folder can be moved or shared; paths outside the project are
/// stored absolute.
pub fn relativize(p: &str, base: &Path) -> String {
    if p.starts_with("builtin:") {
        return p.to_string();
    }
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
    if p.trim().is_empty() {
        return String::new();
    }
    if p.starts_with("builtin:") {
        return p.to_string();
    }
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

/// One project file: the semantic workspace state.
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

/// One window's hand-picked subjects as the file stores them: the two buses
/// each get their own key, so each half is its own list of tuples; Ethernet
/// flows carry their full L2 identity.
type CanPicks = Vec<(u8, u32)>;
type FrPicks = Vec<(u8, u16)>;
type EthPicks = Vec<crate::aggregate::EthFlowKey>;

/// A window's hand-picked subjects, split for the project file: CAN ids in
/// `manual`, FlexRay slots in `fr_manual`, Ethernet flows in `eth_manual`.
/// Sorted so re-saving an unchanged project does not shuffle the lists (a
/// `HashSet` has no order of its own).
fn split_picks(
    picks: &std::collections::HashSet<crate::workspace::Pick>,
) -> (CanPicks, FrPicks, EthPicks) {
    let mut can: Vec<_> = picks
        .iter()
        .filter_map(|p| match *p {
            crate::workspace::Pick::Can { ch, id } => Some((ch, id)),
            crate::workspace::Pick::Fr { .. } | crate::workspace::Pick::Eth(_) => None,
        })
        .collect();
    let mut fr: Vec<_> = picks
        .iter()
        .filter_map(|p| match *p {
            crate::workspace::Pick::Fr { bus, slot } => Some((bus, slot)),
            crate::workspace::Pick::Can { .. } | crate::workspace::Pick::Eth(_) => None,
        })
        .collect();
    let mut eth: Vec<_> = picks
        .iter()
        .filter_map(|p| match *p {
            crate::workspace::Pick::Eth(flow) => Some(flow),
            crate::workspace::Pick::Can { .. } | crate::workspace::Pick::Fr { .. } => None,
        })
        .collect();
    can.sort_unstable();
    fr.sort_unstable();
    eth.sort_unstable();
    (can, fr, eth)
}

/// ...and merged back on load.
fn merge_picks(
    can: CanPicks,
    fr: FrPicks,
    eth: EthPicks,
) -> std::collections::HashSet<crate::workspace::Pick> {
    can.into_iter()
        .map(|(ch, id)| crate::workspace::Pick::Can { ch, id })
        .chain(
            fr.into_iter()
                .map(|(bus, slot)| crate::workspace::Pick::Fr { bus, slot }),
        )
        .chain(eth.into_iter().map(crate::workspace::Pick::Eth))
        .collect()
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TraceCfg {
    pub name: String,
    pub opened: bool,
    pub scope: SigScope,
    /// The window's hand-picked CAN ids. FlexRay slot picks live beside them
    /// in `fr_manual`; the two numbering spaces stay separate.
    #[serde(default)]
    pub manual: CanPicks,
    #[serde(default)]
    pub fr_manual: FrPicks,
    /// Ethernet flow picks; empty when none are selected.
    #[serde(default)]
    pub eth_manual: EthPicks,
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

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MsgCfg {
    pub name: String,
    pub opened: bool,
    pub scope: SigScope,
    #[serde(default)]
    pub manual: CanPicks,
    #[serde(default)]
    pub fr_manual: FrPicks,
    #[serde(default)]
    pub eth_manual: EthPicks,
    #[serde(default)]
    pub filter: String,
    #[serde(default)]
    pub dbc_only: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StatsCfg {
    pub name: String,
    pub opened: bool,
    pub scope: SigScope,
    #[serde(default)]
    pub manual: CanPicks,
    #[serde(default)]
    pub fr_manual: FrPicks,
    #[serde(default)]
    pub eth_manual: EthPicks,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SignalCfg {
    pub ch: u8,
    pub id: u32,
    /// The message's frame class. Defaults to standard.
    #[serde(default)]
    pub ext: bool,
    /// The FlexRay slot carrying the signal, with `ch` naming the FlexRay
    /// bus. Absent for a CAN signal.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fr_slot: Option<u16>,
    /// The ARXML Ethernet frame the signal was decoded from, with `ch`
    /// naming the Ethernet bus. Absent for CAN and FlexRay signals.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub eth_frame: Option<String>,
    /// The definition id of a derived channel. Present for observer rows that
    /// read a computed lane instead of a bus signal.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub derived: Option<String>,
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
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StateOverrideCfg {
    pub value: f64,
    pub color: [f32; 3],
}

/// One signal's custom state bands, CANoe's Value Definition rows. A
/// `None` color means automatic.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RuleCfg {
    pub cuts: Vec<f64>,
    pub names: Vec<String>,
    pub colors: Vec<Option<[f32; 3]>>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
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

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
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
    /// See [`SignalCfg::eth_frame`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub eth_frame: Option<String>,
    /// See [`SignalCfg::derived`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub derived: Option<String>,
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
    /// 控制。缺省为 None（独立脚本）。
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

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StateCfg {
    pub name: String,
    pub opened: bool,
    #[serde(default)]
    pub signals: Vec<SignalCfg>,
    #[serde(default = "twenty_default")]
    pub time_window_s: f64,
    /// Minimum display duration in ms for a state band; shorter bands are
    /// absorbed into their neighbour. Defaults to 0.
    #[serde(default)]
    pub min_shown_ms: u64,
}

fn twenty_default() -> f64 {
    20.0
}

/// One driven signal's parameters. `kind` is [`crate::sim::SrcKind::to_u8`];
/// an unknown code drops the whole entry on load rather than guessing a shape.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
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

#[derive(Serialize, Deserialize, Default, Clone, Debug, PartialEq)]
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
    /// SignalCross only: the watched message's frame class. Defaults to
    /// standard.
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

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectSettings {
    /// Trigger-recording context sizes (pre-trigger, post-roll, marker
    /// cap). Absent fields load at the defaults the constants carried.
    #[serde(default)]
    pub limits: LimitsCfg,
    /// 回放倍速。`∞`（as-fast-as-possible）在 JSON 里写不出一个数，所以那一档
    /// 记在 `replay_fastest` 上，这个数只在有限倍速时有意义。
    #[serde(default = "one_default")]
    pub replay_speed: f64,
    /// 上一档的"是不是 ∞"。缺省 false。
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub replay_fastest: bool,
    /// Throttled text refresh for number readouts, in Hz; 0 follows the
    /// frame rate.
    #[serde(default = "ten_default")]
    pub text_rate_hz: u32,
    /// How many frames the trace ring retains; omitted keys use the default.
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
    pub counters: Counters,
    #[serde(default)]
    pub recent_dbc: Vec<String>,
    #[serde(default)]
    pub recent_log: Vec<String>,
    /// Replay sources saved with the project; folders are rescanned on load.
    #[serde(default)]
    pub replay_folders: Vec<String>,
    #[serde(default)]
    pub replay_files: Vec<String>,
    #[serde(default)]
    pub spec: SpecCfg,
    /// Frontend workspace (tabs, cursors, column widths, splitters). Opaque
    /// to the core so the web shell can evolve without a schema bump.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace: Option<serde_json::Value>,
    /// Derived channels: expression, unit and State bands. The core owns
    /// sampling, so the file carries the definitions, not their samples.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub derived: Vec<crate::derived::DerivedDef>,
}

/// The project-file fields a signal key writes: the CAN triple, or the
/// FlexRay bus in `ch` with its slot in `fr_slot`, or the Ethernet bus in
/// `ch` with the frame name in `eth_frame`. The bus is resolved from the
/// key's cluster; these fields keep the bus numbering.
fn sig_cfg_of(
    app: &App,
    key: &SigKey,
) -> (u8, u32, bool, Option<u16>, Option<String>, Option<String>) {
    match key {
        SigKey::Can { id, ext, .. } => (app.key_bus(key).unwrap_or(0), *id, *ext, None, None, None),
        SigKey::Fr { slot, .. } => (
            app.key_bus(key).unwrap_or(0),
            0,
            false,
            Some(*slot),
            None,
            None,
        ),
        SigKey::Eth { frame, .. } => (
            app.key_bus(key).unwrap_or(0),
            0,
            false,
            None,
            Some(frame.clone()),
            None,
        ),
        SigKey::Derived { id, .. } => (0, 0, false, None, None, Some(id.clone())),
    }
}

/// The bus-count to cluster map a window field set is rebuilt against.
struct KeyMaps<'a> {
    can: &'a [u32],
    fr: &'a std::collections::BTreeMap<u8, u32>,
    eth: &'a std::collections::BTreeMap<u8, u32>,
}

/// The placement fields shared by a curve/row and a Monitor row.
struct SignalSlot<'a> {
    fr_slot: Option<u16>,
    eth_frame: Option<&'a str>,
    derived: Option<&'a str>,
    signal: &'a str,
}

impl SignalSlot<'_> {
    fn key(&self, maps: &KeyMaps<'_>, ch: u8, id: u32, ext: bool) -> SigKey {
        if let Some(id) = self.derived {
            return SigKey::derived(id, self.signal);
        }
        if let Some(frame) = self.eth_frame {
            let cluster = maps.eth.get(&ch).copied().unwrap_or(0);
            return SigKey::eth_in(crate::model::ClusterId(cluster), frame, self.signal);
        }
        match self.fr_slot {
            Some(slot) => {
                let cluster = maps.fr.get(&ch).copied().unwrap_or(0);
                SigKey::fr_in(crate::model::ClusterId(cluster), slot, self.signal)
            }
            None => {
                let cluster = maps.can.get(ch as usize).copied().unwrap_or(0);
                SigKey::can_in(crate::model::ClusterId(cluster), id, ext, self.signal)
            }
        }
    }
}

fn sig_key(maps: &KeyMaps<'_>, s: &SignalCfg) -> SigKey {
    SignalSlot {
        fr_slot: s.fr_slot,
        eth_frame: s.eth_frame.as_deref(),
        derived: s.derived.as_deref(),
        signal: &s.signal,
    }
    .key(maps, s.ch, s.id, s.ext)
}

fn sig_cfgs(app: &App, signals: &[GfxSignal]) -> Vec<SignalCfg> {
    signals
        .iter()
        .map(|s| {
            let (ch, id, ext, fr_slot, eth_frame, derived) = sig_cfg_of(app, &s.key);
            SignalCfg {
                ch,
                id,
                ext,
                fr_slot,
                eth_frame,
                derived,
                signal: s.key.name().to_string(),
                visible: s.visible,
                y_mode: s.y_mode.to_u8(),
                state_rule: None,
                state_overrides: None,
            }
        })
        .collect()
}

pub(crate) fn src_cfg(s: &ValueSrc) -> SrcCfg {
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

pub(crate) fn value_src(c: SrcCfg) -> Option<ValueSrc> {
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

fn sig_keys(maps: &KeyMaps<'_>, signals: &[SignalCfg]) -> Vec<GfxSignal> {
    signals
        .iter()
        .map(|s| GfxSignal {
            key: sig_key(maps, s),
            visible: s.visible,
            y_mode: YMode::from_u8(s.y_mode),
        })
        .collect()
}

impl Default for ProjectSettings {
    fn default() -> Self {
        Self {
            replay_speed: 1.0,
            replay_fastest: false,
            text_rate_hz: 10,
            trace_limit: crate::app::TRACE_LIMIT,
            trace_windows: Vec::new(),
            msg_windows: Vec::new(),
            stats_windows: Vec::new(),
            graphics: Vec::new(),
            data_windows: Vec::new(),
            monitor_rows: Vec::new(),
            state_trackers: Vec::new(),
            nodes: Vec::new(),
            blocks: Vec::new(),
            sysvars: Vec::new(),
            counters: Counters::default(),
            recent_dbc: Vec::new(),
            recent_log: Vec::new(),
            replay_folders: Vec::new(),
            replay_files: Vec::new(),
            spec: SpecCfg::default(),
            workspace: None,
            limits: LimitsCfg::default(),
            derived: Vec::new(),
        }
    }
}

impl ProjectSettings {
    pub fn from_app(app: &App, _base: Option<&Path>) -> Self {
        ProjectSettings {
            // ∞ 存不成数，所以那一档走 `replay_fastest`，数本身留一个中性值。
            replay_fastest: !app.replay_speed.is_finite(),
            replay_speed: if app.replay_speed.is_finite() {
                app.replay_speed
            } else {
                1.0
            },
            text_rate_hz: app.text_rate_hz,
            trace_limit: app.trace_limit,
            limits: app.limits,
            trace_windows: app
                .trace_windows
                .iter()
                .map(|w| {
                    let (manual, fr_manual, eth_manual) = split_picks(&w.manual);
                    TraceCfg {
                        name: w.name.clone(),
                        opened: w.opened,
                        scope: w.scope,
                        manual,
                        fr_manual,
                        eth_manual,
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
                    let (manual, fr_manual, eth_manual) = split_picks(&w.manual);
                    MsgCfg {
                        name: w.name.clone(),
                        opened: w.opened,
                        scope: w.scope,
                        manual,
                        fr_manual,
                        eth_manual,
                        filter: w.filter.clone(),
                        dbc_only: w.dbc_only,
                    }
                })
                .collect(),
            stats_windows: app
                .stats_windows
                .iter()
                .map(|w| {
                    let (manual, fr_manual, eth_manual) = split_picks(&w.manual);
                    StatsCfg {
                        name: w.name.clone(),
                        opened: w.opened,
                        scope: w.scope,
                        manual,
                        fr_manual,
                        eth_manual,
                    }
                })
                .collect(),
            graphics: app
                .graphics
                .iter()
                .map(|g| GfxCfg {
                    name: g.name.clone(),
                    opened: g.opened,
                    signals: sig_cfgs(app, &g.signals),
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
                    signals: sig_cfgs(app, &d.signals),
                })
                .collect(),
            monitor_rows: app
                .monitor_view
                .rows
                .iter()
                .map(|r| {
                    let (ch, id, ext, fr_slot, eth_frame, derived) = sig_cfg_of(app, &r.key);
                    MonitorRowCfg {
                        ch,
                        id,
                        ext,
                        fr_slot,
                        eth_frame,
                        derived,
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
                            let (ch, id, ext, fr_slot, eth_frame, derived) =
                                sig_cfg_of(app, &s.key);
                            SignalCfg {
                                ch,
                                id,
                                ext,
                                fr_slot,
                                eth_frame,
                                derived,
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
            counters: app.window_counters(),
            recent_dbc: app.recent_dbc.clone(),
            recent_log: app.recent_log.clone(),
            replay_folders: app.replay_folders.clone(),
            replay_files: app.replay_files.clone(),
            spec: SpecCfg {
                tolerance_percent: app.spec_tol_pct,
                grace_cycles: app.spec_grace,
            },
            workspace: app.web_workspace.clone(),
            derived: app.derived_defs.clone(),
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
    pub fn apply(self, app: &mut App) {
        app.web_workspace = self.workspace;
        app.set_derived_defs(self.derived);
        // Every window kind is restored as the file states it -- an empty list
        // means the project has none of that kind; letting the default windows
        // leak back in would resurrect what the user deleted.
        app.trace_windows = self
            .trace_windows
            .into_iter()
            .map(|w| TraceWin {
                name: w.name,
                opened: w.opened,
                scope: w.scope,
                manual: merge_picks(w.manual, w.fr_manual, w.eth_manual),
                filter: w.filter,
                dir: w.dir.min(2),
                dbc_only: w.dbc_only,
                payload: w.payload,
                flags_kind: w.flags_kind,
                time_from: w.time_from,
                time_to: w.time_to,
                filters_open: w.filters_open,
                fr_expand: w.fr_expand,
                #[cfg(test)]
                rows: std::collections::VecDeque::new(),
                #[cfg(test)]
                rows_build: None,
                #[cfg(test)]
                rows_sorted: false,
                #[cfg(test)]
                shown_t_us: u64::MAX,
                #[cfg(test)]
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
                manual: merge_picks(w.manual, w.fr_manual, w.eth_manual),
                filter: w.filter,
                dbc_only: w.dbc_only,
                #[cfg(test)]
                text_keys: Vec::new(),
                #[cfg(test)]
                text_header: String::new(),
                #[cfg(test)]
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
                manual: merge_picks(w.manual, w.fr_manual, w.eth_manual),
                #[cfg(test)]
                text_keys: Vec::new(),
                #[cfg(test)]
                text_header: String::new(),
                #[cfg(test)]
                text_rows: Vec::new(),
            })
            .collect();
        // The snapshot's cluster ids are the bus-count-to-cluster map the
        // window fields are rebuilt against.
        let can_ids = app.snap.cluster_ids.can.clone();
        let fr_ids = app.snap.cluster_ids.fr.clone();
        let eth_ids = app.snap.cluster_ids.eth.clone();
        let maps = KeyMaps {
            can: &can_ids,
            fr: &fr_ids,
            eth: &eth_ids,
        };
        app.graphics = self
            .graphics
            .into_iter()
            .map(|g| GraphicsWindow {
                name: g.name,
                signals: sig_keys(&maps, &g.signals),
                time_window_s: g.time_window_s.clamp(0.1, 3600.0),
                stacked: g.stacked,
                opened: g.opened,
                #[cfg(test)]
                t_offset_s: 0.0,
                show_cursor: g.show_cursor,
                zoom_enabled: g.zoom_enabled,
                show_markers: g.show_markers,
                #[cfg(test)]
                y_locks: HashMap::new(),
                #[cfg(test)]
                legend_keys: Vec::new(),
                #[cfg(test)]
                legend: Vec::new(),
            })
            .collect();
        app.data_windows = self
            .data_windows
            .into_iter()
            .map(|d| DataWindow {
                name: d.name,
                signals: sig_keys(&maps, &d.signals),
                opened: d.opened,
                #[cfg(test)]
                text_keys: Vec::new(),
                #[cfg(test)]
                text_cache: Vec::new(),
            })
            .collect();
        if !self.monitor_rows.is_empty() {
            app.monitor_view.rows = self
                .monitor_rows
                .into_iter()
                .map(|m| crate::app::MonitorRow {
                    key: SignalSlot {
                        fr_slot: m.fr_slot,
                        eth_frame: m.eth_frame.as_deref(),
                        derived: m.derived.as_deref(),
                        signal: &m.signal,
                    }
                    .key(&maps, m.ch, m.id, m.ext),
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
                let signals = sig_keys(&maps, &w.signals);
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
        app.spec_tol_pct = self.spec.tolerance_percent;
        app.spec_grace = self.spec.grace_cycles.max(1);
        app.replay_speed = if self.replay_fastest {
            f64::INFINITY
        } else {
            self.replay_speed.clamp(0.01, 100.0)
        };
        app.set_window_counters(self.counters);
        app.recent_dbc = self.recent_dbc;
        app.recent_log = self.recent_log;
        app.replay_folders = self.replay_folders;
        app.replay_files = self.replay_files;
        app.refresh_replay_sources();
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
            .chain(app.monitor_view.rows.iter().map(|r| r.key.clone()))
            .collect();
        for key in keys {
            app.subscribe(key);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::NodeRole;
    use crate::can::frame::FrameFlags;
    use crate::model::ProjectV2;
    use crate::test_assets::TestBusLoad;

    #[test]
    fn config_round_trips_through_json() {
        let mut app = App::headless();
        app.replay_speed = 2.0;
        app.trace_windows[0].filter = "Motor".to_string();
        app.trace_windows[0].scope = SigScope::Bus(1);
        app.trace_windows[0]
            .manual
            .insert(crate::workspace::Pick::Can { ch: 1, id: 0x123 });
        // A hand-picked FlexRay slot rides along in its own list.
        app.trace_windows[0]
            .manual
            .insert(crate::workspace::Pick::Fr { bus: 2, slot: 71 });
        // A FlexRay cluster scope is its own numbering space and its own
        // variant, so a project can hold one of each without either moving.
        app.stats_windows[0].scope = SigScope::FrBus(2);
        app.tx_list[0].active = true;
        app.tx_list[0].cycle_us = 50_000;
        app.tx_list[0].flags = FrameFlags::FD.union(FrameFlags::BRS);
        app.refresh_snapshot();

        let json = serde_json::to_string(&ProjectV2::from_app(&app, None)).unwrap();
        let mut restored = App::headless();
        serde_json::from_str::<ProjectV2>(&json)
            .unwrap()
            .apply(&mut restored);

        assert_eq!(restored.replay_speed, 2.0);
        assert_eq!(restored.trace_windows[0].filter, "Motor");
        assert_eq!(restored.trace_windows[0].scope, SigScope::Bus(1));
        assert_eq!(restored.stats_windows[0].scope, SigScope::FrBus(2));
        assert!(
            restored.trace_windows[0]
                .manual
                .contains(&crate::workspace::Pick::Can { ch: 1, id: 0x123 })
        );
        assert!(
            restored.trace_windows[0]
                .manual
                .contains(&crate::workspace::Pick::Fr { bus: 2, slot: 71 })
        );
        assert!(restored.tx_list[0].active);
        assert_eq!(restored.tx_list[0].cycle_us, 50_000);
        assert!(restored.tx_list[0].flags.contains(FrameFlags::FD));
        assert!(restored.tx_list[0].flags.contains(FrameFlags::BRS));
        assert_eq!(restored.can_count(), app.can_count());
    }

    /// A project with no analysis windows gets none back: an empty saved list
    /// means "none", and the default windows must not leak back in.
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

        let json = serde_json::to_string(&ProjectV2::from_app(&app, None)).unwrap();
        let mut restored = App::headless();
        serde_json::from_str::<ProjectV2>(&json)
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
    /// those; the databases' own message lists must not refill the panel.
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

        let json = serde_json::to_string(&ProjectV2::from_app(&app, None)).unwrap();
        let mut restored = App::headless();
        serde_json::from_str::<ProjectV2>(&json)
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
        let cluster = app.cluster_ids.allocate();
        app.fr_cluster_ids.insert(1, cluster);
        assert_eq!(
            app.sig_bus_label(&crate::observe::SigKey::fr_in(cluster, 13, "DriveTorque")),
            "动力总成",
            "the legends a FlexRay signal appears in follow the name"
        );

        let json = serde_json::to_string(&ProjectV2::from_app(&app, None)).unwrap();
        let mut restored = App::headless();
        serde_json::from_str::<ProjectV2>(&json)
            .expect("a file with the new key loads")
            .apply(&mut restored);
        assert_eq!(restored.fr_bus_name(1), "动力总成");
        assert_eq!(restored.fr_bus_label(1), "动力总成 (FR1)");

        // Typing the index back, or blanking the box, is "no name": nothing is
        // stored, so a project whose buses were never renamed stays as clean as
        // it was before the field existed.
        app.set_flexray_name(1, "FR1");
        assert_eq!(app.fr_bus_name(1), "FR1");
        assert!(unnamed(&app), "a 路 called by its own index stores no name");
        app.set_flexray_name(1, "   ");
        assert_eq!(app.fr_bus_name(1), "FR1");
        assert!(unnamed(&app), "a blank box stores no name");
    }

    /// True when `app` would write no FlexRay names into its project file.
    fn unnamed(app: &App) -> bool {
        ProjectV2::from_app(app, None)
            .clusters
            .iter()
            .filter(|cluster| cluster.kind == crate::model::ClusterKind::FlexRay)
            .all(|cluster| {
                cluster.name
                    == cluster
                        .bus
                        .map_or_else(String::new, |bus| format!("FR{bus}"))
            })
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
        let arxml = crate::test_assets::path("PowerTrain.arxml");
        let text =
            crate::dbc::text_from_bytes(std::fs::read(arxml).expect("the asset is committed"));
        let db = crate::fr_db::FrDb::parse(&text).expect("parses");
        let slot = (0..db.frames.len())
            .find_map(|ix| {
                db.frame_sender(ix)?;
                Some(db.frame_index(ix)?.triggering.slot_id as u16)
            })
            .expect("the asset binds a slot to a sending ECU");
        app.add_fr_description(0, crate::channel::description_source(arxml, None))
            .expect("the FlexRay description binds");
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

        let json = serde_json::to_string(&ProjectV2::from_app(&app, None)).unwrap();
        let mut restored = App::headless();
        serde_json::from_str::<ProjectV2>(&json)
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

        // The same file with the key removed still loads.
        let mut doc: serde_json::Value = serde_json::from_str(&json).unwrap();
        doc.as_object_mut().unwrap().remove("generators");
        assert!(
            !doc.to_string().contains("generators"),
            "the key is really gone"
        );
        let mut restored = App::headless();
        serde_json::from_value::<ProjectV2>(doc)
            .expect("a file without the key loads")
            .apply(&mut restored);
        assert!(
            restored.fr_tx_list.is_empty(),
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
        let json = serde_json::to_string(&ProjectV2::from_app(&app, None)).unwrap();
        let mut doc: serde_json::Value = serde_json::from_str(&json).unwrap();
        // All three window kinds gained the key, so an old file lacks it in all
        // three -- stripping one and leaving the others would not be the file
        // this test claims to read.
        for kind in ["traceWindows", "msgWindows", "statsWindows"] {
            for w in doc[kind].as_array_mut().unwrap() {
                w.as_object_mut().unwrap().remove("fr_manual");
            }
        }
        assert!(
            !doc.to_string().contains("fr_manual"),
            "the edited file really looks like an old one"
        );

        let mut restored = App::headless();
        serde_json::from_value::<ProjectV2>(doc)
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

        let json = serde_json::to_string(&ProjectV2::from_app(&app, None)).unwrap();
        assert!(json.contains(r#""srcs""#), "the key is written");
        let mut restored = App::headless();
        serde_json::from_str::<ProjectV2>(&json)
            .unwrap()
            .apply(&mut restored);
        assert_eq!(restored.tx_list[0].srcs, app.tx_list[0].srcs);
    }

    /// The Monitor window's rows persist with the project: the watched
    /// keys, labels, digits, and coloring rules all round-trip.
    #[test]
    fn monitor_rows_round_trip_with_the_project() {
        let mut app = App::headless();
        let key = app.can_key(0, 0x100, false, "EngineStatus".to_string());
        app.set_win_signal(crate::workspace::PopupTarget::Monitor, key, true);
        assert_eq!(app.monitor_view.rows.len(), 1, "the row was added");
        app.monitor_view.rows[0].digits = 2;
        app.monitor_view.rows[0].rule_on = true;
        app.monitor_view.rows[0].rising = false;
        app.monitor_view.rows[0].threshold = 87.5;
        app.monitor_view.rows[0].color = 3;

        let json = serde_json::to_string(&ProjectV2::from_app(&app, None)).unwrap();
        let mut restored = App::headless();
        serde_json::from_str::<ProjectV2>(&json)
            .unwrap()
            .apply(&mut restored);
        assert_eq!(restored.monitor_view.rows.len(), 1);
        let r = &restored.monitor_view.rows[0];
        assert!(
            matches!(
                r.key,
                crate::observe::SigKey::Can {
                    cluster: crate::model::ClusterId(1),
                    id: 0x100,
                    ext: false,
                    ..
                }
            ),
            "the round trip keeps a standard-class CAN key"
        );
        assert_eq!(r.key.name(), "EngineStatus");
        assert_eq!(
            (r.digits, r.rule_on, r.rising, r.threshold, r.color),
            (2, true, false, 87.5, 3)
        );
    }

    /// Derived definitions and the observer rows that read them belong to
    /// the project now, not to the browser's workspace blob.
    #[test]
    fn derived_definitions_and_rows_round_trip_with_the_project() {
        let mut app = App::headless();
        let defs = vec![crate::derived::DerivedDef {
            id: "d1".into(),
            name: "Sum".into(),
            unit: "V".into(),
            expression: "[EngineSpeed] + 1".into(),
            cuts: vec![10.0],
            names: vec!["low".into(), "high".into()],
            sources: Vec::new(),
        }];
        app.set_derived_defs(defs.clone());
        let key = crate::observe::SigKey::derived("d1", "Sum");
        app.set_win_signal(crate::workspace::PopupTarget::Monitor, key.clone(), true);
        let cond = crate::trigger::TriggerCond::DerivedSignal {
            id: "d1".into(),
            threshold: 3.0,
            rising: true,
        };
        app.send(crate::bus::BusCommand::AddTrigger {
            cond: cond.clone(),
            action: crate::trigger::TriggerAction::InsertMarker,
        });
        let config = ProjectV2::from_app(&app, None);
        assert_eq!(
            config.settings.derived, defs,
            "the file carries the definitions"
        );
        assert_eq!(
            config.settings.monitor_rows[0].derived.as_deref(),
            Some("d1"),
            "the row stores the definition id, not a fake CAN address"
        );
        assert_eq!(config.triggers[0].kind, 7, "derived rule kind");
        assert_eq!(config.triggers[0].signal, "d1", "the id rides signal");
        let json = serde_json::to_string(&config).unwrap();
        let mut restored = App::headless();
        serde_json::from_str::<ProjectV2>(&json)
            .unwrap()
            .apply(&mut restored);
        assert_eq!(restored.derived_defs, defs);
        assert_eq!(restored.monitor_view.rows[0].key, key);
        assert_eq!(
            restored.snap.triggers[0].cond, cond,
            "the rule reads the same definition after reload"
        );
    }

    /// Which roles a bus declares has to outlive the session, or the bus
    /// composition a user set up is lost on every reload. Restoring it must
    /// not, however, start any traffic by itself.
    #[test]
    fn node_roles_round_trip_without_starting_traffic() {
        let mut app = App::headless();
        app.set_role_for_test(1, "ABS", NodeRole::Absent);
        app.set_role_for_test(1, "GearBox", NodeRole::Simulated);
        app.refresh_snapshot();
        let json = serde_json::to_string(&ProjectV2::from_app(&app, None)).unwrap();
        let mut restored = App::headless();
        serde_json::from_str::<ProjectV2>(&json)
            .unwrap()
            .apply(&mut restored);
        assert_eq!(
            restored.can_role_of(1, "ABS"),
            NodeRole::Absent,
            "an offline node stays offline"
        );
        assert_eq!(
            restored.can_role_of(1, "GearBox"),
            NodeRole::Simulated,
            "a simulated node keeps its declaration"
        );
        assert_eq!(
            restored.can_role_of(1, "DashBoard"),
            NodeRole::Absent,
            "an undeclared node is absent, not an error"
        );
        assert!(
            restored.can_node_roles(0).is_empty(),
            "the other bus keeps its own map"
        );
        assert!(
            restored.tx_list.iter().all(|t| !t.active),
            "a read-only look at a saved project must not begin transmitting"
        );
    }

    /// Deleting a 路 leaves a hole, and the hole is exactly what the project
    /// file states: every description carries its own cluster index, so the
    /// survivor comes back as FR1 rather than shifted into FR0's place -- which
    /// would point a recording's `clusterNo 1` rows, a script's `fr_sig(1, ..)`
    /// and the filter's `FR1:5` at the wrong schedule.
    #[test]
    fn a_removed_flexray_bus_does_not_renumber_the_survivor_through_a_save() {
        let arxml = crate::test_assets::path("PowerTrain.arxml");
        let fibex = crate::test_assets::path("PowerTrain_v2.xml");
        if !std::path::Path::new(arxml).exists() || !std::path::Path::new(fibex).exists() {
            panic!(
                "{arxml} or {fibex} missing -- it is tracked in the repository, so a broken checkout must fail, not skip"
            );
        }
        let mut app = App::headless();
        assert_eq!(app.load_cluster_description(arxml, Some(0)), Some(0));
        assert_eq!(app.load_cluster_description(fibex, Some(1)), Some(1));
        app.set_flexray_name(0, "动力总成");
        app.set_flexray_name(1, "底盘");
        app.remove_fr_bus(0);

        let json = serde_json::to_string(&ProjectV2::from_app(&app, None)).expect("serialises");
        assert!(
            !json.contains("动力总成"),
            "the deleted 路's name went with it: {json}"
        );
        let mut again = App::headless();
        serde_json::from_str::<ProjectV2>(&json)
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

    /// The bitrates feed the load view's arithmetic, so a saved opinion about
    /// them must come back exactly.
    #[test]
    fn bitrates_round_trip() {
        let mut app = App::headless();
        app.set_can_bitrates_for_test(0, Some(1_000), Some(5_000));
        app.refresh_snapshot();
        let json = serde_json::to_string(&ProjectV2::from_app(&app, None)).unwrap();
        let mut restored = App::headless();
        serde_json::from_str::<ProjectV2>(&json)
            .unwrap()
            .apply(&mut restored);
        assert_eq!(restored.can_bitrates(0).0, 1_000);
        assert_eq!(restored.can_bitrates(0).1, 5_000);
        assert_eq!(
            restored.can_bitrates(1).0,
            crate::model::CanParams::DEFAULT_BITRATE_KBPS,
            "the other bus keeps its own rate"
        );
    }

    /// The trace ring's retention is a project opinion about how much
    /// history to keep, so it survives a save.
    #[test]
    fn the_trace_capacity_round_trips() {
        let mut app = App::headless();
        app.trace_limit = 500_000;
        app.set_trace_limit(500_000);
        let json = serde_json::to_string(&ProjectV2::from_app(&app, None)).unwrap();
        let mut restored = App::headless();
        serde_json::from_str::<ProjectV2>(&json)
            .unwrap()
            .apply(&mut restored);
        assert_eq!(restored.trace_limit, 500_000);
        let default = App::headless();
        let plain = serde_json::to_string(&ProjectV2::from_app(&default, None)).unwrap();
        assert!(
            plain.contains(&format!(r#""traceLimit":{}"#, crate::app::TRACE_LIMIT)),
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
        let json = serde_json::to_string(&ProjectV2::from_app(&app, None)).unwrap();

        let mut restored = App::headless();
        serde_json::from_str::<ProjectV2>(&json)
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

        // A file omitting these keys reads with their defaults.
        let mut doc: serde_json::Value = serde_json::from_str(&json).unwrap();
        for w in doc["traceWindows"].as_array_mut().unwrap() {
            let obj = w.as_object_mut().unwrap();
            for key in ["time_from", "time_to", "filters_open", "fr_expand"] {
                obj.remove(key);
            }
        }
        let mut restored = App::headless();
        serde_json::from_value::<ProjectV2>(doc)
            .expect("a file omitting them loads")
            .apply(&mut restored);
        let w = &restored.trace_windows[0];
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
        let json = serde_json::to_string(&ProjectV2::from_app(&app, None)).unwrap();
        let mut restored = App::headless();
        serde_json::from_str::<ProjectV2>(&json)
            .unwrap()
            .apply(&mut restored);
        assert_eq!(restored.limits.pre_frames, 1_024);
        assert_eq!(restored.limits.post_frames, 128);
        assert_eq!(restored.limits.marker_cap, 2_048);
    }

    /// The two monitor thresholds are a project-level opinion about how
    /// strictly to read the database, so they must survive a save.
    #[test]
    fn spec_settings_survive_a_project_round_trip() {
        let mut app = App::headless();
        app.spec_tol_pct = 25;
        app.spec_grace = 8;
        let json = serde_json::to_string(&ProjectV2::from_app(&app, None)).unwrap();
        let mut restored = App::headless();
        serde_json::from_str::<ProjectV2>(&json)
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
        serde_json::from_str::<ProjectV2>(r#"{"channels":[]}"#)
            .unwrap()
            .apply(&mut restored);
        assert_eq!(restored.spec_tol_pct, crate::spec::TOLERANCE_PERCENT);
        assert_eq!(restored.spec_grace, crate::spec::GRACE_CYCLES);

        let partial: ProjectV2 = serde_json::from_str(r#"{"spec":{"grace_cycles":5}}"#).unwrap();
        assert_eq!(
            partial.settings.spec.tolerance_percent,
            crate::spec::TOLERANCE_PERCENT,
            "the key that was not written keeps its own default"
        );

        let mut zeroed = App::headless();
        serde_json::from_str::<ProjectV2>(r#"{"spec":{"grace_cycles":0}}"#)
            .unwrap()
            .apply(&mut zeroed);
        assert_eq!(
            zeroed.spec_grace, 1,
            "a grace of zero cycles is not a thing"
        );
    }

    #[test]
    fn missing_fields_fall_back_to_defaults() {
        let cfg: ProjectV2 = serde_json::from_str("{}").unwrap();
        assert_eq!(cfg.settings.replay_speed, 1.0);
        assert!(cfg.clusters.is_empty());
    }

    /// Omitted keys keep markers on.
    #[test]
    fn graphics_config_defaults_markers_on() {
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
        let rel = relativize("data/sample.dbc", base);
        assert!(
            Path::new(&rel).is_absolute(),
            "CWD-relative paths are stored absolute when outside the project"
        );
        assert!(
            Path::new(&rel).ends_with("data/sample.dbc"),
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
    fn as_fast_as_possible_survives_a_project_round_trip() {
        let mut app = App::headless();
        app.replay_speed = f64::INFINITY;
        let cfg = ProjectV2::from_app(&app, None);
        assert!(cfg.settings.replay_fastest, "∞ 记在开关上，不记在数上");
        let json = serde_json::to_string(&cfg).unwrap();
        assert!(json.contains("\"replayFastest\":true"));

        let mut restored = App::headless();
        serde_json::from_str::<ProjectV2>(&json)
            .expect("a project carrying the flag loads")
            .apply(&mut restored);
        assert_eq!(restored.replay_speed, f64::INFINITY);

        // 正常倍速不受牵连，也不该多出这一项。
        let mut finite = App::headless();
        finite.replay_speed = 4.0;
        let json = serde_json::to_string(&ProjectV2::from_app(&finite, None)).unwrap();
        assert!(!json.contains("replayFastest"), "没到 ∞ 就不写这一项");
        let mut back = App::headless();
        serde_json::from_str::<ProjectV2>(&json)
            .unwrap()
            .apply(&mut back);
        assert_eq!(back.replay_speed, 4.0);
    }
}

#[cfg(test)]
mod eth_config_tests {
    use super::*;
    use crate::model::ProjectV2;
    use crate::test_assets::TestBusLoad;

    const ETH_SAMPLE: &str = r#"<?xml version="1.0"?>
<AUTOSAR><AR-PACKAGES><AR-PACKAGE><SHORT-NAME>N</SHORT-NAME><ELEMENTS>
<ETHERNET-CLUSTER><SHORT-NAME>C</SHORT-NAME></ETHERNET-CLUSTER>
<ETHERNET-FRAME><SHORT-NAME>F</SHORT-NAME><FRAME-LENGTH>8</FRAME-LENGTH></ETHERNET-FRAME>
</ELEMENTS></AR-PACKAGE></AR-PACKAGES></AUTOSAR>"#;

    #[test]
    fn an_ethernet_matrix_round_trips_with_the_project() {
        let path =
            std::env::temp_dir().join(format!("roxy_can_eth_cfg_{}.arxml", std::process::id()));
        std::fs::write(&path, ETH_SAMPLE).expect("write fixture");
        let path_s = path.to_string_lossy().to_string();

        let mut app = App::headless();
        assert_eq!(app.load_eth_description(&path_s, Some(2)), Some(2));
        assert_eq!(app.eth_buses.len(), 1);
        app.set_eth_length_includes_header(true);

        let json = serde_json::to_string(&ProjectV2::from_app(&app, None)).unwrap();
        let mut restored = App::headless();
        serde_json::from_str::<ProjectV2>(&json)
            .unwrap()
            .apply(&mut restored);
        assert!(restored.eth_buses.contains_key(&2));
        assert_eq!(restored.eth_db(2).unwrap().frames.len(), 1);
        assert!(
            restored.eth_length_includes_header,
            "the frame-length convention travels with the matrices"
        );
        std::fs::remove_file(path).ok();
    }

    /// An Ethernet signal's project fields name its bus and ARXML frame, and
    /// rebuild the very same `SigKey::Eth` -- otherwise a saved curve would
    /// come back pointing at a CAN id no frame ever carries.
    #[test]
    fn ethernet_signal_keys_round_trip_through_project_fields() {
        let mut app = App::headless();
        let cluster = app.cluster_ids.allocate();
        app.eth_cluster_ids.insert(2, cluster);
        let key = app.eth_key(2, "SpeedFrame".to_string(), "Speed".to_string());
        let gfx = GfxSignal {
            key: key.clone(),
            visible: true,
            y_mode: YMode::Auto,
        };
        let cfg = sig_cfgs(&app, &[gfx]);
        assert_eq!(cfg[0].ch, 2);
        assert_eq!(cfg[0].eth_frame.as_deref(), Some("SpeedFrame"));
        assert_eq!(cfg[0].fr_slot, None);

        let json = serde_json::to_string(&cfg[0]).unwrap();
        assert!(
            json.contains("\"eth_frame\":\"SpeedFrame\""),
            "the frame is stated in the file: {json}"
        );
        let back: SignalCfg = serde_json::from_str(&json).unwrap();
        let mut eth = std::collections::BTreeMap::new();
        eth.insert(2u8, cluster.0);
        let can: Vec<u32> = Vec::new();
        let fr = std::collections::BTreeMap::new();
        let maps = KeyMaps {
            can: &can,
            fr: &fr,
            eth: &eth,
        };
        let keys = sig_keys(&maps, &[back]);
        assert_eq!(keys[0].key, key);
    }

    /// A hand-picked Ethernet flow is part of the window's Manual selection,
    /// so it must survive a project round trip -- otherwise a scoped window
    /// comes back showing All buses and a different set of rows.
    #[test]
    fn ethernet_manual_picks_round_trip_with_the_project() {
        let flow = crate::aggregate::EthFlowKey {
            bus: 0,
            vlan: Some(100),
            dst: [9, 8, 7, 6, 5, 4],
            src: [1, 2, 3, 4, 5, 6],
            ethertype: 0x0800,
            len: 8,
        };
        let pick = crate::workspace::Pick::Eth(flow);
        let mut app = App::headless();
        app.trace_windows[0].manual.clear();
        app.trace_windows[0].manual.insert(pick);
        app.trace_windows[0].scope = SigScope::Manual;

        let json = serde_json::to_string(&ProjectV2::from_app(&app, None)).unwrap();
        assert!(json.contains("eth_manual"), "the pick has its own list");
        let mut restored = App::headless();
        serde_json::from_str::<ProjectV2>(&json)
            .unwrap()
            .apply(&mut restored);
        assert!(
            restored.trace_windows[0].manual.contains(&pick),
            "the Ethernet flow comes back with the window"
        );
        assert_eq!(restored.trace_windows[0].scope, SigScope::Manual);
    }
}
