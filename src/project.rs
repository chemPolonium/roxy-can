use std::path::{Path, PathBuf};

use crate::config::{Config, PROJECT_EXT};

use crate::app::App;

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
        let mut cfg = Config::from_app(self, None);
        cfg.web = None;
        serde_json::to_string(&cfg).unwrap_or_default()
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
        let config = Config::from_app(self, base.as_deref());
        let mut project = crate::model::project_from_config(&config);
        project.replay_speed = config.replay_speed;
        project.replay_fastest = config.replay_fastest;
        project.text_rate_hz = config.text_rate_hz;
        project.trace_limit = config.trace_limit;
        project.counters = config.counters.clone();
        project.recent_dbc = config.recent_dbc.clone();
        project.recent_log = config.recent_log.clone();
        project.spec = config.spec;
        project.limits = config.limits;
        let ids = &self.snap.cluster_ids;
        let mut fr_order: Vec<u8> = config.fr_buses.iter().map(|bus| bus.bus).collect();
        fr_order.sort_unstable();
        let mut eth_order: Vec<u8> = config.eth_buses.iter().map(|bus| bus.bus).collect();
        eth_order.sort_unstable();
        let mut can_index = 0usize;
        let mut fr_index = 0usize;
        let mut eth_index = 0usize;
        for cluster in &mut project.clusters {
            match cluster.kind {
                crate::model::ClusterKind::Can => {
                    if let Some(id) = ids.can.get(can_index) {
                        cluster.id = *id;
                    }
                    can_index += 1;
                }
                crate::model::ClusterKind::FlexRay => {
                    if let Some(bus) = fr_order.get(fr_index)
                        && let Some(id) = ids.fr.get(bus)
                    {
                        cluster.id = *id;
                    }
                    fr_index += 1;
                }
                crate::model::ClusterKind::Ethernet => {
                    if let Some(bus) = eth_order.get(eth_index)
                        && let Some(id) = ids.eth.get(bus)
                    {
                        cluster.id = *id;
                    }
                    eth_index += 1;
                }
            }
        }
        if !ids.can.is_empty() || !ids.fr.is_empty() || !ids.eth.is_empty() {
            project.next_cluster_id = ids.next;
            project.bus_map = Some(crate::model::BusMap {
                can: ids
                    .can
                    .iter()
                    .copied()
                    .map(crate::model::ClusterId)
                    .collect(),
                flexray: {
                    let mut buses: Vec<_> = fr_order.iter().collect();
                    buses.sort_unstable();
                    let mut map = Vec::new();
                    for bus in buses {
                        if let Some(id) = ids.fr.get(bus) {
                            while map.len() <= *bus as usize {
                                map.push(crate::model::ClusterId(0));
                            }
                            map[*bus as usize] = crate::model::ClusterId(*id);
                        }
                    }
                    map
                },
                ethernet: {
                    let mut map = Vec::new();
                    for bus in &eth_order {
                        if let Some(id) = ids.eth.get(bus) {
                            while map.len() <= *bus as usize {
                                map.push(crate::model::ClusterId(0));
                            }
                            map[*bus as usize] = crate::model::ClusterId(*id);
                        }
                    }
                    map
                },
            });
        }
        project.replay.clear();
        for (raw, target) in self
            .snap
            .replay_map
            .configured(crate::replay_map::LogBusKind::Can)
        {
            project.replay.push(crate::model::ReplayCfg {
                kind: "can".to_string(),
                file_channel: raw,
                cluster: target.map_or(0, |cluster| cluster.0),
                disabled: target.is_none(),
            });
        }
        for (raw, target) in self
            .snap
            .replay_map
            .configured(crate::replay_map::LogBusKind::Fr)
        {
            project.replay.push(crate::model::ReplayCfg {
                kind: "flexray".to_string(),
                file_channel: raw,
                cluster: target.map_or(0, |cluster| cluster.0),
                disabled: target.is_none(),
            });
        }
        for (raw, target) in self
            .snap
            .replay_map
            .configured(crate::replay_map::LogBusKind::Eth)
        {
            project.replay.push(crate::model::ReplayCfg {
                kind: "ethernet".to_string(),
                file_channel: raw,
                cluster: target.map_or(0, |cluster| cluster.0),
                disabled: target.is_none(),
            });
        }
        let mut generators = Vec::new();
        for entry in &config.tx {
            if let Some(cluster) = ids.can.get(entry.channel as usize) {
                generators.push(crate::model::GeneratorCfg::Can {
                    cluster: *cluster,
                    id: entry.id,
                    active: entry.active,
                    data_text: entry.data_text.clone(),
                    data: entry.data.clone(),
                    cycle_us: entry.cycle_us,
                    fd: entry.fd,
                    brs: entry.brs,
                    srcs: entry.srcs.clone(),
                });
            }
        }
        for entry in &config.fr_tx {
            if let Some(cluster) = ids.fr.get(&entry.bus) {
                generators.push(crate::model::GeneratorCfg::FlexRay {
                    cluster: *cluster,
                    slot: entry.slot,
                    active: entry.active,
                    data_text: entry.data_text.clone(),
                    cycle_us: entry.cycle_us,
                    srcs: entry.srcs.clone(),
                });
            }
        }
        project.generators = Some(generators);
        let mut triggers = Vec::new();
        for cfg in &config.triggers {
            let cluster = match cfg.kind {
                0..=3 => ids
                    .can
                    .get(cfg.ch as usize)
                    .copied()
                    .map(crate::model::ClusterId),
                5 | 6 => ids.fr.get(&cfg.ch).copied().map(crate::model::ClusterId),
                _ => Some(crate::model::ClusterId(0)),
            };
            let Some(cluster) = cluster else {
                continue;
            };
            let action_cluster = if cfg.action == 2 {
                ids.can
                    .get(cfg.send_ch as usize)
                    .copied()
                    .map(crate::model::ClusterId)
            } else {
                None
            };
            triggers.push(crate::model::TriggerCfgV2 {
                kind: cfg.kind,
                cluster: cluster.0,
                id: cfg.id,
                signal: cfg.signal.clone(),
                threshold: cfg.threshold,
                rising: cfg.rising,
                extended: cfg.ext,
                action: cfg.action,
                action_cluster: action_cluster.map(|id| id.0),
                action_id: cfg.send_id,
                enabled: cfg.enabled,
            });
        }
        project.triggers = Some(triggers);
        // Workspace sections move out of the compat blob and into typed v2
        // fields; their picks still name buses by index for now.
        project.trace_windows = config.trace_windows.clone();
        for window in &mut project.trace_windows {
            window.manual_clusters = window
                .manual
                .iter()
                .filter_map(|&(bus, id)| ids.can.get(bus as usize).map(|cluster| (*cluster, id)))
                .collect();
            window.fr_manual_clusters = window
                .fr_manual
                .iter()
                .filter_map(|&(bus, slot)| ids.fr.get(&bus).map(|cluster| (*cluster, slot)))
                .collect();
        }
        project.msg_windows = config.msg_windows.clone();
        project.stats_windows = config.stats_windows.clone();
        project.graphics = config.graphics.clone();
        project.data_windows = config.data_windows.clone();
        project.monitor_rows = config.monitor_rows.clone();
        project.state_trackers = config.state_trackers.clone();
        project.nodes = config.nodes.clone();
        project.blocks = config.blocks.clone();
        project.sysvars = config.sysvars.clone();
        project.derived = config.derived.clone();
        project.workspace = config.web.clone();
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
            // The project is the whole on-disk schema: build the apply DTO
            // from its typed sections, never from a compat blob.
            let mut cfg = Config::default();
            restore_buses_from_clusters(&project, &mut cfg);
            cfg.replay_speed = project.replay_speed;
            cfg.replay_fastest = project.replay_fastest;
            cfg.text_rate_hz = project.text_rate_hz;
            cfg.trace_limit = project.trace_limit;
            cfg.counters = project.counters.clone();
            cfg.recent_dbc = project.recent_dbc.clone();
            cfg.recent_log = project.recent_log.clone();
            cfg.spec = project.spec;
            cfg.limits = project.limits;
            cfg.trace_windows = project.trace_windows.clone();
            cfg.msg_windows = project.msg_windows.clone();
            cfg.stats_windows = project.stats_windows.clone();
            cfg.graphics = project.graphics.clone();
            cfg.data_windows = project.data_windows.clone();
            cfg.monitor_rows = project.monitor_rows.clone();
            cfg.state_trackers = project.state_trackers.clone();
            cfg.nodes = project.nodes.clone();
            cfg.blocks = project.blocks.clone();
            cfg.sysvars = project.sysvars.clone();
            cfg.derived = project.derived.clone();
            cfg.web = project.workspace.clone();

            // generator 段以 cluster id 记路，打开时换回路号。
            if let Some(generators) = &project.generators {
                let bus_of = |ids: &[crate::model::ClusterId], cluster: u32| {
                    ids.iter()
                        .position(|id| id.0 == cluster)
                        .map(|bus| bus as u8)
                };
                let map = project.bus_map.as_ref();
                cfg.tx.clear();
                cfg.fr_tx.clear();
                for generator in generators {
                    match generator {
                        crate::model::GeneratorCfg::Can {
                            cluster,
                            id,
                            active,
                            data_text,
                            data,
                            cycle_us,
                            fd,
                            brs,
                            srcs,
                        } => {
                            let Some(channel) = map.and_then(|m| bus_of(&m.can, *cluster)) else {
                                continue;
                            };
                            cfg.tx.push(crate::config::TxCfg {
                                channel,
                                id: *id,
                                active: *active,
                                data_text: data_text.clone(),
                                data: data.clone(),
                                cycle_us: *cycle_us,
                                fd: *fd,
                                brs: *brs,
                                srcs: srcs.clone(),
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
                            let Some(bus) = map.and_then(|m| bus_of(&m.flexray, *cluster)) else {
                                continue;
                            };
                            cfg.fr_tx.push(crate::config::FrTxCfg {
                                bus,
                                slot: *slot,
                                active: *active,
                                data_text: data_text.clone(),
                                cycle_us: *cycle_us,
                                srcs: srcs.clone(),
                            });
                        }
                    }
                }
            }
            // v2 trigger rows name their buses by cluster id; the core still
            // keys rules by 路号, so convert back through the saved map. A
            // rule whose cluster is gone is dropped rather than pointed at a
            // different bus.
            if let Some(triggers) = &project.triggers {
                let map = project.bus_map.as_ref();
                let bus_of = |ids: Option<&Vec<crate::model::ClusterId>>, cluster: u32| {
                    ids?.iter()
                        .position(|id| id.0 == cluster)
                        .map(|bus| bus as u8)
                };
                let (can_ids, fr_ids) = map
                    .map(|m| (Some(&m.can), Some(&m.flexray)))
                    .unwrap_or((None, None));
                cfg.triggers = triggers
                    .iter()
                    .filter_map(|t| {
                        let ch = match t.kind {
                            0..=3 => bus_of(can_ids, t.cluster)?,
                            5 | 6 => bus_of(fr_ids, t.cluster)?,
                            _ => 0,
                        };
                        let send_ch = if t.action == 2 {
                            t.action_cluster
                                .and_then(|cluster| bus_of(can_ids, cluster))
                                .unwrap_or(0)
                        } else {
                            0
                        };
                        Some(crate::config::TriggerCfg {
                            kind: t.kind,
                            ch,
                            id: t.id,
                            signal: t.signal.clone(),
                            threshold: t.threshold,
                            rising: t.rising,
                            ext: t.extended,
                            action: t.action,
                            send_ch,
                            send_id: t.action_id,
                            enabled: t.enabled,
                        })
                    })
                    .collect();
            }
            // Typed v2 workspace sections win over the compat blob; an empty
            // section means the project legitimately has none (new saves do
            // not write those keys into `config` any more).
            if !project.trace_windows.is_empty() {
                cfg.trace_windows = project.trace_windows.clone();
            }
            if !project.msg_windows.is_empty() {
                cfg.msg_windows = project.msg_windows.clone();
            }
            if !project.stats_windows.is_empty() {
                cfg.stats_windows = project.stats_windows.clone();
            }
            if !project.graphics.is_empty() {
                cfg.graphics = project.graphics.clone();
            }
            if !project.data_windows.is_empty() {
                cfg.data_windows = project.data_windows.clone();
            }
            if !project.monitor_rows.is_empty() {
                cfg.monitor_rows = project.monitor_rows.clone();
            }
            if !project.state_trackers.is_empty() {
                cfg.state_trackers = project.state_trackers.clone();
            }
            if !project.nodes.is_empty() {
                cfg.nodes = project.nodes.clone();
            }
            if !project.blocks.is_empty() {
                cfg.blocks = project.blocks.clone();
            }
            if !project.sysvars.is_empty() {
                cfg.sysvars = project.sysvars.clone();
            }
            if !project.derived.is_empty() {
                cfg.derived = project.derived.clone();
            }
            if let Some(workspace) = &project.workspace {
                cfg.web = Some(workspace.clone());
            }
            // Window picks saved by cluster id turn back into the bus indexes
            // the trace tables use.
            if let Some(map) = &project.bus_map {
                for window in &mut cfg.trace_windows {
                    if !window.manual_clusters.is_empty() {
                        window.manual = window
                            .manual_clusters
                            .iter()
                            .filter_map(|&(cluster, id)| {
                                map.can
                                    .iter()
                                    .position(|saved| saved.0 == cluster)
                                    .map(|bus| (bus as u8, id))
                            })
                            .collect();
                    }
                    if !window.fr_manual_clusters.is_empty() {
                        window.fr_manual = window
                            .fr_manual_clusters
                            .iter()
                            .filter_map(|&(cluster, slot)| {
                                map.flexray
                                    .iter()
                                    .position(|saved| saved.0 == cluster)
                                    .map(|bus| (bus as u8, slot))
                            })
                            .collect();
                    }
                }
            }
            self.reset_to_defaults();
            cfg.resolve_paths(path.parent());
            let problems = cfg.apply(self);
            self.settle();

            if let Some(map) = &project.bus_map {
                self.send(crate::bus::BusCommand::SetClusterIds {
                    next: project.next_cluster_id,
                    can: map.can.iter().map(|id| id.0).collect(),
                    fr: map
                        .flexray
                        .iter()
                        .enumerate()
                        .filter(|(_, id)| id.0 != 0)
                        .map(|(bus, id)| (bus as u8, id.0))
                        .collect(),
                    eth: map
                        .ethernet
                        .iter()
                        .enumerate()
                        .filter(|(_, id)| id.0 != 0)
                        .map(|(bus, id)| (bus as u8, id.0))
                        .collect(),
                });
                if !project.replay.is_empty() {
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
                    self.send(crate::bus::BusCommand::SetReplayMap(replay));
                }
                // `SetClusterIds` rebuilt the adapter models around the
                // restored ids; the ARXML models have to be read once more
                // or the document's container/mux structure is lost.
                self.reload_arxml_models();
                self.settle();
            }
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

/// Rebuilds the bus description lists from the project's typed clusters.
/// The config blob deliberately carries no channels, so this is the only
/// place bus names, paths, bitrates and node roles come back on open.
fn restore_buses_from_clusters(project: &crate::model::ProjectV2, cfg: &mut Config) {
    use crate::model::{ClusterKind, ClusterSource};
    cfg.channels.clear();
    cfg.can_arxml.clear();
    cfg.fr_buses.clear();
    cfg.fr_names.clear();
    cfg.eth_buses.clear();
    let (mut can, mut fr, mut eth) = (0u8, 0u8, 0u8);
    for cluster in &project.clusters {
        match cluster.kind {
            ClusterKind::Can => {
                let mut paths = Vec::new();
                for source in &cluster.sources {
                    match source {
                        ClusterSource::Dbc { path } => paths.push(path.clone()),
                        ClusterSource::Arxml { path, cluster } => {
                            cfg.can_arxml.push(crate::config::CanArxmlFile {
                                bus: can,
                                path: path.clone(),
                                cluster: cluster.clone(),
                            });
                        }
                        _ => {}
                    }
                }
                let can_cfg = cluster.can.as_ref();
                cfg.channels.push(crate::config::ChannelCfg {
                    name: cluster.name.clone(),
                    dbc_path: paths.first().cloned().unwrap_or_default(),
                    dbc_paths_extra: paths.into_iter().skip(1).collect(),
                    node_roles: cluster.node_roles.clone(),
                    bitrate_kbps: can_cfg
                        .map(|params| params.bitrate_kbps)
                        .unwrap_or(crate::model::CanParams::DEFAULT_BITRATE_KBPS),
                    fd_data_kbps: can_cfg
                        .map(|params| params.fd_data_kbps)
                        .unwrap_or(crate::model::CanParams::DEFAULT_FD_DATA_KBPS),
                });
                can += 1;
            }
            ClusterKind::FlexRay => {
                let (path, name) = description_source(&cluster.sources);
                cfg.fr_buses.push(crate::config::FrBusFile {
                    bus: fr,
                    path,
                    cluster: name,
                });
                cfg.fr_names.push((fr, cluster.name.clone()));
                fr += 1;
            }
            ClusterKind::Ethernet => {
                let (path, name) = description_source(&cluster.sources);
                cfg.eth_buses.push(crate::config::EthBusFile {
                    bus: eth,
                    path,
                    cluster: name,
                });
                eth += 1;
            }
        }
    }
}

fn description_source(sources: &[crate::model::ClusterSource]) -> (String, String) {
    for source in sources {
        match source {
            crate::model::ClusterSource::Arxml { path, cluster } => {
                return (path.clone(), cluster.clone());
            }
            crate::model::ClusterSource::Fibex { path } => return (path.clone(), String::new()),
            _ => {}
        }
    }
    (String::new(), String::new())
}
