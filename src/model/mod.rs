//! The unified AUTOSAR communication model.
//!
//! Cluster → Network → Frame → PDU → Signal is the single shape every
//! importer feeds, and the only shape the runtime will read after the
//! switch-over. DBC and FIBEX are adapters into this model; ARXML is the
//! canonical source.

mod arxml;
mod dbc_import;
mod id;
mod import;
mod key;
mod plan;
mod project_cfg;
mod runtime;
mod schema;

#[cfg(test)]
mod arxml_tests;
#[cfg(test)]
mod dbc_tests;
#[cfg(test)]
mod import_tests;
#[cfg(test)]
mod plan_tests;
#[cfg(test)]
mod runtime_tests;
#[cfg(test)]
mod tests;

// Phase 2 importers and the Phase 4 runtime consume this surface; until then
// the public re-exports are intentionally ahead of their callers.
#[allow(unused_imports)]
pub use arxml::{ScannedCluster, import_cluster, scan_clusters};
#[allow(unused_imports)]
pub use dbc_import::{
    empty_can_comm, import_dbc, import_symbol_table, import_symbol_table_sources,
};
#[allow(unused_imports)]
pub use id::{ClusterIds, ElementIds, ElementKind};
#[allow(unused_imports)]
pub use import::{import_eth_db, import_eth_db_obj, import_fibex, import_fr_db};
#[allow(unused_imports)]
pub use key::BusMap;
#[allow(unused_imports)]
pub use plan::{PlanCache, build_decode_plan};
#[allow(unused_imports)]
pub use project_cfg::project_from_config;
#[allow(unused_imports)]
pub use runtime::{DecodedSignal, E2eReading, decode_e2e, decode_payload, encode_payload};
#[allow(unused_imports)]
pub use schema::{
    CanCfg, ClusterCfg, EtherCfg, FlexRayCfg, FrameOverrideCfg, GeneratorCfg, IdAliasCfg,
    PROJECT_FORMAT, PROJECT_VERSION, ProjectV2, ReplayCfg, TriggerCfgV2,
};

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

macro_rules! model_id {
    ($name:ident, $raw:ty) => {
        #[derive(
            Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
        )]
        #[serde(transparent)]
        pub struct $name(pub $raw);

        impl $name {
            pub fn get(self) -> $raw {
                self.0
            }

            pub fn text(self) -> String {
                self.0.to_string()
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                write!(f, "{}", self.0)
            }
        }
    };
}

model_id!(ClusterId, u32);
model_id!(NetworkId, u32);
model_id!(EcuId, u32);
model_id!(ConnectorId, u32);
model_id!(FrameId, u64);
model_id!(PduId, u64);
model_id!(SignalId, u64);
model_id!(MappingId, u64);
model_id!(SystemSignalId, u32);
model_id!(SignalGroupId, u32);
model_id!(TransformationId, u32);
model_id!(E2eProtectionId, u32);
model_id!(BaseTypeId, u32);
model_id!(CompuMethodId, u32);

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ClusterKind {
    Can,
    FlexRay,
    Ethernet,
}

impl ClusterKind {
    pub fn label(self) -> &'static str {
        match self {
            ClusterKind::Can => "CAN",
            ClusterKind::FlexRay => "FlexRay",
            ClusterKind::Ethernet => "Ethernet",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Cluster {
    pub id: ClusterId,
    pub kind: ClusterKind,
    pub name: String,
    #[serde(default)]
    pub aliases: Vec<String>,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub sources: Vec<ClusterSource>,
    #[serde(default)]
    pub networks: Vec<NetworkId>,
}

impl Cluster {
    pub fn matches_name(&self, name: &str) -> bool {
        self.name.eq_ignore_ascii_case(name)
            || self.aliases.iter().any(|a| a.eq_ignore_ascii_case(name))
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Network {
    pub id: NetworkId,
    pub cluster: ClusterId,
    pub name: String,
    pub params: NetworkParams,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "protocol", rename_all = "lowercase")]
pub enum NetworkParams {
    Can(CanParams),
    FlexRay(FlexRayParams),
    Ethernet(EthernetParams),
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct CanParams {
    pub bitrate_kbps: u32,
    pub fd_data_kbps: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sample_point_permille: Option<u16>,
}

impl CanParams {
    pub const DEFAULT_BITRATE_KBPS: u32 = 500;
    pub const DEFAULT_FD_DATA_KBPS: u32 = 2000;

    pub fn supports_fd(&self) -> bool {
        self.fd_data_kbps > 0
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FlexRayParams {
    pub speed_kbps: u32,
    pub cycle_time_ms: f64,
    #[serde(default)]
    pub macrotick_us: f64,
    #[serde(default)]
    pub coldstart_attempts: u32,
    #[serde(default)]
    pub action_point_offset: u32,
    #[serde(default)]
    pub minislot_action_point_offset: u32,
    #[serde(default)]
    pub dynamic_slot_idle_phase: u32,
    #[serde(default)]
    pub minor_version: u32,
    #[serde(default)]
    pub extra: BTreeMap<String, String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum EthernetPhy {
    #[serde(rename = "10BASE-T1S")]
    Base10T1s,
    #[serde(rename = "100BASE-T1")]
    Base100T1,
    #[serde(rename = "1000BASE-T1")]
    Base1000T1,
    #[serde(rename = "100BASE-TX")]
    Base100Tx,
    #[serde(rename = "1000BASE-T")]
    Base1000T,
    #[serde(rename = "IEEE802.11P")]
    Ieee80211p,
}

impl EthernetPhy {
    pub fn from_xml(text: &str) -> Option<Self> {
        match text.trim() {
            "10BASE-T1S" => Some(Self::Base10T1s),
            "100BASE-T1" => Some(Self::Base100T1),
            "1000BASE-T1" => Some(Self::Base1000T1),
            "100BASE-TX" => Some(Self::Base100Tx),
            "1000BASE-T" => Some(Self::Base1000T),
            "IEEE802.11P" => Some(Self::Ieee80211p),
            _ => None,
        }
    }

    pub fn xml_name(self) -> &'static str {
        match self {
            Self::Base10T1s => "10BASE-T1S",
            Self::Base100T1 => "100BASE-T1",
            Self::Base1000T1 => "1000BASE-T1",
            Self::Base100Tx => "100BASE-TX",
            Self::Base1000T => "1000BASE-T",
            Self::Ieee80211p => "IEEE802.11P",
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct EthernetParams {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub physical_layer: Option<EthernetPhy>,
    #[serde(default)]
    pub vlans: Vec<u16>,
    #[serde(default)]
    pub frame_length_includes_header: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Ecu {
    pub id: EcuId,
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mac: Option<[u8; 6]>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ConnectorKind {
    Can,
    FlexRay,
    Ethernet,
    Lin,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Connector {
    pub id: ConnectorId,
    pub ecu: EcuId,
    pub network: NetworkId,
    pub kind: ConnectorKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_vlan: Option<u16>,
    #[serde(default)]
    pub vlan_memberships: Vec<u16>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum ByteOrder {
    #[default]
    LittleEndian,
    BigEndian,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Frame {
    pub id: FrameId,
    pub cluster: ClusterId,
    pub network: NetworkId,
    pub name: String,
    pub length: u32,
    pub address: FrameAddress,
    pub timing: FrameTiming,
    /// The document's frame comment; the DBC adapter copies `CM_ BO_`.
    #[serde(default)]
    pub comment: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ethernet: Option<EthernetFrameMeta>,
    #[serde(default)]
    pub pdu_mappings: Vec<MappingId>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum FrameAddress {
    Can {
        id: u32,
        extended: bool,
        fd: bool,
    },
    FlexRay {
        slot: u16,
        base_cycle: u8,
        repetition: u8,
        payload_preamble: bool,
    },
    Ethernet {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        vlan: Option<u16>,
    },
}

impl FrameAddress {
    pub fn can_id(self) -> Option<u32> {
        match self {
            FrameAddress::Can { id, .. } => Some(id),
            _ => None,
        }
    }

    pub fn slot(self) -> Option<u16> {
        match self {
            FrameAddress::FlexRay { slot, .. } => Some(slot),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FrameTiming {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cycle_us: Option<u64>,
    #[serde(default)]
    pub startup: bool,
    #[serde(default)]
    pub event: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "variant", rename_all = "camelCase")]
pub enum EthernetFrameMeta {
    Base,
    Generic,
    UserDefined,
    Ieee1722 {
        stream_identifier: u64,
        sub_type: u64,
        version: u64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        presentation_time_s: Option<f64>,
    },
}

/// Project-level facts AUTOSAR does not carry (EtherType, L2 addresses) plus
/// overrides for conventions that differ between OEM files.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FrameOverride {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ethertype: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dst_mac: Option<[u8; 6]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub src_mac: Option<[u8; 6]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub length_includes_header: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vlan: Option<u16>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PduKind {
    #[default]
    Signal,
    Container,
    Multiplexed,
    Secured,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ContainerHeaderType {
    #[default]
    Short,
    Long,
    None,
}

impl ContainerHeaderType {
    pub fn header_size(self) -> u32 {
        match self {
            ContainerHeaderType::Short => 4,
            ContainerHeaderType::Long => 8,
            ContainerHeaderType::None => 0,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SecuredPduHeader {
    #[default]
    NoHeader,
    Header08,
    Header16,
    Header32,
}

impl SecuredPduHeader {
    pub fn header_size(self) -> u32 {
        match self {
            SecuredPduHeader::NoHeader => 0,
            SecuredPduHeader::Header08 => 1,
            SecuredPduHeader::Header16 => 2,
            SecuredPduHeader::Header32 => 4,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SecVerification {
    /// Only the structure is understood; no key material is available.
    #[default]
    Unverified,
    /// Layout is internally consistent; authentication still not checked.
    Structural,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SelectorField {
    pub start_bit: u32,
    pub length: u32,
    #[serde(default)]
    pub byte_order: ByteOrder,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signal: Option<SignalId>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SecuredPduMeta {
    pub header: SecuredPduHeader,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub payload: Option<PduId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data_id: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub freshness_value_id: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub secured_area: Option<(u32, u32)>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth_info_length: Option<u32>,
    #[serde(default)]
    pub verification: SecVerification,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Pdu {
    pub id: PduId,
    pub name: String,
    pub length: u32,
    #[serde(default)]
    pub kind: PduKind,
    #[serde(default)]
    pub header_type: ContainerHeaderType,
    /// Declared cyclic transmission period, when the matrix states one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cycle_us: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selector: Option<SelectorField>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub secured: Option<SecuredPduMeta>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transformation: Option<TransformationId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub e2e: Option<E2eProtectionId>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Signal {
    pub id: SignalId,
    pub pdu: PduId,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub system_signal: Option<SystemSignalId>,
    pub length: u32,
    pub start_bit: u32,
    #[serde(default)]
    pub byte_order: ByteOrder,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub update_bit: Option<u32>,
    #[serde(default)]
    pub signed: bool,
    #[serde(default)]
    pub float: bool,
    #[serde(default = "one_f64")]
    pub factor: f64,
    #[serde(default)]
    pub offset: f64,
    #[serde(default)]
    pub unit: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max: Option<f64>,
    #[serde(default)]
    pub min_open: bool,
    #[serde(default)]
    pub max_open: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub initial_raw: Option<f64>,
    #[serde(default)]
    pub values: Vec<(i64, String)>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_type: Option<BaseTypeId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compu: Option<CompuMethodId>,
    #[serde(default)]
    pub comment: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transformation: Option<TransformationId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group: Option<SignalGroupId>,
    /// All conditions must hold for the signal to be present. A DBC message
    /// can have several switches; an ARXML multiplexed part contributes one.
    #[serde(default)]
    pub mux: Vec<MuxCondition>,
}

fn one_f64() -> f64 {
    1.0
}

impl Signal {
    pub fn physical(&self, raw: f64) -> f64 {
        raw * self.factor + self.offset
    }

    pub fn raw(&self, physical: f64) -> f64 {
        if self.factor == 0.0 {
            physical - self.offset
        } else {
            (physical - self.offset) / self.factor
        }
    }

    pub fn initial_physical(&self) -> Option<f64> {
        self.initial_raw.map(|raw| self.physical(raw))
    }

    pub fn in_range(&self, value: f64) -> bool {
        let above = self.min.is_none_or(|min| {
            if self.min_open {
                value > min
            } else {
                value >= min
            }
        });
        let below = self.max.is_none_or(|max| {
            if self.max_open {
                value < max
            } else {
                value <= max
            }
        });
        above && below
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SystemSignal {
    pub id: SystemSignalId,
    pub name: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SignalGroup {
    pub id: SignalGroupId,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sender: Option<EcuId>,
    /// E2E protection declared on the group itself (AUTOSAR 4.2
    /// `TRANSFORMATION-I-SIGNAL-PROPSS` → `TRANSFORMER-REF`). The profile and
    /// data ids are known; the counter/CRC member positions are not stated, so
    /// the decode plan does not derive frame-level counter continuity from it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub e2e: Option<GroupE2eProtection>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GroupE2eProtection {
    /// `PROFILE-NAME` as written (`Profile2`, `Profile5`, ...).
    pub profile_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile: Option<u32>,
    #[serde(default)]
    pub data_ids: Vec<u32>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BaseType {
    pub id: BaseTypeId,
    pub name: String,
    pub size: u32,
    pub encoding: String,
}

impl BaseType {
    pub fn signed(&self) -> bool {
        self.encoding == "2C"
    }

    pub fn float(&self) -> bool {
        matches!(self.encoding.as_str(), "IEEE754" | "IEE754")
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CompuCategory {
    Identical,
    Linear,
    TextTable,
    Unknown,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CompuMethod {
    pub id: CompuMethodId,
    pub name: String,
    pub category: CompuCategory,
    #[serde(default = "one_f64")]
    pub factor: f64,
    #[serde(default)]
    pub offset: f64,
    #[serde(default)]
    pub values: Vec<(i64, String)>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unit: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max: Option<f64>,
    #[serde(default)]
    pub min_open: bool,
    #[serde(default)]
    pub max_open: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FramePduMapping {
    pub id: MappingId,
    pub frame: FrameId,
    pub pdu: PduId,
    #[serde(default)]
    pub start_bit: u32,
    #[serde(default)]
    pub byte_order: ByteOrder,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SignalPduMapping {
    pub id: MappingId,
    pub signal: SignalId,
    pub pdu: PduId,
    #[serde(default)]
    pub start_bit: u32,
    #[serde(default)]
    pub byte_order: ByteOrder,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub update_bit: Option<u32>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContainedPduMapping {
    pub id: MappingId,
    pub container: PduId,
    pub pdu: PduId,
    #[serde(default)]
    pub header_id: u32,
    #[serde(default)]
    pub offset: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MultiplexedPduPart {
    pub id: MappingId,
    pub mux: PduId,
    pub part: PduId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selector_code: Option<u32>,
    #[serde(default)]
    pub initial: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RelationRole {
    Sender,
    Receiver,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FrameEcuRelation {
    pub frame: FrameId,
    pub ecu: EcuId,
    pub role: RelationRole,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PduEcuRelation {
    pub pdu: PduId,
    pub ecu: EcuId,
    pub role: RelationRole,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SignalEcuRelation {
    pub signal: SignalId,
    pub ecu: EcuId,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Transformation {
    pub id: TransformationId,
    pub name: String,
    pub kind: TransformationKind,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum TransformationKind {
    Identity,
    Linear { scale: f64, offset: f64 },
    TextTable { values: Vec<(i64, String)> },
    Unknown { category: String },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct E2eProtection {
    pub id: E2eProtectionId,
    pub name: String,
    #[serde(default)]
    pub profile_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data_id: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub system_signal: Option<SystemSignalId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub payload: Option<PduId>,
    /// Counter position when the document states it (bit offset inside the
    /// PDU and bit width). Profiles 1/2 fall back to their fixed layout.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub counter_offset: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub counter_length: Option<u32>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum IssueSeverity {
    Info,
    Warning,
    Error,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum IssueKind {
    DanglingRef,
    DuplicateShortName,
    Unsupported,
    AmbiguousFrame,
    SignalLayoutConflict,
    UnsupportedTransformation,
    UnknownCompuMethod,
    LengthConvention,
    Other,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Issue {
    pub severity: IssueSeverity,
    pub kind: IssueKind,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub location: Option<String>,
}

impl Issue {
    pub fn warning(kind: IssueKind, message: impl Into<String>) -> Self {
        Self {
            severity: IssueSeverity::Warning,
            kind,
            message: message.into(),
            location: None,
        }
    }

    pub fn error(kind: IssueKind, message: impl Into<String>) -> Self {
        Self {
            severity: IssueSeverity::Error,
            kind,
            message: message.into(),
            location: None,
        }
    }

    pub fn at(mut self, location: impl Into<String>) -> Self {
        self.location = Some(location.into());
        self
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum FrameMatchKey {
    Can { id: u32, extended: bool },
    FlexRay { slot: u16, cycle: u8 },
    Ethernet { vlan: Option<u16>, flow: EthFlowKey },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EthFlowKey {
    pub vlan: Option<u16>,
    pub dst: [u8; 6],
    pub src: [u8; 6],
    pub ethertype: u16,
    pub payload_len: usize,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FrameRoute {
    pub cluster: ClusterId,
    pub key: FrameMatchKey,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum MuxSelector {
    /// A DBC switch signal decoded like any other signal.
    Signal { signal: SignalId },
    /// An ARXML `SELECTOR-FIELD`: bits inside the mux PDU, with no I-Signal
    /// to name them.
    Field {
        start_bit: u32,
        length: u32,
        #[serde(default)]
        byte_order: ByteOrder,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MuxCondition {
    pub selector: MuxSelector,
    /// Inclusive raw ranges of the selector that make the signal present.
    pub ranges: Vec<(u32, u32)>,
}

/// The container header that decides whether a contained PDU is present.
/// `header_id` 0 means the document did not assign one, so the child is read
/// wherever it sits (the pre-header behaviour).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContainerGate {
    pub header_start_bit: u32,
    pub header_type: ContainerHeaderType,
    pub header_id: u32,
}

/// A linear signal transformation: `app = scale * net + offset` on decode,
/// inverted on encode. Identity needs no entry.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LinearTransform {
    pub scale: f64,
    pub offset: f64,
}

impl LinearTransform {
    pub fn decode(self, value: f64) -> f64 {
        self.scale * value + self.offset
    }

    pub fn encode(self, value: f64) -> f64 {
        if self.scale == 0.0 {
            value
        } else {
            (value - self.offset) / self.scale
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SignalRead {
    pub signal: SignalId,
    pub bit_offset: u32,
    pub bit_len: u32,
    pub byte_order: ByteOrder,
    pub signed: bool,
    pub float: bool,
    pub factor: f64,
    pub offset: f64,
    #[serde(default)]
    pub mux: Vec<MuxCondition>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub container: Option<ContainerGate>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transform: Option<LinearTransform>,
    /// Absolute PDU bit position of this signal's update indication, if the
    /// document declares one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub update_bit: Option<u32>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SignalWrite {
    pub signal: SignalId,
    pub bit_offset: u32,
    pub bit_len: u32,
    pub byte_order: ByteOrder,
    pub signed: bool,
    pub float: bool,
    pub factor: f64,
    pub offset: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub container: Option<ContainerGate>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transform: Option<LinearTransform>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub update_bit: Option<u32>,
}

/// What a reader can check about an E2E-protected PDU without a CRC key:
/// the counter's fixed position for the profiles whose header layout is
/// fixed, plus the configured Data ID. Profiles with a variable layout stay
/// structure-only.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct E2ePlan {
    pub profile: u32,
    pub counter_bit_offset: u32,
    pub counter_bits: u32,
    pub data_id: Option<u32>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct DecodePlan {
    pub frame: Option<FrameId>,
    pub version: u32,
    pub reads: Vec<SignalRead>,
    pub writes: Vec<SignalWrite>,
    #[serde(default)]
    pub e2e: Vec<E2ePlan>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Comm {
    pub clusters: BTreeMap<ClusterId, Cluster>,
    pub networks: BTreeMap<NetworkId, Network>,
    pub ecus: BTreeMap<EcuId, Ecu>,
    pub connectors: BTreeMap<ConnectorId, Connector>,
    pub frames: BTreeMap<FrameId, Frame>,
    pub pdus: BTreeMap<PduId, Pdu>,
    pub signals: BTreeMap<SignalId, Signal>,
    pub system_signals: BTreeMap<SystemSignalId, SystemSignal>,
    pub groups: BTreeMap<SignalGroupId, SignalGroup>,
    pub base_types: BTreeMap<BaseTypeId, BaseType>,
    pub compu_methods: BTreeMap<CompuMethodId, CompuMethod>,
    pub transformations: BTreeMap<TransformationId, Transformation>,
    pub e2e: BTreeMap<E2eProtectionId, E2eProtection>,
    pub frame_pdu_mappings: BTreeMap<MappingId, FramePduMapping>,
    pub signal_pdu_mappings: BTreeMap<MappingId, SignalPduMapping>,
    pub contained_pdu_mappings: BTreeMap<MappingId, ContainedPduMapping>,
    pub multiplexed_parts: BTreeMap<MappingId, MultiplexedPduPart>,
    pub frame_relations: Vec<FrameEcuRelation>,
    pub pdu_relations: Vec<PduEcuRelation>,
    pub signal_relations: Vec<SignalEcuRelation>,
    pub overrides: BTreeMap<FrameId, FrameOverride>,
    pub issues: Vec<Issue>,
}

impl Comm {
    /// Points a one-cluster import at the id the bus core assigned to the
    /// channel. The ARXML reader numbers its cluster before the core has
    /// handed out a global id, so the receiver rewrites the references once,
    /// on the way in. A model with more than one cluster is left alone: that
    /// is not the shape `import_cluster` produces, and guessing which cluster
    /// belongs to the channel would be worse than decoding without it.
    pub fn rebind_cluster(&mut self, to: ClusterId) {
        if self.clusters.len() != 1 {
            return;
        }
        let Some(old) = self.clusters.keys().next().copied() else {
            return;
        };
        if old == to {
            return;
        }
        if let Some(mut cluster) = self.clusters.remove(&old) {
            cluster.id = to;
            self.clusters.insert(to, cluster);
        }
        let old_network = NetworkId(old.0);
        let new_network = NetworkId(to.0);
        if let Some(mut network) = self.networks.remove(&old_network) {
            network.id = new_network;
            network.cluster = to;
            self.networks.insert(new_network, network);
        }
        for network in self.networks.values_mut() {
            if network.cluster == old {
                network.cluster = to;
            }
        }
        for frame in self.frames.values_mut() {
            if frame.cluster == old {
                frame.cluster = to;
            }
            if frame.network == old_network {
                frame.network = new_network;
            }
        }
    }

    /// Merges a second description of the same physical cluster into this
    /// model. Both sides must already name `cluster`; ids stay unique because
    /// each source was imported under its own temporary cluster id before the
    /// rebind. Duplicate declarations keep the first model's entry.
    pub fn merge_from(&mut self, mut other: Comm, cluster: ClusterId) {
        if let Some(extra) = other.clusters.remove(&cluster) {
            match self.clusters.get_mut(&cluster) {
                Some(base) => {
                    for source in extra.sources {
                        if !base.sources.contains(&source) {
                            base.sources.push(source);
                        }
                    }
                    for alias in extra.aliases {
                        if !base.aliases.contains(&alias) {
                            base.aliases.push(alias);
                        }
                    }
                    if base.description.is_empty() {
                        base.description = extra.description;
                    }
                }
                None => {
                    self.clusters.insert(cluster, extra);
                }
            }
        }
        for (id, network) in other.networks {
            self.networks.entry(id).or_insert(network);
        }
        self.ecus.extend(other.ecus);
        self.connectors.extend(other.connectors);
        self.frames.extend(other.frames);
        self.pdus.extend(other.pdus);
        self.signals.extend(other.signals);
        self.system_signals.extend(other.system_signals);
        self.groups.extend(other.groups);
        self.base_types.extend(other.base_types);
        self.compu_methods.extend(other.compu_methods);
        self.transformations.extend(other.transformations);
        self.e2e.extend(other.e2e);
        self.frame_pdu_mappings.extend(other.frame_pdu_mappings);
        self.signal_pdu_mappings.extend(other.signal_pdu_mappings);
        self.contained_pdu_mappings
            .extend(other.contained_pdu_mappings);
        self.multiplexed_parts.extend(other.multiplexed_parts);
        self.frame_relations.extend(other.frame_relations);
        self.pdu_relations.extend(other.pdu_relations);
        self.signal_relations.extend(other.signal_relations);
        for (frame, over) in other.overrides {
            self.overrides.entry(frame).or_insert(over);
        }
        self.issues.extend(other.issues);
    }

    pub fn cluster_by_name(&self, name: &str) -> Option<&Cluster> {
        self.clusters.values().find(|c| c.matches_name(name))
    }

    pub fn cluster_by_name_exact(&self, name: &str) -> Option<&Cluster> {
        self.clusters
            .values()
            .find(|c| c.name.eq_ignore_ascii_case(name))
    }

    pub fn frames_in_cluster(&self, cluster: ClusterId) -> impl Iterator<Item = &Frame> {
        self.frames.values().filter(move |f| f.cluster == cluster)
    }

    pub fn frame_by_name(&self, cluster: ClusterId, name: &str) -> Option<&Frame> {
        self.frames_in_cluster(cluster)
            .find(|f| f.name.eq_ignore_ascii_case(name))
    }

    /// The ECU a frame's sender port names, if the document declares one.
    pub fn frame_sender(&self, frame: &Frame) -> Option<&str> {
        self.frame_relations
            .iter()
            .find(|relation| relation.frame == frame.id && relation.role == RelationRole::Sender)
            .and_then(|relation| self.ecus.get(&relation.ecu))
            .map(|ecu| ecu.name.as_str())
    }

    /// Frames in `cluster` whose sender port names `ecu`.
    pub fn frames_sent_by(&self, cluster: ClusterId, ecu: &str) -> Vec<&Frame> {
        let senders: Vec<EcuId> = self
            .ecus
            .values()
            .filter(|candidate| candidate.name == ecu)
            .map(|candidate| candidate.id)
            .collect();
        if senders.is_empty() {
            return Vec::new();
        }
        self.frames_in_cluster(cluster)
            .filter(|frame| {
                self.frame_relations.iter().any(|relation| {
                    relation.frame == frame.id
                        && relation.role == RelationRole::Sender
                        && senders.contains(&relation.ecu)
                })
            })
            .collect()
    }

    pub fn frame_by_can_id(&self, cluster: ClusterId, id: u32) -> Vec<&Frame> {
        self.frames_in_cluster(cluster)
            .filter(|f| f.address.can_id() == Some(id))
            .collect()
    }

    pub fn frame_by_slot(&self, cluster: ClusterId, slot: u16) -> Vec<&Frame> {
        self.frames_in_cluster(cluster)
            .filter(|f| f.address.slot() == Some(slot))
            .collect()
    }

    pub fn pdus_in_frame(&self, frame: &Frame) -> Vec<&Pdu> {
        frame
            .pdu_mappings
            .iter()
            .filter_map(|id| self.frame_pdu_mappings.get(id))
            .filter_map(|mapping| self.pdus.get(&mapping.pdu))
            .collect()
    }

    pub fn signals_in_pdu(&self, pdu: PduId) -> Vec<&Signal> {
        self.signals.values().filter(|s| s.pdu == pdu).collect()
    }

    pub fn signals_in_frame(&self, frame: &Frame) -> Vec<&Signal> {
        self.pdus_in_frame(frame)
            .into_iter()
            .flat_map(|pdu| self.signals_in_pdu(pdu.id))
            .collect()
    }

    pub fn signal_by_name(&self, frame: &Frame, name: &str) -> Vec<&Signal> {
        self.signals_in_frame(frame)
            .into_iter()
            .filter(|s| s.name.eq_ignore_ascii_case(name))
            .collect()
    }

    pub fn frame_route(&self, frame: &Frame) -> FrameRoute {
        let key = match frame.address {
            FrameAddress::Can { id, extended, .. } => FrameMatchKey::Can { id, extended },
            FrameAddress::FlexRay {
                slot, base_cycle, ..
            } => FrameMatchKey::FlexRay {
                slot,
                cycle: base_cycle,
            },
            FrameAddress::Ethernet { vlan } => {
                let vlan = self.overrides.get(&frame.id).and_then(|o| o.vlan).or(vlan);
                FrameMatchKey::Ethernet {
                    vlan,
                    flow: EthFlowKey {
                        vlan,
                        dst: self
                            .overrides
                            .get(&frame.id)
                            .and_then(|o| o.dst_mac)
                            .unwrap_or([0; 6]),
                        src: self
                            .overrides
                            .get(&frame.id)
                            .and_then(|o| o.src_mac)
                            .unwrap_or([0; 6]),
                        ethertype: self
                            .overrides
                            .get(&frame.id)
                            .and_then(|o| o.ethertype)
                            .unwrap_or(0),
                        payload_len: frame.length as usize,
                    },
                }
            }
        };
        FrameRoute {
            cluster: frame.cluster,
            key,
        }
    }

    pub fn sender_of_frame(&self, frame: FrameId) -> Option<&Ecu> {
        self.frame_relations
            .iter()
            .find(|r| r.frame == frame && r.role == RelationRole::Sender)
            .and_then(|r| self.ecus.get(&r.ecu))
    }

    pub fn sender_of_pdu(&self, pdu: PduId) -> Option<&Ecu> {
        self.pdu_relations
            .iter()
            .find(|r| r.pdu == pdu && r.role == RelationRole::Sender)
            .and_then(|r| self.ecus.get(&r.ecu))
    }

    fn frame_for_pdu(&self, pdu: PduId) -> Option<FrameId> {
        self.frame_pdu_mappings
            .values()
            .find(|m| m.pdu == pdu)
            .map(|m| m.frame)
    }

    /// Structural checks that need no XML: every reference points at an
    /// entity, keys are consistent, and names are unique where the UI treats
    /// them as keys.
    pub fn integrity_issues(&self) -> Vec<Issue> {
        let mut issues = Vec::new();
        for frame in self.frames.values() {
            if !self.clusters.contains_key(&frame.cluster) {
                issues.push(Issue::error(
                    IssueKind::DanglingRef,
                    format!("frame {} refers to a missing cluster", frame.name),
                ));
            }
            if !self.networks.contains_key(&frame.network) {
                issues.push(Issue::error(
                    IssueKind::DanglingRef,
                    format!("frame {} refers to a missing network", frame.name),
                ));
            }
            for mapping in &frame.pdu_mappings {
                if !self.frame_pdu_mappings.contains_key(mapping) {
                    issues.push(Issue::error(
                        IssueKind::DanglingRef,
                        format!("frame {} refers to a missing PDU mapping", frame.name),
                    ));
                }
            }
        }
        for fpm in self.frame_pdu_mappings.values() {
            if !self.frames.contains_key(&fpm.frame) || !self.pdus.contains_key(&fpm.pdu) {
                issues.push(Issue::error(
                    IssueKind::DanglingRef,
                    "frame-PDU mapping has a missing end".to_string(),
                ));
            }
        }
        for signal in self.signals.values() {
            if !self.pdus.contains_key(&signal.pdu) {
                issues.push(Issue::error(
                    IssueKind::DanglingRef,
                    format!("signal {} refers to a missing PDU", signal.name),
                ));
            }
            if signal.length == 0 {
                issues.push(Issue::warning(
                    IssueKind::Other,
                    format!("signal {} has zero length", signal.name),
                ));
            }
        }
        for pdu in self.pdus.values() {
            if let Some(secured) = &pdu.secured
                && let Some(payload) = secured.payload
                && !self.pdus.contains_key(&payload)
            {
                issues.push(Issue::error(
                    IssueKind::DanglingRef,
                    format!("secured PDU {} refers to a missing payload", pdu.name),
                ));
            }
            if let Some(selector) = &pdu.selector
                && let Some(signal) = selector.signal
                && !self.signals.contains_key(&signal)
            {
                issues.push(Issue::error(
                    IssueKind::DanglingRef,
                    format!("multiplexed PDU {} refers to a missing selector", pdu.name),
                ));
            }
        }
        for group in self.groups.values() {
            if let Some(sender) = group.sender
                && !self.ecus.contains_key(&sender)
            {
                issues.push(Issue::error(
                    IssueKind::DanglingRef,
                    format!("signal group {} refers to a missing sender", group.name),
                ));
            }
        }
        issues
    }
}

/// A source file plus the cluster element chosen inside it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum ClusterSource {
    Dbc {
        path: String,
    },
    Arxml {
        path: String,
        #[serde(default)]
        cluster: String,
    },
    Fibex {
        path: String,
    },
    Manual,
}

impl ClusterSource {
    pub fn path(&self) -> Option<&str> {
        match self {
            ClusterSource::Dbc { path }
            | ClusterSource::Arxml { path, .. }
            | ClusterSource::Fibex { path } => Some(path),
            ClusterSource::Manual => None,
        }
    }

    pub fn accepts_path(path: &str) -> Option<&'static str> {
        let lower = path.to_ascii_lowercase();
        if lower.ends_with(".dbc") {
            Some("dbc")
        } else if lower.ends_with(".arxml") {
            Some("arxml")
        } else if lower.ends_with(".xml") || lower.ends_with(".fibex") {
            Some("fibex")
        } else {
            None
        }
    }
}

pub struct ImportOutcome {
    pub comm: Comm,
    pub issues: Vec<Issue>,
}
