use super::*;

fn import_sample() -> Comm {
    let mut ids = ElementIds::new();
    let source = ClusterSource::Dbc {
        path: "sample.dbc".to_string(),
    };
    import_dbc(
        crate::sample_data::MOTBUS_DBC,
        "CAN1",
        ClusterId(1),
        &source,
        &mut ids,
    )
    .comm
}

#[test]
fn imports_dbc_frames_signals_and_nodes() {
    let comm = import_sample();
    assert_eq!(comm.clusters.len(), 1);
    assert_eq!(comm.frames.len(), 6);
    assert_eq!(comm.pdus.len(), 6);
    let frame = comm
        .frames
        .values()
        .find(|frame| frame.name == "GearBoxInfo")
        .unwrap();
    assert_eq!(
        frame.address,
        FrameAddress::Can {
            id: 1020,
            extended: false,
            fd: false
        }
    );
    assert_eq!(frame.length, 1);

    let signal = comm
        .signals
        .values()
        .find(|signal| signal.name == "CarSpeed")
        .unwrap();
    assert_eq!(signal.length, 10);
    assert_eq!(signal.start_bit, 0);
    assert_eq!(signal.factor, 0.5);
    assert_eq!(signal.unit, "km/h");
    assert_eq!(signal.max, Some(300.0));
    assert!(!signal.float);

    let gear = comm
        .signals
        .values()
        .find(|signal| signal.name == "Gear")
        .unwrap();
    assert_eq!(gear.values.len(), 5);
    assert!(gear.values.contains(&(3, "Gear_3".to_string())));

    let float = comm
        .signals
        .values()
        .find(|signal| signal.name == "WheelSpeedFR" && signal.length == 32)
        .unwrap();
    assert!(float.float);

    let gearbox = comm
        .frames
        .values()
        .find(|f| f.name == "GearBoxInfo")
        .unwrap();
    assert_eq!(comm.sender_of_frame(gearbox.id).unwrap().name, "GearBox");
    assert!(
        comm.frame_relations
            .iter()
            .any(|relation| relation.frame == gearbox.id && relation.role == RelationRole::Receiver)
    );
}

#[test]
fn dbc_initial_value_is_converted_back_to_raw() {
    let mut ids = ElementIds::new();
    let source = ClusterSource::Dbc {
        path: "inline.dbc".to_string(),
    };
    let text = "VERSION \"\"\nNS_ :\nBS_:\nBU_: ECU\nBO_ 300 Demo: 8 ECU\n SG_ Temp : 0|8@1+ (2,-40) [-40|215] \"degC\" ECU\nBA_DEF_ SG_ \"GenSigStartValue\" INT 0 0;\nBA_ \"GenSigStartValue\" SG_ 300 Temp 80;\n";
    let outcome = import_dbc(text, "CAN1", ClusterId(1), &source, &mut ids);
    let signal = outcome.comm.signals.values().next().unwrap();
    // GenSigStartValue is raw: 80 raw becomes 120 physical with (2,-40).
    assert_eq!(signal.initial_raw, Some(80.0));
    assert_eq!(signal.initial_physical(), Some(120.0));
}

#[test]
fn dbc_multiplexed_signals_carry_their_selector_conditions() {
    const TEXT: &str = "VERSION \"\"\nNS_ :\nBU_: ECU\nBO_ 512 MuxMsg: 8 ECU\n SG_ Mode M : 0|8@1+ (1,0) [0|255] \"\" ECU\n SG_ First m0 : 8|8@1+ (1,0) [0|255] \"\" ECU\n SG_ Second m1 : 8|8@1+ (1,0) [0|255] \"\" ECU\n";
    let mut ids = ElementIds::new();
    let source = ClusterSource::Dbc {
        path: "mux.dbc".to_string(),
    };
    let comm = import_dbc(TEXT, "CAN1", ClusterId(1), &source, &mut ids).comm;
    let mode = comm
        .signals
        .values()
        .find(|signal| signal.name == "Mode")
        .unwrap();
    let first = comm
        .signals
        .values()
        .find(|signal| signal.name == "First")
        .unwrap();
    let second = comm
        .signals
        .values()
        .find(|signal| signal.name == "Second")
        .unwrap();
    assert_eq!(
        first.mux,
        vec![MuxCondition {
            selector: MuxSelector::Signal { signal: mode.id },
            ranges: vec![(0, 0)]
        }]
    );
    assert_eq!(
        second.mux,
        vec![MuxCondition {
            selector: MuxSelector::Signal { signal: mode.id },
            ranges: vec![(1, 1)]
        }]
    );

    let (plan, issues) = build_decode_plan(&comm, comm.frames.values().next().unwrap(), 1);
    assert!(issues.is_empty());
    let decode = |payload: [u8; 8]| crate::model::decode_payload(&comm, &plan, &payload);
    let zero = decode([0, 0x11, 0, 0, 0, 0, 0, 0]);
    assert!(zero.iter().any(|value| value.signal == first.id));
    assert!(zero.iter().all(|value| value.signal != second.id));
    let one = decode([1, 0x22, 0, 0, 0, 0, 0, 0]);
    assert!(one.iter().any(|value| value.signal == second.id));
    assert!(one.iter().all(|value| value.signal != first.id));
}
