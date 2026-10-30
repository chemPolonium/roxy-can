use std::path::{Path, PathBuf};

use crate::app::App;
use crate::config::{PROJECT_EXT, ProjectSettings};
use crate::model::{
    CanCfg, ClusterCfg, ClusterKind, ClusterSource, EtherCfg, FlexRayCfg, GeneratorCfg, ProjectV2,
    TriggerCfgV2,
};

/// Newest-first recent list: dedups and caps at 8 entries.
fn push_recent(list: &mut Vec<String>, path: String) {
    list.retain(|p| p != &path);
    list.insert(0, path);
    list.truncate(8);
}

/// Whether a record draft is still one of the tool's own defaults
/// (empty, the bare "record" name, or a full path ending in a
/// Record/record folder): only those may be silently retargeted on
/// project open/save.
fn app_record_draft_is_default(draft: &str) -> bool {
    let draft = draft.trim();
    if draft.is_empty() {
        return true;
    }
    if draft.eq_ignore_ascii_case("record") {
        return true;
    }
    let norm = draft.replace('\\', "/").to_ascii_lowercase();
    norm.ends_with("/record/record")
}

impl ProjectV2 {
    /// The whole workspace as a project file: bus descriptions first, then
    /// the settings/exports that reference them by cluster id.
    pub fn from_app(app: &App, base: Option<&Path>) -> Self {
        let rel = |path: &String| match base {
            Some(base) => crate::config::relativize(path, base),
            None => path.clone(),
        };
        let ids = &app.snap.cluster_ids;
        let mut clusters = Vec::new();

        for (index, channel) in app.snap.channels.iter().enumerate() {
            let mut sources: Vec<ClusterSource> = channel
                .dbc_paths
                .iter()
                .filter(|path| !path.trim().is_empty())
                .map(|path| ClusterSource::Dbc { path: rel(path) })
                .collect();
            if let Some(list) = app.can_arxml.get(&(index as u8)) {
                sources.extend(list.iter().map(|(path, cluster)| ClusterSource::Arxml {
                    path: rel(path),
                    cluster: cluster.clone(),
                }));
            }
            clusters.push(ClusterCfg {
                id: ids.can.get(index).copied().unwrap_or(0),
                kind: ClusterKind::Can,
                name: channel.name.clone(),
                bus: None,
                aliases: Vec::new(),
                sources,
                can: Some(CanCfg {
                    bitrate_kbps: channel.bitrate_kbps,
                    fd_data_kbps: channel.fd_data_kbps,
                }),
                flexray: None,
                ether: None,
                node_roles: channel
                    .node_roles
                    .iter()
                    .map(|(name, role)| (name.clone(), role.tag().to_string()))
                    .collect(),
            });
        }

        let fr_buses: Vec<u8> = app
            .fr_buses
            .keys()
            .chain(app.fr_sources.keys())
            .chain(app.fr_names.keys())
            .copied()
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect();
        for bus in &fr_buses {
            let sources = app
                .fr_sources
                .get(bus)
                .cloned()
                .filter(|list| !list.is_empty())
                .unwrap_or_else(|| {
                    vec![fallback_source(
                        app.fr_description_path(*bus),
                        app.fr_cluster_label(*bus),
                    )]
                });
            let params = app
                .fr_buses
                .get(bus)
                .map(|db| (db.params.speed_kbps, db.params.cycle_time_ms))
                .unwrap_or((0, 0.0));
            clusters.push(ClusterCfg {
                id: ids.fr.get(bus).copied().unwrap_or(0),
                kind: ClusterKind::FlexRay,
                bus: Some(*bus),
                name: app
                    .fr_names
                    .get(bus)
                    .cloned()
                    .unwrap_or_else(|| format!("FR{bus}")),
                aliases: Vec::new(),
                sources: sources
                    .into_iter()
                    .map(|source| relative_source(source, &rel))
                    .collect(),
                can: None,
                flexray: Some(FlexRayCfg {
                    speed_kbps: params.0,
                    cycle_time_ms: params.1,
                }),
                ether: None,
                node_roles: Default::default(),
            });
        }

        let eth_buses: Vec<u8> = app
            .eth_buses
            .keys()
            .chain(app.eth_sources.keys())
            .copied()
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect();
        for bus in &eth_buses {
            let sources = app
                .eth_sources
                .get(bus)
                .cloned()
                .filter(|list| !list.is_empty())
                .unwrap_or_else(|| {
                    vec![fallback_source(
                        app.eth_description_path(*bus),
                        app.eth_cluster_label(*bus),
                    )]
                });
            clusters.push(ClusterCfg {
                id: ids.eth.get(bus).copied().unwrap_or(0),
                kind: ClusterKind::Ethernet,
                name: format!("ETH{bus}"),
                bus: Some(*bus),
                aliases: Vec::new(),
                sources: sources
                    .into_iter()
                    .map(|source| relative_source(source, &rel))
                    .collect(),
                can: None,
                flexray: None,
                ether: Some(EtherCfg {
                    physical_layer: None,
                    vlans: Vec::new(),
                    frame_length_includes_header: app.eth_length_includes_header,
                }),
                node_roles: Default::default(),
            });
        }

        let settings = ProjectSettings::from_app(app, base);

        let mut project = ProjectV2::new(clusters);
        project.next_cluster_id = ids.next;
        project.bus_counter = app.snap.bus_counter;
        project.settings = settings;
        project.replay = replay_entries(app);
        project.generators = generators(app);
        project.triggers = triggers(app);
        project
    }

    /// Resolves every path a project file carries relative to its directory.
    pub fn resolve_paths(&mut self, base: Option<&Path>) {
        for cluster in &mut self.clusters {
            for source in &mut cluster.sources {
                match source {
                    ClusterSource::Dbc { path }
                    | ClusterSource::Arxml { path, .. }
                    | ClusterSource::Fibex { path } => {
                        *path = crate::config::resolve_dbc(path, base);
                    }
                    ClusterSource::Manual => {}
                }
            }
        }
    }

    /// Replaces the live workspace with this project. Buses are created in
    /// cluster order; the saved cluster ids are restored afterwards so
    /// generators, triggers and window picks keep pointing at the same
    /// clusters.
    pub fn apply(self, app: &mut App) -> Vec<String> {
        let mut problems = Vec::new();
        app.reset_to_defaults();
        let can_clusters: Vec<&ClusterCfg> = self
            .clusters
            .iter()
            .filter(|cluster| cluster.kind == ClusterKind::Can)
            .collect();
        let mut count = app.snap.channel_count;
        while count > can_clusters.len() {
            app.remove_channel(count - 1);
            count -= 1;
        }
        while count < can_clusters.len() {
            app.add_channel();
            count += 1;
        }
        for (index, cluster) in can_clusters.iter().enumerate() {
            let can = cluster.can.clone().unwrap_or_default();
            let roles = cluster
                .node_roles
                .iter()
                .filter_map(|(name, role)| {
                    crate::app::NodeRole::parse(role).map(|role| (name.clone(), role))
                })
                .collect();
            let paths: Vec<String> = cluster
                .sources
                .iter()
                .filter_map(|source| match source {
                    ClusterSource::Dbc { path } => Some(path.clone()),
                    _ => None,
                })
                .collect();
            app.send(crate::bus::BusCommand::SetChannelConfig {
                ch: index as u8,
                name: Some(cluster.name.clone()),
                dbc_path: paths.first().cloned(),
                bitrate_kbps: Some(can.bitrate_kbps),
                fd_data_kbps: Some(can.fd_data_kbps),
                node_roles: Some(roles),
            });
            app.send(crate::bus::BusCommand::LoadDbc {
                ch: index as u8,
                paths,
            });
            for source in &cluster.sources {
                if let ClusterSource::Arxml {
                    path,
                    cluster: name,
                } = source
                    && let Err(e) = app.load_can_arxml_on_bus(path, name, index as u8)
                {
                    problems.push(format!("CAN ARXML load failed: {e}"));
                }
            }
        }
        app.set_bus_counter(self.bus_counter.max(can_clusters.len()));
        app.settle();

        for cluster in self
            .clusters
            .iter()
            .filter(|cluster| cluster.kind == ClusterKind::FlexRay)
        {
            let bus = cluster
                .bus
                .or_else(|| app.next_flexray_bus())
                .unwrap_or(u8::MAX);
            app.ensure_flexray_bus(bus);
            app.set_flexray_name(bus, &cluster.name);
            for source in &cluster.sources {
                if let Err(e) = app.add_fr_description(bus, source.clone()) {
                    problems.push(format!("FlexRay description load failed: {e}"));
                }
            }
        }
        for cluster in self
            .clusters
            .iter()
            .filter(|cluster| cluster.kind == ClusterKind::Ethernet)
        {
            let bus = cluster
                .bus
                .or_else(|| app.next_eth_bus())
                .unwrap_or(u8::MAX);
            app.ensure_eth_bus(bus);
            for source in &cluster.sources {
                if let Err(e) = app.add_eth_description(bus, source.clone()) {
                    problems.push(format!("Ethernet ARXML load failed: {e}"));
                }
            }
        }
        if let Some(flag) = self
            .clusters
            .iter()
            .find(|cluster| cluster.kind == ClusterKind::Ethernet)
            .and_then(|cluster| cluster.ether.as_ref())
            .map(|ether| ether.frame_length_includes_header)
        {
            app.set_eth_length_includes_header(flag);
        }
        app.settle();

        // Drop the bootstrap generator rows; the project's own entries follow.
        let stale: Vec<(u8, u32)> = app.snap.tx.iter().map(|t| (t.channel, t.id)).collect();
        for (ch, id) in stale {
            app.send(crate::bus::BusCommand::RemoveEntry { ch, id });
        }
        let stale_fr: Vec<(u8, u16)> = app.snap.fr_tx.iter().map(|t| (t.bus, t.slot)).collect();
        for (bus, slot) in stale_fr {
            app.send(crate::bus::BusCommand::RemoveFrEntry { bus, slot });
        }
        restore_generators(&self, app);
        restore_triggers(&self, app);
        restore_replay(&self, app);

        self.settings.apply(app);
        app.settle();

        app.send(crate::bus::BusCommand::SetClusterIds {
            next: self.next_cluster_id,
            can: self
                .clusters
                .iter()
                .filter(|cluster| cluster.kind == ClusterKind::Can)
                .map(|cluster| cluster.id)
                .collect(),
            fr: self
                .clusters
                .iter()
                .filter(|cluster| cluster.kind == ClusterKind::FlexRay)
                .filter_map(|cluster| cluster.bus.map(|bus| (bus, cluster.id)))
                .collect(),
            eth: self
                .clusters
                .iter()
                .filter(|cluster| cluster.kind == ClusterKind::Ethernet)
                .filter_map(|cluster| cluster.bus.map(|bus| (bus, cluster.id)))
                .collect(),
        });
        app.reload_arxml_models();
        app.settle();
        problems
    }
}

fn restore_generators(project: &ProjectV2, app: &mut App) {
    for generator in &project.generators {
        match generator {
            crate::model::GeneratorCfg::Can {
                cluster,
                id,
                active,
                data_text,
                cycle_us,
                fd,
                brs,
                srcs,
                ..
            } => {
                let Some(channel) = can_bus_of(project, *cluster) else {
                    continue;
                };
                app.add_tx(channel, *id);
                app.send(crate::bus::BusCommand::SetEntryConfig {
                    ch: channel,
                    id: *id,
                    active: *active,
                    cycle_us: if *cycle_us == 0 {
                        0
                    } else {
                        (*cycle_us).max(1_000)
                    },
                    fd: *fd,
                    flags: Some(if *fd {
                        let flags = crate::can::frame::FrameFlags::FD;
                        if *brs {
                            flags.union(crate::can::frame::FrameFlags::BRS)
                        } else {
                            flags
                        }
                    } else {
                        crate::can::frame::FrameFlags::NONE
                    }),
                    data_text: (!data_text.is_empty()).then(|| data_text.clone()),
                    srcs: srcs.iter().map(src_from_cfg).collect(),
                });
            }
            crate::model::GeneratorCfg::FlexRay {
                cluster,
                slot,
                active,
                data_text,
                cycle_us,
                srcs,
            } => {
                let Some(bus) = flexray_bus_of(project, *cluster) else {
                    continue;
                };
                app.add_fr_tx(bus, *slot);
                app.send(crate::bus::BusCommand::SetFrEntryCycle {
                    bus,
                    slot: *slot,
                    cycle_us: if *cycle_us == 0 {
                        0
                    } else {
                        (*cycle_us).max(1_000)
                    },
                });
                if !data_text.is_empty() {
                    app.send(crate::bus::BusCommand::SetFrEntryHex {
                        bus,
                        slot: *slot,
                        text: data_text.clone(),
                    });
                }
                for src in srcs.iter().map(src_from_cfg) {
                    app.send(crate::bus::BusCommand::SetFrEntrySource {
                        bus,
                        slot: *slot,
                        src,
                    });
                }
                app.send(crate::bus::BusCommand::SetFrEntryActive {
                    bus,
                    slot: *slot,
                    on: *active,
                });
            }
        }
    }
}

fn src_from_cfg(cfg: &crate::config::SrcCfg) -> crate::sim::ValueSrc {
    crate::config::value_src(cfg.clone()).unwrap_or_else(|| {
        crate::sim::ValueSrc::new(&cfg.name, crate::sim::SrcKind::Ramp, cfg.lo, cfg.hi)
    })
}

fn restore_triggers(project: &ProjectV2, app: &mut App) {
    let rows = project
        .triggers
        .iter()
        .filter_map(|row| {
            let ch = match row.kind {
                0..=3 => can_bus_of(project, row.cluster)?,
                5 | 6 => flexray_bus_of(project, row.cluster)?,
                _ => 0,
            };
            let send_ch = if row.action == 2 {
                row.action_cluster
                    .and_then(|cluster| can_bus_of(project, cluster))
                    .unwrap_or(0)
            } else {
                0
            };
            Some(crate::config::TriggerCfg {
                kind: row.kind,
                ch,
                id: row.id,
                signal: row.signal.clone(),
                threshold: row.threshold,
                rising: row.rising,
                ext: row.extended,
                action: row.action,
                send_ch,
                send_id: row.action_id,
                enabled: row.enabled,
            })
        })
        .collect::<Vec<_>>();
    let triggers = rows
        .iter()
        .filter_map(|c| {
            let cond = match c.kind {
                0 => crate::trigger::TriggerCond::SignalCross {
                    ch: c.ch,
                    id: c.id,
                    ext: c.ext,
                    signal: c.signal.clone(),
                    threshold: c.threshold,
                    rising: c.rising,
                },
                1 => crate::trigger::TriggerCond::IdPresent { ch: c.ch, id: c.id },
                2 => crate::trigger::TriggerCond::ErrorFrame { ch: c.ch },
                3 => crate::trigger::TriggerCond::CycleTimeout { ch: c.ch, id: c.id },
                4 => crate::trigger::TriggerCond::SysVar {
                    key: c.signal.clone(),
                    threshold: c.threshold,
                    rising: c.rising,
                },
                5 => crate::trigger::TriggerCond::FrFramePresent {
                    bus: c.ch,
                    slot: c.id.min(u32::from(u16::MAX)) as u16,
                },
                6 => crate::trigger::TriggerCond::FrSignalCross {
                    bus: c.ch,
                    slot: c.id.min(u32::from(u16::MAX)) as u16,
                    signal: c.signal.clone(),
                    threshold: c.threshold,
                    rising: c.rising,
                },
                7 => crate::trigger::TriggerCond::DerivedSignal {
                    id: c.signal.clone(),
                    threshold: c.threshold,
                    rising: c.rising,
                },
                _ => return None,
            };
            let action = match c.action {
                1 => crate::trigger::TriggerAction::StopRecording,
                2 => crate::trigger::TriggerAction::Send {
                    ch: c.send_ch,
                    id: c.send_id,
                },
                3 => crate::trigger::TriggerAction::ClearTrace,
                4 => crate::trigger::TriggerAction::InsertMarker,
                _ => crate::trigger::TriggerAction::StartRecording,
            };
            let mut trigger = crate::trigger::Trigger::new(cond, action);
            trigger.enabled = c.enabled;
            Some(trigger)
        })
        .collect();
    app.send(crate::bus::BusCommand::SetTriggers(triggers));
}

fn restore_replay(project: &ProjectV2, app: &mut App) {
    if project.replay.is_empty() {
        return;
    }
    let mut replay = crate::replay_map::ReplayMap::default();
    for entry in &project.replay {
        use crate::replay_map::LogBusKind;
        let kind = match entry.kind.as_str() {
            "can" => LogBusKind::Can,
            "flexray" | "fr" => LogBusKind::Fr,
            "ethernet" | "eth" => LogBusKind::Eth,
            _ => continue,
        };
        let target = if entry.disabled {
            None
        } else {
            Some(crate::model::ClusterId(entry.cluster))
        };
        replay.set(kind, entry.file_channel, target);
    }
    app.send(crate::bus::BusCommand::SetReplayMap(replay));
}

fn relative_source(source: ClusterSource, rel: &impl Fn(&String) -> String) -> ClusterSource {
    match source {
        ClusterSource::Dbc { path } => ClusterSource::Dbc { path: rel(&path) },
        ClusterSource::Arxml { path, cluster } => ClusterSource::Arxml {
            path: rel(&path),
            cluster,
        },
        ClusterSource::Fibex { path } => ClusterSource::Fibex { path: rel(&path) },
        ClusterSource::Manual => ClusterSource::Manual,
    }
}

fn fallback_source(path: Option<String>, cluster: String) -> ClusterSource {
    match path {
        Some(path) if !path.trim().is_empty() => {
            crate::channel::description_source(&path, Some(&cluster))
        }
        _ => ClusterSource::Manual,
    }
}

fn can_bus_of(project: &ProjectV2, cluster: u32) -> Option<u8> {
    project
        .clusters
        .iter()
        .filter(|entry| entry.kind == ClusterKind::Can)
        .position(|entry| entry.id == cluster)
        .map(|bus| bus as u8)
}

fn flexray_bus_of(project: &ProjectV2, cluster: u32) -> Option<u8> {
    project
        .clusters
        .iter()
        .find(|entry| entry.kind == ClusterKind::FlexRay && entry.id == cluster)
        .and_then(|entry| entry.bus)
}

fn replay_entries(app: &App) -> Vec<crate::model::ReplayCfg> {
    use crate::replay_map::LogBusKind;
    let mut entries = Vec::new();
    for (kind, label) in [
        (LogBusKind::Can, "can"),
        (LogBusKind::Fr, "flexray"),
        (LogBusKind::Eth, "ethernet"),
    ] {
        for (raw, target) in app.snap.replay_map.configured(kind) {
            entries.push(crate::model::ReplayCfg {
                kind: label.to_string(),
                file_channel: raw,
                cluster: target.map_or(0, |cluster| cluster.0),
                disabled: target.is_none(),
            });
        }
    }
    entries
}

fn generators(app: &App) -> Vec<GeneratorCfg> {
    let ids = &app.snap.cluster_ids;
    let mut entries = Vec::new();
    for entry in &app.snap.tx {
        if let Some(cluster) = ids.can.get(entry.channel as usize) {
            entries.push(GeneratorCfg::Can {
                cluster: *cluster,
                id: entry.id,
                active: entry.active,
                data_text: entry.data_text.clone(),
                data: crate::generator::parse_hex_bytes(&entry.data_text).unwrap_or_default(),
                cycle_us: entry.cycle_us,
                fd: entry.fd,
                brs: entry.brs,
                srcs: entry.srcs.iter().map(crate::config::src_cfg).collect(),
            });
        }
    }
    for entry in &app.snap.fr_tx {
        if let Some(cluster) = ids.fr.get(&entry.bus) {
            entries.push(GeneratorCfg::FlexRay {
                cluster: *cluster,
                slot: entry.slot,
                active: entry.active,
                data_text: entry.data_text.clone(),
                cycle_us: entry.cycle_us,
                srcs: entry.srcs.iter().map(crate::config::src_cfg).collect(),
            });
        }
    }
    entries
}

fn triggers(app: &App) -> Vec<TriggerCfgV2> {
    let ids = &app.snap.cluster_ids;
    let mut rows = Vec::new();
    for trigger in &app.snap.triggers {
        let mut send_ch = 0u8;
        let mut send_id = 0u32;
        let (kind, ch, id, ext, signal, threshold, rising) = match &trigger.cond {
            crate::trigger::TriggerCond::SignalCross {
                ch,
                id,
                ext,
                signal,
                threshold,
                rising,
            } => (0, *ch, *id, *ext, signal.clone(), *threshold, *rising),
            crate::trigger::TriggerCond::IdPresent { ch, id } => {
                (1, *ch, *id, false, String::new(), 0.0, false)
            }
            crate::trigger::TriggerCond::ErrorFrame { ch } => {
                (2, *ch, 0, false, String::new(), 0.0, false)
            }
            crate::trigger::TriggerCond::CycleTimeout { ch, id } => {
                (3, *ch, *id, false, String::new(), 0.0, false)
            }
            crate::trigger::TriggerCond::SysVar {
                key,
                threshold,
                rising,
            } => (4, 0, 0, false, key.clone(), *threshold, *rising),
            crate::trigger::TriggerCond::FrFramePresent { bus, slot } => {
                (5, *bus, u32::from(*slot), false, String::new(), 0.0, false)
            }
            crate::trigger::TriggerCond::FrSignalCross {
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
            crate::trigger::TriggerCond::DerivedSignal {
                id,
                threshold,
                rising,
            } => (7, 0, 0, false, id.clone(), *threshold, *rising),
        };
        let cluster = match kind {
            0..=3 => ids
                .can
                .get(ch as usize)
                .copied()
                .map(crate::model::ClusterId),
            5 | 6 => ids.fr.get(&ch).copied().map(crate::model::ClusterId),
            _ => Some(crate::model::ClusterId(0)),
        };
        let Some(cluster) = cluster else {
            continue;
        };
        let action = match trigger.action {
            crate::trigger::TriggerAction::StartRecording => 0,
            crate::trigger::TriggerAction::StopRecording => 1,
            crate::trigger::TriggerAction::Send { ch, id } => {
                send_ch = ch;
                send_id = id;
                2
            }
            crate::trigger::TriggerAction::ClearTrace => 3,
            crate::trigger::TriggerAction::InsertMarker => 4,
        };
        let action_cluster = if action == 2 {
            ids.can
                .get(send_ch as usize)
                .copied()
                .map(crate::model::ClusterId)
        } else {
            None
        };
        rows.push(TriggerCfgV2 {
            kind,
            cluster: cluster.0,
            id,
            signal,
            threshold,
            rising,
            extended: ext,
            action,
            action_cluster: action_cluster.map(|id| id.0),
            action_id: send_id,
            enabled: trigger.enabled,
        });
    }
    rows
}

impl App {
    pub fn push_recent_dbc(&mut self, path: String) {
        push_recent(&mut self.recent_dbc, path);
    }

    pub fn push_recent_log(&mut self, path: String) {
        push_recent(&mut self.recent_log, path);
    }

    /// Rebuilds the whole workspace back to factory defaults, keeping only
    /// the machine-local recents.
    pub fn reset_to_defaults(&mut self) {
        self.stop();
        let recent_dbc = std::mem::take(&mut self.recent_dbc);
        let recent_log = std::mem::take(&mut self.recent_log);
        let recent_projects = std::mem::take(&mut self.recent_projects);
        // Revisions must keep increasing across a reset, or the browser would
        // not notice that the new project's workspace arrived.
        let workspace_rev = self.workspace_rev;
        // Rebuild on the same drive the workspace was on: the web host's
        // reset replaces the core thread (the old one exits with its dropped
        // sender), tests rebuild their hand-cranked core.
        let threaded = matches!(self.drive, crate::app::CoreDrive::Threaded);
        *self = if threaded {
            App::new()
        } else {
            App::headless()
        };
        self.recent_dbc = recent_dbc;
        self.recent_log = recent_log;
        self.recent_projects = recent_projects;
        self.workspace_rev = workspace_rev;
    }

    /// Display name of the current project: the file stem, or "Untitled".
    pub fn project_name(&self) -> String {
        self.project_path
            .as_ref()
            .and_then(|p| p.file_stem())
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| "Untitled".to_string())
    }

    pub fn push_recent_project(&mut self, path: String) {
        push_recent(&mut self.recent_projects, path);
    }

    /// Serializable snapshot of the workspace configuration, used to tell
    /// whether anything changed since the last load/save/reset.
    pub(crate) fn config_snapshot(&self) -> String {
        // The web workspace is saved but never marks a project dirty: moving
        // a panel or a cursor is not a semantic change to the bench setup.
        let mut project = ProjectV2::from_app(self, None);
        project.settings.workspace = None;
        serde_json::to_string(&project).unwrap_or_default()
    }

    pub fn is_dirty(&self) -> bool {
        self.config_snapshot() != self.baseline
    }

    /// Marks the current workspace as the clean baseline (after load/save).
    pub fn mark_clean(&mut self) {
        self.baseline = self.config_snapshot();
    }

    /// Saves the workspace as a .rxproj file. `None` opens a picker.
    /// Returns true when a file was written.
    pub fn save_project(&mut self, path: Option<PathBuf>) -> bool {
        let path = match path {
            Some(p) => p,
            None => {
                let mut dlg = rfd::FileDialog::new().add_filter("roxy-can project", &[PROJECT_EXT]);
                if let Some(dir) = self.project_path.as_ref().and_then(|p| p.parent()) {
                    dlg = dlg.set_directory(dir);
                }
                dlg = dlg.set_file_name(format!("{}.rxproj", self.project_name()));
                match dlg.save_file() {
                    Some(p) => p,
                    None => return false,
                }
            }
        };
        let base = path.parent().map(|d| d.to_path_buf());
        let project = ProjectV2::from_app(self, base.as_deref());
        let written = serde_json::to_string_pretty(&project)
            .map_err(|e| e.to_string())
            .and_then(|j| std::fs::write(&path, j).map_err(|e| e.to_string()));
        match written {
            Ok(()) => {
                self.push_recent_project(path.to_string_lossy().to_string());
                self.project_path = Some(path.clone());
                self.retarget_record_draft();
                self.baseline = self.config_snapshot();
                self.status = format!("project saved: {}", path.display());
                true
            }
            Err(e) => {
                self.fail(format!("project save failed: {e}"));
                false
            }
        }
    }

    /// Loads a .rxproj file, replacing the current workspace.
    pub fn open_project_path(&mut self, path: &Path) {
        let text = match std::fs::read_to_string(path) {
            Ok(t) => t,
            Err(e) => {
                self.fail(format!("project read failed: {e}"));
                return;
            }
        };
        let value: serde_json::Value = match serde_json::from_str(&text) {
            Ok(value) => value,
            Err(e) => {
                self.fail(format!("project ignored: {e}"));
                return;
            }
        };
        if value.get("version").and_then(serde_json::Value::as_u64)
            == Some(u64::from(crate::model::PROJECT_VERSION))
        {
            let project: crate::model::ProjectV2 = match serde_json::from_value(value) {
                Ok(project) => project,
                Err(e) => {
                    self.fail(format!("project ignored: {e}"));
                    return;
                }
            };
            let mut project = project;
            project.resolve_paths(path.parent());
            let problems = project.apply(self);
            self.workspace_rev = self.workspace_rev.wrapping_add(1);
            self.project_path = Some(path.to_path_buf());
            self.retarget_record_draft();
            self.baseline = self.config_snapshot();
            self.push_recent_project(path.to_string_lossy().to_string());
            self.status = format!("project loaded: {}", path.display());
            self.report_load_problems(problems);
            return;
        }
        self.fail(
            "project ignored: unsupported project schema; this build reads v2 projects only"
                .to_string(),
        );
    }

    /// Resets the record draft to its default file name. The recording
    /// lands in the project's Record/ folder (resolved at arm time);
    /// only the name shows in the toolbar input, and a draft the user
    /// typed themselves is left alone.
    pub fn retarget_record_draft(&mut self) {
        if app_record_draft_is_default(&self.record_path_buf) {
            self.record_path_buf = "record".to_string();
        }
    }

    /// Starts a fresh, completely empty untitled workspace: no DBCs, no
    /// observer windows, no generator entries.
    #[cfg(test)]
    pub fn new_project(&mut self) {
        self.reset_to_defaults();
        self.workspace_rev = self.workspace_rev.wrapping_add(1);
        // The bus half of "empty" is a command; wait for it to land so
        // the baseline below describes the emptied workspace.
        self.send(crate::bus::BusCommand::ClearDatabases);
        self.settle();
        self.trace_windows.clear();
        self.msg_windows.clear();
        self.stats_windows.clear();
        self.graphics.clear();
        self.data_windows.clear();
        // The sixth kind the bootstrap opens. Missing it left a State Tracker
        // window standing in every "new project".
        self.state_trackers.clear();
        self.baseline = self.config_snapshot();
        self.status = "new project".to_string();
    }
}
