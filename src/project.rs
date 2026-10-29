use std::path::{Path, PathBuf};

use crate::config::{Config, PROJECT_EXT, ProjectFile};

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
        let config_value = serde_json::to_value(&config).unwrap_or_default();
        let (mut project, _report) = crate::model::migrate_v1_clusters(&config);
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
            project.legacy_bus_map = Some(crate::model::LegacyBusMap {
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
        project.rest.insert("config".to_string(), config_value);
        project.replay.clear();
        for (raw, target) in self
            .snap
            .replay_map
            .configured(crate::replay_map::LogBusKind::Can)
        {
            if let Some(bus) = target
                && let Some(id) = ids.can.get(bus as usize)
            {
                project.replay.push(crate::model::ReplayCfg {
                    kind: "can".to_string(),
                    file_channel: raw,
                    cluster: *id,
                });
            }
        }
        for (raw, target) in self
            .snap
            .replay_map
            .configured(crate::replay_map::LogBusKind::Fr)
        {
            if let Some(bus) = target
                && let Some(id) = ids.fr.get(&bus)
            {
                project.replay.push(crate::model::ReplayCfg {
                    kind: "flexray".to_string(),
                    file_channel: raw,
                    cluster: *id,
                });
            }
        }
        for (raw, target) in self
            .snap
            .replay_map
            .configured(crate::replay_map::LogBusKind::Eth)
        {
            if let Some(bus) = target
                && let Some(id) = ids.eth.get(&bus)
            {
                project.replay.push(crate::model::ReplayCfg {
                    kind: "ethernet".to_string(),
                    file_channel: raw,
                    cluster: *id,
                });
            }
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
            let Some(legacy) = project
                .rest
                .get("legacyConfig")
                .or_else(|| project.rest.get("config"))
            else {
                self.fail("project ignored: schema v2 file has no legacyConfig yet");
                return;
            };
            let mut cfg: Config = match serde_json::from_value(legacy.clone()) {
                Ok(cfg) => cfg,
                Err(e) => {
                    self.fail(format!("project ignored: legacyConfig: {e}"));
                    return;
                }
            };
            // v2 的 generator 段以 cluster id 记路，打开时换回旧配置里的
            // 路号：`config` 兼容段只是同一批行的一份拷贝，以 v2 段为准。
            if let Some(generators) = &project.generators {
                let bus_of = |ids: &[crate::model::ClusterId], cluster: u32| {
                    ids.iter()
                        .position(|id| id.0 == cluster)
                        .map(|bus| bus as u8)
                };
                let map = project.legacy_bus_map.as_ref();
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
            self.reset_to_defaults();
            cfg.resolve_paths(path.parent());
            let problems = cfg.apply(self);
            if let Some(map) = &project.legacy_bus_map {
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
                        let bus = match kind {
                            LogBusKind::Can => map
                                .can
                                .iter()
                                .position(|id| id.0 == entry.cluster)
                                .map(|index| index as u8),
                            LogBusKind::Fr => map
                                .flexray
                                .iter()
                                .position(|id| id.0 == entry.cluster)
                                .map(|bus| bus as u8),
                            LogBusKind::Eth => map
                                .ethernet
                                .iter()
                                .position(|id| id.0 == entry.cluster)
                                .map(|bus| bus as u8),
                        };
                        if let Some(bus) = bus {
                            replay.set(kind, entry.file_channel, Some(bus));
                        }
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
        match serde_json::from_str::<ProjectFile>(&text) {
            Ok(proj) => {
                self.reset_to_defaults();
                let mut cfg = proj.config;
                cfg.resolve_paths(path.parent());
                let problems = cfg.apply(self);
                self.workspace_rev = self.workspace_rev.wrapping_add(1);
                self.project_path = Some(path.to_path_buf());
                self.retarget_record_draft();
                self.baseline = self.config_snapshot();
                self.push_recent_project(path.to_string_lossy().to_string());
                self.status = format!("project loaded: {}", path.display());
                // After the "loaded" line: a half-restored project has to say
                // so on the bar the user is looking at, and keep the detail
                // where it can still be read.
                self.report_load_problems(problems);
            }
            Err(e) => self.fail(format!("project ignored: {e}")),
        }
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
