//! Project-level cluster registry.
//!
//! One registry owns the global `ClusterId` space and every imported `Comm`.
//! Importers feed it through the same trait, so a project can mix an ARXML
//! matrix, a DBC and a FIBEX file without the old per-protocol bus maps.

use super::*;
use std::collections::BTreeMap;

pub struct ClusterEntry {
    pub id: ClusterId,
    pub cfg: ClusterCfg,
    pub comm: Comm,
}

#[derive(Default)]
pub struct ClusterRegistry {
    cluster_ids: ClusterIds,
    elements: ElementIds,
    clusters: BTreeMap<ClusterId, ClusterEntry>,
}

impl ClusterRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn cluster_ids(&self) -> &ClusterIds {
        &self.cluster_ids
    }

    pub fn elements(&self) -> &ElementIds {
        &self.elements
    }

    pub fn clusters(&self) -> impl Iterator<Item = &ClusterEntry> {
        self.clusters.values()
    }

    pub fn get(&self, id: ClusterId) -> Option<&ClusterEntry> {
        self.clusters.get(&id)
    }

    pub fn find_source(&self, source: &ClusterSource) -> Option<ClusterId> {
        self.clusters
            .values()
            .find(|entry| {
                entry
                    .cfg
                    .sources
                    .iter()
                    .any(|candidate| candidate == source)
            })
            .map(|entry| entry.id)
    }

    /// Imports every cluster in an ARXML file. The returned errors are per
    /// cluster: one broken cluster does not hide the others.
    pub fn add_arxml(&mut self, text: &str, path: &str) -> Vec<Result<ClusterId, Issue>> {
        let scanned = match scan_clusters(text) {
            Ok(scanned) => scanned,
            Err(error) => {
                return vec![Err(Issue::error(
                    IssueKind::Other,
                    format!("{path}: {error}"),
                ))];
            }
        };
        scanned
            .into_iter()
            .map(|cluster| {
                let source = ClusterSource::Arxml {
                    path: path.to_string(),
                    cluster: cluster.name.clone(),
                };
                if self.find_source(&source).is_some() {
                    return Err(Issue::warning(
                        IssueKind::Other,
                        format!("cluster '{}' is already imported", cluster.name),
                    )
                    .at(path));
                }
                let id = self.cluster_ids.allocate();
                let outcome = ArxmlImporter.import(
                    ImportInput {
                        cluster: id,
                        source: &source,
                        text,
                    },
                    &mut self.elements,
                );
                self.insert(id, outcome)
            })
            .collect()
    }

    pub fn add_dbc(&mut self, text: &str, path: &str) -> Result<ClusterId, Issue> {
        let source = ClusterSource::Dbc {
            path: path.to_string(),
        };
        if self.find_source(&source).is_some() {
            return Err(Issue::warning(
                IssueKind::Other,
                format!("database '{path}' is already imported"),
            ));
        }
        let id = self.cluster_ids.allocate();
        let outcome = DbcImporter.import(
            ImportInput {
                cluster: id,
                source: &source,
                text,
            },
            &mut self.elements,
        );
        self.insert(id, outcome)
    }

    pub fn add_fibex(
        &mut self,
        text: &str,
        path: &str,
        ethernet: bool,
    ) -> Result<ClusterId, Issue> {
        let source = ClusterSource::Fibex {
            path: path.to_string(),
        };
        if self.find_source(&source).is_some() {
            return Err(Issue::warning(
                IssueKind::Other,
                format!("database '{path}' is already imported"),
            ));
        }
        let id = self.cluster_ids.allocate();
        let importer: &dyn ClusterImporter = if ethernet {
            &EthernetDbImporter
        } else {
            &FibexImporter
        };
        let outcome = importer.import(
            ImportInput {
                cluster: id,
                source: &source,
                text,
            },
            &mut self.elements,
        );
        self.insert(id, outcome)
    }

    /// Converts the registry to the persisted schema. Views still live in
    /// `ProjectV2::rest` until the Phase 7 migration.
    pub fn to_project(&self) -> ProjectV2 {
        let clusters = self
            .clusters
            .values()
            .map(|entry| entry.cfg.clone())
            .collect();
        let mut project = ProjectV2::new(clusters);
        project.next_cluster_id = self.cluster_ids.next();
        project.aliases_from_ids(&self.elements);
        project
    }

    /// Rebuilds the registry from a saved schema v2 project. Sources are
    /// re-imported; ids come from the persisted aliases so a reopened project
    /// keeps the same signal keys.
    pub fn from_project(project: &ProjectV2, base: Option<&std::path::Path>) -> (Self, Vec<Issue>) {
        let mut registry = Self {
            cluster_ids: ClusterIds::new(project.next_cluster_id),
            elements: project.element_ids().unwrap_or_default(),
            clusters: BTreeMap::new(),
        };
        let mut issues = Vec::new();
        for cfg in &project.clusters {
            let id = ClusterId(cfg.id);
            registry.cluster_ids.reserve(id);
            let mut outcome = import_cfg(cfg, base, &mut registry.elements);
            if outcome.comm.clusters.is_empty() {
                // A missing source still leaves the cluster row: the user can
                // relink the file without losing the cluster's identity.
                let network_id = NetworkId(id.0);
                outcome.comm.clusters.insert(
                    id,
                    Cluster {
                        id,
                        kind: cfg.kind,
                        name: cfg.name.clone(),
                        aliases: cfg.aliases.clone(),
                        description: String::new(),
                        sources: cfg.sources.clone(),
                        networks: vec![network_id],
                    },
                );
                outcome.comm.networks.insert(
                    network_id,
                    Network {
                        id: network_id,
                        cluster: id,
                        name: cfg.name.clone(),
                        params: params_from_cfg(cfg),
                    },
                );
            }
            if let Some(cluster) = outcome.comm.clusters.get_mut(&id) {
                cluster.name = cfg.name.clone();
                cluster.aliases = cfg.aliases.clone();
                cluster.kind = cfg.kind;
                cluster.sources = cfg.sources.clone();
            }
            issues.append(&mut outcome.issues);
            let cfg = cluster_cfg_for(&outcome.comm, id, cfg);
            registry.clusters.insert(
                id,
                ClusterEntry {
                    id,
                    cfg,
                    comm: outcome.comm,
                },
            );
        }
        (registry, issues)
    }

    fn insert(&mut self, id: ClusterId, outcome: ImportOutcome) -> Result<ClusterId, Issue> {
        let Some(cluster) = outcome.comm.clusters.get(&id) else {
            return Err(outcome.issues.first().cloned().unwrap_or_else(|| {
                Issue::error(IssueKind::Other, "importer produced no cluster")
            }));
        };
        let networks: Vec<&Network> = cluster
            .networks
            .iter()
            .filter_map(|network| outcome.comm.networks.get(network))
            .collect();
        let cfg = ClusterCfg::from_model(cluster, &networks, BTreeMap::new());
        self.clusters.insert(
            id,
            ClusterEntry {
                id,
                cfg,
                comm: outcome.comm,
            },
        );
        Ok(id)
    }
}

fn import_cfg(
    cfg: &ClusterCfg,
    base: Option<&std::path::Path>,
    ids: &mut ElementIds,
) -> ImportOutcome {
    let Some(source) = cfg.sources.first() else {
        return ImportOutcome {
            comm: Comm::default(),
            issues: Vec::new(),
        };
    };
    let mut issues = Vec::new();
    if cfg.sources.len() > 1 {
        issues.push(Issue::warning(
            IssueKind::Unsupported,
            format!(
                "cluster '{}' has {} sources; only the first is re-imported",
                cfg.name,
                cfg.sources.len()
            ),
        ));
    }
    let Some(relative) = source.path() else {
        return ImportOutcome {
            comm: Comm::default(),
            issues,
        };
    };
    let path = {
        let path = std::path::Path::new(relative);
        match base {
            Some(base) if !path.is_absolute() => base.join(path),
            _ => path.to_path_buf(),
        }
    };
    let text = match std::fs::read(&path) {
        Ok(bytes) => crate::dbc::text_from_bytes(bytes),
        Err(error) => {
            issues.push(
                Issue::error(
                    IssueKind::Other,
                    format!("cannot read {}: {error}", path.display()),
                )
                .at(relative),
            );
            return ImportOutcome {
                comm: Comm::default(),
                issues,
            };
        }
    };
    let outcome = match (cfg.kind, source) {
        (ClusterKind::Can, ClusterSource::Dbc { .. }) => {
            let db = match crate::dbc::load_dbc_str(&text) {
                Ok(db) => db,
                Err(error) => {
                    issues.push(Issue::error(
                        IssueKind::Other,
                        format!("{}: {error}", path.display()),
                    ));
                    return ImportOutcome {
                        comm: Comm::default(),
                        issues,
                    };
                }
            };
            let can = cfg.can.clone().unwrap_or_default();
            import_symbol_table(
                &db,
                &cfg.name,
                ClusterId(cfg.id),
                source,
                ids,
                can.bitrate_kbps,
                can.fd_data_kbps,
            )
        }
        (_, ClusterSource::Arxml { .. }) => ArxmlImporter.import(
            ImportInput {
                cluster: ClusterId(cfg.id),
                source,
                text: &text,
            },
            ids,
        ),
        (ClusterKind::FlexRay, ClusterSource::Fibex { .. }) => {
            import_fibex(&text, &cfg.name, ClusterId(cfg.id), source, ids)
        }
        (ClusterKind::Ethernet, ClusterSource::Fibex { .. }) => {
            import_eth_db(&text, &cfg.name, ClusterId(cfg.id), source, ids)
        }
        _ => ImportOutcome {
            comm: Comm::default(),
            issues: vec![Issue::error(
                IssueKind::Unsupported,
                format!(
                    "cluster '{}' cannot be imported from its declared source",
                    cfg.name
                ),
            )],
        },
    };
    let mut outcome = outcome;
    outcome.issues.append(&mut issues);
    outcome
}

fn params_from_cfg(cfg: &ClusterCfg) -> NetworkParams {
    match cfg.kind {
        ClusterKind::Can => {
            let can = cfg.can.clone().unwrap_or_default();
            NetworkParams::Can(CanParams {
                bitrate_kbps: can.bitrate_kbps,
                fd_data_kbps: can.fd_data_kbps,
                sample_point_permille: None,
            })
        }
        ClusterKind::FlexRay => {
            let fr = cfg.flexray.clone().unwrap_or_default();
            NetworkParams::FlexRay(FlexRayParams {
                speed_kbps: fr.speed_kbps,
                cycle_time_ms: fr.cycle_time_ms,
                macrotick_us: 0.0,
                coldstart_attempts: 0,
                action_point_offset: 0,
                minislot_action_point_offset: 0,
                dynamic_slot_idle_phase: 0,
                minor_version: 0,
                extra: BTreeMap::new(),
            })
        }
        ClusterKind::Ethernet => {
            let eth = cfg.ether.clone().unwrap_or_default();
            NetworkParams::Ethernet(EthernetParams {
                physical_layer: eth
                    .physical_layer
                    .as_deref()
                    .and_then(EthernetPhy::from_xml),
                vlans: eth.vlans.clone(),
                frame_length_includes_header: eth.frame_length_includes_header,
            })
        }
    }
}

fn cluster_cfg_for(comm: &Comm, id: ClusterId, original: &ClusterCfg) -> ClusterCfg {
    let Some(cluster) = comm.clusters.get(&id) else {
        return original.clone();
    };
    let networks: Vec<&Network> = cluster
        .networks
        .iter()
        .filter_map(|network| comm.networks.get(network))
        .collect();
    ClusterCfg::from_model(cluster, &networks, original.node_roles.clone())
}
