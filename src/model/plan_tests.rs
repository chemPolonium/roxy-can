use super::*;

fn signal(id: u64, pdu: u64) -> Signal {
    Signal {
        id: SignalId(id),
        pdu: PduId(pdu),
        name: format!("S{id}"),
        system_signal: None,
        length: 16,
        start_bit: 0,
        byte_order: ByteOrder::LittleEndian,
        update_bit: None,
        signed: false,
        float: false,
        factor: 1.0,
        offset: 0.0,
        unit: String::new(),
        min: None,
        max: None,
        min_open: false,
        max_open: false,
        initial_raw: None,
        values: Vec::new(),
        base_type: None,
        compu: None,
        comment: String::new(),
        transformation: None,
        group: None,
        mux: Vec::new(),
    }
}

fn pdu(id: u64, kind: PduKind) -> Pdu {
    Pdu {
        id: PduId(id),
        name: format!("P{id}"),
        length: 8,
        kind,
        header_type: ContainerHeaderType::Short,
        cycle_us: None,
        selector: None,
        secured: None,
        transformation: None,
        e2e: None,
    }
}

fn frame(id: u64, _pdu: u64) -> Frame {
    Frame {
        id: FrameId(id),
        cluster: ClusterId(1),
        network: NetworkId(1),
        name: format!("F{id}"),
        length: 8,
        address: FrameAddress::Can {
            id: id as u32,
            extended: false,
            fd: false,
        },
        timing: FrameTiming::default(),
        ethernet: None,
        pdu_mappings: vec![MappingId(id)],
    }
}

fn fixture() -> Comm {
    let mut comm = Comm::default();
    comm.clusters.insert(
        ClusterId(1),
        Cluster {
            id: ClusterId(1),
            kind: ClusterKind::Can,
            name: "CAN1".to_string(),
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
            name: "CAN1".to_string(),
            params: NetworkParams::Can(CanParams::default()),
        },
    );

    // Plain signal PDU at bit 8.
    comm.pdus.insert(PduId(10), pdu(10, PduKind::Signal));
    comm.signals.insert(SignalId(100), signal(100, 10));
    comm.signal_pdu_mappings.insert(
        MappingId(100),
        SignalPduMapping {
            id: MappingId(100),
            signal: SignalId(100),
            pdu: PduId(10),
            start_bit: 0,
            byte_order: ByteOrder::LittleEndian,
            update_bit: None,
        },
    );
    comm.frame_pdu_mappings.insert(
        MappingId(1),
        FramePduMapping {
            id: MappingId(1),
            frame: FrameId(1),
            pdu: PduId(10),
            start_bit: 8,
            byte_order: ByteOrder::LittleEndian,
        },
    );
    comm.frames.insert(FrameId(1), frame(1, 10));

    // Container at bit 0 containing Pdu 10 at byte offset 4.
    comm.pdus.insert(PduId(20), pdu(20, PduKind::Container));
    comm.contained_pdu_mappings.insert(
        MappingId(20),
        ContainedPduMapping {
            id: MappingId(20),
            container: PduId(20),
            pdu: PduId(10),
            header_id: 1,
            offset: 4,
        },
    );
    comm.frame_pdu_mappings.insert(
        MappingId(2),
        FramePduMapping {
            id: MappingId(2),
            frame: FrameId(2),
            pdu: PduId(20),
            start_bit: 0,
            byte_order: ByteOrder::LittleEndian,
        },
    );
    comm.frames.insert(FrameId(2), frame(2, 20));

    // Multiplexed PDU: selector signal 101, dynamic part Pdu 10 at code 2.
    let mut mux = pdu(30, PduKind::Multiplexed);
    mux.selector = Some(SelectorField {
        start_bit: 0,
        length: 8,
        byte_order: ByteOrder::LittleEndian,
        signal: Some(SignalId(101)),
    });
    comm.pdus.insert(PduId(30), mux);
    let mut selector = signal(101, 30);
    selector.length = 8;
    comm.signals.insert(SignalId(101), selector);
    comm.signal_pdu_mappings.insert(
        MappingId(101),
        SignalPduMapping {
            id: MappingId(101),
            signal: SignalId(101),
            pdu: PduId(30),
            start_bit: 0,
            byte_order: ByteOrder::LittleEndian,
            update_bit: None,
        },
    );
    comm.multiplexed_parts.insert(
        MappingId(30),
        MultiplexedPduPart {
            id: MappingId(30),
            mux: PduId(30),
            part: PduId(10),
            selector_code: Some(2),
            initial: false,
        },
    );
    comm.frame_pdu_mappings.insert(
        MappingId(3),
        FramePduMapping {
            id: MappingId(3),
            frame: FrameId(3),
            pdu: PduId(30),
            start_bit: 0,
            byte_order: ByteOrder::LittleEndian,
        },
    );
    comm.frames.insert(FrameId(3), frame(3, 30));

    // Secured PDU with a 16-bit header and Pdu 10 as payload.
    let mut secured_pdu = pdu(40, PduKind::Secured);
    // 16-bit header over an 8-byte payload needs twelve bytes.
    secured_pdu.length = 12;
    secured_pdu.secured = Some(SecuredPduMeta {
        header: SecuredPduHeader::Header16,
        payload: Some(PduId(10)),
        data_id: Some(1),
        freshness_value_id: None,
        secured_area: None,
        auth_info_length: None,
        verification: SecVerification::Unverified,
    });
    comm.pdus.insert(PduId(40), secured_pdu);
    comm.frame_pdu_mappings.insert(
        MappingId(4),
        FramePduMapping {
            id: MappingId(4),
            frame: FrameId(4),
            pdu: PduId(40),
            start_bit: 0,
            byte_order: ByteOrder::LittleEndian,
        },
    );
    comm.frames.insert(FrameId(4), frame(4, 40));
    comm
}

#[test]
fn plain_pdu_offsets_include_the_frame_mapping() {
    let comm = fixture();
    let (plan, issues) = build_decode_plan(&comm, comm.frames.get(&FrameId(1)).unwrap(), 1);
    assert!(issues.is_empty());
    let read = plan
        .reads
        .iter()
        .find(|r| r.signal == SignalId(100))
        .unwrap();
    assert_eq!(read.bit_offset, 8);
    assert_eq!(read.bit_len, 16);
    assert_eq!(plan.frame, Some(FrameId(1)));
    assert_eq!(plan.writes.len(), plan.reads.len());
}

#[test]
fn container_expands_child_byte_offset() {
    let comm = fixture();
    let (plan, issues) = build_decode_plan(&comm, comm.frames.get(&FrameId(2)).unwrap(), 1);
    assert!(issues.is_empty());
    let read = plan
        .reads
        .iter()
        .find(|r| r.signal == SignalId(100))
        .unwrap();
    assert_eq!(read.bit_offset, 4 * 8);
}

#[test]
fn multiplexed_part_carries_the_selector_condition() {
    let comm = fixture();
    let (plan, issues) = build_decode_plan(&comm, comm.frames.get(&FrameId(3)).unwrap(), 1);
    assert!(issues.is_empty());
    let read = plan
        .reads
        .iter()
        .find(|r| r.signal == SignalId(100))
        .unwrap();
    assert_eq!(
        read.mux,
        vec![MuxCondition {
            selector: MuxSelector::Signal {
                signal: SignalId(101)
            },
            ranges: vec![(2, 2)]
        }]
    );
    // The selector signal itself is decoded unconditionally.
    let selector = plan
        .reads
        .iter()
        .find(|r| r.signal == SignalId(101))
        .unwrap();
    assert!(selector.mux.is_empty());
}

#[test]
fn secured_payload_starts_after_the_header() {
    let comm = fixture();
    let (plan, issues) = build_decode_plan(&comm, comm.frames.get(&FrameId(4)).unwrap(), 1);
    assert!(issues.is_empty());
    let read = plan
        .reads
        .iter()
        .find(|r| r.signal == SignalId(100))
        .unwrap();
    assert_eq!(read.bit_offset, 16);
}

#[test]
fn plan_cache_holds_one_plan_per_frame() {
    let comm = fixture();
    let (cache, issues) = PlanCache::build(&comm, 7);
    assert!(issues.is_empty());
    assert_eq!(cache.len(), 4);
    assert_eq!(cache.version, 7);
    assert_eq!(cache.get(FrameId(3)).unwrap().frame, Some(FrameId(3)));
    assert!(cache.get(FrameId(99)).is_none());
}

#[test]
fn a_secured_pdu_that_cannot_hold_its_payload_is_reported() {
    let mut comm = Comm::default();
    comm.pdus.insert(PduId(1), pdu(1, PduKind::Signal));
    let mut secured = pdu(2, PduKind::Secured);
    secured.length = 4;
    secured.header_type = ContainerHeaderType::None;
    secured.secured = Some(SecuredPduMeta {
        header: SecuredPduHeader::NoHeader,
        payload: Some(PduId(1)),
        data_id: Some(1),
        freshness_value_id: None,
        secured_area: Some((0, 64)),
        auth_info_length: Some(5),
        verification: SecVerification::Unverified,
    });
    comm.pdus.insert(PduId(2), secured);
    let mut frame = frame(2, 2);
    frame.length = 4;
    comm.frames.insert(FrameId(2), frame);
    comm.frame_pdu_mappings.insert(
        MappingId(2),
        FramePduMapping {
            id: MappingId(2),
            frame: FrameId(2),
            pdu: PduId(2),
            start_bit: 0,
            byte_order: ByteOrder::LittleEndian,
        },
    );
    let (_, issues) = build_decode_plan(&comm, comm.frames.get(&FrameId(2)).unwrap(), 1);
    assert!(
        issues
            .iter()
            .any(|issue| issue.message.contains("header plus payload")),
        "{issues:?}"
    );
    assert!(
        issues
            .iter()
            .any(|issue| issue.message.contains("secured area")),
        "{issues:?}"
    );
    assert!(
        issues
            .iter()
            .any(|issue| issue.message.contains("authenticator")),
        "{issues:?}"
    );
}

#[test]
fn e2e_protection_structure_is_checked_without_authenticating() {
    let mut comm = Comm::default();
    let mut pdu = pdu(1, PduKind::Signal);
    pdu.e2e = Some(E2eProtectionId(1));
    comm.pdus.insert(PduId(1), pdu);
    comm.frames.insert(FrameId(1), frame(1, 1));
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
    comm.e2e.insert(
        E2eProtectionId(1),
        E2eProtection {
            id: E2eProtectionId(1),
            name: "E".to_string(),
            profile_name: "Profile7".to_string(),
            profile: Some(7),
            data_id: None,
            system_signal: None,
            payload: None,
            counter_offset: None,
            counter_length: None,
        },
    );
    let (_, issues) = build_decode_plan(&comm, comm.frames.get(&FrameId(1)).unwrap(), 1);
    assert!(
        issues.iter().any(|issue| issue.message.contains("data ID")),
        "{issues:?}"
    );

    comm.e2e.clear();
    let (_, issues) = build_decode_plan(&comm, comm.frames.get(&FrameId(1)).unwrap(), 1);
    assert!(
        issues
            .iter()
            .any(|issue| issue.message.contains("missing E2E protection")),
        "{issues:?}"
    );
}

#[test]
fn profile_1_counter_position_enters_the_plan() {
    let mut comm = Comm::default();
    let mut pdu = pdu(1, PduKind::Signal);
    pdu.e2e = Some(E2eProtectionId(1));
    comm.pdus.insert(PduId(1), pdu);
    comm.frames.insert(FrameId(1), frame(1, 1));
    comm.frame_pdu_mappings.insert(
        MappingId(1),
        FramePduMapping {
            id: MappingId(1),
            frame: FrameId(1),
            pdu: PduId(1),
            start_bit: 8,
            byte_order: ByteOrder::LittleEndian,
        },
    );
    comm.e2e.insert(
        E2eProtectionId(1),
        E2eProtection {
            id: E2eProtectionId(1),
            name: "E".to_string(),
            profile_name: "Profile1".to_string(),
            profile: Some(1),
            data_id: Some(291),
            system_signal: None,
            payload: None,
            counter_offset: None,
            counter_length: None,
        },
    );
    let (plan, issues) = build_decode_plan(&comm, comm.frames.get(&FrameId(1)).unwrap(), 1);
    assert!(issues.is_empty(), "{issues:?}");
    assert_eq!(plan.e2e.len(), 1);
    assert_eq!(plan.e2e[0].counter_bit_offset, 16, "PDU base 8 + byte 1");
    assert_eq!(plan.e2e[0].counter_bits, 4);
    assert_eq!(plan.e2e[0].data_id, Some(291));
    let reading = decode_e2e(&plan, &[0, 0, 0x0B, 0, 0, 0, 0, 0]);
    assert_eq!(reading[0].counter, 11);

    // A document that states the counter position overrides the default.
    let protection = comm.e2e.get_mut(&E2eProtectionId(1)).unwrap();
    protection.counter_offset = Some(16);
    protection.counter_length = Some(8);
    let (plan, _) = build_decode_plan(&comm, comm.frames.get(&FrameId(1)).unwrap(), 1);
    assert_eq!(plan.e2e[0].counter_bit_offset, 24);
    assert_eq!(plan.e2e[0].counter_bits, 8);
}
