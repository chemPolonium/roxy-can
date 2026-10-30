use super::*;
use serde_json::{Value, json};

fn can_cluster() -> Cluster {
    Cluster {
        id: ClusterId(1),
        kind: ClusterKind::Can,
        name: "CAN1".to_string(),
        aliases: vec!["Powertrain".to_string()],
        description: String::new(),
        sources: vec![ClusterSource::Arxml {
            path: "Matrix.arxml".to_string(),
            cluster: "CanCluster".to_string(),
        }],
        networks: vec![NetworkId(1)],
    }
}

fn fixture() -> Comm {
    let mut comm = Comm::default();
    comm.clusters.insert(ClusterId(1), can_cluster());

    comm.networks.insert(
        NetworkId(1),
        Network {
            id: NetworkId(1),
            cluster: ClusterId(1),
            name: "CAN1".to_string(),
            params: NetworkParams::Can(CanParams {
                bitrate_kbps: 500,
                fd_data_kbps: 2000,
                sample_point_permille: None,
            }),
        },
    );

    for (id, name) in [(17u64, "EngineData"), (18, "AbsData")] {
        comm.frames.insert(
            FrameId(id),
            Frame {
                id: FrameId(id),
                cluster: ClusterId(1),
                network: NetworkId(1),
                name: name.to_string(),
                length: 8,
                address: FrameAddress::Can {
                    id: if id == 17 { 0x100 } else { 0x200 },
                    extended: false,
                    fd: false,
                },
                timing: FrameTiming::default(),
                comment: String::new(),
                ethernet: None,
                pdu_mappings: if id == 17 { vec![MappingId(1)] } else { vec![] },
            },
        );
    }
    comm.frames.insert(
        FrameId(19),
        Frame {
            id: FrameId(19),
            cluster: ClusterId(1),
            network: NetworkId(1),
            name: "EngineDataExt".to_string(),
            length: 8,
            address: FrameAddress::Can {
                id: 0x101,
                extended: true,
                fd: false,
            },
            timing: FrameTiming::default(),
            comment: String::new(),
            ethernet: None,
            pdu_mappings: vec![MappingId(3)],
        },
    );

    for (id, name) in [(100u64, "EnginePdu"), (101, "EngineExtPdu")] {
        comm.pdus.insert(
            PduId(id),
            Pdu {
                id: PduId(id),
                name: name.to_string(),
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
    }
    comm.frame_pdu_mappings.insert(
        MappingId(1),
        FramePduMapping {
            id: MappingId(1),
            frame: FrameId(17),
            pdu: PduId(100),
            start_bit: 0,
            byte_order: ByteOrder::LittleEndian,
        },
    );
    comm.frame_pdu_mappings.insert(
        MappingId(3),
        FramePduMapping {
            id: MappingId(3),
            frame: FrameId(19),
            pdu: PduId(101),
            start_bit: 0,
            byte_order: ByteOrder::LittleEndian,
        },
    );

    for (id, pdu, name) in [
        (42u64, 100u64, "Speed"),
        (43, 100, "Temp"),
        (44, 101, "Speed"),
    ] {
        comm.signals.insert(
            SignalId(id),
            Signal {
                id: SignalId(id),
                pdu: PduId(pdu),
                name: name.to_string(),
                system_signal: None,
                length: 16,
                start_bit: 0,
                byte_order: ByteOrder::LittleEndian,
                update_bit: None,
                signed: false,
                float: false,
                factor: 0.1,
                offset: 0.0,
                unit: "km/h".to_string(),
                min: Some(0.0),
                max: Some(250.0),
                min_open: false,
                max_open: false,
                initial_raw: Some(0.0),
                values: vec![],
                base_type: None,
                compu: None,
                comment: String::new(),
                transformation: None,
                group: None,
                mux: Vec::new(),
            },
        );
    }
    comm.signal_pdu_mappings.insert(
        MappingId(2),
        SignalPduMapping {
            id: MappingId(2),
            signal: SignalId(42),
            pdu: PduId(100),
            start_bit: 0,
            byte_order: ByteOrder::LittleEndian,
            update_bit: None,
        },
    );

    comm.issues.push(Issue::warning(
        IssueKind::LengthConvention,
        "frame lengths are payload lengths",
    ));
    comm
}

fn eth_comm() -> Comm {
    let mut comm = Comm::default();
    comm.clusters.insert(
        ClusterId(2),
        Cluster {
            id: ClusterId(2),
            kind: ClusterKind::Ethernet,
            name: "ETH1".to_string(),
            aliases: vec![],
            description: String::new(),
            sources: vec![],
            networks: vec![NetworkId(2)],
        },
    );
    comm.networks.insert(
        NetworkId(2),
        Network {
            id: NetworkId(2),
            cluster: ClusterId(2),
            name: "ETH1".to_string(),
            params: NetworkParams::Ethernet(EthernetParams {
                physical_layer: Some(EthernetPhy::Base100T1),
                vlans: vec![100],
                frame_length_includes_header: false,
            }),
        },
    );
    comm.frames.insert(
        FrameId(30),
        Frame {
            id: FrameId(30),
            cluster: ClusterId(2),
            network: NetworkId(2),
            name: "CamFrame".to_string(),
            length: 64,
            address: FrameAddress::Ethernet { vlan: Some(100) },
            timing: FrameTiming::default(),
            comment: String::new(),
            ethernet: Some(EthernetFrameMeta::Generic),
            pdu_mappings: vec![MappingId(4)],
        },
    );
    comm.pdus.insert(
        PduId(103),
        Pdu {
            id: PduId(103),
            name: "CamPdu".to_string(),
            length: 64,
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
        MappingId(4),
        FramePduMapping {
            id: MappingId(4),
            frame: FrameId(30),
            pdu: PduId(103),
            start_bit: 0,
            byte_order: ByteOrder::LittleEndian,
        },
    );
    comm.signals.insert(
        SignalId(60),
        Signal {
            id: SignalId(60),
            pdu: PduId(103),
            name: "Speed".to_string(),
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
            values: vec![],
            base_type: None,
            compu: None,
            comment: String::new(),
            transformation: None,
            group: None,
            mux: Vec::new(),
        },
    );
    comm
}

#[test]
fn element_ids_are_order_independent() {
    let sources = [
        "arxml:1:/Pkg/Frames/A",
        "arxml:1:/Pkg/Frames/B",
        "arxml:1:/Pkg/Signals/A",
    ];
    let mut forward = ElementIds::new();
    let expected: Vec<u64> = sources
        .iter()
        .map(|source| forward.assign(ElementKind::Frame, source))
        .collect();

    let mut backward = ElementIds::new();
    for source in sources.iter().rev() {
        backward.assign(ElementKind::Frame, source);
    }
    let actual: Vec<u64> = sources
        .iter()
        .map(|source| backward.id_for(ElementKind::Frame, source).unwrap())
        .collect();

    assert_eq!(actual, expected);
    assert_eq!(
        forward.id_for(ElementKind::Frame, sources[0]),
        backward.id_for(ElementKind::Frame, sources[0])
    );
}

#[test]
fn ids_do_not_collide_across_many_sources() {
    let mut ids = ElementIds::new();
    let mut seen = std::collections::BTreeSet::new();
    for index in 0..10_000 {
        let id = ids.assign(ElementKind::Signal, &format!("arxml:1:/S/{index}"));
        assert!(id != 0);
        assert!(seen.insert(id), "duplicate id {id}");
    }
}

#[test]
fn rename_keeps_the_element_id() {
    let mut ids = ElementIds::new();
    let old = ids.assign(ElementKind::Signal, "arxml:1:/Pkg/Speed");
    assert_eq!(
        ids.rebind(
            ElementKind::Signal,
            "arxml:1:/Pkg/Speed",
            "arxml:1:/Pkg/VehicleSpeed"
        ),
        Some(old)
    );
    assert_eq!(
        ids.assign(ElementKind::Signal, "arxml:1:/Pkg/VehicleSpeed"),
        old
    );
    assert_eq!(
        ids.id_for(ElementKind::Signal, "arxml:1:/Pkg/Speed"),
        Some(old)
    );
}

#[test]
fn cluster_ids_are_monotonic_and_never_reused() {
    let mut ids = ClusterIds::default();
    let first = ids.allocate();
    let second = ids.allocate();
    assert!(second > first);
    let mut ids = ClusterIds::new(5);
    assert_eq!(ids.allocate(), ClusterId(5));
    ids.reserve(ClusterId(9));
    assert_eq!(ids.allocate(), ClusterId(10));
}

#[test]
fn canonical_signal_key_roundtrips() {
    let comm = fixture();
    let key = SignalKey {
        cluster: ClusterId(1),
        frame: FrameId(17),
        signal: SignalId(42),
    };
    assert_eq!(key.text(), "sig:1:17:42");
    let parsed = SignalRef::parse(&key.text(), &comm).unwrap();
    assert_eq!(parsed, SignalRef::Signal(key));
    assert_eq!(key.display(&comm), "[CAN1:EngineData:Speed]");
}

#[test]
fn alias_resolves_names_ids_and_cluster_aliases() {
    let comm = fixture();
    let expected = SignalRef::Signal(SignalKey {
        cluster: ClusterId(1),
        frame: FrameId(17),
        signal: SignalId(42),
    });
    for text in [
        "[CAN1:EngineData:Speed]",
        "[Powertrain:EngineData:Speed]",
        "[CAN1:0x100:Speed]",
        "[CAN1:256:Speed]",
    ] {
        assert_eq!(SignalRef::parse(text, &comm).unwrap(), expected, "{text}");
    }
}

fn fixture_with_shared_can_id() -> Comm {
    let mut comm = fixture();
    comm.frames.get_mut(&FrameId(19)).unwrap().address = FrameAddress::Can {
        id: 0x100,
        extended: true,
        fd: false,
    };
    comm
}

#[test]
fn alias_reports_ambiguity_instead_of_guessing() {
    let comm = fixture_with_shared_can_id();
    let error = SignalRef::parse("[CAN1:0x100:Speed]", &comm).unwrap_err();
    assert!(matches!(error, KeyError::Ambiguous(_, 2)), "{error}");
    assert!(error.to_string().contains("matches 2"));
}

#[test]
fn alias_reports_unknown_parts() {
    let comm = fixture();
    assert!(matches!(
        SignalRef::parse("[Nope:EngineData:Speed]", &comm),
        Err(KeyError::UnknownCluster(_))
    ));
    assert!(matches!(
        SignalRef::parse("[CAN1:Nope:Speed]", &comm),
        Err(KeyError::UnknownFrame(_))
    ));
    assert!(matches!(
        SignalRef::parse("[CAN1:EngineData:Nope]", &comm),
        Err(KeyError::UnknownSignal(_))
    ));
    assert!(matches!(SignalRef::parse("", &comm), Err(KeyError::Empty)));
}

#[test]
fn derived_refs_keep_their_id() {
    let comm = fixture();
    let parsed = SignalRef::parse("derived:brake_pressure", &comm).unwrap();
    assert_eq!(parsed.text(), "derived:brake_pressure");
    assert_eq!(parsed.display(&comm), "[derived:brake_pressure]");
}

#[test]
fn canonical_keys_reject_a_signal_from_another_frame() {
    let comm = fixture();
    let error = SignalRef::parse("sig:1:17:44", &comm).unwrap_err();
    assert!(matches!(error, KeyError::UnknownSignal(_)));
}

#[test]
fn frame_route_uses_the_address_and_overrides() {
    let comm = fixture();
    let frame = comm.frames.get(&FrameId(17)).unwrap();
    assert_eq!(
        comm.frame_route(frame).key,
        FrameMatchKey::Can {
            id: 0x100,
            extended: false
        }
    );

    let mut eth = eth_comm();
    let frame = eth.frames.get(&FrameId(30)).unwrap();
    assert_eq!(
        eth.frame_route(frame).key,
        FrameMatchKey::Ethernet {
            vlan: Some(100),
            flow: EthFlowKey {
                vlan: Some(100),
                dst: [0; 6],
                src: [0; 6],
                ethertype: 0,
                payload_len: 64,
            },
        }
    );

    eth.overrides.insert(
        FrameId(30),
        FrameOverride {
            ethertype: Some(0x22F0),
            dst_mac: Some([1, 2, 3, 4, 5, 6]),
            src_mac: Some([7, 8, 9, 10, 11, 12]),
            length_includes_header: Some(false),
            vlan: Some(200),
        },
    );
    let route = eth.frame_route(eth.frames.get(&FrameId(30)).unwrap());
    let FrameMatchKey::Ethernet { vlan, flow } = route.key else {
        panic!("expected Ethernet route");
    };
    assert_eq!(vlan, Some(200));
    assert_eq!(flow.vlan, Some(200));
    assert_eq!(flow.ethertype, 0x22F0);
    assert_eq!(flow.dst, [1, 2, 3, 4, 5, 6]);
}

#[test]
fn comm_lookup_and_integrity() {
    let mut comm = fixture();
    assert_eq!(comm.cluster_by_name("Powertrain").unwrap().id, ClusterId(1));
    assert_eq!(
        comm.frame_by_name(ClusterId(1), "EngineData").unwrap().id,
        FrameId(17)
    );
    assert_eq!(
        comm.signal_by_name(comm.frames.get(&FrameId(17)).unwrap(), "Speed")
            .len(),
        1
    );
    assert_eq!(comm.sender_of_frame(FrameId(17)), None);
    assert!(comm.integrity_issues().is_empty());

    comm.clusters.remove(&ClusterId(1));
    let issues = comm.integrity_issues();
    assert!(
        issues
            .iter()
            .any(|issue| issue.kind == IssueKind::DanglingRef)
    );
}

#[test]
fn signal_physical_range_and_initial_value() {
    let comm = fixture();
    let signal = comm.signals.get(&SignalId(42)).unwrap();
    assert!((signal.physical(100.0) - 10.0).abs() < 1e-9);
    assert!((signal.raw(10.0) - 100.0).abs() < 1e-9);
    assert_eq!(signal.initial_physical(), Some(0.0));
    assert!(signal.in_range(10.0));
    assert!(!signal.in_range(-1.0));
    assert!(!signal.in_range(260.0));
}

fn cluster_cfg(id: u32, kind: ClusterKind, name: &str) -> ClusterCfg {
    ClusterCfg {
        id,
        kind,
        name: name.to_string(),
        aliases: vec![],
        sources: vec![ClusterSource::Arxml {
            path: "Matrix.arxml".to_string(),
            cluster: format!("{name}Cluster"),
        }],
        can: (kind == ClusterKind::Can).then(CanCfg::default),
        flexray: (kind == ClusterKind::FlexRay).then(FlexRayCfg::default),
        ether: (kind == ClusterKind::Ethernet).then(EtherCfg::default),
        node_roles: Default::default(),
    }
}

#[test]
fn schema_v2_roundtrips_and_preserves_unknown_sections() {
    let mut project = ProjectV2::new(vec![
        cluster_cfg(1, ClusterKind::Can, "CAN1"),
        cluster_cfg(2, ClusterKind::Ethernet, "ETH1"),
    ]);
    project.rest.insert(
        "views".to_string(),
        json!({ "graphics": [{ "signals": ["sig:1:17:42"] }] }),
    );
    project
        .rest
        .insert("web".to_string(), json!({ "panelBook": { "pages": [] } }));

    let text = serde_json::to_string_pretty(&project).unwrap();
    let value: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(value["format"], PROJECT_FORMAT);
    assert_eq!(value["version"], 2);
    assert_eq!(value["nextClusterId"], 3);
    assert_eq!(value["clusters"][1]["kind"], "ethernet");
    assert_eq!(value["views"]["graphics"][0]["signals"][0], "sig:1:17:42");

    let parsed: ProjectV2 = serde_json::from_value(value).unwrap();
    assert_eq!(parsed, project);
    assert!(parsed.validate().is_empty(), "{:?}", parsed.validate());
    assert!(ProjectV2::is_v2(&serde_json::to_value(&parsed).unwrap()));
}

#[test]
fn schema_validation_flags_bus_identity_problems() {
    let mut project = ProjectV2::new(vec![
        cluster_cfg(1, ClusterKind::Can, "CAN1"),
        cluster_cfg(1, ClusterKind::FlexRay, "FR1"),
    ]);
    project.next_cluster_id = 1;
    project.frame_overrides.push(FrameOverrideCfg {
        cluster: 1,
        frame: "Frame".to_string(),
        ethertype: Some(0x88B5),
        dst_mac: Some("01-00-5E-00-00-01".to_string()),
        src_mac: None,
        length_includes_header: None,
        vlan: Some(100),
    });
    project.replay.push(ReplayCfg {
        kind: "ethernet".to_string(),
        file_channel: 0,
        cluster: 1,
        disabled: false,
    });
    let problems = project.validate();
    assert!(problems.iter().any(|p| p.contains("duplicate cluster id")));
    assert!(problems.iter().any(|p| p.contains("nextClusterId")));
    assert!(problems.iter().any(|p| p.contains("replay channel")));
}

#[test]
fn frame_override_mac_and_kind_validation() {
    let good = FrameOverrideCfg {
        cluster: 1,
        frame: "Cam".to_string(),
        ethertype: Some(0x88B5),
        dst_mac: Some("02:00:00:00:00:01".to_string()),
        src_mac: Some("020000000002".to_string()),
        length_includes_header: Some(true),
        vlan: None,
    };
    assert_eq!(good.dst_mac_bytes().unwrap(), Some([2, 0, 0, 0, 0, 1]));
    assert_eq!(good.src_mac_bytes().unwrap(), Some([2, 0, 0, 0, 0, 2]));
    let bad = FrameOverrideCfg {
        dst_mac: Some("02-00-00".to_string()),
        ..good
    };
    assert!(bad.to_model().is_err());
}

#[test]
fn schema_id_aliases_rebuild_the_allocator() {
    let mut ids = ElementIds::new();
    let frame = ids.assign(ElementKind::Frame, "arxml:1:/Pkg/Frame");
    let mut project = ProjectV2::new(vec![cluster_cfg(1, ClusterKind::Can, "CAN1")]);
    project.aliases_from_ids(&ids);
    assert_eq!(
        project.id_aliases,
        vec![IdAliasCfg {
            kind: "frame".to_string(),
            source: "arxml:1:/Pkg/Frame".to_string(),
            id: frame,
        }]
    );
    let rebuilt = project.element_ids().unwrap();
    assert_eq!(
        rebuilt.id_for(ElementKind::Frame, "arxml:1:/Pkg/Frame"),
        Some(frame)
    );
    assert!(project.validate().is_empty(), "{:?}", project.validate());
}
