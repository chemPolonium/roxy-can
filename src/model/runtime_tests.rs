use super::*;

fn signal(id: u64, start: u32, length: u32) -> Signal {
    Signal {
        id: SignalId(id),
        pdu: PduId(10),
        name: format!("S{id}"),
        system_signal: None,
        length,
        start_bit: start,
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

fn base_comm() -> Comm {
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
    comm.pdus.insert(
        PduId(10),
        Pdu {
            id: PduId(10),
            name: "P10".to_string(),
            length: 8,
            kind: PduKind::Signal,
            header_type: ContainerHeaderType::Short,
            cycle_us: None,
            selector: None,
            secured: None,
            transformation: None,
            e2e: None,
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
    comm.frames.insert(
        FrameId(1),
        Frame {
            id: FrameId(1),
            cluster: ClusterId(1),
            network: NetworkId(1),
            name: "F1".to_string(),
            length: 8,
            address: FrameAddress::Can {
                id: 0x100,
                extended: false,
                fd: false,
            },
            timing: FrameTiming::default(),
            ethernet: None,
            pdu_mappings: vec![MappingId(1)],
        },
    );
    comm
}

fn add_signal(comm: &mut Comm, signal: Signal) {
    let id = signal.id;
    let pdu = signal.pdu;
    comm.signal_pdu_mappings.insert(
        MappingId(id.0),
        SignalPduMapping {
            id: MappingId(id.0),
            signal: id,
            pdu,
            start_bit: signal.start_bit,
            byte_order: signal.byte_order,
            update_bit: None,
        },
    );
    comm.signals.insert(id, signal);
}

fn plan(comm: &Comm) -> DecodePlan {
    let (plan, issues) = build_decode_plan(comm, comm.frames.get(&FrameId(1)).unwrap(), 1);
    assert!(issues.is_empty());
    plan
}

#[test]
fn decodes_physical_values_through_frame_offset() {
    let mut comm = base_comm();
    let mut speed = signal(100, 0, 16);
    speed.factor = 0.1;
    add_signal(&mut comm, speed);
    let plan = plan(&comm);
    let decoded = decode_payload(&comm, &plan, &[0, 0x34, 0x12, 0, 0, 0, 0, 0]);
    assert_eq!(decoded.len(), 1);
    assert_eq!(decoded[0].raw, 0x1234);
    assert!((decoded[0].physical.unwrap() - 466.0).abs() < 1e-9);
}

#[test]
fn encodes_physical_values_and_reads_them_back() {
    let mut comm = base_comm();
    let mut speed = signal(100, 0, 16);
    speed.factor = 0.1;
    add_signal(&mut comm, speed);
    let plan = plan(&comm);
    let mut payload = [0u8; 8];
    let written = encode_payload(&comm, &plan, &[(SignalId(100), 250.0)], &mut payload);
    assert_eq!(written, 1);
    assert_eq!(&payload[1..3], &[0xC4, 0x09]);
    let decoded = decode_payload(&comm, &plan, &payload);
    assert!((decoded[0].physical.unwrap() - 250.0).abs() < 1e-9);
}

#[test]
fn decodes_signed_and_float_signals() {
    let mut comm = base_comm();
    let mut signed = signal(100, 0, 12);
    signed.signed = true;
    add_signal(&mut comm, signed);
    let mut float = signal(101, 16, 32);
    float.float = true;
    add_signal(&mut comm, float);
    let plan = plan(&comm);

    let mut payload = [0u8; 8];
    payload[1] = 0xFF;
    payload[2] = 0x0F;
    let bits = f32::to_bits(1.5);
    payload[3..7].copy_from_slice(&bits.to_le_bytes());
    let decoded = decode_payload(&comm, &plan, &payload);
    let signed_value = decoded
        .iter()
        .find(|value| value.signal == SignalId(100))
        .unwrap();
    assert!((signed_value.physical.unwrap() + 1.0).abs() < 1e-9);
    let float_value = decoded
        .iter()
        .find(|value| value.signal == SignalId(101))
        .unwrap();
    assert!((float_value.physical.unwrap() - 1.5).abs() < 1e-9);
}

#[test]
fn mux_selector_gates_the_dynamic_part() {
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
    let mut mux = Pdu {
        id: PduId(30),
        name: "Mux".to_string(),
        length: 8,
        kind: PduKind::Multiplexed,
        header_type: ContainerHeaderType::Short,
        cycle_us: None,
        selector: None,
        secured: None,
        transformation: None,
        e2e: None,
    };
    mux.selector = Some(SelectorField {
        start_bit: 0,
        length: 8,
        byte_order: ByteOrder::LittleEndian,
        signal: Some(SignalId(101)),
    });
    comm.pdus.insert(PduId(30), mux);
    comm.pdus.insert(
        PduId(10),
        Pdu {
            id: PduId(10),
            name: "P10".to_string(),
            length: 8,
            kind: PduKind::Signal,
            header_type: ContainerHeaderType::Short,
            cycle_us: None,
            selector: None,
            secured: None,
            transformation: None,
            e2e: None,
        },
    );
    let mut selector = signal(101, 0, 8);
    selector.pdu = PduId(30);
    add_signal(&mut comm, selector);
    let mut part = signal(100, 8, 8);
    part.pdu = PduId(10);
    add_signal(&mut comm, part);
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
        MappingId(1),
        FramePduMapping {
            id: MappingId(1),
            frame: FrameId(1),
            pdu: PduId(30),
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
            name: "F1".to_string(),
            length: 8,
            address: FrameAddress::Can {
                id: 0x100,
                extended: false,
                fd: false,
            },
            timing: FrameTiming::default(),
            ethernet: None,
            pdu_mappings: vec![MappingId(1)],
        },
    );
    let plan = plan(&comm);
    let matching = decode_payload(&comm, &plan, &[2, 0x55, 0, 0, 0, 0, 0, 0]);
    assert_eq!(
        matching
            .iter()
            .find(|value| value.signal == SignalId(100))
            .unwrap()
            .raw,
        0x55
    );
    let other = decode_payload(&comm, &plan, &[1, 0x55, 0, 0, 0, 0, 0, 0]);
    assert!(other.iter().all(|value| value.signal != SignalId(100)));
}

#[test]
fn a_container_child_is_gated_by_the_header_id() {
    let mut comm = base_comm();
    add_signal(&mut comm, signal(100, 0, 16));
    comm.pdus.insert(
        PduId(20),
        Pdu {
            id: PduId(20),
            name: "Container".to_string(),
            length: 8,
            kind: PduKind::Container,
            header_type: ContainerHeaderType::Short,
            cycle_us: None,
            selector: None,
            secured: None,
            transformation: None,
            e2e: None,
        },
    );
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
        MappingId(1),
        FramePduMapping {
            id: MappingId(1),
            frame: FrameId(1),
            pdu: PduId(20),
            start_bit: 0,
            byte_order: ByteOrder::LittleEndian,
        },
    );
    let plan = plan(&comm);
    let matching = decode_payload(&comm, &plan, &[0, 0, 1, 4, 0x34, 0x12, 0, 0]);
    assert_eq!(
        matching
            .iter()
            .find(|value| value.signal == SignalId(100))
            .unwrap()
            .raw,
        0x1234
    );
    let other = decode_payload(&comm, &plan, &[0, 0, 2, 4, 0x34, 0x12, 0, 0]);
    assert!(
        other.iter().all(|value| value.signal != SignalId(100)),
        "a header naming another contained PDU hides this one"
    );
    let truncated = decode_payload(&comm, &plan, &[0, 0, 1, 1, 0x34, 0x12, 0, 0]);
    assert!(
        truncated.iter().all(|value| value.signal != SignalId(100)),
        "a header length that does not reach the signal hides it too"
    );

    let mut payload = [0, 0, 2, 4, 0, 0, 0, 0];
    assert_eq!(
        encode_payload(
            &comm,
            &plan,
            &[(SignalId(100), 0x1234u16 as f64)],
            &mut payload
        ),
        0,
        "writing a child the frame does not carry is not an error, it is nothing"
    );
    let mut payload = [0, 0, 1, 4, 0, 0, 0, 0];
    assert_eq!(
        encode_payload(
            &comm,
            &plan,
            &[(SignalId(100), 0x1234u16 as f64)],
            &mut payload
        ),
        1
    );
    assert_eq!(&payload[4..6], &[0x34, 0x12]);
}

#[test]
fn a_linear_transformation_maps_the_application_value_both_ways() {
    let mut comm = base_comm();
    comm.transformations.insert(
        TransformationId(1),
        Transformation {
            id: TransformationId(1),
            name: "T1".to_string(),
            kind: TransformationKind::Linear {
                scale: 2.0,
                offset: -10.0,
            },
        },
    );
    let mut speed = signal(100, 0, 16);
    speed.transformation = Some(TransformationId(1));
    add_signal(&mut comm, speed);
    let plan = plan(&comm);
    let decoded = decode_payload(&comm, &plan, &[0, 5, 0, 0, 0, 0, 0, 0]);
    assert_eq!(decoded[0].physical.unwrap(), 0.0, "app = 2 * net - 10");
    assert_eq!(decoded[0].raw, 5, "the wire value is still reported raw");

    let mut payload = [0u8; 8];
    assert_eq!(
        encode_payload(&comm, &plan, &[(SignalId(100), 10.0)], &mut payload),
        1
    );
    assert_eq!(payload[1], 10, "net = (app + 10) / 2");
}

#[test]
fn a_text_table_transformation_is_flagged_and_left_raw() {
    let mut comm = base_comm();
    comm.transformations.insert(
        TransformationId(1),
        Transformation {
            id: TransformationId(1),
            name: "T1".to_string(),
            kind: TransformationKind::TextTable {
                values: vec![(0, "off".to_string())],
            },
        },
    );
    let mut speed = signal(100, 0, 16);
    speed.transformation = Some(TransformationId(1));
    add_signal(&mut comm, speed);
    let (plan, issues) = build_decode_plan(&comm, comm.frames.get(&FrameId(1)).unwrap(), 1);
    assert!(
        issues
            .iter()
            .any(|issue| issue.kind == IssueKind::UnsupportedTransformation),
        "the unsupported kind is reported: {issues:?}"
    );
    assert!(plan.reads[0].transform.is_none());
}
