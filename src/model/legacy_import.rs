//! FIBEX and Ethernet-database adapters.
//!
//! These reuse the existing readers, which already know the two dialects, and
//! only translate their frame/PDU/signal tables into the unified model.

use super::*;
use crate::eth::db::{EthDb, EthSignal};
use crate::fr_db::FrDb;

pub struct FibexImporter;

impl ClusterImporter for FibexImporter {
    fn name(&self) -> &'static str {
        "fibex"
    }

    fn accepts(&self, source: &ClusterSource) -> bool {
        matches!(source, ClusterSource::Fibex { .. })
    }

    fn import(&self, input: ImportInput<'_>, ids: &mut ElementIds) -> ImportOutcome {
        let name = input
            .source
            .path()
            .and_then(|path| std::path::Path::new(path).file_stem())
            .map(|stem| stem.to_string_lossy().to_string())
            .filter(|stem| !stem.is_empty())
            .unwrap_or_else(|| format!("FR{}", input.cluster.0));
        import_fibex(input.text, &name, input.cluster, input.source, ids)
    }
}

pub struct EthernetDbImporter;

impl ClusterImporter for EthernetDbImporter {
    fn name(&self) -> &'static str {
        "ethernet-db"
    }

    fn accepts(&self, source: &ClusterSource) -> bool {
        matches!(source, ClusterSource::Fibex { .. })
    }

    fn import(&self, input: ImportInput<'_>, ids: &mut ElementIds) -> ImportOutcome {
        let name = input
            .source
            .path()
            .and_then(|path| std::path::Path::new(path).file_stem())
            .map(|stem| stem.to_string_lossy().to_string())
            .filter(|stem| !stem.is_empty())
            .unwrap_or_else(|| format!("ETH{}", input.cluster.0));
        import_eth_db(input.text, &name, input.cluster, input.source, ids)
    }
}

pub fn import_fibex(
    text: &str,
    cluster_name: &str,
    cluster_id: ClusterId,
    source: &ClusterSource,
    ids: &mut ElementIds,
) -> ImportOutcome {
    let db = match FrDb::parse(text) {
        Ok(db) => db,
        Err(error) => return failure(format!("FIBEX parse error: {error}")),
    };
    import_fr_db(&db, cluster_name, cluster_id, source, ids)
}

/// Builds the unified model from an already-parsed FlexRay database. The bus
/// core holds the parsed `FrDb`, so this is its entry.
pub fn import_fr_db(
    db: &FrDb,
    cluster_name: &str,
    cluster_id: ClusterId,
    source: &ClusterSource,
    ids: &mut ElementIds,
) -> ImportOutcome {
    let mut comm = Comm::default();
    let network_id = NetworkId(cluster_id.0);
    comm.clusters.insert(
        cluster_id,
        Cluster {
            id: cluster_id,
            kind: ClusterKind::FlexRay,
            name: cluster_name.to_string(),
            aliases: Vec::new(),
            description: db.params.name.clone(),
            sources: vec![source.clone()],
            networks: vec![network_id],
        },
    );
    comm.networks.insert(
        network_id,
        Network {
            id: network_id,
            cluster: cluster_id,
            name: db.params.name.clone(),
            params: NetworkParams::FlexRay(FlexRayParams {
                speed_kbps: db.params.speed_kbps,
                cycle_time_ms: db.params.cycle_time_ms,
                macrotick_us: db.params.macrotick_duration_us,
                coldstart_attempts: db.params.coldstart_attempts,
                action_point_offset: db.params.action_point_offset,
                minislot_action_point_offset: db.params.minislot_action_point_offset,
                dynamic_slot_idle_phase: db.params.dynamic_slot_idle_phase,
                minor_version: db.params.minor_version,
                extra: std::collections::BTreeMap::new(),
            }),
        },
    );
    for name in &db.ecus {
        let id = EcuId(ids.assign(
            ElementKind::Ecu,
            &format!("fibex:{}:ecu:{name}", cluster_id.0),
        ) as u32);
        comm.ecus.insert(
            id,
            Ecu {
                id,
                name: name.clone(),
                description: String::new(),
                mac: None,
            },
        );
    }

    let mut pdu_ids: std::collections::BTreeMap<String, PduId> = std::collections::BTreeMap::new();
    for frame in &db.frames {
        let frame_id = FrameId(ids.assign(
            ElementKind::Frame,
            &format!("fibex:{}:frame:{}", cluster_id.0, frame.name),
        ));
        let mut mappings = Vec::new();
        // The logged payload begins at the frame's first PDU, while ARXML
        // START-POSITIONs are measured from the frame head. Rebasing is what
        // the legacy decoder does in `frame_children`; the plan must store the
        // same relative offsets.
        let base = frame
            .pdus
            .iter()
            .map(|(_, start)| *start)
            .min()
            .unwrap_or(0);
        for (index, (pdu_name, pdu_start)) in frame.pdus.iter().enumerate() {
            let pdu_id = match pdu_ids.get(pdu_name) {
                Some(id) => *id,
                None => {
                    let id = PduId(ids.assign(
                        ElementKind::Pdu,
                        &format!("fibex:{}:pdu:{pdu_name}", cluster_id.0),
                    ));
                    let length = db
                        .pdus
                        .iter()
                        .find(|pdu| &pdu.name == pdu_name)
                        .map_or(0, |pdu| pdu.length);
                    comm.pdus.insert(
                        id,
                        Pdu {
                            id,
                            name: pdu_name.clone(),
                            length,
                            kind: PduKind::Signal,
                            header_type: ContainerHeaderType::Short,
                            cycle_us: None,
                            selector: None,
                            secured: None,
                            transformation: None,
                            e2e: None,
                        },
                    );
                    pdu_ids.insert(pdu_name.clone(), id);
                    add_pdu_signals(&mut comm, ids, cluster_id, id, pdu_name, db, *pdu_start);
                    id
                }
            };
            let mapping_id = MappingId(ids.assign(
                ElementKind::Mapping,
                &format!(
                    "fibex:{}:frame-pdu:{}:{index}:{pdu_name}",
                    cluster_id.0, frame.name
                ),
            ));
            comm.frame_pdu_mappings.insert(
                mapping_id,
                FramePduMapping {
                    id: mapping_id,
                    frame: frame_id,
                    pdu: pdu_id,
                    start_bit: pdu_start.saturating_sub(base),
                    byte_order: ByteOrder::LittleEndian,
                },
            );
            mappings.push(mapping_id);
        }
        comm.frames.insert(
            frame_id,
            Frame {
                id: frame_id,
                cluster: cluster_id,
                network: network_id,
                name: frame.name.clone(),
                length: frame.length,
                address: FrameAddress::FlexRay {
                    slot: frame.triggering.slot_id as u16,
                    base_cycle: frame.triggering.base_cycle as u8,
                    repetition: frame.triggering.cycle_repetition.max(1) as u8,
                    payload_preamble: frame.payload_preamble,
                },
                timing: FrameTiming {
                    cycle_us: Some((db.params.cycle_time_ms * 1000.0) as u64),
                    startup: frame.triggering.startup,
                    event: false,
                },
                ethernet: None,
                pdu_mappings: mappings,
            },
        );
    }
    let issues = comm.integrity_issues();
    comm.issues.extend(issues);
    ImportOutcome {
        issues: comm.issues.clone(),
        comm,
    }
}

fn add_pdu_signals(
    comm: &mut Comm,
    ids: &mut ElementIds,
    cluster: ClusterId,
    pdu: PduId,
    pdu_name: &str,
    db: &FrDb,
    _pdu_start: u32,
) {
    let Some(definition) = db.pdus.iter().find(|candidate| candidate.name == pdu_name) else {
        return;
    };
    for signal in &definition.signals {
        let signal_id = SignalId(ids.assign(
            ElementKind::Signal,
            &format!("fibex:{}:pdu:{pdu_name}:signal:{}", cluster.0, signal.name),
        ));
        let mapping_id = MappingId(ids.assign(
            ElementKind::Mapping,
            &format!(
                "fibex:{}:pdu:{pdu_name}:signal-mapping:{}",
                cluster.0, signal.name
            ),
        ));
        comm.signals.insert(
            signal_id,
            Signal {
                id: signal_id,
                pdu,
                name: signal.name.clone(),
                system_signal: None,
                length: signal.length_bits,
                start_bit: signal.start_bit,
                byte_order: if signal.big_endian {
                    ByteOrder::BigEndian
                } else {
                    ByteOrder::LittleEndian
                },
                update_bit: None,
                signed: signal.signed,
                float: false,
                factor: signal.factor,
                offset: signal.offset,
                unit: signal.unit.clone(),
                min: (signal.min != 0.0 || signal.max != 0.0).then_some(signal.min),
                max: (signal.min != 0.0 || signal.max != 0.0).then_some(signal.max),
                min_open: false,
                max_open: false,
                initial_raw: None,
                values: signal.value_descriptions.clone(),
                base_type: None,
                compu: None,
                comment: signal.comment.clone(),
                transformation: None,
                group: None,
                mux: Vec::new(),
            },
        );
        comm.signal_pdu_mappings.insert(
            mapping_id,
            SignalPduMapping {
                id: mapping_id,
                signal: signal_id,
                pdu,
                start_bit: signal.start_bit,
                byte_order: if signal.big_endian {
                    ByteOrder::BigEndian
                } else {
                    ByteOrder::LittleEndian
                },
                update_bit: None,
            },
        );
    }
}

pub fn import_eth_db(
    text: &str,
    cluster_name: &str,
    cluster_id: ClusterId,
    source: &ClusterSource,
    ids: &mut ElementIds,
) -> ImportOutcome {
    let db = match EthDb::parse(text) {
        Ok(db) => db,
        Err(error) => return failure(format!("Ethernet database parse error: {error}")),
    };
    import_eth_db_obj(&db, cluster_name, cluster_id, source, ids)
}

/// Builds the unified model from an already-parsed Ethernet database.
pub fn import_eth_db_obj(
    db: &EthDb,
    cluster_name: &str,
    cluster_id: ClusterId,
    source: &ClusterSource,
    ids: &mut ElementIds,
) -> ImportOutcome {
    let mut comm = Comm::default();
    let network_id = NetworkId(cluster_id.0);
    let cluster_name = if cluster_name.is_empty() {
        db.clusters
            .first()
            .map_or_else(|| "Ethernet".to_string(), |cluster| cluster.name.clone())
    } else {
        cluster_name.to_string()
    };
    let vlans = db
        .clusters
        .iter()
        .find(|cluster| cluster.name == cluster_name)
        .map_or_else(Vec::new, |cluster| cluster.vlans.clone());
    comm.clusters.insert(
        cluster_id,
        Cluster {
            id: cluster_id,
            kind: ClusterKind::Ethernet,
            name: cluster_name.clone(),
            aliases: Vec::new(),
            description: String::new(),
            sources: vec![source.clone()],
            networks: vec![network_id],
        },
    );
    comm.networks.insert(
        network_id,
        Network {
            id: network_id,
            cluster: cluster_id,
            name: cluster_name,
            params: NetworkParams::Ethernet(EthernetParams {
                physical_layer: None,
                vlans,
                frame_length_includes_header: false,
            }),
        },
    );

    for frame in &db.frames {
        let frame_id = FrameId(ids.assign(
            ElementKind::Frame,
            &format!("eth-db:{}:frame:{}", cluster_id.0, frame.name),
        ));
        let mut mappings = Vec::new();
        for (index, pdu) in frame.pdus.iter().enumerate() {
            let pdu_id = PduId(ids.assign(
                ElementKind::Pdu,
                &format!(
                    "eth-db:{}:frame:{}:pdu:{index}:{}",
                    cluster_id.0, frame.name, pdu.name
                ),
            ));
            comm.pdus.insert(
                pdu_id,
                Pdu {
                    id: pdu_id,
                    name: pdu.name.clone(),
                    length: pdu.length as u32,
                    kind: PduKind::Signal,
                    header_type: ContainerHeaderType::Short,
                    cycle_us: None,
                    selector: None,
                    secured: None,
                    transformation: None,
                    e2e: None,
                },
            );
            let mapping_id = MappingId(ids.assign(
                ElementKind::Mapping,
                &format!(
                    "eth-db:{}:frame:{}:pdu-mapping:{index}",
                    cluster_id.0, frame.name
                ),
            ));
            comm.frame_pdu_mappings.insert(
                mapping_id,
                FramePduMapping {
                    id: mapping_id,
                    frame: frame_id,
                    pdu: pdu_id,
                    start_bit: pdu.start_bit as u32,
                    byte_order: ByteOrder::LittleEndian,
                },
            );
            mappings.push(mapping_id);
            add_eth_signals(
                &mut comm,
                ids,
                cluster_id,
                pdu_id,
                &frame.name,
                index,
                &pdu.signals,
            );
        }
        comm.frames.insert(
            frame_id,
            Frame {
                id: frame_id,
                cluster: cluster_id,
                network: network_id,
                name: frame.name.clone(),
                length: frame.length as u32,
                address: FrameAddress::Ethernet { vlan: frame.vlan },
                timing: FrameTiming::default(),
                ethernet: Some(EthernetFrameMeta::Base),
                pdu_mappings: mappings,
            },
        );
        if let Some(ethertype) = frame.ethertype_hint {
            comm.overrides.insert(
                frame_id,
                FrameOverride {
                    ethertype: Some(ethertype),
                    ..FrameOverride::default()
                },
            );
        }
    }
    let issues = comm.integrity_issues();
    comm.issues.extend(issues);
    ImportOutcome {
        issues: comm.issues.clone(),
        comm,
    }
}

fn add_eth_signals(
    comm: &mut Comm,
    ids: &mut ElementIds,
    cluster: ClusterId,
    pdu: PduId,
    frame_name: &str,
    pdu_index: usize,
    signals: &[EthSignal],
) {
    for signal in signals {
        let signal_id = SignalId(ids.assign(
            ElementKind::Signal,
            &format!(
                "eth-db:{}:{}:{pdu_index}:signal:{}",
                cluster.0, frame_name, signal.name
            ),
        ));
        let mapping_id = MappingId(ids.assign(
            ElementKind::Mapping,
            &format!(
                "eth-db:{}:{}:{pdu_index}:signal-mapping:{}",
                cluster.0, frame_name, signal.name
            ),
        ));
        comm.signals.insert(
            signal_id,
            Signal {
                id: signal_id,
                pdu,
                name: signal.name.clone(),
                system_signal: None,
                length: signal.size as u32,
                start_bit: signal.start_bit as u32,
                byte_order: if signal.big_endian {
                    ByteOrder::BigEndian
                } else {
                    ByteOrder::LittleEndian
                },
                update_bit: None,
                signed: signal.signed,
                float: false,
                factor: signal.factor,
                offset: signal.offset,
                unit: signal.unit.clone(),
                min: None,
                max: None,
                min_open: false,
                max_open: false,
                initial_raw: None,
                values: signal.values.clone(),
                base_type: None,
                compu: None,
                comment: String::new(),
                transformation: None,
                group: None,
                mux: Vec::new(),
            },
        );
        comm.signal_pdu_mappings.insert(
            mapping_id,
            SignalPduMapping {
                id: mapping_id,
                signal: signal_id,
                pdu,
                start_bit: signal.start_bit as u32,
                byte_order: if signal.big_endian {
                    ByteOrder::BigEndian
                } else {
                    ByteOrder::LittleEndian
                },
                update_bit: None,
            },
        );
    }
}

fn failure(message: String) -> ImportOutcome {
    ImportOutcome {
        comm: Comm::default(),
        issues: vec![Issue::error(IssueKind::Other, message)],
    }
}
