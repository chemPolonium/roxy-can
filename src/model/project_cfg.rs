//! Builds the schema-v2 project sections from the live workspace config.
//! The editor and the core hand over the same `Config`; this module only
//! reshapes it into `clusters` and the compatibility blob that `open` feeds
//! back into the same apply path.

use super::*;
use crate::config::{CanArxmlFile, ChannelCfg, Config, EthBusFile, FrBusFile};

/// Reshapes the bus lists into the single `clusters` list. Ids are assigned
/// in order here; the caller replaces them with the live cluster ids.
pub fn project_from_config(config: &Config) -> ProjectV2 {
    let mut clusters = Vec::new();
    let mut next_id = 1u32;

    for channel in &config.channels {
        clusters.push(can_cluster_cfg(next_id, channel, &config.can_arxml));
        next_id += 1;
    }

    let mut fr_buses: Vec<&FrBusFile> = config.fr_buses.iter().collect();
    fr_buses.sort_by_key(|bus| bus.bus);
    for bus in fr_buses {
        let alias = config
            .fr_names
            .iter()
            .find(|(index, _)| *index == bus.bus)
            .map(|(_, name)| name.clone())
            .unwrap_or_else(|| format!("FR{}", bus.bus));
        clusters.push(ClusterCfg {
            id: next_id,
            kind: ClusterKind::FlexRay,
            name: alias,
            aliases: Vec::new(),
            sources: vec![source_for_path(&bus.path, Some(&bus.cluster))],
            can: None,
            flexray: Some(FlexRayCfg::default()),
            ether: None,
            node_roles: Default::default(),
        });
        next_id += 1;
    }

    let mut eth_buses: Vec<&EthBusFile> = config.eth_buses.iter().collect();
    eth_buses.sort_by_key(|bus| bus.bus);
    for bus in eth_buses {
        clusters.push(ClusterCfg {
            id: next_id,
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
        next_id += 1;
    }

    let mut project = ProjectV2::new(clusters);
    project.next_cluster_id = next_id;
    project
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
