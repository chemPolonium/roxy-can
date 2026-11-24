//! Bus core tests: ingestion, publishing, triggers and replay wiring.

use super::*;

/// The wire note names where frames actually go; "(virtual)" must not be
/// read as "not on real hardware" while a channel is attached.
#[test]
fn the_wire_note_names_the_wire_the_frames_go_out_on() {
    assert_eq!(wire_note(0, true), "simulation");
    assert_eq!(wire_note(0, false), "simulation", "nothing to park");
    assert_eq!(wire_note(2, true), "real bus · 2");
    assert_eq!(wire_note(2, false), "simulation · hardware parked");
}

/// `send_fd` sets FD even at eight bytes, where length alone would mean a
/// classic frame; the BRS bit rides on the same send flags.
#[test]
fn script_send_flags_reach_the_frame() {
    let payload = [0xAA; 8];
    let classic = BusCore::node_frame(0, 0x100, false, &payload, 0, 0);
    assert!(!classic.is_fd(), "eight bytes alone stays classic");
    assert_eq!(classic.len, 8);

    let fd = BusCore::node_frame(0, 0x100, false, &payload, crate::script::CAN_SEND_FD, 0);
    assert!(fd.is_fd(), "send_fd forces the FD flag");
    assert!(!fd.brs());
    assert_eq!(fd.len, 8);

    let brs = BusCore::node_frame(
        0,
        0x101,
        true,
        &payload,
        crate::script::CAN_SEND_FD | crate::script::CAN_SEND_BRS,
        0,
    );
    assert!(brs.is_fd());
    assert!(brs.brs());
    assert!(brs.extended);
    assert_eq!(brs.len, 8);
}

fn fr_row(t_us: u64, slot: u16) -> crate::trace::FrRow {
    crate::trace::FrRow {
        bus: 0,
        t_us,
        ab: 2,
        slot,
        cycle: 3,
        payload: vec![0x11; 8],
        header_crc: 0xBEEF,
        flags: 0,
        name: None,
    }
}

/// Per-frame tallies behind the Messages FR rows: count and the EMA cycle
/// from real arrivals. The row is the frame, not the slot -- see
/// [`a_slot_held_by_two_frames_counts_two_rows`].
#[test]
fn fr_slot_aggregates_track_arrivals() {
    let mut core = BusCore::new(Vec::new());
    let mut a = fr_row(1_000_000, 10);
    a.cycle = 0;
    let mut b = fr_row(1_005_000, 10);
    b.cycle = 4; // rep-4 frame's next occurrence
    core.ingest_fr_row(a);
    core.ingest_fr_row(b);

    core.publish_trace();
    let snap = core.snapshot();
    assert_eq!(snap.fr_aggs.len(), 1, "no description: one unnamed row");
    let agg = &snap.fr_aggs[0];
    assert_eq!(agg.slot, 10);
    assert_eq!(agg.count, 2);
    assert!(agg.name.is_none(), "nothing named this frame");
    assert_eq!(agg.cycle_us, 5000.0, "first interval seeds the EMA");
}

/// A description that gives one static slot to two frames in alternate
/// A description that arrives mid-run re-tallies the rows already in the
/// ring: frames counted before the file was loaded belong to the same
/// frames as the ones after it, so they must not sit in a nameless row
/// beside them. (`crate::fr_db::two_frames_one_slot_db` builds the
/// description: one slot, two frames, alternate cycles.)
#[test]
fn loading_a_description_re_tallies_the_rows_on_screen() {
    let mut core = BusCore::new(Vec::new());
    // Both phases of the repeated slot, before any description exists: at
    // this point the tool cannot tell them apart, and counts them as one.
    for (i, cycle) in [0u8, 1, 2, 3].iter().enumerate() {
        let mut r = fr_row(1_000_000 + (i as u64) * 1_000_000, 10);
        r.cycle = *cycle;
        core.ingest_fr_row(r);
    }
    core.publish_trace();
    let snap = core.snapshot();
    assert_eq!(snap.fr_aggs.len(), 1, "unnamed until something can name it");
    assert!(snap.fr_aggs[0].name.is_none());
    assert_eq!(snap.fr_aggs[0].count, 4);

    let mut dbs = std::collections::BTreeMap::new();
    dbs.insert(0u8, crate::fr_db::two_frames_one_slot_db(10));
    core.handle(
        BusCommand::SetFrDbs {
            dbs,
            sources: Default::default(),
        },
        &mut String::new(),
    );
    let snap = core.snapshot();
    let mut got: Vec<(&str, u64)> = snap
        .fr_aggs
        .iter()
        .map(|a| (a.name.as_deref().unwrap_or(""), a.count))
        .collect();
    got.sort_unstable();
    assert_eq!(
        got,
        [("FA", 2), ("FB", 2)],
        "the four arrivals belong to two occupants, two cycles each"
    );
}

/// The regression the Messages window needed: one tally per `(bus, slot)`
/// merged the two occupants of a repeated slot into a single row whose
/// frame name changed with whichever arrival came last, and whose period
/// was the slot's rather than any frame's. A slot held by two frames is now
/// two rows, each with its own name, count and 2 ms period.
#[test]
fn a_slot_held_by_two_frames_counts_two_rows() {
    let mut core = BusCore::new(Vec::new());
    core.fr_dbs
        .insert(0, crate::fr_db::two_frames_one_slot_db(10));
    for (i, cycle) in [0u8, 2, 4, 6].iter().enumerate() {
        let mut r = fr_row(1_000_000 + (i as u64) * 2_000_000, 10);
        r.cycle = *cycle;
        core.ingest_fr_row(r);
    }
    for (i, cycle) in [1u8, 3, 5, 7].iter().enumerate() {
        let mut r = fr_row(2_000_000 + (i as u64) * 2_000_000, 10);
        r.cycle = *cycle;
        core.ingest_fr_row(r);
    }
    core.publish_trace();
    let snap = core.snapshot();
    let rows: Vec<(String, u64, f64)> = snap
        .fr_aggs
        .iter()
        .map(|a| (a.name.clone().unwrap_or_default(), a.count, a.cycle_us))
        .collect();
    assert_eq!(rows.len(), 2, "one row per occupant: {rows:?}");
    assert!(
        rows.iter().all(|(_, c, _)| *c == 4),
        "each frame counted on its own: {rows:?}"
    );
    assert!(
        rows.iter()
            .all(|(_, _, cy)| (*cy - 2_000_000.0).abs() < 1.0),
        "the period is the frame's, not the slot's: {rows:?}"
    );
    let mut names: Vec<&str> = rows.iter().map(|(n, _, _)| n.as_str()).collect();
    names.sort_unstable();
    assert_eq!(names, ["FA", "FB"], "stable names, one per row");
}

/// Slot numbers repeat across clusters, so the tally is keyed by bus and
/// slot together: the same slot on two FlexRay buses stays two rows, each
/// with its own cycle history. Without the bus in the key, watching a
/// second port would silently merge its traffic into the first.
#[test]
fn the_same_slot_on_two_flexray_buses_stays_two_rows() {
    let mut core = BusCore::new(Vec::new());
    let mut on_bus1 = fr_row(1_000_000, 10);
    on_bus1.bus = 1;
    let mut later_on_bus1 = fr_row(1_002_000, 10);
    later_on_bus1.bus = 1;
    core.ingest_fr_row(fr_row(1_000_000, 10));
    core.ingest_fr_row(on_bus1);
    core.ingest_fr_row(later_on_bus1);

    core.publish_trace();
    let snap = core.snapshot();
    assert_eq!(snap.fr_aggs.len(), 2, "one row per (bus, slot)");
    assert_eq!(
        snap.fr_aggs
            .iter()
            .map(|a| (a.bus, a.slot))
            .collect::<Vec<_>>(),
        vec![(0, 10), (1, 10)],
        "ordered by bus first, so the Messages list groups one cluster's slots together"
    );
    let second = snap.fr_aggs.iter().find(|a| a.bus == 1).expect("bus 1 row");
    assert_eq!(second.count, 2);
    assert_eq!(
        second.cycle_us, 2000.0,
        "its cycle mixes only its own arrivals"
    );
}

/// A one-PDU, one-frame description whose single 8-bit signal at slot
/// `slot` reads the payload's first byte, scheduled on `channel`.
fn fr_db_one_signal_on(
    slot: u16,
    sig_name: &str,
    unit: &str,
    channel: crate::fr_db::FrChannel,
) -> std::sync::Arc<crate::fr_db::FrDb> {
    use crate::fr_db::{FrDb, FrFrameDb, FrPdu, FrSignal, FrTriggering};
    let sig = FrSignal {
        name: sig_name.into(),
        start_bit: 0,
        length_bits: 8,
        big_endian: false,
        signed: false,
        factor: 1.0,
        offset: 0.0,
        min: 0.0,
        max: 255.0,
        unit: unit.into(),
        comment: String::new(),
        value_descriptions: vec![],
    };
    let pdu = FrPdu {
        name: "P1".into(),
        length: 8,
        dynamic: false,
        comment: String::new(),
        signals: vec![sig],
    };
    let frame = FrFrameDb {
        name: format!("F{slot}"),
        length: 8,
        payload_preamble: false,
        triggering: FrTriggering {
            channel,
            slot_id: slot as u32,
            base_cycle: 0,
            cycle_repetition: 1,
            startup: false,
        },
        pdus: vec![("P1".into(), 0u32)],
        comment: String::new(),
    };
    std::sync::Arc::new(FrDb::assemble(
        Default::default(),
        vec![],
        vec![pdu],
        vec![frame],
    ))
}

fn fr_db_one_signal(slot: u16, sig_name: &str, unit: &str) -> std::sync::Arc<crate::fr_db::FrDb> {
    fr_db_one_signal_on(slot, sig_name, unit, crate::fr_db::FrChannel::Both)
}

/// FlexRay subscriptions decode through the unified model plan.
#[test]
fn fr_signal_subscriptions_use_the_unified_decode_plan() {
    let mut core = BusCore::new(Vec::new());
    let mut dbs = std::collections::BTreeMap::new();
    dbs.insert(0u8, fr_db_one_signal(12, "Speed", "km/h"));
    core.handle(
        BusCommand::SetFrDbs {
            dbs,
            sources: Default::default(),
        },
        &mut String::new(),
    );
    assert!(
        core.fr_models.contains_key(&0),
        "pushing a cluster description builds the FR model"
    );
    let runtime = core.fr_models.get(&0).expect("FR runtime");
    assert!(Arc::ptr_eq(&runtime.db, core.fr_dbs.get(&0).unwrap()));

    let key = core.fr_key(0, 12, "Speed".to_string());
    core.subscribe_signal(key.clone());
    let mut row = fr_row(1_000, 12);
    row.cycle = 0;
    row.payload = vec![80, 0, 0, 0, 0, 0, 0, 0];
    core.ingest_fr_row(row);
    assert_eq!(
        core.subs.get(&key).expect("sub").latest,
        80.0,
        "the model plan decodes the same slot"
    );
}

/// Ethernet subscriptions decode through the unified model once a matrix
/// is loaded.
#[test]
fn eth_signal_subscriptions_use_the_unified_decode_plan() {
    let text = std::fs::read_to_string(crate::test_assets::eth_arxml()).unwrap();
    let db = std::sync::Arc::new(crate::eth::db::EthDb::parse(&text).unwrap());
    let mut core = BusCore::new(Vec::new());
    let mut dbs = std::collections::BTreeMap::new();
    dbs.insert(0u8, db);
    core.handle(
        BusCommand::SetEthDbs {
            dbs,
            sources: Default::default(),
        },
        &mut String::new(),
    );
    assert!(
        core.eth_models.contains_key(&0),
        "pushing a matrix builds the ETH model"
    );
    let runtime = core.eth_models.get(&0).expect("ETH runtime");
    assert!(Arc::ptr_eq(&runtime.db, core.eth_dbs.get(&0).unwrap()));

    let key = core.eth_key(0, "EthFrame".to_string(), "Sig".to_string());
    core.subscribe_signal(key.clone());
    let mut payload = vec![0u8; 22];
    payload[1] = 50;
    let values = core
        .model_eth_values(0, "EthFrame", &payload)
        .expect("the model knows the frame");
    assert_eq!(values.len(), 1);
    assert!((values[0].phys - 5.0).abs() < 1e-9, "{}", values[0].phys);
}

/// The FR generator uses the model for driven signals too.
#[test]
fn fr_generator_payload_uses_the_unified_encoder() {
    let mut core = BusCore::new(Vec::new());
    let mut dbs = std::collections::BTreeMap::new();
    dbs.insert(0u8, fr_db_one_signal(12, "Speed", "km/h"));
    core.handle(
        BusCommand::SetFrDbs {
            dbs,
            sources: Default::default(),
        },
        &mut String::new(),
    );
    let tx = crate::generator::FrTxMsg {
        bus: 0,
        slot: 12,
        name: "F12".to_string(),
        node: String::new(),
        len: 8,
        data: vec![0u8; 8],
        data_text: String::new(),
        cycle_us: 10_000,
        active: false,
        next_t_us: 0,
        srcs: vec![crate::sim::ValueSrc::new(
            "Speed",
            crate::sim::SrcKind::Ramp,
            80.0,
            80.0,
        )],
        last_sent: Vec::new(),
    };
    let runtime = core.fr_models.get(&0).expect("FR runtime");
    let payload = crate::generator::fr_tx_payload_with_model(
        *core.fr_cluster_ids.get(&0).unwrap(),
        &runtime.comm,
        &runtime.plans,
        &tx,
        0,
    )
    .expect("model knows the FR frame");
    assert_eq!(payload[0], 80);
}

/// A description that schedules the slot on channel A still decodes a row
/// the log says arrived on B. `(slot, cycle)` is what the cluster schedule
/// fixes, and the same frame off a live port -- whose channel is not known
/// at all -- has always decoded, so requiring the declaration made a
/// replay stricter than the watch and left a disagreeing description
/// showing nothing.
#[test]
fn a_channel_disagreement_still_resolves_the_scheduled_frame() {
    let mut core = BusCore::new(Vec::new());
    core.fr_dbs.insert(
        0,
        fr_db_one_signal_on(12, "Speed", "km/h", crate::fr_db::FrChannel::A),
    );
    core.rebuild_fr_models();
    let key = core.fr_key(0, 12, "Speed".to_string());
    core.subscribe_signal(key.clone());

    let mut row = fr_row(1_000, 12);
    row.ab = 1; // logged on B, declared on A
    row.cycle = 0;
    row.payload = vec![80, 0, 0, 0, 0, 0, 0, 0];
    core.ingest_fr_row(row);

    assert_eq!(
        core.subs.get(&key).expect("sub").latest,
        80.0,
        "the frame the schedule puts in that slot at that cycle decodes"
    );
}

/// The channel is a preference, not a fallback: when two frames really do
/// share a slot and cycle and differ by channel, the matching one wins and
/// the other is only reached when nothing declares this channel.
#[test]
fn the_declared_channel_breaks_a_tie_between_frames_in_one_slot() {
    use crate::fr_db::{FrChannel, FrDb, FrFrameDb, FrPdu, FrSignal, FrTriggering};
    let sig = |name: &str| FrSignal {
        name: name.into(),
        start_bit: 0,
        length_bits: 8,
        big_endian: false,
        signed: false,
        factor: 1.0,
        offset: 0.0,
        min: 0.0,
        max: 255.0,
        unit: String::new(),
        comment: String::new(),
        value_descriptions: vec![],
    };
    let frame = |name: &str, channel: FrChannel| FrFrameDb {
        name: name.into(),
        length: 8,
        payload_preamble: false,
        triggering: FrTriggering {
            channel,
            slot_id: 12,
            base_cycle: 0,
            cycle_repetition: 1,
            startup: false,
        },
        pdus: vec![(format!("P{name}"), 0u32)],
        comment: String::new(),
    };
    let pdus = vec![
        FrPdu {
            name: "PA".into(),
            length: 8,
            dynamic: false,
            comment: String::new(),
            signals: vec![sig("OnA")],
        },
        FrPdu {
            name: "PB".into(),
            length: 8,
            dynamic: false,
            comment: String::new(),
            signals: vec![sig("OnB")],
        },
    ];
    let db = FrDb::assemble(
        Default::default(),
        vec![],
        pdus,
        vec![frame("A", FrChannel::A), frame("B", FrChannel::B)],
    );
    assert_eq!(db.frame_at(12, 0, 0).map(|f| f.name.as_str()), Some("A"));
    assert_eq!(db.frame_at(12, 0, 1).map(|f| f.name.as_str()), Some("B"));
    // An unknown channel takes the first scheduled frame either way, and a
    // channel nothing declares falls back rather than resolving nothing.
    assert!(db.frame_at(12, 0, 2).is_some());
    let only_a = fr_db_one_signal_on(9, "S", "", FrChannel::A);
    assert_eq!(
        only_a.frame_at(9, 0, 1).map(|f| f.name.as_str()),
        Some("F9"),
        "a row logged on B still finds the A-declared frame in its slot"
    );
}

/// With a FlexRay description database loaded, an arriving FR frame
/// decodes into its signals and folds into the subscriptions -- the same
/// live-value path a CAN frame drives, which is what lets the observers
/// plot a FlexRay signal during replay.
#[test]
fn flexray_frames_fold_signals_into_subscriptions() {
    let mut core = BusCore::new(Vec::new());
    core.fr_dbs.insert(0, fr_db_one_signal(12, "Speed", "km/h"));
    core.rebuild_fr_models();
    let key = core.fr_key(0, 12, "Speed".to_string());
    core.subscribe_signal(key.clone());

    let mut row = fr_row(1_000, 12);
    row.ab = 0;
    row.cycle = 0;
    row.payload = vec![42, 0, 0, 0, 0, 0, 0, 0];
    core.ingest_fr_row(row);

    let sub = core.subs.get(&key).expect("FR signal subscribed");
    assert_eq!(sub.latest, 42.0, "physical value folded in");
    assert_eq!(sub.last_raw, 42);
    assert_eq!(sub.unit, "km/h", "unit carried from the description");
}

/// A FlexRay signal crossing fires on the value the description computes,
/// not on the raw slot: the level follows the decoded signal, a second high
/// frame is not a second edge, and only a frame that says otherwise clears
/// it -- the same level discipline the CAN `SignalCross` arm uses.
#[test]
fn a_flexray_signal_crossing_fires_on_the_decoded_value() {
    let mut core = BusCore::new(Vec::new());
    core.fr_dbs.insert(0, fr_db_one_signal(12, "Speed", "km/h"));
    core.rebuild_fr_models();
    core.triggers.push(crate::trigger::Trigger::new(
        crate::trigger::TriggerCond::FrSignalCross {
            bus: 0,
            slot: 12,
            signal: "Speed".into(),
            threshold: 100.0,
            rising: true,
        },
        crate::trigger::TriggerAction::InsertMarker,
    ));
    let mut status = String::new();
    // A free item rather than a closure: the closure would hold `status`
    // borrowed across the direct calls below.
    fn fr_fire(core: &mut BusCore, status: &mut String, t_us: u64, v: u8) {
        let mut row = fr_row(t_us, 12);
        row.ab = 0;
        row.cycle = 0;
        row.payload = vec![v, 0, 0, 0, 0, 0, 0, 0];
        core.eval_fr_triggers(&row, status);
        core.ingest_fr_row(row);
    }

    fr_fire(&mut core, &mut status, 1_000, 17);
    assert!(core.markers.is_empty(), "17 km/h has not crossed 100");
    fr_fire(&mut core, &mut status, 2_000, 120);
    assert_eq!(core.markers, vec![2_000], "the crossing edge marks");
    fr_fire(&mut core, &mut status, 3_000, 130);
    assert_eq!(core.markers.len(), 1, "still high is not a new edge");
    fr_fire(&mut core, &mut status, 4_000, 50);
    assert_eq!(core.markers.len(), 1, "going low marks nothing");
    fr_fire(&mut core, &mut status, 5_000, 150);
    assert_eq!(
        core.markers,
        vec![2_000, 5_000],
        "and the next crossing is a fresh edge"
    );

    // Another cluster's slot 12 is not this rule's slot 12, and a rule
    // naming a signal the description lacks never fires.
    let mut other = fr_row(6_000, 12);
    other.bus = 1;
    other.ab = 0;
    other.cycle = 0;
    other.payload = vec![200, 0, 0, 0, 0, 0, 0, 0];
    core.eval_fr_triggers(&other, &mut status);
    assert_eq!(core.markers.len(), 2, "the other bus is not this one's");
    core.triggers.push(crate::trigger::Trigger::new(
        crate::trigger::TriggerCond::FrSignalCross {
            bus: 0,
            slot: 12,
            signal: "Absent".into(),
            threshold: 0.0,
            rising: true,
        },
        crate::trigger::TriggerAction::InsertMarker,
    ));
    fr_fire(&mut core, &mut status, 7_000, 200);
    assert_eq!(
        core.markers.len(),
        2,
        "an undecodable signal holds the level and never fires"
    );
}

/// An Ethernet frame matched to its ARXML frame definition decodes into
/// the same subscription space as CAN and FlexRay, keyed by the Ethernet
/// bus and the frame the matrix matched -- so the observers plot it.
#[test]
fn someip_catalog_is_adopted_as_comm_services() {
    let text = std::fs::read_to_string(crate::test_assets::eth_arxml()).unwrap();
    let db = std::sync::Arc::new(crate::eth::db::EthDb::parse(&text).unwrap());
    let mut core = BusCore::new(Vec::new());
    let mut dbs = std::collections::BTreeMap::new();
    dbs.insert(0u8, db);
    core.handle(
        BusCommand::SetEthDbs {
            dbs,
            sources: Default::default(),
        },
        &mut String::new(),
    );
    core.handle(
        BusCommand::SetSomeIpServices(crate::eth::arxml_service::ServiceCatalog {
            services: vec![crate::eth::arxml_service::ServiceDef {
                name: "SpeedService".to_string(),
                id: 0x1234,
                major: 1,
                methods: Vec::new(),
                events: vec![crate::eth::arxml_service::ServiceEvent {
                    name: "Speed".to_string(),
                    id: 0x8001,
                    fields: Vec::new(),
                }],
                instances: Vec::new(),
                buses: vec![0],
                issues: Vec::new(),
            }],
        }),
        &mut String::new(),
    );
    let runtime = core.eth_models.get(&0).unwrap();
    let cluster = *runtime.comm.clusters.keys().next().unwrap();
    let service = runtime
        .comm
        .service(cluster, 0x1234)
        .expect("service became a Comm entity");
    assert_eq!(service.name, "SpeedService");
    assert_eq!(service.events[0].id, 0x8001);
}

#[test]
fn comm_resolves_dbc_node_to_ecu_identity() {
    let mut comm = crate::model::Comm::default();
    comm.ecus.insert(
        crate::model::EcuId(7),
        crate::model::Ecu {
            id: crate::model::EcuId(7),
            name: "Gateway".to_string(),
            description: String::new(),
            mac: None,
        },
    );
    assert_eq!(comm.ecu_for_node("gateway"), Some(crate::model::EcuId(7)));
    assert_eq!(comm.ecu_for_node("missing"), None);
}

#[test]
fn ethernet_frames_fold_signals_into_subscriptions() {
    let mut core = BusCore::new(Vec::new());
    let db = crate::eth::db::EthDb {
        clusters: vec![crate::eth::db::EthCluster {
            name: "Cluster".into(),
            vlans: vec![100],
        }],
        frames: vec![crate::eth::db::EthFrameDef {
            name: "SpeedFrame".into(),
            cluster: "Cluster".into(),
            vlan: Some(100),
            length: 8,
            ethertype_hint: Some(0x0800),
            pdus: vec![crate::eth::db::EthPdu {
                name: "SpeedPdu".into(),
                length: 8,
                start_bit: 0,
                signals: vec![crate::eth::db::EthSignal {
                    name: "Speed".into(),
                    start_bit: 0,
                    size: 8,
                    big_endian: false,
                    signed: false,
                    factor: 2.0,
                    offset: 1.0,
                    unit: "km/h".into(),
                    values: vec![],
                }],
            }],
        }],
        ..Default::default()
    };
    core.eth_dbs.insert(0, std::sync::Arc::new(db));
    core.rebuild_eth_models();
    let key = core.eth_key(0, "SpeedFrame".to_string(), "Speed".to_string());
    core.subscribe_signal(key.clone());

    core.ingest_eth(crate::eth::EthFrame::new(
        1_000,
        0,
        crate::can::frame::Direction::Rx,
        [9, 8, 7, 6, 5, 4],
        [1, 2, 3, 4, 5, 6],
        Some(100),
        0x0800,
        vec![42, 0, 0, 0, 0, 0, 0, 0],
    ));

    let sub = core.subs.get(&key).expect("Ethernet signal subscribed");
    assert_eq!(sub.latest, 85.0, "factor and offset applied");
    assert_eq!(sub.last_raw, 42);
    assert_eq!(sub.unit, "km/h", "unit carried from the matrix");
    assert_eq!(sub.type_tag, "u8");
    assert_eq!(sub.history.len(), 1, "sampled like a CAN signal");
}

/// A matrix whose `FRAME-LENGTH` includes the 14-byte L2 header matches
/// the captured payload once the project says so, and decodes the same
/// way; the default (payload length) does not bind.
#[test]
fn the_frame_length_convention_decides_whether_a_flow_matches() {
    let db = crate::eth::db::EthDb {
        clusters: vec![crate::eth::db::EthCluster {
            name: "Cluster".into(),
            vlans: vec![100],
        }],
        frames: vec![crate::eth::db::EthFrameDef {
            name: "LenFrame".into(),
            cluster: "Cluster".into(),
            vlan: Some(100),
            // 8 payload bytes + 14-byte L2 header.
            length: 22,
            ethertype_hint: Some(0x0800),
            pdus: vec![crate::eth::db::EthPdu {
                name: "P".into(),
                length: 8,
                start_bit: 0,
                signals: vec![crate::eth::db::EthSignal {
                    name: "Speed".into(),
                    start_bit: 0,
                    size: 8,
                    big_endian: false,
                    signed: false,
                    factor: 1.0,
                    offset: 0.0,
                    unit: "km/h".into(),
                    values: vec![],
                }],
            }],
        }],
        ..Default::default()
    };
    let key = SigKey::eth_in(crate::model::ClusterId(1), "LenFrame", "Speed");
    let run = |includes_header: bool| {
        let mut core = BusCore::new(Vec::new());
        core.eth_dbs.insert(0, std::sync::Arc::new(db.clone()));
        core.eth_length_includes_header = includes_header;
        core.rebuild_eth_models();
        core.subscribe_signal(key.clone());
        core.ingest_eth(crate::eth::EthFrame::new(
            1_000,
            0,
            crate::can::frame::Direction::Rx,
            [9, 8, 7, 6, 5, 4],
            [1, 2, 3, 4, 5, 6],
            Some(100),
            0x0800,
            vec![42, 0, 0, 0, 0, 0, 0, 0],
        ));
        core
    };
    let unmatched = run(false);
    let sub = unmatched.subs.get(&key).expect("subscribed");
    assert_eq!(sub.latest, 0.0);
    assert_eq!(
        sub.last_update_us, 0,
        "the payload-length convention does not bind a header-inclusive definition"
    );
    assert_eq!(
        run(true).subs.get(&key).expect("matched").latest,
        42.0,
        "with the convention set, the same frame decodes"
    );
}

/// Two clusters, two descriptions, the same slot number: each bus decodes
/// against its own file. Merging them into one description -- which is what
/// a single global database amounted to -- would hand bus 1's frames bus 0's
/// signal names, or decode to nothing.
#[test]
fn each_flexray_bus_decodes_against_its_own_description() {
    let mut core = BusCore::new(Vec::new());
    core.fr_dbs.insert(0, fr_db_one_signal(12, "Speed", "km/h"));
    core.fr_dbs.insert(1, fr_db_one_signal(12, "Torque", "Nm"));
    core.rebuild_fr_models();
    let on0 = core.fr_key(0, 12, "Speed".to_string());
    let on1 = core.fr_key(1, 12, "Torque".to_string());
    // The names that belong to the *other* cluster: subscribing them
    // proves no row leaks across buses.
    let wrong0 = core.fr_key(0, 12, "Torque".to_string());
    let wrong1 = core.fr_key(1, 12, "Speed".to_string());
    for key in [&on0, &on1, &wrong0, &wrong1] {
        core.subscribe_signal(key.clone());
    }

    for bus in [0u8, 1u8] {
        let mut row = fr_row(1_000, 12);
        row.bus = bus;
        row.ab = 0;
        row.cycle = 0;
        row.payload = vec![7, 0, 0, 0, 0, 0, 0, 0];
        core.ingest_fr_row(row);
    }

    assert_eq!(core.subs.get(&on0).expect("sub").latest, 7.0);
    assert_eq!(core.subs.get(&on1).expect("sub").latest, 7.0);
    assert_eq!(core.subs.get(&on0).expect("sub").unit, "km/h");
    assert_eq!(core.subs.get(&on1).expect("sub").unit, "Nm");
    assert_eq!(
        core.subs.get(&wrong0).expect("sub").latest,
        0.0,
        "bus 0 has no Torque signal to fold"
    );
    assert_eq!(
        core.subs.get(&wrong1).expect("sub").latest,
        0.0,
        "bus 1 has no Speed signal to fold"
    );
}

/// The FR ring publishes through the same lifecycle as the CAN ring:
/// rows stamp against the sim clock when the source had no time of
/// its own, and the snapshot carries the published view.
#[test]
fn fr_rows_ingest_publish_and_stamp() {
    let mut core = BusCore::new(Vec::new());
    core.sim_t_us = 1_500;
    core.ingest_fr_row(fr_row(0, 10));
    core.ingest_fr_row(fr_row(2_000, 11));
    core.publish_trace();

    let snap = core.snapshot();
    assert_eq!(snap.fr_trace.len(), 2);
    assert_eq!(
        snap.fr_trace[0].t_us, 1_500,
        "a zero time stamps against the sim clock"
    );
    assert_eq!(snap.fr_trace[1].t_us, 2_000, "a real time survives");
    assert_eq!(snap.fr_trace[1].slot, 11);
    assert_eq!(snap.fr_dropped, 0);
    assert_eq!(
        core.frame_counter, 2,
        "a FlexRay arrival counts like any other traffic"
    );

    // A clear resets both ring and view.
    core.fr_trace.clear();
    core.publish_trace();
    assert!(core.snapshot().fr_trace.is_empty());
}

/// The FR ring shares the trace limit: rows past it trim head-first
/// and the loss is counted for the window's note.
#[test]
fn fr_rows_honor_the_trace_limit() {
    let mut core = BusCore::new(Vec::new());
    core.trace_limit = 3;
    for slot in 1..=6u16 {
        core.ingest_fr_row(fr_row(0, slot));
    }
    core.publish_trace();
    let snap = core.snapshot();
    assert_eq!(snap.fr_trace.len(), 3);
    assert_eq!(snap.fr_trace[0].slot, 4, "the newest survive");
    assert_eq!(snap.fr_dropped, 3);
}

/// A watch attach with a missing description file fails cleanly and
/// leaves nothing attached; detaching without a watch is a no-op.
#[test]
fn fr_watch_attach_failures_leave_no_partial_state() {
    let mut core = BusCore::new(Vec::new());
    let mut status = String::new();
    core.handle(
        BusCommand::SetFrWatch {
            bus: 0,
            channel_index: Some(0),
            fibex_path: "target/definitely-missing.fibex".into(),
        },
        &mut status,
    );
    assert!(
        status.contains("failed"),
        "attach failure reports: {status}"
    );
    assert!(core.hw.fr_watches.is_empty());

    core.handle(
        BusCommand::SetFrWatch {
            bus: 0,
            channel_index: None,
            fibex_path: String::new(),
        },
        &mut status,
    );
    assert!(status.contains("detached"));
    assert!(core.hw.fr_watches.is_empty());
}

/// The watch configures itself from the database pushed with `SetFrDbs`, so
/// a request for a bus with no description of its own -- or after the one
/// the core held was cleared -- is refused without a driver call.
/// Description *validity* (a DBC is no cluster spec) is now the frontend's
/// gate: `FrDb::parse` there rejects it before anything reaches the bus.
#[test]
fn fr_watch_requires_a_description_the_core_holds() {
    let mut core = BusCore::new(Vec::new());
    let mut status = String::new();
    let watch = || BusCommand::SetFrWatch {
        bus: 0,
        channel_index: Some(0),
        fibex_path: "test.dbc".into(),
    };
    core.handle(watch(), &mut status);
    assert!(
        status.contains("description not loaded"),
        "no description pushed yet: {status}"
    );

    let bytes = std::fs::read(crate::test_assets::path("PowerTrain.arxml"));
    if let Ok(bytes) = bytes {
        let text = crate::dbc::text_from_bytes(bytes);
        let db = std::sync::Arc::new(crate::fr_db::FrDb::parse(&text).expect("parses"));
        let mut dbs = std::collections::BTreeMap::new();
        // The description belongs to bus 1; asking for a watch on bus 0 is
        // still a bus with no description.
        dbs.insert(1u8, db);
        core.handle(
            BusCommand::SetFrDbs {
                dbs,
                sources: Default::default(),
            },
            &mut status,
        );
        core.handle(watch(), &mut status);
        assert!(
            status.contains("description not loaded"),
            "another bus's description does not count: {status}"
        );
        core.handle(
            BusCommand::SetFrDbs {
                dbs: Default::default(),
                sources: Default::default(),
            },
            &mut status,
        );
        core.handle(watch(), &mut status);
        assert!(
            status.contains("description not loaded"),
            "a cleared description is refused too: {status}"
        );
    }
    assert!(core.hw.fr_watches.is_empty(), "a refusal leaves no watch");
}

// ---- 回放通道映射 ----

/// CAN signal subscriptions decode through the unified model plan.
#[test]
fn can_signal_subscriptions_use_the_unified_decode_plan() {
    let mut core = BusCore::new(Vec::new());
    let mut status = String::new();
    core.add_channel(&mut status);
    assert!(
        core.model_comms[0].is_some(),
        "loading a DBC builds the unified model"
    );
    assert!(core.model_plans[0].is_some());
    assert!(
        core.model_dbs[0].is_some(),
        "the model records the database it was built from"
    );

    let key = SigKey::can_in(crate::model::ClusterId(1), 0x100, false, "EngineSpeed");
    core.subscribe_signal(key.clone());
    let mut frame = frame_on(0, 0x100);
    frame.data[0] = 0x80;
    frame.data[1] = 0x3E;
    assert!(
        core.model_can_values(&frame).is_some(),
        "ordinary DBC frames take the model path"
    );
    core.ingest(frame, 1_000);
    let latest = core.subs.get(&key).expect("subscribed").latest;
    assert!((latest - 4000.0).abs() < 1e-9, "{latest}");
}

/// FR and Ethernet Panel physical sends use the same unified encoder.
#[test]
fn fr_and_eth_panel_payloads_use_the_unified_encoder() {
    let mut core = BusCore::new(Vec::new());
    let mut dbs = std::collections::BTreeMap::new();
    dbs.insert(0u8, fr_db_one_signal(12, "Speed", "km/h"));
    core.handle(
        BusCommand::SetFrDbs {
            dbs,
            sources: Default::default(),
        },
        &mut String::new(),
    );
    let mut values = std::collections::BTreeMap::new();
    values.insert("Speed".to_string(), 80.0);
    let payload = core
        .model_fr_signal_payload(0, "F12", &values)
        .expect("model knows the FR frame")
        .expect("FR encoding succeeds");
    assert_eq!(payload[0], 80);

    let text = std::fs::read_to_string(crate::test_assets::eth_arxml()).unwrap();
    let db = std::sync::Arc::new(crate::eth::db::EthDb::parse(&text).unwrap());
    let mut dbs = std::collections::BTreeMap::new();
    dbs.insert(0u8, db);
    core.handle(
        BusCommand::SetEthDbs {
            dbs,
            sources: Default::default(),
        },
        &mut String::new(),
    );
    let mut values = std::collections::BTreeMap::new();
    values.insert("Sig".to_string(), 5.0);
    let payload = core
        .model_eth_signal_payload(0, "EthFrame", &values)
        .expect("model knows the ETH frame")
        .expect("ETH encoding succeeds");
    assert_eq!(payload.len(), 22);
    assert_eq!(payload[1], 50);
}

/// Panel physical sends use the same model plan as subscriptions.
#[test]
fn panel_physical_can_payload_uses_the_unified_encoder() {
    let mut core = BusCore::new(Vec::new());
    let mut status = String::new();
    core.add_channel(&mut status);
    let mut values = std::collections::BTreeMap::new();
    values.insert("EngineSpeed".to_string(), 4000.0);
    let payload = core
        .model_can_signal_payload(0, 0x100, false, &values)
        .expect("the model knows the message")
        .expect("encoding succeeds");
    assert_eq!(&payload[..2], &[0x80, 0x3E]);
    let mut unknown = std::collections::BTreeMap::new();
    unknown.insert("NoSuchSignal".to_string(), 1.0);
    assert!(
        core.model_can_signal_payload(0, 0x100, false, &unknown)
            .expect("model knows the message")
            .is_err()
    );
}

/// Cluster ids follow the channel row through removal and are never
/// reused for a new bus.
#[test]
fn channel_cluster_ids_survive_removal_without_reuse() {
    let mut core = BusCore::new(Vec::new());
    let mut status = String::new();
    core.add_channel(&mut status);
    core.add_channel(&mut status);
    let ids = core.can_cluster_ids.clone();
    assert_eq!(ids.len(), 2);
    assert!(ids[0] < ids[1]);
    core.remove_channel(0, &mut status);
    assert_eq!(core.can_cluster_ids, vec![ids[1]]);
    core.add_channel(&mut status);
    assert_eq!(core.can_cluster_ids.len(), 2);
    assert!(core.can_cluster_ids[1] > ids[1]);
}

/// Expanded Messages rows carry model-decoded children in the snapshot.
#[test]
fn expanded_messages_are_decoded_in_the_core_snapshot() {
    let mut core = BusCore::new(Vec::new());
    let mut status = String::new();
    core.add_channel(&mut status);
    let mut frame = frame_on(0, 0x100);
    frame.data[0] = 0x80;
    frame.data[1] = 0x3E;
    core.ingest(frame, 1_000);
    let key = "can:0:256:false".to_string();
    core.handle(
        BusCommand::SetExpandedMessages(vec![key.clone()]),
        &mut status,
    );
    let snap = core.snapshot();
    let rows = snap
        .expanded_signals
        .get(&key)
        .expect("expanded row decoded");
    assert!(
        rows.iter()
            .any(|row| row.name == "EngineSpeed" && row.value == "4000"),
        "{rows:?}"
    );
}

/// 帧只带通道编号；`t_us = 0` 让第一次 step 就把它们发完。
fn frame_on(channel: u8, id: u32) -> crate::can::frame::CanFrame {
    crate::can::frame::CanFrame {
        t_us: 0,
        channel,
        id,
        extended: false,
        len: 8,
        data: [0; crate::can::frame::MAX_CAN_FD_LEN],
        dir: crate::can::frame::Direction::Rx,
        flags: crate::can::frame::FrameFlags::NONE,
    }
}

/// 一条总线、一段日志。日志自带的编号换成本工程的第几路，回放时钟照常推进。
fn replay_core(
    frames: Vec<crate::can::frame::CanFrame>,
    rows: Vec<crate::trace::FrRow>,
) -> BusCore {
    let mut core = BusCore::new(Vec::new());
    let mut status = String::new();
    core.add_channel(&mut status);
    core.mode = Mode::Replay;
    core.source = Box::new(crate::source::replay::ReplaySource::new(Box::new(
        crate::log::vec_stream::VecStream::with_fr(frames, rows),
    )));
    core
}

/// 别人的日志里通道编号是他工程里的，文件里又不带总线名。编号在本工程
/// 对应不上一条路时不冒充"第 n 路"——那种行会让人以为总线对上了，
/// 而它其实挂的是别的描述文件。它停在映射表里，条数照算。
#[test]
fn a_log_channel_with_no_bus_here_is_not_replayed() {
    let mut core = replay_core(vec![frame_on(0, 0x100), frame_on(1, 0x200)], Vec::new());
    let mut status = String::new();
    core.step_to(0, 0, 1, 1, &mut status);
    core.publish_trace();
    let snap = core.snapshot();
    assert_eq!(snap.aggs.len(), 1, "只有能对上的一路进得来");
    assert_eq!(snap.aggs[0].id, 0x100);
    assert!(
        status.contains("log channel 2"),
        "被挡在门外的那一路要说一句，不能只让人发现 Trace 少了东西: {status}"
    );
    assert_eq!(
        snap.replay_census_can,
        vec![(0, 1), (1, 1)],
        "被挡掉的那一路在映射表里数得出来"
    );
}

#[test]
fn pointing_a_log_channel_at_a_bus_replays_it_there() {
    use crate::replay_map::LogBusKind;
    let mut core = replay_core(vec![frame_on(0, 0x100), frame_on(1, 0x200)], Vec::new());
    let mut map = crate::replay_map::ReplayMap::default();
    map.set(LogBusKind::Can, 1, core.cluster_of_bus(LogBusKind::Can, 0));
    core.replay_map = map;
    let mut status = String::new();
    core.step_to(0, 0, 1, 1, &mut status);
    core.publish_trace();
    let snap = core.snapshot();
    assert_eq!(snap.aggs.len(), 2, "两条都落在唯一的那一路上");
    assert!(
        snap.aggs.iter().all(|a| a.channel == 0),
        "日志的第 2 路现在就是这里的第 1 路"
    );
    // 计数按文件里的编号记，映射改的是归属，不是"这条帧是谁"。
    assert_eq!(snap.replay_census_can, vec![(0, 1), (1, 1)]);
}

/// 映射是回放中途也能改的：下一条命令被抽干，接下来的帧就按新表走。
#[test]
fn a_mapping_change_applies_to_the_frames_after_it() {
    use crate::replay_map::LogBusKind;
    let mut core = replay_core(Vec::new(), Vec::new());
    let mut status = String::new();
    let mut map = crate::replay_map::ReplayMap::default();
    map.set(LogBusKind::Can, 1, core.cluster_of_bus(LogBusKind::Can, 0));
    core.handle(BusCommand::SetReplayMap(map), &mut status);
    assert_eq!(
        core.replay_map.target(LogBusKind::Can, 1),
        crate::replay_map::ReplayTarget::Cluster(core.cluster_of_bus(LogBusKind::Can, 0).unwrap())
    );
}

/// 日志静音集合存的是文件编号，喂给生成器的那两份按映射推。改了映射
/// 不必把文件重读一遍——这是同一趟扫描的结果换一套编号而已。
#[test]
fn the_generator_mute_set_follows_the_map() {
    use crate::replay_map::LogBusKind;
    let mut core = BusCore::new(Vec::new());
    let mut status = String::new();
    core.add_channel(&mut status);
    core.add_channel(&mut status);
    core.log_ids_file = [(1u8, 0x100u32)].into_iter().collect();
    core.log_fr_ids_file = [(2u8, 10u16)].into_iter().collect();
    let mut map = crate::replay_map::ReplayMap::default();
    map.set(LogBusKind::Can, 1, core.cluster_of_bus(LogBusKind::Can, 0));
    map.set(LogBusKind::Fr, 2, None);
    core.replay_map = map;
    core.remap_log_ids();
    assert!(core.replay_ids.contains(&(0, 0x100)));
    assert!(!core.replay_ids.contains(&(1, 0x100)), "按新编号静音");
    assert!(
        core.replay_fr_ids.is_empty(),
        "不回放的那一路没有帧进环，条目也不该被静音"
    );
}

#[test]
fn a_flexray_cluster_moves_onto_the_bus_the_operator_chose() {
    use crate::replay_map::LogBusKind;
    // `fr_row` 造的 bus 是 0；这里要的是"日志里第 1 号 cluster"。
    let mut row = fr_row(0, 10);
    row.bus = 1;
    let mut core = replay_core(Vec::new(), vec![row]);
    core.fr_cluster_ids.insert(0, core.cluster_ids.allocate());
    let mut map = crate::replay_map::ReplayMap::default();
    map.set(LogBusKind::Fr, 1, core.cluster_of_bus(LogBusKind::Fr, 0));
    core.replay_map = map;
    let mut status = String::new();
    core.step_to(0, 0, 1, 1, &mut status);
    core.publish_trace();
    core.publish_loads();
    let snap = core.snapshot();
    assert_eq!(snap.fr_trace.len(), 1, "指过来的 cluster 照样进环");
    assert!(
        snap.fr_loads.contains_key(&0),
        "占用率算在被指过去的那一路上"
    );
    assert_eq!(
        snap.replay_census_fr,
        vec![(1, 1)],
        "表上说的是日志里的编号，不是本工程的路"
    );
    assert!(!snap.fr_loads.contains_key(&1), "1 号那路没被凭空造出来");
}

/// An Ethernet capture's channel is the recording tool's numbering too;
/// the map moves it onto the project's Ethernet bus, and the census keeps
/// counting by the file's number. An explicit "don't replay" drops it.
#[test]
fn an_ethernet_capture_channel_follows_the_replay_map() {
    use crate::replay_map::LogBusKind;
    let frame = |bus: u8| {
        crate::eth::EthFrame::new(
            0,
            bus,
            crate::can::frame::Direction::Rx,
            [9, 8, 7, 6, 5, 4],
            [1, 2, 3, 4, 5, 6],
            Some(100),
            0x0800,
            vec![0xAA; 8],
        )
    };
    let run = |mapping: Option<u8>| {
        let mut core = BusCore::new(Vec::new());
        core.mode = Mode::Replay;
        // The targets are clusters; give the project an Ethernet bus 2 so
        // the chosen cluster has somewhere to resolve to.
        core.eth_cluster_ids.insert(1, crate::model::ClusterId(1));
        core.eth_cluster_ids.insert(2, crate::model::ClusterId(2));
        core.source = Box::new(crate::source::replay::ReplaySource::new(Box::new(
            crate::log::vec_stream::VecStream::with_all(Vec::new(), Vec::new(), vec![frame(1)]),
        )));
        let mut map = crate::replay_map::ReplayMap::default();
        map.set(
            LogBusKind::Eth,
            1,
            mapping.map(|bus| crate::model::ClusterId(u32::from(bus))),
        );
        core.replay_map = map;
        let mut status = String::new();
        core.step_to(0, 0, 1, 1, &mut status);
        core.publish_trace();
        core.snapshot()
    };

    let snap = run(Some(2));
    assert_eq!(snap.eth_trace.len(), 1, "the frame reached the ring");
    assert_eq!(snap.eth_trace.last().unwrap().bus, 2, "on the chosen bus");
    assert_eq!(
        snap.replay_census_eth,
        vec![(1, 1)],
        "the map lists the file's channel, not the project's bus"
    );

    let dropped = run(None);
    assert!(
        dropped.eth_trace.is_empty(),
        "an explicit drop replays nothing"
    );
    assert_eq!(
        dropped.replay_census_eth,
        vec![(1, 1)],
        "and the mapping table still reports how much was dropped"
    );
}

/// CAN 的一路删掉，后面的往前挪：指到被删那一路的映射变成不回放，指到
/// 它后面的跟着挪一号。留着行，映射表就会自己说出少了哪一路。
#[test]
fn deleting_a_bus_re_aims_the_map_instead_of_silently_unsetting_it() {
    use crate::replay_map::LogBusKind;
    let mut core = BusCore::new(Vec::new());
    let mut status = String::new();
    core.add_channel(&mut status);
    core.add_channel(&mut status);
    core.add_channel(&mut status);
    let mut map = crate::replay_map::ReplayMap::default();
    let middle = core.cluster_of_bus(LogBusKind::Can, 1).unwrap();
    let last = core.cluster_of_bus(LogBusKind::Can, 2).unwrap();
    map.set(LogBusKind::Can, 4, Some(middle));
    map.set(LogBusKind::Can, 5, Some(last));
    core.replay_map = map;
    // 删掉中间那一路（下标 1）：指向它的通道变成不回放，指向最后一
    // 路的表项仍然指着同一个 cluster，只是那个 cluster 现在排在前一格。
    core.remove_channel(1, &mut status);
    assert_eq!(
        core.replay_map.target(LogBusKind::Can, 4),
        crate::replay_map::ReplayTarget::Drop
    );
    assert_eq!(
        core.replay_map.target(LogBusKind::Can, 5),
        crate::replay_map::ReplayTarget::Cluster(last)
    );
    assert_eq!(core.replay_bus(LogBusKind::Can, 5), Some(1));
}

/// A `frame(...)` send fills unwritten signals from the database's
/// declared start value, in physical units; the explicit field wins.
#[test]
fn frame_sends_start_from_the_declared_initial_values() {
    let dbc = r#"VERSION ""
NS_ :

BU_: ECU

BO_ 512 Status: 8 ECU
 SG_ RPM : 0|16@1+ (0.25,10) [0|255] "rpm" ECU
 SG_ Temp : 16|8@1+ (1,0) [0|255] "degC" ECU

BA_DEF_ SG_ "GenSigStartValue" FLOAT 0 100000;
BA_ "GenSigStartValue" SG_ 512 RPM 4000;
BA_ "GenSigStartValue" SG_ 512 Temp 80;
"#;
    let db = std::sync::Arc::new(crate::dbc::load_dbc_str(dbc).expect("DBC parses"));
    let mut core = BusCore::new(Vec::new());
    core.add_channel_with_dbc("CAN1".to_string(), db, 500, &mut String::new());
    let mut frame = crate::script::FrameInstance::new("Status");
    frame.values.insert("Temp".to_string(), 20.0);
    core.send_script_can_frame(0, 0, "Status", false, &frame)
        .expect("the frame encodes");
    let out = core.buf.last().expect("one frame queued");
    assert_eq!(
        out.payload(),
        [0xA0, 0x0F, 20, 0, 0, 0, 0, 0],
        "RPM keeps raw 4000 (physical 1010), Temp is the explicit 20"
    );
}

/// One CAN bus can carry the same signal name in two messages; the
/// `Bus:Frame:Signal` form picks one, while `Bus:Signal` stays ambiguous.
#[test]
fn frame_qualified_derived_refs_disambiguate_can_ids() {
    let db = std::sync::Arc::new(
        crate::dbc::load_dbc_str(crate::sample_data::SAMPLE_DBC).expect("sample DBC"),
    );
    let mut core = BusCore::new(Vec::new());
    core.add_channel_with_dbc("CAN1".to_string(), db.clone(), 500, &mut String::new());
    core.add_channel_with_dbc("CAN2".to_string(), db, 500, &mut String::new());
    let keys = vec![
        core.can_key(1, 0x0c7, false, "WheelSpeedFR".to_string()),
        core.can_key(1, 0x0c8, false, "WheelSpeedFR".to_string()),
    ];
    let own = SigKey::derived("x", "self");
    assert_eq!(
        resolve_derived_ref("CAN2:0x0C7:WheelSpeedFR", &keys, &core, &own),
        Some(keys[0].clone())
    );
    assert_eq!(
        resolve_derived_ref("CAN2:0x0c8:WheelSpeedFR", &keys, &core, &own),
        Some(keys[1].clone())
    );
    assert_eq!(
        resolve_derived_ref("CAN2:WheelSpeedFR", &keys, &core, &own),
        None
    );
}

/// The frame qualifier also accepts the DBC message name, not only the
/// hex id.
#[test]
fn derived_refs_accept_message_names() {
    let db = std::sync::Arc::new(
        crate::dbc::load_dbc_str(crate::sample_data::SAMPLE_DBC).expect("sample DBC"),
    );
    let mut core = BusCore::new(Vec::new());
    core.add_channel_with_dbc("CAN1".to_string(), db, 500, &mut String::new());
    let keys = vec![SigKey::can_in(
        crate::model::ClusterId(1),
        0x100,
        false,
        "EngineSpeed",
    )];
    let own = SigKey::derived("x", "self");
    assert_eq!(
        resolve_derived_ref("CAN1:EngineStatus:EngineSpeed", &keys, &core, &own),
        Some(keys[0].clone())
    );
    assert_eq!(
        resolve_derived_ref("CAN1:0x100:EngineSpeed", &keys, &core, &own),
        Some(keys[0].clone())
    );
}

#[test]
fn e2e_counter_deltas_separate_repeats_gaps_and_jumps() {
    let mut view = E2eView::default();
    for counter in [1u8, 2, 2, 5, 9] {
        note_e2e(
            &mut view,
            crate::model::E2eReading {
                profile: 1,
                counter,
                data_id: Some(7),
                crc: None,
                crc_ok: None,
            },
        );
    }
    assert_eq!(view.frames, 5);
    assert_eq!(view.counter, Some(9));
    assert_eq!(view.data_id, Some(7));
    assert_eq!(view.repeats, 1, "2 followed by 2 is a repeat");
    assert_eq!(view.gaps, 1, "delta 3 is within Profile 1's tolerance");
    assert_eq!(view.jumps, 1, "delta 4 is a jump");
}

#[test]
fn a_canonical_key_binds_its_live_bus() {
    let mut core = BusCore::new(Vec::new());
    let db = std::sync::Arc::new(
        crate::dbc::load_dbc_str(crate::sample_data::SAMPLE_DBC).expect("sample DBC"),
    );
    core.add_channel_with_dbc("CAN1".to_string(), db, 500, &mut String::new());
    let cluster = core.can_cluster_ids[0];
    let canonical = format!("sig:can:{}:256:false:EngineSpeed", cluster.0);
    let key = core.resolve_key_text(&canonical).expect("canonical key");
    assert_eq!(key.text(), canonical, "cluster survives the round trip");
    assert_eq!(key.cluster(), Some(cluster));
    assert!(
        core.resolve_key_text("0:256:EngineSpeed").is_none(),
        "bus-numbered text is gone: there are no v1 keys to upgrade"
    );
}

#[test]
fn flexray_e2e_counters_are_tracked_from_the_plan() {
    use crate::model::*;
    let mut core = BusCore::new(Vec::new());
    let db = fr_db_one_signal(12, "Speed", "km/h");
    let frame_name = db.frame_index(0).expect("a frame").name.clone();
    core.fr_dbs.insert(0, db);
    core.rebuild_fr_models();
    let mut comm = Comm::default();
    comm.clusters.insert(
        ClusterId(1),
        Cluster {
            id: ClusterId(1),
            kind: ClusterKind::FlexRay,
            name: "FR0".to_string(),
            aliases: Vec::new(),
            description: String::new(),
            sources: Vec::new(),
            networks: vec![NetworkId(1)],
        },
    );
    comm.networks.insert(
        NetworkId(1),
        Network {
            id: NetworkId(1),
            cluster: ClusterId(1),
            name: "FR0".to_string(),
            params: NetworkParams::FlexRay(FlexRayParams {
                speed_kbps: 10_000,
                cycle_time_ms: 5.0,
                macrotick_us: 0.0,
                coldstart_attempts: 0,
                action_point_offset: 0,
                minislot_action_point_offset: 0,
                dynamic_slot_idle_phase: 0,
                minor_version: 0,
                extra: std::collections::BTreeMap::new(),
            }),
        },
    );
    comm.pdus.insert(
        PduId(1),
        Pdu {
            id: PduId(1),
            name: "P".to_string(),
            length: 8,
            kind: PduKind::Signal,
            header_type: ContainerHeaderType::Short,
            cycle_us: None,
            selector: None,
            secured: None,
            transformation: None,
            e2e: Some(E2eProtectionId(1)),
        },
    );
    comm.e2e.insert(
        E2eProtectionId(1),
        E2eProtection {
            id: E2eProtectionId(1),
            name: "E".to_string(),
            profile_name: "Profile1".to_string(),
            profile: Some(1),
            data_id: Some(7),
            system_signal: None,
            payload: None,
            counter_offset: None,
            counter_length: None,
            crc_offset: None,
            crc_length: None,
        },
    );
    comm.frame_pdu_mappings.insert(
        MappingId(1),
        FramePduMapping {
            id: MappingId(1),
            frame: FrameId(1),
            pdu: PduId(1),
            start_bit: 0,
            byte_order: ByteOrder::LittleEndian,
        },
    );
    comm.frames.insert(
        FrameId(1),
        Frame {
            id: FrameId(1),
            cluster: ClusterId(1),
            network: NetworkId(1),
            name: frame_name,
            length: 8,
            address: FrameAddress::FlexRay {
                slot: 12,
                base_cycle: 0,
                repetition: 1,
                payload_preamble: false,
            },
            timing: FrameTiming::default(),
            comment: String::new(),
            ethernet: None,
            pdu_mappings: vec![MappingId(1)],
        },
    );
    core.set_fr_model(0, std::sync::Arc::new(comm));
    for (t, counter) in [(1_000u64, 0u8), (2_000, 1), (3_000, 1)] {
        let mut row = fr_row(t, 12);
        row.payload[1] = counter;
        core.ingest_fr_row(row);
    }
    let view = core.fr_e2e_tracks.get(&(0, 12)).expect("FR E2E tracked");
    assert_eq!(view.frames, 3);
    assert_eq!(view.counter, Some(1));
    assert_eq!(view.repeats, 1);
}
