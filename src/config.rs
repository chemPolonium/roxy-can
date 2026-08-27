//! Workspace persistence as CANoe-style project files (.rxproj): buses,
//! analysis windows, signals, filters, the generator and the imgui window
//! layout are bundled in one JSON file. A small `roxy-can.meta.json`
//! remembers the last opened project. The legacy `roxy-can.json` (no
//! project path) is still read once as a migration fallback.
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::app::{
    App, Channel, DataWindow, Desktop, GfxSignal, GraphicsWindow, MsgWin, SigScope, StatsWin,
    TraceWin, WindowKind,
};
use crate::can::frame::{FrameFlags, MAX_CAN_FD_LEN};

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
fn sixty_default() -> f64 {
    60.0
}

#[derive(Serialize, Deserialize)]
pub struct ChannelCfg {
    pub name: String,
    pub dbc_path: String,
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
}

#[derive(Serialize, Deserialize)]
pub struct DataCfg {
    pub name: String,
    pub opened: bool,
    #[serde(default)]
    pub signals: Vec<SignalCfg>,
    #[serde(default = "true_default")]
    pub viz_bar: bool,
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
    pub show_id_filter: bool,
    #[serde(default = "one_default")]
    pub replay_speed: f64,
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
    #[serde(default)]
    pub recent_asc: Vec<String>,
    #[serde(default)]
    pub desktops: Vec<DesktopCfg>,
    #[serde(default)]
    pub active_desktop: usize,
}

fn sig_cfgs(signals: &[GfxSignal]) -> Vec<SignalCfg> {
    signals
        .iter()
        .map(|s| SignalCfg {
            ch: s.key.0,
            id: s.key.1,
            signal: s.key.2.clone(),
            visible: s.visible,
        })
        .collect()
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
        show_id_filter: d.show_id_filter,
    }
}

fn sig_keys(signals: Vec<SignalCfg>) -> Vec<GfxSignal> {
    signals
        .into_iter()
        .map(|s| GfxSignal {
            key: (s.ch, s.id, s.signal),
            visible: s.visible,
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
                })
                .collect(),
            bus_counter: app.bus_counter(),
            show_tx: app.show_tx,
            show_network: app.show_network,
            show_measurement: app.show_measurement,
            show_buses: app.show_buses,
            show_id_filter: app.show_id_filter,
            replay_speed: app.replay_speed,
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
                })
                .collect(),
            data_windows: app
                .data_windows
                .iter()
                .map(|d| DataCfg {
                    name: d.name.clone(),
                    opened: d.opened,
                    signals: sig_cfgs(&d.signals),
                    viz_bar: d.viz_bar,
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
                })
                .collect(),
            counters: app.window_counters(),
            recent_dbc: app.recent_dbc.clone(),
            recent_asc: app.recent_asc.clone(),
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
                m.cycle_us = t.cycle_us.max(1_000);
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
                    viz_bar: d.viz_bar,
                })
                .collect();
        }
        app.show_tx = self.show_tx;
        app.show_network = self.show_network;
        app.show_measurement = self.show_measurement;
        app.show_buses = self.show_buses;
        app.show_id_filter = self.show_id_filter;
        app.replay_speed = self.replay_speed.clamp(0.01, 100.0);
        app.set_window_counters(self.counters);
        app.recent_dbc = self.recent_dbc;
        app.recent_asc = self.recent_asc;
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

    #[test]
    fn missing_fields_fall_back_to_defaults() {
        let cfg: Config = serde_json::from_str("{}").unwrap();
        assert!(cfg.show_tx);
        assert_eq!(cfg.replay_speed, 1.0);
        assert!(cfg.channels.is_empty());
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
