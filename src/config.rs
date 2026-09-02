//! Workspace persistence as project files (.rxproj): buses,
//! analysis windows, signals, filters, the generator and the imgui window
//! layout are bundled in one JSON file. A small `roxy-can.meta.json`
//! remembers the last opened project. The legacy `roxy-can.json` (no
//! project path) is still read once as a migration fallback.
use std::collections::HashMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::app::{
    App, Channel, DataWindow, Desktop, GfxSignal, GraphicsWindow, MsgWin, SigScope, StatsWin,
    TraceWin, WindowKind, YMode,
};
use crate::can::frame::{FrameFlags, MAX_CAN_FD_LEN};
use crate::sim::{SrcKind, ValueSrc};
use crate::trigger::{TriggerAction, TriggerCond};

pub const CONFIG_PATH: &str = "roxy-can.json";
pub const META_PATH: &str = "roxy-can.meta.json";
pub const AUTOSAVE_PATH: &str = "roxy-can.autosave.rxproj";
pub const PROJECT_EXT: &str = "rxproj";

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
fn sixty_default() -> f64 {
    60.0
}

#[derive(Serialize, Deserialize)]
pub struct ChannelCfg {
    pub name: String,
    pub dbc_path: String,
    /// DBC nodes ticked as simulated on this bus. Absent from projects saved
    /// before v0.5, which then load with nothing simulated.
    #[serde(default)]
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

#[derive(Serialize, Deserialize)]
pub struct TraceCfg {
    pub name: String,
    pub opened: bool,
    pub scope: SigScope,
    #[serde(default)]
    pub manual: Vec<(u8, u32)>,
    #[serde(default)]
    pub filter: String,
    #[serde(default)]
    pub dir: usize,
    #[serde(default)]
    pub dbc_only: bool,
}

#[derive(Serialize, Deserialize)]
pub struct MsgCfg {
    pub name: String,
    pub opened: bool,
    pub scope: SigScope,
    #[serde(default)]
    pub manual: Vec<(u8, u32)>,
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
    pub manual: Vec<(u8, u32)>,
}

#[derive(Serialize, Deserialize)]
pub struct SignalCfg {
    pub ch: u8,
    pub id: u32,
    pub signal: String,
    #[serde(default = "true_default")]
    pub visible: bool,
    /// [`crate::observe::YMode`] code; unknown values load as Auto.
    #[serde(default)]
    pub y_mode: u8,
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

#[derive(Serialize, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub channels: Vec<ChannelCfg>,
    #[serde(default)]
    pub bus_counter: usize,
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
    #[serde(default = "one_default")]
    pub replay_speed: f64,
    /// Throttled text refresh for number readouts, in Hz; 0 follows the
    /// frame rate.
    #[serde(default = "ten_default")]
    pub text_rate_hz: u32,
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
    #[serde(default)]
    pub tx: Vec<TxCfg>,
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

fn sig_cfgs(signals: &[GfxSignal]) -> Vec<SignalCfg> {
    signals
        .iter()
        .map(|s| SignalCfg {
            ch: s.key.0,
            id: s.key.1,
            signal: s.key.2.clone(),
            visible: s.visible,
            y_mode: s.y_mode.to_u8(),
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
    }
}

fn sig_keys(signals: Vec<SignalCfg>) -> Vec<GfxSignal> {
    signals
        .into_iter()
        .map(|s| GfxSignal {
            key: (s.ch, s.id, s.signal),
            visible: s.visible,
            y_mode: YMode::from_u8(s.y_mode),
        })
        .collect()
}

impl Config {
    pub fn from_app(app: &App, base: Option<&Path>) -> Self {
        Config {
            channels: app
                .channels
                .iter()
                .map(|c| ChannelCfg {
                    name: c.name.clone(),
                    dbc_path: match base {
                        Some(b) => relativize(&c.dbc_path, b),
                        None => c.dbc_path.clone(),
                    },
                    sim_nodes: c.sim_nodes.clone(),
                    bitrate_kbps: c.bitrate_kbps,
                    fd_data_kbps: c.fd_data_kbps,
                })
                .collect(),
            bus_counter: app.bus_counter(),
            show_tx: app.show_tx,
            show_network: app.show_network,
            show_measurement: app.show_measurement,
            show_buses: app.show_buses,
            show_triggers: app.show_triggers,
            show_bus_stats: app.show_bus_stats,
            show_spec: app.show_spec,
            show_id_filter: app.show_id_filter,
            replay_speed: app.replay_speed,
            text_rate_hz: app.text_rate_hz,
            trace_windows: app
                .trace_windows
                .iter()
                .map(|w| TraceCfg {
                    name: w.name.clone(),
                    opened: w.opened,
                    scope: w.scope,
                    manual: w.manual.iter().copied().collect(),
                    filter: w.filter.clone(),
                    dir: w.dir,
                    dbc_only: w.dbc_only,
                })
                .collect(),
            msg_windows: app
                .msg_windows
                .iter()
                .map(|w| MsgCfg {
                    name: w.name.clone(),
                    opened: w.opened,
                    scope: w.scope,
                    manual: w.manual.iter().copied().collect(),
                    filter: w.filter.clone(),
                    dbc_only: w.dbc_only,
                })
                .collect(),
            stats_windows: app
                .stats_windows
                .iter()
                .map(|w| StatsCfg {
                    name: w.name.clone(),
                    opened: w.opened,
                    scope: w.scope,
                    manual: w.manual.iter().copied().collect(),
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
            tx: app
                .tx_list
                .iter()
                .map(|t| TxCfg {
                    channel: t.channel,
                    id: t.id,
                    active: t.active,
                    data_text: t.data_text.clone(),
                    data: t.data[..t.len as usize].to_vec(),
                    cycle_us: t.cycle_us,
                    fd: t.flags.contains(FrameFlags::FD),
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
                .triggers
                .iter()
                .map(|t| {
                    let mut send_ch = 0u8;
                    let mut send_id = 0u32;
                    let (kind, ch, id, signal, threshold, rising) = match &t.cond {
                        TriggerCond::SignalCross {
                            ch,
                            id,
                            signal,
                            threshold,
                            rising,
                        } => (0, *ch, *id, signal.clone(), *threshold, *rising),
                        TriggerCond::IdPresent { ch, id } => {
                            (1, *ch, *id, String::new(), 0.0, false)
                        }
                        TriggerCond::ErrorFrame { ch } => (2, *ch, 0, String::new(), 0.0, false),
                        TriggerCond::CycleTimeout { ch, id } => {
                            (3, *ch, *id, String::new(), 0.0, false)
                        }
                    };
                    TriggerCfg {
                        kind,
                        ch,
                        id,
                        signal,
                        threshold,
                        rising,
                        action: match t.action {
                            TriggerAction::StartRecording => 0,
                            TriggerAction::StopRecording => 1,
                            TriggerAction::Send { ch, id } => {
                                send_ch = ch;
                                send_id = id;
                                2
                            }
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
        }
    }

    /// Overwrites the freshly built defaults with the saved workspace.
    pub fn apply(self, app: &mut App) {
        if !self.channels.is_empty() {
            app.channels = self
                .channels
                .into_iter()
                .map(|c| Channel {
                    name: c.name,
                    dbc: None,
                    dbc_path: c.dbc_path,
                    // Intent only. What transmits is decided by each TxCfg's
                    // `active` below, so a restored project never starts
                    // traffic that was stopped when it was saved.
                    sim_nodes: c.sim_nodes,
                    bitrate_kbps: c.bitrate_kbps,
                    fd_data_kbps: c.fd_data_kbps,
                })
                .collect();
            app.set_bus_counter(self.bus_counter.max(app.channels.len()));
            app.load_dbcs();
        }
        // The generator is rebuilt from the (possibly new) DBCs, then the
        // saved per-message state is overlaid.
        app.tx_list.clear();
        let ids: Vec<(u8, u32)> = app
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
        for (ch, id) in ids {
            app.add_tx(ch, id);
        }
        for t in self.tx {
            if let Some(m) = app
                .tx_list
                .iter_mut()
                .find(|m| m.channel == t.channel && m.id == t.id)
            {
                m.active = t.active;
                // 0 is a real state now: a DBC-declared event-triggered
                // message. Everything else keeps the anti-typo floor.
                m.cycle_us = if t.cycle_us == 0 {
                    0
                } else {
                    t.cycle_us.max(1_000)
                };
                m.flags = if t.fd {
                    FrameFlags::FD
                } else {
                    FrameFlags::NONE
                };
                if !t.data_text.is_empty() {
                    m.data_text = t.data_text;
                    let n = t.data.len().min(MAX_CAN_FD_LEN);
                    let mut data = [0u8; MAX_CAN_FD_LEN];
                    data[..n].copy_from_slice(&t.data[..n]);
                    m.data = data;
                    m.len = n as u8;
                }
                m.srcs = t.srcs.into_iter().filter_map(value_src).collect();
            }
        }
        if !self.trace_windows.is_empty() {
            app.trace_windows = self
                .trace_windows
                .into_iter()
                .map(|w| TraceWin {
                    name: w.name,
                    opened: w.opened,
                    scope: w.scope,
                    manual: w.manual.into_iter().collect(),
                    filter: w.filter,
                    dir: w.dir.min(2),
                    dbc_only: w.dbc_only,
                })
                .collect();
        }
        if !self.msg_windows.is_empty() {
            app.msg_windows = self
                .msg_windows
                .into_iter()
                .map(|w| MsgWin {
                    name: w.name,
                    opened: w.opened,
                    scope: w.scope,
                    manual: w.manual.into_iter().collect(),
                    filter: w.filter,
                    dbc_only: w.dbc_only,
                })
                .collect();
        }
        if !self.stats_windows.is_empty() {
            app.stats_windows = self
                .stats_windows
                .into_iter()
                .map(|w| StatsWin {
                    name: w.name,
                    opened: w.opened,
                    scope: w.scope,
                    manual: w.manual.into_iter().collect(),
                })
                .collect();
        }
        if !self.graphics.is_empty() {
            app.graphics = self
                .graphics
                .into_iter()
                .map(|g| GraphicsWindow {
                    name: g.name,
                    signals: sig_keys(g.signals),
                    time_window_s: g.time_window_s.clamp(0.1, 3600.0),
                    stacked: g.stacked,
                    opened: g.opened,
                    t_offset_s: 0.0,
                    show_cursor: g.show_cursor,
                    zoom_enabled: g.zoom_enabled,
                    show_markers: g.show_markers,
                    y_locks: HashMap::new(),
                })
                .collect();
        }
        if !self.data_windows.is_empty() {
            app.data_windows = self
                .data_windows
                .into_iter()
                .map(|d| DataWindow {
                    name: d.name,
                    signals: sig_keys(d.signals),
                    opened: d.opened,
                    text_keys: Vec::new(),
                    text_cache: Vec::new(),
                })
                .collect();
        }
        app.show_tx = self.show_tx;
        app.show_network = self.show_network;
        app.show_measurement = self.show_measurement;
        app.show_buses = self.show_buses;
        app.show_triggers = self.show_triggers;
        app.show_bus_stats = self.show_bus_stats;
        app.show_spec = self.show_spec;
        app.show_id_filter = self.show_id_filter;
        app.text_rate_hz = self.text_rate_hz;
        // An unknown kind code (a project from a future version) drops
        // only that trigger; the rest load.
        app.triggers = self
            .triggers
            .iter()
            .filter_map(|c| {
                let cond = match c.kind {
                    0 => TriggerCond::SignalCross {
                        ch: c.ch,
                        id: c.id,
                        signal: c.signal.clone(),
                        threshold: c.threshold,
                        rising: c.rising,
                    },
                    1 => TriggerCond::IdPresent { ch: c.ch, id: c.id },
                    2 => TriggerCond::ErrorFrame { ch: c.ch },
                    3 => TriggerCond::CycleTimeout { ch: c.ch, id: c.id },
                    _ => return None,
                };
                let action = match c.action {
                    1 => TriggerAction::StopRecording,
                    2 => TriggerAction::Send {
                        ch: c.send_ch,
                        id: c.send_id,
                    },
                    _ => TriggerAction::StartRecording,
                };
                let mut t = crate::trigger::Trigger::new(cond, action);
                t.enabled = c.enabled;
                Some(t)
            })
            .collect();
        app.trigger_sel = None;
        app.trig_edit_sel = None;
        app.spec_tol_pct = self.spec.tolerance_percent;
        app.spec_grace = self.spec.grace_cycles.max(1);
        app.replay_speed = self.replay_speed.clamp(0.01, 100.0);
        app.set_window_counters(self.counters);
        app.recent_dbc = self.recent_dbc;
        app.recent_log = self.recent_log;
        // Restored signal lists need their subscriptions recreated,
        // otherwise they render grey and never receive values.
        let keys: Vec<(u8, u32, String)> = app
            .graphics
            .iter()
            .flat_map(|g| g.signals.iter().map(|s| s.key.clone()))
            .chain(
                app.data_windows
                    .iter()
                    .flat_map(|d| d.signals.iter().map(|s| s.key.clone())),
            )
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
    }
}

impl App {
    /// Restores the legacy `roxy-can.json` workspace if one exists; used
    /// only as a one-time migration when no project meta file is found.
    pub fn load_config(&mut self) {
        let Ok(text) = std::fs::read_to_string(CONFIG_PATH) else {
            return;
        };
        match serde_json::from_str::<Config>(&text) {
            Ok(cfg) => {
                cfg.apply(self);
                self.mark_clean();
            }
            Err(e) => self.status = format!("config ignored: {e}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_round_trips_through_json() {
        let mut app = App::new();
        app.show_id_filter = true;
        app.replay_speed = 2.0;
        app.trace_windows[0].filter = "Motor".to_string();
        app.trace_windows[0].scope = SigScope::Bus(1);
        app.trace_windows[0].manual.insert((1, 0x123));
        app.tx_list[0].active = true;
        app.tx_list[0].cycle_us = 50_000;

        let json = serde_json::to_string(&Config::from_app(&app, None)).unwrap();
        let mut restored = App::new();
        serde_json::from_str::<Config>(&json)
            .unwrap()
            .apply(&mut restored);

        assert!(restored.show_id_filter);
        assert_eq!(restored.replay_speed, 2.0);
        assert_eq!(restored.trace_windows[0].filter, "Motor");
        assert_eq!(restored.trace_windows[0].scope, SigScope::Bus(1));
        assert!(restored.trace_windows[0].manual.contains(&(1, 0x123)));
        assert!(restored.tx_list[0].active);
        assert_eq!(restored.tx_list[0].cycle_us, 50_000);
        assert_eq!(restored.channels.len(), app.channels.len());
    }

    /// A waveform that does not survive a save comes back flat, which is the
    /// kind of loss a project file exists to prevent.
    #[test]
    fn tx_value_sources_round_trip() {
        let mut app = App::new();
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
        let mut restored = App::new();
        serde_json::from_str::<Config>(&json)
            .unwrap()
            .apply(&mut restored);
        assert_eq!(restored.tx_list[0].srcs, app.tx_list[0].srcs);
    }

    /// v0.3 project files have no `srcs` key at all: they must load with
    /// nothing driven, which is exactly how those generators behave today.
    #[test]
    fn legacy_tx_entry_without_srcs_stays_flat() {
        let mut restored = App::new();
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
        let mut restored = App::new();
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
        let mut app = App::new();
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

        let json = serde_json::to_string(&Config::from_app(&app, None)).unwrap();
        let mut restored = App::new();
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

    /// Which nodes a bus simulates has to outlive the session, or the bus
    /// composition a user set up is lost on every reload. Restoring it must
    /// not, however, start any traffic by itself.
    #[test]
    fn simulated_nodes_round_trip_without_starting_traffic() {
        let mut app = App::new();
        app.channels[1].sim_nodes = vec!["ABS".to_string(), "GearBox".to_string()];
        let json = serde_json::to_string(&Config::from_app(&app, None)).unwrap();
        let mut restored = App::new();
        serde_json::from_str::<Config>(&json)
            .unwrap()
            .apply(&mut restored);
        assert_eq!(restored.channels[1].sim_nodes, ["ABS", "GearBox"]);
        assert!(
            restored.channels[0].sim_nodes.is_empty(),
            "the other bus keeps its own list"
        );
        assert!(
            restored.tx_list.iter().all(|t| !t.active),
            "a read-only look at a saved project must not begin transmitting"
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
        assert!(
            serde_json::from_str::<Config>(r#"{"channels":[{"dbc_path":"x.dbc"}]}"#).is_err(),
            "name is still required"
        );
    }

    /// The bitrates feed the load view's arithmetic, so a saved opinion about
    /// them must come back exactly.
    #[test]
    fn bitrates_round_trip() {
        let mut app = App::new();
        app.channels[0].bitrate_kbps = 1_000;
        app.channels[0].fd_data_kbps = 5_000;
        let json = serde_json::to_string(&Config::from_app(&app, None)).unwrap();
        let mut restored = App::new();
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

    /// The two monitor thresholds are a project-level opinion about how
    /// strictly to read the database, so they must survive a save.
    #[test]
    fn spec_settings_survive_a_project_round_trip() {
        let mut app = App::new();
        app.spec_tol_pct = 25;
        app.spec_grace = 8;
        let json = serde_json::to_string(&Config::from_app(&app, None)).unwrap();
        let mut restored = App::new();
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
        let mut restored = App::new();
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

        let mut zeroed = App::new();
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
        let mut restored = App::new();
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
        let app = App::new();
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
