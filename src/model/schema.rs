//! Project schema v2.
//!
//! The imported communication model is not persisted: sources, overrides and
//! id aliases are enough to rebuild it. Everything the workspace needs is
//! either in `clusters` or in the untouched `rest` sections, which keep the
//! window/panel shapes until the Phase 7 migration rewrites them.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

use super::{
    ClusterId, ClusterKind, ClusterSource, ElementIds, ElementKind, FrameOverride, LegacyBusMap,
};

pub const PROJECT_FORMAT: &str = "roxy-can-project";
pub const PROJECT_VERSION: u32 = 2;

fn default_format() -> String {
    PROJECT_FORMAT.to_string()
}

fn default_version() -> u32 {
    PROJECT_VERSION
}

fn one_u32() -> u32 {
    1
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CanCfg {
    #[serde(default = "default_bitrate")]
    pub bitrate_kbps: u32,
    #[serde(default = "default_fd_data")]
    pub fd_data_kbps: u32,
}

fn default_bitrate() -> u32 {
    500
}

fn default_fd_data() -> u32 {
    2000
}

impl Default for CanCfg {
    fn default() -> Self {
        Self {
            bitrate_kbps: default_bitrate(),
            fd_data_kbps: default_fd_data(),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FlexRayCfg {
    #[serde(default)]
    pub speed_kbps: u32,
    #[serde(default)]
    pub cycle_time_ms: f64,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EtherCfg {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub physical_layer: Option<String>,
    #[serde(default)]
    pub vlans: Vec<u16>,
    #[serde(default)]
    pub frame_length_includes_header: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClusterCfg {
    pub id: u32,
    pub kind: ClusterKind,
    pub name: String,
    #[serde(default)]
    pub aliases: Vec<String>,
    #[serde(default)]
    pub sources: Vec<ClusterSource>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub can: Option<CanCfg>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub flexray: Option<FlexRayCfg>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ether: Option<EtherCfg>,
    #[serde(default)]
    pub node_roles: BTreeMap<String, String>,
}

impl ClusterCfg {
    pub fn id(&self) -> ClusterId {
        ClusterId(self.id)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IdAliasCfg {
    pub kind: String,
    pub source: String,
    pub id: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FrameOverrideCfg {
    pub cluster: u32,
    pub frame: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ethertype: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dst_mac: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub src_mac: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub length_includes_header: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vlan: Option<u16>,
}

impl FrameOverrideCfg {
    /// Converts the MAC spelling accepted by ARXML (`02-00-00-00-00-01`,
    /// `02:00:...`, or compact) into six octets.
    pub fn dst_mac_bytes(&self) -> Result<Option<[u8; 6]>, String> {
        parse_mac(self.dst_mac.as_deref())
    }

    pub fn src_mac_bytes(&self) -> Result<Option<[u8; 6]>, String> {
        parse_mac(self.src_mac.as_deref())
    }

    pub fn to_model(&self) -> Result<FrameOverride, String> {
        Ok(FrameOverride {
            ethertype: self.ethertype,
            dst_mac: self.dst_mac_bytes()?,
            src_mac: self.src_mac_bytes()?,
            length_includes_header: self.length_includes_header,
            vlan: self.vlan,
        })
    }
}

fn parse_mac(text: Option<&str>) -> Result<Option<[u8; 6]>, String> {
    let Some(text) = text else {
        return Ok(None);
    };
    let compact: String = text
        .chars()
        .filter(|ch| !matches!(ch, ':' | '-' | '.' | ' '))
        .collect();
    if compact.len() != 12 || !compact.chars().all(|ch| ch.is_ascii_hexdigit()) {
        return Err(format!(
            "MAC '{text}' must be six hexadecimal octets, for example 02-00-00-00-00-01"
        ));
    }
    let mut mac = [0u8; 6];
    for (index, byte) in mac.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&compact[index * 2..index * 2 + 2], 16)
            .map_err(|_| format!("MAC '{text}' is not hexadecimal"))?;
    }
    Ok(Some(mac))
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReplayCfg {
    /// The log's protocol space (`can`, `flexray`, `ethernet`). The same
    /// channel number can appear in more than one space in one log.
    pub kind: String,
    pub file_channel: u8,
    pub cluster: u32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "lowercase",
    rename_all_fields = "camelCase"
)]
pub enum GeneratorCfg {
    Can {
        cluster: u32,
        id: u32,
        #[serde(default)]
        active: bool,
        #[serde(default)]
        data_text: String,
        #[serde(default)]
        data: Vec<u8>,
        #[serde(default = "default_cycle")]
        cycle_us: u64,
        #[serde(default)]
        fd: bool,
        #[serde(default)]
        brs: bool,
        #[serde(default)]
        srcs: Vec<crate::config::SrcCfg>,
    },
    FlexRay {
        cluster: u32,
        slot: u16,
        #[serde(default)]
        active: bool,
        #[serde(default)]
        data_text: String,
        #[serde(default = "default_cycle")]
        cycle_us: u64,
        #[serde(default)]
        srcs: Vec<crate::config::SrcCfg>,
    },
}

fn default_cycle() -> u64 {
    100_000
}

/// One trigger rule with its buses named by cluster id. `kind` keeps the v1
/// numbering (0 signal crossing … 7 derived) so the condition decoder in
/// `Config::apply` stays the single interpreter of these rules.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TriggerCfgV2 {
    pub kind: u8,
    #[serde(default)]
    pub cluster: u32,
    #[serde(default)]
    pub id: u32,
    #[serde(default)]
    pub signal: String,
    #[serde(default)]
    pub threshold: f64,
    #[serde(default)]
    pub rising: bool,
    #[serde(default)]
    pub extended: bool,
    pub action: u8,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub action_cluster: Option<u32>,
    #[serde(default)]
    pub action_id: u32,
    #[serde(default)]
    pub enabled: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectV2 {
    #[serde(default = "default_format")]
    pub format: String,
    #[serde(default = "default_version")]
    pub version: u32,
    #[serde(default = "one_u32")]
    pub next_cluster_id: u32,
    #[serde(default)]
    pub clusters: Vec<ClusterCfg>,
    #[serde(default)]
    pub id_aliases: Vec<IdAliasCfg>,
    #[serde(default)]
    pub frame_overrides: Vec<FrameOverrideCfg>,
    #[serde(default)]
    pub replay: Vec<ReplayCfg>,
    /// `None` means an older v2 file that predates this section; `Some(empty)`
    /// is a project that deliberately has no generator entries.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub generators: Option<Vec<GeneratorCfg>>,
    /// Same rule for trigger rows: `None` predates the section, `Some(empty)`
    /// is a project with no rules.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub triggers: Option<Vec<TriggerCfgV2>>,
    /// Old bus index → cluster id, kept so the frontend can rewrite its
    /// opaque `web` workspace during the v1→v2 migration.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub legacy_bus_map: Option<LegacyBusMap>,
    /// Workspace sections the Phase 7 migration rewrites: windows, triggers,
    /// generators, `web`, derived definitions. Kept verbatim until then.
    #[serde(flatten)]
    pub rest: serde_json::Map<String, Value>,
}

impl Default for ProjectV2 {
    fn default() -> Self {
        Self {
            format: PROJECT_FORMAT.to_string(),
            version: PROJECT_VERSION,
            next_cluster_id: 1,
            clusters: Vec::new(),
            id_aliases: Vec::new(),
            frame_overrides: Vec::new(),
            replay: Vec::new(),
            generators: None,
            triggers: None,
            legacy_bus_map: None,
            rest: serde_json::Map::new(),
        }
    }
}

impl ProjectV2 {
    pub fn new(clusters: Vec<ClusterCfg>) -> Self {
        let next = clusters
            .iter()
            .map(|c| c.id)
            .max()
            .map_or(1, |id| id.saturating_add(1));
        Self {
            next_cluster_id: next,
            clusters,
            ..Self::default()
        }
    }

    pub fn is_v2(value: &Value) -> bool {
        value
            .get("version")
            .and_then(Value::as_u64)
            .is_some_and(|v| v == u64::from(PROJECT_VERSION))
            || value
                .get("format")
                .and_then(Value::as_str)
                .is_some_and(|f| f == PROJECT_FORMAT)
    }

    pub fn cluster(&self, id: ClusterId) -> Option<&ClusterCfg> {
        self.clusters.iter().find(|c| c.id == id.0)
    }

    /// Rebuilds the persisted element-id aliases. Unknown kinds are an error:
    /// silently dropping them would reassign ids on the next import.
    pub fn element_ids(&self) -> Result<ElementIds, String> {
        let mut entries = Vec::with_capacity(self.id_aliases.len());
        for alias in &self.id_aliases {
            let kind = ElementKind::from_str(&alias.kind).ok_or_else(|| {
                format!(
                    "id alias for '{}' has unknown element kind '{}'",
                    alias.source, alias.kind
                )
            })?;
            if alias.id == 0 {
                return Err(format!("id alias for '{}' has id 0", alias.source));
            }
            entries.push((kind, alias.source.clone(), alias.id));
        }
        Ok(ElementIds::from_aliases(entries))
    }

    pub fn aliases_from_ids(&mut self, ids: &ElementIds) {
        self.id_aliases = ids
            .aliases()
            .map(|(kind, source, id)| IdAliasCfg {
                kind: kind.as_str().to_string(),
                source: source.to_string(),
                id,
            })
            .collect();
    }

    /// Returns every structural problem rather than the first, so the open
    /// report can be complete.
    pub fn validate(&self) -> Vec<String> {
        let mut problems = Vec::new();
        if self.format != PROJECT_FORMAT {
            problems.push(format!(
                "project format '{}' is not '{PROJECT_FORMAT}'",
                self.format
            ));
        }
        if self.version != PROJECT_VERSION {
            problems.push(format!(
                "project version {} is not {PROJECT_VERSION}",
                self.version
            ));
        }
        if self.next_cluster_id == 0 {
            problems.push("nextClusterId must be at least 1".to_string());
        }

        let mut ids = std::collections::BTreeSet::new();
        let mut names = std::collections::BTreeSet::new();
        let mut max_id = 0u32;
        for cluster in &self.clusters {
            if cluster.id == 0 {
                problems.push("cluster id 0 is reserved".to_string());
            }
            if !ids.insert(cluster.id) {
                problems.push(format!("duplicate cluster id {}", cluster.id));
            }
            max_id = max_id.max(cluster.id);
            let name = cluster.name.trim();
            if name.is_empty() {
                problems.push(format!("cluster {} has an empty name", cluster.id));
            } else if !names.insert(name.to_ascii_lowercase()) {
                problems.push(format!("duplicate cluster name '{name}'"));
            }
            match cluster.kind {
                ClusterKind::Can if cluster.ether.is_some() || cluster.flexray.is_some() => {
                    problems.push(format!(
                        "CAN cluster '{}' carries another protocol's parameters",
                        cluster.name
                    ));
                }
                ClusterKind::FlexRay if cluster.can.is_some() || cluster.ether.is_some() => {
                    problems.push(format!(
                        "FlexRay cluster '{}' carries another protocol's parameters",
                        cluster.name
                    ));
                }
                ClusterKind::Ethernet if cluster.can.is_some() || cluster.flexray.is_some() => {
                    problems.push(format!(
                        "Ethernet cluster '{}' carries another protocol's parameters",
                        cluster.name
                    ));
                }
                _ => {}
            }
            if let Some(ClusterSource::Arxml { path, .. }) = cluster
                .sources
                .iter()
                .find(|s| matches!(s, ClusterSource::Arxml { .. }))
                && path.trim().is_empty()
            {
                problems.push(format!(
                    "cluster '{}' has an empty ARXML path",
                    cluster.name
                ));
            }
        }
        if self.next_cluster_id <= max_id {
            problems.push(format!(
                "nextClusterId {} must be above the highest cluster id {max_id}",
                self.next_cluster_id
            ));
        }

        for alias in &self.id_aliases {
            if ElementKind::from_str(&alias.kind).is_none() {
                problems.push(format!(
                    "id alias '{}' has unknown kind '{}'",
                    alias.source, alias.kind
                ));
            }
            if alias.id == 0 {
                problems.push(format!("id alias '{}' has id 0", alias.source));
            }
        }

        for over in &self.frame_overrides {
            if !ids.contains(&over.cluster) {
                problems.push(format!(
                    "frame override '{}' refers to missing cluster {}",
                    over.frame, over.cluster
                ));
            }
            if over.frame.trim().is_empty() {
                problems.push("frame override has an empty frame name".to_string());
            }
            if let Err(error) = over.to_model() {
                problems.push(error);
            }
        }

        let mut replay_keys = std::collections::BTreeSet::new();
        for replay in &self.replay {
            let Some(cluster) = self.cluster(ClusterId(replay.cluster)) else {
                problems.push(format!(
                    "replay channel {} refers to missing cluster {}",
                    replay.file_channel, replay.cluster
                ));
                continue;
            };
            let expected = match cluster.kind {
                ClusterKind::Can => "can",
                ClusterKind::FlexRay => "flexray",
                ClusterKind::Ethernet => "ethernet",
            };
            if !replay.kind.eq_ignore_ascii_case(expected) {
                problems.push(format!(
                    "replay channel {} is '{}' but cluster '{}' is {expected}",
                    replay.file_channel, replay.kind, cluster.name
                ));
            }
            if !replay_keys.insert((replay.kind.to_ascii_lowercase(), replay.file_channel)) {
                problems.push(format!(
                    "{} channel {} is mapped more than once",
                    replay.kind, replay.file_channel
                ));
            }
        }

        for key in ["channels", "fr_buses", "fr_names", "eth_buses", "can_arxml"] {
            if self.rest.contains_key(key) {
                problems.push(format!(
                    "v1 field '{key}' must be migrated before saving schema v2"
                ));
            }
        }

        problems
    }
}

impl ClusterCfg {
    pub fn from_model(
        cluster: &super::Cluster,
        networks: &[&super::Network],
        node_roles: BTreeMap<String, String>,
    ) -> Self {
        let can = networks.iter().find_map(|n| match &n.params {
            super::NetworkParams::Can(p) => Some(CanCfg {
                bitrate_kbps: p.bitrate_kbps,
                fd_data_kbps: p.fd_data_kbps,
            }),
            _ => None,
        });
        let flexray = networks.iter().find_map(|n| match &n.params {
            super::NetworkParams::FlexRay(p) => Some(FlexRayCfg {
                speed_kbps: p.speed_kbps,
                cycle_time_ms: p.cycle_time_ms,
            }),
            _ => None,
        });
        let ether = networks.iter().find_map(|n| match &n.params {
            super::NetworkParams::Ethernet(p) => Some(EtherCfg {
                physical_layer: p.physical_layer.map(|p| p.xml_name().to_string()),
                vlans: p.vlans.clone(),
                frame_length_includes_header: p.frame_length_includes_header,
            }),
            _ => None,
        });
        Self {
            id: cluster.id.0,
            kind: cluster.kind,
            name: cluster.name.clone(),
            aliases: cluster.aliases.clone(),
            sources: cluster.sources.clone(),
            can,
            flexray,
            ether,
            node_roles,
        }
    }
}
