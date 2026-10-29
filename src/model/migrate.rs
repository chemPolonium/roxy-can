//! v1 project → schema v2 migration.
//!
//! The first slice converts the three parallel bus lists into the single
//! `clusters` list and records the old bus-index → cluster-id map. Window
//! signal references, generators and triggers still ride `rest` until their
//! v2 shapes are frozen; the report says what was carried over verbatim.

use serde_json::Value;

use super::*;
use crate::config::{CanArxmlFile, ChannelCfg, Config, EthBusFile, FrBusFile};

pub struct MigrationReport {
    pub issues: Vec<Issue>,
    pub legacy_buses: LegacyBusMap,
}

/// Converts the bus half of a v1 config. `next_cluster_id` is one past the
/// highest id assigned here; CAN channels keep their old order, then FlexRay,
/// then Ethernet.
pub fn migrate_v1_clusters(config: &Config) -> (ProjectV2, MigrationReport) {
    let mut clusters = Vec::new();
    let mut issues = Vec::new();
    let mut next_id = 1u32;

    let mut can_ids = Vec::with_capacity(config.channels.len());
    for channel in &config.channels {
        let id = next_id;
        next_id += 1;
        can_ids.push(ClusterId(id));
        clusters.push(can_cluster_cfg(id, channel, &config.can_arxml));
    }
    if !config.can_arxml.is_empty() {
        for arxml in &config.can_arxml {
            if arxml.bus as usize >= config.channels.len() {
                issues.push(Issue::warning(
                    IssueKind::DanglingRef,
                    format!(
                        "CAN ARXML cluster '{}' names bus {}, which no channel uses",
                        arxml.cluster, arxml.bus
                    ),
                ));
            }
        }
    }

    let mut fr_ids = Vec::new();
    let mut fr_buses: Vec<&FrBusFile> = config.fr_buses.iter().collect();
    fr_buses.sort_by_key(|bus| bus.bus);
    for bus in fr_buses {
        let id = next_id;
        next_id += 1;
        while fr_ids.len() <= bus.bus as usize {
            fr_ids.push(ClusterId(0));
        }
        fr_ids[bus.bus as usize] = ClusterId(id);
        let alias = config
            .fr_names
            .iter()
            .find(|(index, _)| *index == bus.bus)
            .map(|(_, name)| name.clone())
            .unwrap_or_else(|| format!("FR{}", bus.bus));
        clusters.push(ClusterCfg {
            id,
            kind: ClusterKind::FlexRay,
            name: alias,
            aliases: Vec::new(),
            sources: vec![source_for_path(&bus.path, Some(&bus.cluster))],
            can: None,
            flexray: Some(FlexRayCfg::default()),
            ether: None,
            node_roles: Default::default(),
        });
    }

    let mut eth_ids = Vec::new();
    let mut eth_buses: Vec<&EthBusFile> = config.eth_buses.iter().collect();
    eth_buses.sort_by_key(|bus| bus.bus);
    for bus in eth_buses {
        let id = next_id;
        next_id += 1;
        while eth_ids.len() <= bus.bus as usize {
            eth_ids.push(ClusterId(0));
        }
        eth_ids[bus.bus as usize] = ClusterId(id);
        clusters.push(ClusterCfg {
            id,
            kind: ClusterKind::Ethernet,
            name: format!("ETH{}", bus.bus),
            aliases: Vec::new(),
            sources: vec![source_for_path(&bus.path, Some(&bus.cluster))],
            can: None,
            flexray: None,
            ether: Some(EtherCfg {
                physical_layer: None,
                vlans: Vec::new(),
                frame_length_includes_header: config.eth_frame_length_includes_header,
            }),
            node_roles: Default::default(),
        });
    }

    let legacy_buses = LegacyBusMap {
        can: can_ids,
        flexray: fr_ids,
        ethernet: eth_ids,
    };
    let mut project = ProjectV2::new(clusters);
    project.next_cluster_id = next_id;
    project.rest = rest_from_v1(config);
    project.legacy_bus_map = Some(legacy_buses.clone());
    (
        project,
        MigrationReport {
            issues,
            legacy_buses,
        },
    )
}

fn can_cluster_cfg(id: u32, channel: &ChannelCfg, arxml: &[CanArxmlFile]) -> ClusterCfg {
    let mut sources = Vec::new();
    if !channel.dbc_path.trim().is_empty() {
        sources.push(ClusterSource::Dbc {
            path: channel.dbc_path.clone(),
        });
    }
    for path in &channel.dbc_paths_extra {
        if !path.trim().is_empty() {
            sources.push(ClusterSource::Dbc { path: path.clone() });
        }
    }
    for file in arxml {
        if file.bus as usize + 1 == id as usize {
            sources.push(ClusterSource::Arxml {
                path: file.path.clone(),
                cluster: file.cluster.clone(),
            });
        }
    }
    ClusterCfg {
        id,
        kind: ClusterKind::Can,
        name: channel.name.clone(),
        aliases: Vec::new(),
        sources,
        can: Some(CanCfg {
            bitrate_kbps: channel.bitrate_kbps,
            fd_data_kbps: channel.fd_data_kbps,
        }),
        flexray: None,
        ether: None,
        node_roles: channel.node_roles.clone(),
    }
}

fn source_for_path(path: &str, cluster: Option<&String>) -> ClusterSource {
    let lower = path.to_ascii_lowercase();
    if lower.ends_with(".arxml") {
        ClusterSource::Arxml {
            path: path.to_string(),
            cluster: cluster.cloned().unwrap_or_default(),
        }
    } else {
        ClusterSource::Fibex {
            path: path.to_string(),
        }
    }
}

fn rest_from_v1(config: &Config) -> serde_json::Map<String, Value> {
    let mut value = serde_json::to_value(config).unwrap_or(Value::Null);
    let Some(object) = value.as_object_mut() else {
        return serde_json::Map::new();
    };
    for field in [
        "channels",
        "bus_counter",
        "fr_buses",
        "fr_names",
        "eth_buses",
        "can_arxml",
        "eth_frame_length_includes_header",
        // Now written as typed v2 sections with cluster ids.
        "triggers",
        "generators",
    ] {
        object.remove(field);
    }
    object.clone()
}
