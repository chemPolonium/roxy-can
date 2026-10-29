//! DBC → unified model adapter.
//!
//! The DBC parser already resolves factors, text tables, start values and
//! multiplexing facts; this adapter only maps its message/signal view onto
//! Frame/PDU/Signal. No decoding lives here.

use super::*;
use crate::dbc::{self, MessageInfo, SignalInfo, SymbolTable};

pub struct DbcImporter;

impl ClusterImporter for DbcImporter {
    fn name(&self) -> &'static str {
        "dbc"
    }

    fn accepts(&self, source: &ClusterSource) -> bool {
        matches!(source, ClusterSource::Dbc { .. })
    }

    fn import(&self, input: ImportInput<'_>, ids: &mut ElementIds) -> ImportOutcome {
        let name = input
            .source
            .path()
            .and_then(|path| std::path::Path::new(path).file_stem())
            .map(|stem| stem.to_string_lossy().to_string())
            .filter(|stem| !stem.is_empty())
            .unwrap_or_else(|| format!("CAN{}", input.cluster.0));
        import_dbc(input.text, &name, input.cluster, input.source, ids)
    }
}

pub fn import_dbc(
    text: &str,
    cluster_name: &str,
    cluster_id: ClusterId,
    source: &ClusterSource,
    ids: &mut ElementIds,
) -> ImportOutcome {
    let db = match dbc::load_dbc_str(text) {
        Ok(db) => db,
        Err(error) => {
            return ImportOutcome {
                comm: Comm::default(),
                issues: vec![Issue::error(
                    IssueKind::Other,
                    format!("DBC parse error: {error}"),
                )],
            };
        }
    };
    import_symbol_table(&db, cluster_name, cluster_id, source, ids, 500, 2000)
}

/// Builds the unified model from an already-parsed DBC table. The bus core
/// holds parsed tables, not source text, so this is the runtime entry.
pub fn import_symbol_table(
    db: &SymbolTable,
    cluster_name: &str,
    cluster_id: ClusterId,
    source: &ClusterSource,
    ids: &mut ElementIds,
    bitrate_kbps: u32,
    fd_data_kbps: u32,
) -> ImportOutcome {
    let mut comm = Comm::default();
    let network_id = NetworkId(cluster_id.0);
    comm.clusters.insert(
        cluster_id,
        Cluster {
            id: cluster_id,
            kind: ClusterKind::Can,
            name: cluster_name.to_string(),
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
            name: cluster_name.to_string(),
            params: NetworkParams::Can(CanParams {
                bitrate_kbps,
                fd_data_kbps,
                sample_point_permille: None,
            }),
        },
    );
    if !db.nodes.is_empty() {
        for node in &db.nodes {
            let _ = ecu(&mut comm, ids, cluster_id, node);
        }
    }

    // `order` is the file's declaration order, not HashMap iteration order.
    for key in &db.order {
        let Some(message) = db.messages.get(key) else {
            continue;
        };
        let (can_id, extended) = *key;
        let frame_source = format!("dbc:{}:frame:{can_id}:{extended}", cluster_id.0);
        let frame_id = FrameId(ids.assign(ElementKind::Frame, &frame_source));
        let pdu_source = format!("dbc:{}:pdu:{can_id}:{extended}", cluster_id.0);
        let pdu_id = PduId(ids.assign(ElementKind::Pdu, &pdu_source));
        let frame_pdu_source = format!("dbc:{}:frame-pdu:{can_id}:{extended}", cluster_id.0);
        let frame_pdu_id = MappingId(ids.assign(ElementKind::Mapping, &frame_pdu_source));

        comm.pdus.insert(
            pdu_id,
            Pdu {
                id: pdu_id,
                name: message.name.clone(),
                length: message.dlc as u32,
                kind: PduKind::Signal,
                header_type: ContainerHeaderType::Short,
                cycle_us: message.cycle_us,
                selector: None,
                secured: None,
                transformation: None,
                e2e: None,
            },
        );
        comm.frame_pdu_mappings.insert(
            frame_pdu_id,
            FramePduMapping {
                id: frame_pdu_id,
                frame: frame_id,
                pdu: pdu_id,
                start_bit: 0,
                byte_order: ByteOrder::LittleEndian,
            },
        );
        comm.frames.insert(
            frame_id,
            Frame {
                id: frame_id,
                cluster: cluster_id,
                network: network_id,
                name: message.name.clone(),
                length: message.dlc as u32,
                address: FrameAddress::Can {
                    id: can_id,
                    extended,
                    fd: message.fd || message.dlc > 8,
                },
                timing: FrameTiming {
                    cycle_us: message.cycle_us,
                    startup: false,
                    event: message.cycle_us == Some(0),
                },
                ethernet: None,
                pdu_mappings: vec![frame_pdu_id],
            },
        );

        if !message.transmitter.is_empty() && message.transmitter != "Vector__XXX" {
            let ecu_id = ecu(&mut comm, ids, cluster_id, &message.transmitter);
            comm.frame_relations.push(FrameEcuRelation {
                frame: frame_id,
                ecu: ecu_id,
                role: RelationRole::Sender,
            });
        }
        let mut receivers: Vec<String> = Vec::new();
        for signal in &message.signals {
            for receiver in &signal.receivers {
                if receiver != "Vector__XXX"
                    && receiver != &message.transmitter
                    && !receivers.contains(receiver)
                {
                    receivers.push(receiver.clone());
                }
            }
            self_signal(&mut comm, ids, cluster_id, pdu_id, signal, db, key);
        }
        apply_dbc_mux(&mut comm, pdu_id, message);
        for receiver in receivers {
            let ecu_id = ecu(&mut comm, ids, cluster_id, &receiver);
            if !comm
                .frame_relations
                .iter()
                .any(|relation| relation.frame == frame_id && relation.ecu == ecu_id)
            {
                comm.frame_relations.push(FrameEcuRelation {
                    frame: frame_id,
                    ecu: ecu_id,
                    role: RelationRole::Receiver,
                });
            }
        }
    }

    let issues = comm.integrity_issues();
    comm.issues.extend(issues);
    ImportOutcome {
        issues: comm.issues.clone(),
        comm,
    }
}

fn apply_dbc_mux(comm: &mut Comm, pdu: PduId, message: &MessageInfo) {
    let ids: std::collections::BTreeMap<String, SignalId> = comm
        .signals
        .values()
        .filter(|signal| signal.pdu == pdu)
        .map(|signal| (signal.name.clone(), signal.id))
        .collect();
    for signal in &message.signals {
        if signal.mux_when.is_empty() {
            continue;
        }
        let Some(&signal_id) = ids.get(&signal.name) else {
            continue;
        };
        let mut conditions = Vec::new();
        for condition in &signal.mux_when {
            let Some(&selector) = ids.get(&condition.switch) else {
                comm.issues.push(Issue::warning(
                    IssueKind::Unsupported,
                    format!(
                        "DBC signal {} gates on {}, which the message does not declare",
                        signal.name, condition.switch
                    ),
                ));
                continue;
            };
            conditions.push(MuxCondition {
                selector: MuxSelector::Signal { signal: selector },
                ranges: condition
                    .ranges
                    .iter()
                    .map(|(lo, hi)| (*lo as u32, *hi as u32))
                    .collect(),
            });
        }
        if let Some(model) = comm.signals.get_mut(&signal_id) {
            model.mux = conditions;
        }
    }
}

fn ecu(comm: &mut Comm, ids: &mut ElementIds, cluster: ClusterId, name: &str) -> EcuId {
    let source = format!("dbc:{}:ecu:{name}", cluster.0);
    let id = EcuId(ids.assign(ElementKind::Ecu, &source) as u32);
    comm.ecus.entry(id).or_insert(Ecu {
        id,
        name: name.to_string(),
        description: String::new(),
        mac: None,
    });
    id
}

fn self_signal(
    comm: &mut Comm,
    ids: &mut ElementIds,
    cluster: ClusterId,
    pdu: PduId,
    signal: &SignalInfo,
    db: &SymbolTable,
    key: &(u32, bool),
) {
    let signal_source = format!(
        "dbc:{}:signal:{}:{}:{}",
        cluster.0, key.0, key.1, signal.name
    );
    let signal_id = SignalId(ids.assign(ElementKind::Signal, &signal_source));
    let mapping_source = format!(
        "dbc:{}:signal-mapping:{}:{}:{}",
        cluster.0, key.0, key.1, signal.name
    );
    let mapping_id = MappingId(ids.assign(ElementKind::Mapping, &mapping_source));
    let initial_raw = signal.initial.map(|physical| {
        if signal.factor == 0.0 {
            physical - signal.offset
        } else {
            (physical - signal.offset) / signal.factor
        }
    });
    let values = db
        .value_tables
        .get(&(*key, signal.name.clone()))
        .map(|table| {
            let mut rows: Vec<(i64, String)> = table
                .iter()
                .map(|(code, text)| (*code, text.clone()))
                .collect();
            rows.sort_by_key(|(code, _)| *code);
            rows
        })
        .unwrap_or_default();
    let declared_min = (signal.min != 0.0 || signal.max != 0.0).then_some(signal.min);
    let declared_max = (signal.min != 0.0 || signal.max != 0.0).then_some(signal.max);
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
            float: signal.is_float,
            factor: signal.factor,
            offset: signal.offset,
            unit: signal.unit.clone(),
            min: declared_min,
            max: declared_max,
            min_open: false,
            max_open: false,
            initial_raw,
            values,
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
