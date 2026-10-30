use super::*;

pub(super) const CAN_TEXT: &str = r#"<?xml version="1.0"?>
<AUTOSAR><AR-PACKAGES><AR-PACKAGE><SHORT-NAME>Pkg</SHORT-NAME><ELEMENTS>
 <CAN-CLUSTER><SHORT-NAME>CanCluster</SHORT-NAME>
  <CAN-CLUSTER-VARIANTS><CAN-CLUSTER-CONDITIONAL>
   <BAUDRATE>500000</BAUDRATE><CAN-FD-BAUDRATE>2000000</CAN-FD-BAUDRATE>
   <CAN-FRAME-TRIGGERINGS>
    <CAN-FRAME-TRIGGERING><SHORT-NAME>FT</SHORT-NAME>
     <FRAME-REF DEST="CAN-FRAME">/Pkg/EngineFrame</FRAME-REF>
     <IDENTIFIER>256</IDENTIFIER><CAN-ADDRESSING-MODE>STANDARD</CAN-ADDRESSING-MODE>
    </CAN-FRAME-TRIGGERING>
   </CAN-FRAME-TRIGGERINGS>
  </CAN-CLUSTER-CONDITIONAL></CAN-CLUSTER-VARIANTS>
 </CAN-CLUSTER>
 <CAN-FRAME><SHORT-NAME>EngineFrame</SHORT-NAME><FRAME-LENGTH>8</FRAME-LENGTH>
  <PDU-TO-FRAME-MAPPINGS><PDU-TO-FRAME-MAPPING><SHORT-NAME>FM</SHORT-NAME>
   <START-POSITION>0</START-POSITION>
   <PDU-REF DEST="I-SIGNAL-I-PDU">/Pkg/EnginePdu</PDU-REF>
  </PDU-TO-FRAME-MAPPING></PDU-TO-FRAME-MAPPINGS>
 </CAN-FRAME>
 <I-SIGNAL-I-PDU><SHORT-NAME>EnginePdu</SHORT-NAME><LENGTH>8</LENGTH>
  <I-SIGNAL-TO-I-PDU-MAPPINGS><I-SIGNAL-TO-I-PDU-MAPPING><SHORT-NAME>SM</SHORT-NAME>
   <START-POSITION>8</START-POSITION>
   <PACKING-BYTE-ORDER>MOST-SIGNIFICANT-BYTE-LAST</PACKING-BYTE-ORDER>
   <I-SIGNAL-REF DEST="I-SIGNAL">/Pkg/Speed</I-SIGNAL-REF>
  </I-SIGNAL-TO-I-PDU-MAPPING></I-SIGNAL-TO-I-PDU-MAPPINGS>
 </I-SIGNAL-I-PDU>
 <I-SIGNAL><SHORT-NAME>Speed</SHORT-NAME><LENGTH>16</LENGTH>
  <INIT-VALUE><NUMERICAL-VALUE-SPECIFICATION><VALUE>100</VALUE></NUMERICAL-VALUE-SPECIFICATION></INIT-VALUE>
  <COMPU-METHOD-REF DEST="COMPU-METHOD">/Pkg/SpeedCompu</COMPU-METHOD-REF>
 </I-SIGNAL>
 <I-SIGNAL-GROUP><SHORT-NAME>SpeedGroup</SHORT-NAME>
  <I-SIGNAL-REFS><I-SIGNAL-REF DEST="I-SIGNAL">/Pkg/Speed</I-SIGNAL-REF></I-SIGNAL-REFS>
 </I-SIGNAL-GROUP>
 <ECU-INSTANCE><SHORT-NAME>ECU_A</SHORT-NAME>
  <FRAME-PORTS><FRAME-PORT><SHORT-NAME>FP</SHORT-NAME>
   <COMMUNICATION-DIRECTION>OUT</COMMUNICATION-DIRECTION>
   <FRAME-REF DEST="CAN-FRAME">/Pkg/EngineFrame</FRAME-REF>
  </FRAME-PORT></FRAME-PORTS>
 </ECU-INSTANCE>
 <COMPU-METHOD><SHORT-NAME>SpeedCompu</SHORT-NAME>
  <COMPU-INTERNAL-TO-PHYS><COMPU-SCALES><COMPU-SCALE>
   <LOWER-LIMIT>0</LOWER-LIMIT><UPPER-LIMIT>250</UPPER-LIMIT>
   <COMPU-RATIONAL-COEFFS>
    <COMPU-NUMERATOR><V>0</V><V>0.1</V></COMPU-NUMERATOR>
    <COMPU-DENOMINATOR><V>1</V></COMPU-DENOMINATOR>
   </COMPU-RATIONAL-COEFFS>
   <UNIT-REF DEST="UNIT">/Pkg/Kmh</UNIT-REF>
  </COMPU-SCALE></COMPU-SCALES></COMPU-INTERNAL-TO-PHYS>
 </COMPU-METHOD>
 <UNIT><SHORT-NAME>Kmh</SHORT-NAME><DISPLAY-NAME>km/h</DISPLAY-NAME></UNIT>
</ELEMENTS></AR-PACKAGE></AR-PACKAGES></AUTOSAR>"#;

pub(super) const FR_TEXT: &str = r#"<?xml version="1.0"?>
<AUTOSAR><AR-PACKAGES><AR-PACKAGE><SHORT-NAME>Pkg</SHORT-NAME><ELEMENTS>
 <FLEXRAY-CLUSTER><SHORT-NAME>FrCluster</SHORT-NAME>
  <SPEED>10000</SPEED><CYCLE-TIME-ms>5</CYCLE-TIME-ms>
  <FLEXRAY-PHYSICAL-CHANNEL><SHORT-NAME>FrA</SHORT-NAME>
   <FLEXRAY-FRAME-TRIGGERINGS>
    <FLEXRAY-FRAME-TRIGGERING><SHORT-NAME>FT</SHORT-NAME>
     <FRAME-REF DEST="FLEXRAY-FRAME">/Pkg/Frame25</FRAME-REF>
     <SLOT-ID>25</SLOT-ID><BASE-CYCLE>2</BASE-CYCLE>
     <CYCLE-REPETITION>2</CYCLE-REPETITION>
     <PAYLOAD-PREAMBLE-INDICATOR>true</PAYLOAD-PREAMBLE-INDICATOR>
    </FLEXRAY-FRAME-TRIGGERING>
   </FLEXRAY-FRAME-TRIGGERINGS>
  </FLEXRAY-PHYSICAL-CHANNEL>
 </FLEXRAY-CLUSTER>
 <FLEXRAY-FRAME><SHORT-NAME>Frame25</SHORT-NAME><FRAME-LENGTH>8</FRAME-LENGTH>
  <PDU-TO-FRAME-MAPPINGS><PDU-TO-FRAME-MAPPING><SHORT-NAME>FM</SHORT-NAME>
   <PDU-REF DEST="I-SIGNAL-I-PDU">/Pkg/FrPdu</PDU-REF>
  </PDU-TO-FRAME-MAPPING></PDU-TO-FRAME-MAPPINGS>
 </FLEXRAY-FRAME>
 <I-SIGNAL-I-PDU><SHORT-NAME>FrPdu</SHORT-NAME><LENGTH>8</LENGTH>
  <I-SIGNAL-TO-I-PDU-MAPPINGS><I-SIGNAL-TO-I-PDU-MAPPING><SHORT-NAME>SM</SHORT-NAME>
   <START-POSITION>0</START-POSITION>
   <I-SIGNAL-REF DEST="I-SIGNAL">/Pkg/FrSpeed</I-SIGNAL-REF>
  </I-SIGNAL-TO-I-PDU-MAPPING></I-SIGNAL-TO-I-PDU-MAPPINGS>
 </I-SIGNAL-I-PDU>
 <I-SIGNAL><SHORT-NAME>FrSpeed</SHORT-NAME><LENGTH>8</LENGTH></I-SIGNAL>
</ELEMENTS></AR-PACKAGE></AR-PACKAGES></AUTOSAR>"#;

fn import(text: &str, cluster: &str, id: u32) -> Comm {
    let mut ids = ElementIds::new();
    let source = ClusterSource::Arxml {
        path: "test.arxml".to_string(),
        cluster: cluster.to_string(),
    };
    import_cluster(text, cluster, ClusterId(id), &source, &mut ids).comm
}

#[test]
fn scans_every_cluster_kind() {
    let text = format!(
        "<AUTOSAR><AR-PACKAGES><AR-PACKAGE><SHORT-NAME>P</SHORT-NAME><ELEMENTS>{}</ELEMENTS></AR-PACKAGE></AR-PACKAGES></AUTOSAR>",
        "<CAN-CLUSTER><SHORT-NAME>C</SHORT-NAME></CAN-CLUSTER>\
         <FLEXRAY-CLUSTER><SHORT-NAME>F</SHORT-NAME></FLEXRAY-CLUSTER>\
         <ETHERNET-CLUSTER><SHORT-NAME>E</SHORT-NAME></ETHERNET-CLUSTER>"
    );
    let found = scan_clusters(&text).unwrap();
    assert_eq!(
        found,
        vec![
            ScannedCluster {
                kind: ClusterKind::Can,
                name: "C".to_string()
            },
            ScannedCluster {
                kind: ClusterKind::FlexRay,
                name: "F".to_string()
            },
            ScannedCluster {
                kind: ClusterKind::Ethernet,
                name: "E".to_string()
            },
        ]
    );
}

#[test]
fn imports_can_frame_pdu_and_signal() {
    let comm = import(CAN_TEXT, "CanCluster", 1);
    assert_eq!(comm.clusters.len(), 1);
    let cluster = comm.clusters.get(&ClusterId(1)).unwrap();
    assert_eq!(cluster.kind, ClusterKind::Can);
    assert_eq!(cluster.name, "CanCluster");
    let network = comm.networks.get(&NetworkId(1)).unwrap();
    let NetworkParams::Can(can) = &network.params else {
        panic!("expected CAN network");
    };
    assert_eq!(can.bitrate_kbps, 500);
    assert_eq!(can.fd_data_kbps, 2000);

    let frame = comm.frames.values().next().unwrap();
    assert_eq!(frame.name, "EngineFrame");
    assert_eq!(frame.length, 8);
    assert_eq!(
        frame.address,
        FrameAddress::Can {
            id: 0x100,
            extended: false,
            fd: false
        }
    );
    let pdu = comm.pdus.values().next().unwrap();
    assert_eq!(pdu.kind, PduKind::Signal);
    let signal = comm.signals.values().next().unwrap();
    assert_eq!(signal.name, "Speed");
    assert_eq!(signal.pdu, pdu.id);
    assert_eq!(signal.length, 16);
    assert_eq!(signal.start_bit, 8);
    assert_eq!(signal.byte_order, ByteOrder::LittleEndian);
    assert_eq!(signal.factor, 0.1);
    assert_eq!(signal.min, Some(0.0));
    assert_eq!(signal.max, Some(250.0));
    assert_eq!(signal.unit, "km/h");
    assert_eq!(signal.initial_raw, Some(100.0));
    assert_eq!(signal.initial_physical(), Some(10.0));
    assert_eq!(signal.id, comm.signals.keys().next().copied().unwrap());

    let key = comm.signal_key(signal).unwrap();
    assert_eq!(
        SignalRef::parse(&key.text(), &comm, None).unwrap(),
        SignalRef::Signal(key)
    );
    assert_eq!(key.display(&comm), "[CanCluster:EngineFrame:Speed]");
    let ecu = comm.ecus.values().find(|ecu| ecu.name == "ECU_A").unwrap();
    assert_eq!(comm.sender_of_frame(frame.id), Some(ecu));
    let group = comm.groups.values().next().unwrap();
    assert_eq!(signal.group, Some(group.id));
    assert!(comm.integrity_issues().is_empty());
}

#[test]
fn imports_flexray_triggering_and_signal() {
    let comm = import(FR_TEXT, "FrCluster", 7);
    let frame = comm.frames.values().next().unwrap();
    assert_eq!(
        frame.address,
        FrameAddress::FlexRay {
            slot: 25,
            base_cycle: 2,
            repetition: 2,
            payload_preamble: true
        }
    );
    let NetworkParams::FlexRay(fr) = &comm.networks.get(&NetworkId(7)).unwrap().params else {
        panic!("expected FlexRay network");
    };
    assert_eq!(fr.speed_kbps, 10_000);
    assert_eq!(fr.cycle_time_ms, 5.0);
    let signal = comm.signals.values().next().unwrap();
    assert_eq!(signal.length, 8);
    assert_eq!(signal.pdu, comm.pdus.values().next().unwrap().id);
}

#[test]
fn imports_ethernet_vlan_and_factor() {
    let text = std::fs::read_to_string(crate::test_assets::eth_arxml()).unwrap();
    let comm = import(&text, "EthCluster", 3);
    let frame = comm.frames.values().next().unwrap();
    assert_eq!(frame.name, "EthFrame");
    assert_eq!(frame.length, 22);
    assert_eq!(frame.address, FrameAddress::Ethernet { vlan: Some(100) });
    assert_eq!(frame.ethernet, Some(EthernetFrameMeta::Base));
    let signal = comm.signals.values().next().unwrap();
    assert_eq!(signal.start_bit, 8);
    assert_eq!(signal.factor, 0.1);
    assert_eq!(signal.length, 16);
    let NetworkParams::Ethernet(eth) = &comm.networks.get(&NetworkId(3)).unwrap().params else {
        panic!("expected Ethernet network");
    };
    assert_eq!(eth.vlans, vec![100]);
}

#[test]
fn importing_one_cluster_does_not_take_another_clusters_frames() {
    let combined = format!(
        "<AUTOSAR><AR-PACKAGES><AR-PACKAGE><SHORT-NAME>P</SHORT-NAME><ELEMENTS>\
         {}\
         <ETHERNET-CLUSTER><SHORT-NAME>Eth</SHORT-NAME>\
          <ETHERNET-PHYSICAL-CHANNEL><SHORT-NAME>V</SHORT-NAME>\
           <VLAN-CONFIG><VLAN-IDENTIFIER>9</VLAN-IDENTIFIER></VLAN-CONFIG>\
           <ETHERNET-FRAME-TRIGGERING><SHORT-NAME>ET</SHORT-NAME>\
            <FRAME-REF DEST=\"ETHERNET-FRAME\">/P/EthFrame</FRAME-REF>\
           </ETHERNET-FRAME-TRIGGERING>\
          </ETHERNET-PHYSICAL-CHANNEL>\
         </ETHERNET-CLUSTER>\
         <ETHERNET-FRAME><SHORT-NAME>EthFrame</SHORT-NAME><FRAME-LENGTH>64</FRAME-LENGTH>\
          <PDU-TO-FRAME-MAPPING><SHORT-NAME>M</SHORT-NAME>\
           <PDU-REF DEST=\"I-SIGNAL-I-PDU\">/P/EthPdu</PDU-REF>\
          </PDU-TO-FRAME-MAPPING>\
         </ETHERNET-FRAME>\
         <I-SIGNAL-I-PDU><SHORT-NAME>EthPdu</SHORT-NAME><LENGTH>64</LENGTH></I-SIGNAL-I-PDU>\
         </ELEMENTS></AR-PACKAGE></AR-PACKAGES></AUTOSAR>",
        // Strip the outer AUTOSAR wrapper from the CAN fixture.
        CAN_TEXT
            .split("<ELEMENTS>")
            .nth(1)
            .unwrap()
            .split("</ELEMENTS>")
            .next()
            .unwrap()
    );
    let comm = import(&combined, "CanCluster", 1);
    assert_eq!(comm.frames.len(), 1);
    assert_eq!(comm.frames.values().next().unwrap().name, "EngineFrame");
    assert_eq!(comm.clusters.len(), 1);
}

const ADVANCED_TEXT: &str = r#"<?xml version="1.0"?>
<AUTOSAR><AR-PACKAGES><AR-PACKAGE><SHORT-NAME>P</SHORT-NAME><ELEMENTS>
 <CAN-CLUSTER><SHORT-NAME>C</SHORT-NAME></CAN-CLUSTER>
 <PDU-TRIGGERING><SHORT-NAME>TrigA</SHORT-NAME>
  <I-PDU-REF DEST="I-SIGNAL-I-PDU">/P/PduA</I-PDU-REF>
 </PDU-TRIGGERING>
 <PDU-TRIGGERING><SHORT-NAME>TrigB</SHORT-NAME>
  <I-PDU-REF DEST="I-SIGNAL-I-PDU">/P/PduB</I-PDU-REF>
 </PDU-TRIGGERING>
 <I-SIGNAL-I-PDU><SHORT-NAME>PduA</SHORT-NAME><LENGTH>4</LENGTH>
  <CONTAINED-I-PDU-PROPS><HEADER-ID-SHORT-HEADER>1</HEADER-ID-SHORT-HEADER></CONTAINED-I-PDU-PROPS>
 </I-SIGNAL-I-PDU>
 <I-SIGNAL-I-PDU><SHORT-NAME>PduB</SHORT-NAME><LENGTH>6</LENGTH>
  <CONTAINED-I-PDU-PROPS><HEADER-ID-SHORT-HEADER>2</HEADER-ID-SHORT-HEADER></CONTAINED-I-PDU-PROPS>
 </I-SIGNAL-I-PDU>
 <CONTAINER-I-PDU><SHORT-NAME>Cont</SHORT-NAME><LENGTH>32</LENGTH>
  <CONTAINED-PDU-TRIGGERING-REFS>
   <CONTAINED-PDU-TRIGGERING-REF DEST="PDU-TRIGGERING">/P/TrigA</CONTAINED-PDU-TRIGGERING-REF>
   <CONTAINED-PDU-TRIGGERING-REF DEST="PDU-TRIGGERING">/P/TrigB</CONTAINED-PDU-TRIGGERING-REF>
  </CONTAINED-PDU-TRIGGERING-REFS>
  <HEADER-TYPE>SHORT-HEADER</HEADER-TYPE>
 </CONTAINER-I-PDU>
 <I-SIGNAL-I-PDU><SHORT-NAME>Static</SHORT-NAME><LENGTH>8</LENGTH></I-SIGNAL-I-PDU>
 <I-SIGNAL-I-PDU><SHORT-NAME>M0</SHORT-NAME><LENGTH>8</LENGTH></I-SIGNAL-I-PDU>
 <I-SIGNAL-I-PDU><SHORT-NAME>M1</SHORT-NAME><LENGTH>8</LENGTH></I-SIGNAL-I-PDU>
 <MULTIPLEXED-I-PDU><SHORT-NAME>Mux</SHORT-NAME><LENGTH>8</LENGTH>
  <SELECTOR-FIELD-START-POSITION>0</SELECTOR-FIELD-START-POSITION>
  <SELECTOR-FIELD-LENGTH>8</SELECTOR-FIELD-LENGTH>
  <DYNAMIC-PARTS><DYNAMIC-PART><DYNAMIC-PART-ALTERNATIVES>
   <DYNAMIC-PART-ALTERNATIVE><SELECTOR-FIELD-CODE>0</SELECTOR-FIELD-CODE>
    <I-PDU-REF DEST="I-SIGNAL-I-PDU">/P/M0</I-PDU-REF>
   </DYNAMIC-PART-ALTERNATIVE>
   <DYNAMIC-PART-ALTERNATIVE><SELECTOR-FIELD-CODE>1</SELECTOR-FIELD-CODE>
    <I-PDU-REF DEST="I-SIGNAL-I-PDU">/P/M1</I-PDU-REF>
   </DYNAMIC-PART-ALTERNATIVE>
  </DYNAMIC-PART-ALTERNATIVES></DYNAMIC-PART></DYNAMIC-PARTS>
  <STATIC-PARTS><STATIC-PART>
   <I-PDU-REF DEST="I-SIGNAL-I-PDU">/P/Static</I-PDU-REF>
  </STATIC-PART></STATIC-PARTS>
 </MULTIPLEXED-I-PDU>
 <SECURED-I-PDU><SHORT-NAME>Sec</SHORT-NAME><LENGTH>16</LENGTH>
  <USE-SECURED-PDU-HEADER>SECURED-PDU-HEADER-16-BIT</USE-SECURED-PDU-HEADER>
  <PAYLOAD-REF DEST="I-SIGNAL-I-PDU">/P/PduA</PAYLOAD-REF>
  <AUTHENTICATION-PROPS-REF DEST="SECURE-COMMUNICATION-AUTHENTICATION-PROPS">/P/Auth</AUTHENTICATION-PROPS-REF>
  <SECURE-COMMUNICATION-PROPS><DATA-ID>291</DATA-ID><FRESHNESS-VALUE-ID>1</FRESHNESS-VALUE-ID>
   <SECURED-AREA-OFFSET>64</SECURED-AREA-OFFSET><SECURED-AREA-LENGTH>32</SECURED-AREA-LENGTH>
  </SECURE-COMMUNICATION-PROPS>
 </SECURED-I-PDU>
 <SECURE-COMMUNICATION-AUTHENTICATION-PROPS><SHORT-NAME>Auth</SHORT-NAME>
  <AUTH-INFO-TX-LENGTH>4</AUTH-INFO-TX-LENGTH>
 </SECURE-COMMUNICATION-AUTHENTICATION-PROPS>
</ELEMENTS></AR-PACKAGE></AR-PACKAGES></AUTOSAR>"#;

#[test]
fn imports_container_multiplexed_and_secured_pdus() {
    let comm = import(ADVANCED_TEXT, "C", 1);
    let container = comm.pdus.values().find(|pdu| pdu.name == "Cont").unwrap();
    assert_eq!(container.kind, PduKind::Container);
    assert_eq!(container.header_type, ContainerHeaderType::Short);
    let mut headers: Vec<u32> = comm
        .contained_pdu_mappings
        .values()
        .filter(|mapping| mapping.container == container.id)
        .map(|mapping| mapping.header_id)
        .collect();
    headers.sort_unstable();
    assert_eq!(headers, vec![1, 2]);

    let mux = comm.pdus.values().find(|pdu| pdu.name == "Mux").unwrap();
    assert_eq!(mux.kind, PduKind::Multiplexed);
    let selector = mux.selector.as_ref().unwrap();
    assert_eq!(selector.start_bit, 0);
    assert_eq!(selector.length, 8);
    let parts: Vec<&MultiplexedPduPart> = comm
        .multiplexed_parts
        .values()
        .filter(|part| part.mux == mux.id)
        .collect();
    assert_eq!(parts.len(), 3);
    assert!(parts.iter().any(|part| part.selector_code == Some(0)));
    assert!(parts.iter().any(|part| part.selector_code == Some(1)));
    assert!(parts.iter().any(|part| part.selector_code.is_none()));

    let secured = comm.pdus.values().find(|pdu| pdu.name == "Sec").unwrap();
    assert_eq!(secured.kind, PduKind::Secured);
    let meta = secured.secured.as_ref().unwrap();
    assert_eq!(meta.header, SecuredPduHeader::Header16);
    assert_eq!(meta.data_id, Some(291));
    assert_eq!(meta.freshness_value_id, Some(1));
    assert_eq!(
        meta.secured_area,
        Some((64, 32)),
        "the secured area is read from SECURE-COMMUNICATION-PROPS"
    );
    assert_eq!(
        meta.auth_info_length,
        Some(4),
        "the authenticator length is read from the referenced props object"
    );
    assert_eq!(meta.verification, SecVerification::Unverified);
    let pdu_a = comm.pdus.values().find(|pdu| pdu.name == "PduA").unwrap();
    assert_eq!(meta.payload, Some(pdu_a.id));
}

#[test]
fn system_signal_data_constr_and_initial_value_fill_the_signal() {
    const TEXT: &str = r#"<AUTOSAR><AR-PACKAGES><AR-PACKAGE><SHORT-NAME>P</SHORT-NAME><ELEMENTS>
 <CAN-CLUSTER><SHORT-NAME>C</SHORT-NAME>
  <CAN-CLUSTER-VARIANTS><CAN-CLUSTER-CONDITIONAL>
   <CAN-FRAME-TRIGGERINGS><CAN-FRAME-TRIGGERING><SHORT-NAME>FT</SHORT-NAME>
    <FRAME-REF DEST="CAN-FRAME">/P/F</FRAME-REF><IDENTIFIER>256</IDENTIFIER>
   </CAN-FRAME-TRIGGERING></CAN-FRAME-TRIGGERINGS>
  </CAN-CLUSTER-CONDITIONAL></CAN-CLUSTER-VARIANTS>
 </CAN-CLUSTER>
 <CAN-FRAME><SHORT-NAME>F</SHORT-NAME><FRAME-LENGTH>8</FRAME-LENGTH>
  <PDU-TO-FRAME-MAPPING><SHORT-NAME>FM</SHORT-NAME>
   <PDU-REF DEST="I-SIGNAL-I-PDU">/P/Pdu</PDU-REF>
  </PDU-TO-FRAME-MAPPING>
 </CAN-FRAME>
 <I-SIGNAL-I-PDU><SHORT-NAME>Pdu</SHORT-NAME><LENGTH>8</LENGTH>
  <I-SIGNAL-TO-I-PDU-MAPPING><SHORT-NAME>SM</SHORT-NAME>
   <I-SIGNAL-REF DEST="I-SIGNAL">/P/Sig</I-SIGNAL-REF>
  </I-SIGNAL-TO-I-PDU-MAPPING>
 </I-SIGNAL-I-PDU>
 <I-SIGNAL><SHORT-NAME>Sig</SHORT-NAME><LENGTH>8</LENGTH>
  <SYSTEM-SIGNAL-REF DEST="SYSTEM-SIGNAL">/P/Sys</SYSTEM-SIGNAL-REF>
 </I-SIGNAL>
 <SYSTEM-SIGNAL><SHORT-NAME>Sys</SHORT-NAME>
  <INIT-VALUE><NUMERICAL-VALUE-SPECIFICATION><VALUE>7</VALUE></NUMERICAL-VALUE-SPECIFICATION></INIT-VALUE>
  <DATA-CONSTR-REF DEST="DATA-CONSTR">/P/Dc</DATA-CONSTR-REF>
 </SYSTEM-SIGNAL>
 <DATA-CONSTR><SHORT-NAME>Dc</SHORT-NAME>
  <DATA-CONSTR-RULES><DATA-CONSTR-RULE>
   <PHYS-CONSTRS>
    <LOWER-LIMIT>1</LOWER-LIMIT>
    <UPPER-LIMIT INTERVAL-TYPE="OPEN">9</UPPER-LIMIT>
   </PHYS-CONSTRS>
  </DATA-CONSTR-RULE></DATA-CONSTR-RULES>
 </DATA-CONSTR>
</ELEMENTS></AR-PACKAGE></AR-PACKAGES></AUTOSAR>"#;
    let comm = import(TEXT, "C", 1);
    let signal = comm.signals.values().next().unwrap();
    assert_eq!(signal.min, Some(1.0));
    assert_eq!(signal.max, Some(9.0));
    assert!(signal.max_open);
    assert_eq!(signal.initial_raw, Some(7.0));
}

#[test]
fn arxml_multiplexed_selector_field_gates_dynamic_parts() {
    const TEXT: &str = r#"<AUTOSAR><AR-PACKAGES><AR-PACKAGE><SHORT-NAME>P</SHORT-NAME><ELEMENTS>
 <CAN-CLUSTER><SHORT-NAME>C</SHORT-NAME>
  <CAN-CLUSTER-VARIANTS><CAN-CLUSTER-CONDITIONAL>
   <CAN-FRAME-TRIGGERINGS><CAN-FRAME-TRIGGERING><SHORT-NAME>FT</SHORT-NAME>
    <FRAME-REF DEST="CAN-FRAME">/P/F</FRAME-REF><IDENTIFIER>256</IDENTIFIER>
   </CAN-FRAME-TRIGGERING></CAN-FRAME-TRIGGERINGS>
  </CAN-CLUSTER-CONDITIONAL></CAN-CLUSTER-VARIANTS>
 </CAN-CLUSTER>
 <CAN-FRAME><SHORT-NAME>F</SHORT-NAME><FRAME-LENGTH>8</FRAME-LENGTH>
  <PDU-TO-FRAME-MAPPING><SHORT-NAME>FM</SHORT-NAME>
   <PDU-REF DEST="MULTIPLEXED-I-PDU">/P/Mux</PDU-REF>
  </PDU-TO-FRAME-MAPPING>
 </CAN-FRAME>
 <I-SIGNAL-I-PDU><SHORT-NAME>Pdu0</SHORT-NAME><LENGTH>8</LENGTH>
  <I-SIGNAL-TO-I-PDU-MAPPING><SHORT-NAME>M0</SHORT-NAME>
   <START-POSITION>8</START-POSITION>
   <I-SIGNAL-REF DEST="I-SIGNAL">/P/Sig0</I-SIGNAL-REF>
  </I-SIGNAL-TO-I-PDU-MAPPING>
 </I-SIGNAL-I-PDU>
 <I-SIGNAL-I-PDU><SHORT-NAME>Pdu1</SHORT-NAME><LENGTH>8</LENGTH>
  <I-SIGNAL-TO-I-PDU-MAPPING><SHORT-NAME>M1</SHORT-NAME>
   <START-POSITION>8</START-POSITION>
   <I-SIGNAL-REF DEST="I-SIGNAL">/P/Sig1</I-SIGNAL-REF>
  </I-SIGNAL-TO-I-PDU-MAPPING>
 </I-SIGNAL-I-PDU>
 <I-SIGNAL><SHORT-NAME>Sig0</SHORT-NAME><LENGTH>8</LENGTH></I-SIGNAL>
 <I-SIGNAL><SHORT-NAME>Sig1</SHORT-NAME><LENGTH>8</LENGTH></I-SIGNAL>
 <MULTIPLEXED-I-PDU><SHORT-NAME>Mux</SHORT-NAME><LENGTH>8</LENGTH>
  <SELECTOR-FIELD-START-POSITION>0</SELECTOR-FIELD-START-POSITION>
  <SELECTOR-FIELD-LENGTH>8</SELECTOR-FIELD-LENGTH>
  <DYNAMIC-PARTS><DYNAMIC-PART><DYNAMIC-PART-ALTERNATIVES>
   <DYNAMIC-PART-ALTERNATIVE><SELECTOR-FIELD-CODE>0</SELECTOR-FIELD-CODE>
    <I-PDU-REF DEST="I-SIGNAL-I-PDU">/P/Pdu0</I-PDU-REF>
   </DYNAMIC-PART-ALTERNATIVE>
   <DYNAMIC-PART-ALTERNATIVE><SELECTOR-FIELD-CODE>1</SELECTOR-FIELD-CODE>
    <I-PDU-REF DEST="I-SIGNAL-I-PDU">/P/Pdu1</I-PDU-REF>
   </DYNAMIC-PART-ALTERNATIVE>
  </DYNAMIC-PART-ALTERNATIVES></DYNAMIC-PART></DYNAMIC-PARTS>
 </MULTIPLEXED-I-PDU>
</ELEMENTS></AR-PACKAGE></AR-PACKAGES></AUTOSAR>"#;
    let comm = import(TEXT, "C", 1);
    let frame = comm.frames.values().next().unwrap();
    let (plan, issues) = build_decode_plan(&comm, frame, 1);
    assert!(issues.is_empty(), "{issues:?}");
    let sig0 = comm
        .signals
        .values()
        .find(|signal| signal.name == "Sig0")
        .unwrap()
        .id;
    let sig1 = comm
        .signals
        .values()
        .find(|signal| signal.name == "Sig1")
        .unwrap()
        .id;
    let zero = crate::model::decode_payload(&comm, &plan, &[0, 0x11, 0, 0, 0, 0, 0, 0]);
    assert!(zero.iter().any(|value| value.signal == sig0));
    assert!(zero.iter().all(|value| value.signal != sig1));
    let one = crate::model::decode_payload(&comm, &plan, &[1, 0x22, 0, 0, 0, 0, 0, 0]);
    assert!(one.iter().any(|value| value.signal == sig1));
    assert!(one.iter().all(|value| value.signal != sig0));
}

#[test]
fn e2e_profile_links_to_system_signal_and_pdu() {
    const TEXT: &str = r#"<AUTOSAR><AR-PACKAGES><AR-PACKAGE><SHORT-NAME>P</SHORT-NAME><ELEMENTS>
 <CAN-CLUSTER><SHORT-NAME>C</SHORT-NAME></CAN-CLUSTER>
 <E2E-PROFILE-COMPATIBLE-PROTECTION><SHORT-NAME>E2EProt</SHORT-NAME>
  <PROFILE-NAME>Profile1</PROFILE-NAME>
  <PROFILE-CONFIGURATION><E2E-PROFILE-SETTINGS><DATA-ID>291</DATA-ID>
   <COUNTER-OFFSET>16</COUNTER-OFFSET><COUNTER-LENGTH>8</COUNTER-LENGTH>
  </E2E-PROFILE-SETTINGS></PROFILE-CONFIGURATION>
 </E2E-PROFILE-COMPATIBLE-PROTECTION>
 <SYSTEM-SIGNAL><SHORT-NAME>Sys</SHORT-NAME>
  <PROTECTION-HANDLE><PROTECTION-PROPS-REF DEST="E2E-PROFILE-COMPATIBLE-PROTECTION">/P/E2EProt</PROTECTION-PROPS-REF></PROTECTION-HANDLE>
 </SYSTEM-SIGNAL>
 <I-SIGNAL-I-PDU><SHORT-NAME>Pdu</SHORT-NAME><LENGTH>8</LENGTH>
  <I-SIGNAL-TO-I-PDU-MAPPING><SHORT-NAME>M</SHORT-NAME>
   <I-SIGNAL-REF DEST="I-SIGNAL">/P/Sig</I-SIGNAL-REF>
   <TRANSFORMATION-PROP-SET><DATA-TRANSFORM-PROPS>
    <PACKING-BYTE-ORDER>MOST-SIGNIFICANT-BYTE-FIRST</PACKING-BYTE-ORDER>
   </DATA-TRANSFORM-PROPS></TRANSFORMATION-PROP-SET>
  </I-SIGNAL-TO-I-PDU-MAPPING>
 </I-SIGNAL-I-PDU>
 <I-SIGNAL><SHORT-NAME>Sig</SHORT-NAME><LENGTH>8</LENGTH>
  <SYSTEM-SIGNAL-REF DEST="SYSTEM-SIGNAL">/P/Sys</SYSTEM-SIGNAL-REF>
 </I-SIGNAL>
</ELEMENTS></AR-PACKAGE></AR-PACKAGES></AUTOSAR>"#;
    let comm = import(TEXT, "C", 1);
    let protection = comm.e2e.values().next().expect("E2E profile read");
    assert_eq!(protection.profile_name, "Profile1");
    assert_eq!(protection.profile, Some(1));
    assert_eq!(protection.data_id, Some(291));
    assert_eq!(protection.counter_offset, Some(16));
    assert_eq!(protection.counter_length, Some(8));
    let pdu = comm.pdus.values().next().unwrap();
    assert_eq!(pdu.e2e, Some(protection.id));
    assert_eq!(protection.payload, Some(pdu.id));
    assert!(protection.system_signal.is_some());
    assert_eq!(
        comm.signals.values().next().unwrap().byte_order,
        ByteOrder::BigEndian,
        "packing order inside TRANSFORMATION-PROP-SET is honored"
    );
}

#[test]
fn imports_real_flexray_matrix() {
    let path = crate::test_assets::path("PowerTrain.arxml");
    let text = String::from_utf8_lossy(&std::fs::read(path).unwrap()).into_owned();
    let comm = import(&text, "PowerTrain", 1);
    assert_eq!(comm.frames.len(), 48);
    assert_eq!(comm.pdus.len(), 24);
    assert!(!comm.signals.is_empty());
    assert!(comm.signals.len() <= 38);
    assert!(
        comm.frames
            .values()
            .all(|frame| matches!(frame.address, FrameAddress::FlexRay { .. }))
    );
    let NetworkParams::FlexRay(fr) = &comm.networks.get(&NetworkId(1)).unwrap().params else {
        panic!("expected FlexRay network");
    };
    assert!(fr.speed_kbps > 0);
    assert!(comm.frames.values().all(|frame| matches!(
        frame.address,
        FrameAddress::FlexRay { slot, .. } if slot > 0
    )));
    assert!(
        comm.frames
            .values()
            .any(|frame| frame.timing.cycle_us == Some(10_000)),
        "a PDU TIME-PERIOD of 0.01 s becomes a 10 ms frame cycle"
    );
    assert!(
        comm.integrity_issues().is_empty(),
        "{:?}",
        comm.integrity_issues()
    );
}

#[test]
fn missing_cluster_is_an_error_not_a_panic() {
    let mut ids = ElementIds::new();
    let source = ClusterSource::Arxml {
        path: "test.arxml".to_string(),
        cluster: "Nope".to_string(),
    };
    let outcome = import_cluster(CAN_TEXT, "Nope", ClusterId(1), &source, &mut ids);
    assert!(outcome.comm.clusters.is_empty());
    assert!(
        outcome
            .issues
            .iter()
            .any(|issue| issue.message.contains("not found"))
    );
}

/// An I-SIGNAL's `DATA-TRANSFORMATIONS` reference is recorded on the signal
/// and materialised as a transformation object. Its numeric behaviour is not
/// interpreted (there is no real corpus for the chain), so the decode plan
/// reports `UnsupportedTransformation` instead of silently showing raw values.
#[test]
fn a_signal_transformation_reference_links_to_a_transformation() {
    let text = CAN_TEXT.replace(
        "<I-SIGNAL><SHORT-NAME>Speed</SHORT-NAME>",
        "<DATA-TRANSFORMATION><SHORT-NAME>Scramble</SHORT-NAME>\
         <TRANSFORMER-CHAIN-REFS><TRANSFORMER-CHAIN-REF DEST=\"TRANSFORMATION-TECHNOLOGY\">\
         /Pkg/Tech</TRANSFORMER-CHAIN-REF></TRANSFORMER-CHAIN-REFS></DATA-TRANSFORMATION>\
         <I-SIGNAL><SHORT-NAME>Speed</SHORT-NAME>\
         <DATA-TRANSFORMATIONS><DATA-TRANSFORMATION-REF-CONDITIONAL>\
         <DATA-TRANSFORMATION-REF DEST=\"DATA-TRANSFORMATION\">/Pkg/Scramble</DATA-TRANSFORMATION-REF>\
         </DATA-TRANSFORMATION-REF-CONDITIONAL></DATA-TRANSFORMATIONS>",
    );
    let comm = import(&text, "CanCluster", 1);
    let signal = comm.signals.values().next().expect("signal imported");
    let transformation = signal.transformation.expect("reference linked");
    assert_eq!(
        comm.transformations
            .get(&transformation)
            .map(|t| t.name.as_str()),
        Some("Scramble")
    );
    let (_, issues) = PlanCache::build(&comm, 1);
    assert!(
        issues
            .iter()
            .any(|issue| issue.kind == IssueKind::UnsupportedTransformation),
        "an unknown chain is reported, not silently ignored: {issues:?}"
    );
}

/// Frame ports declare who sends a frame; the model exposes that so the
/// generator's node table no longer has to read the DBC's transmitter field.
#[test]
fn frame_ports_name_the_sender_ecu() {
    let comm = import(CAN_TEXT, "CanCluster", 1);
    let frame = comm.frames.values().next().expect("frame imported");
    assert_eq!(comm.frame_sender(frame), Some("ECU_A"));
    let sent = comm.frames_sent_by(frame.cluster, "ECU_A");
    assert_eq!(sent.len(), 1);
    assert!(comm.frames_sent_by(frame.cluster, "ECU_B").is_empty());
}

/// A frame's DESC is part of the model now, so the catalog no longer needs
/// the legacy DBC table just to show the comment.
#[test]
fn a_frame_comment_comes_from_the_document() {
    let text = CAN_TEXT.replace(
        "<CAN-FRAME><SHORT-NAME>EngineFrame</SHORT-NAME>",
        "<CAN-FRAME><SHORT-NAME>EngineFrame</SHORT-NAME>\
         <DESC><L-2 L=\"FOR-ALL\">Engine frame</L-2></DESC>",
    );
    let comm = import(&text, "CanCluster", 1);
    let frame = comm.frames.values().next().expect("frame imported");
    assert_eq!(frame.comment, "Engine frame");
}

/// The update-indication position travels from the mapping into the
/// decode/encode plan, offset by the PDU's start inside the frame.
#[test]
fn update_indication_bit_reaches_the_plan() {
    let text = CAN_TEXT.replace(
        "<PACKING-BYTE-ORDER>MOST-SIGNIFICANT-BYTE-LAST</PACKING-BYTE-ORDER>",
        "<PACKING-BYTE-ORDER>MOST-SIGNIFICANT-BYTE-LAST</PACKING-BYTE-ORDER>\
         <UPDATE-INDICATION-BIT-POSITION>0</UPDATE-INDICATION-BIT-POSITION>",
    );
    let comm = import(&text, "CanCluster", 1);
    let (plans, _) = PlanCache::build(&comm, 1);
    let frame = comm.frames.values().next().unwrap();
    let plan = plans.get(frame.id).expect("plan");
    assert_eq!(
        plan.reads[0].update_bit,
        Some(0),
        "declared PDU-relative bit"
    );
    assert_eq!(plan.writes[0].update_bit, Some(0));
}

/// The declared update bit is not just copied into the plan: a clear bit
/// hides the signal at bit level, and encoding a value raises it.
#[test]
fn update_indication_bit_gates_decode_and_is_set_by_encode() {
    let text = CAN_TEXT.replace(
        "<PACKING-BYTE-ORDER>MOST-SIGNIFICANT-BYTE-LAST</PACKING-BYTE-ORDER>",
        "<PACKING-BYTE-ORDER>MOST-SIGNIFICANT-BYTE-LAST</PACKING-BYTE-ORDER>\
         <UPDATE-INDICATION-BIT-POSITION>0</UPDATE-INDICATION-BIT-POSITION>",
    );
    let comm = import(&text, "CanCluster", 1);
    let (plans, _) = PlanCache::build(&comm, 1);
    let frame = comm.frames.values().next().unwrap();
    let plan = plans.get(frame.id).expect("plan");

    // Little-endian 16-bit Speed at bit 8: raw 0x1234 is bytes 1..3.
    let clear = [0x00, 0x34, 0x12, 0, 0, 0, 0, 0];
    assert!(
        crate::model::decode_payload(&comm, plan, &clear).is_empty(),
        "a clear update bit means the frame carries no new value"
    );
    let set = [0x01, 0x34, 0x12, 0, 0, 0, 0, 0];
    let decoded = crate::model::decode_payload(&comm, plan, &set);
    assert_eq!(decoded.len(), 1);
    assert_eq!(decoded[0].raw, 0x1234);
    assert!((decoded[0].physical.expect("physical") - 466.0).abs() < 1e-9);

    let signal = plan.writes[0].signal;
    let mut payload = [0u8; 8];
    let written = crate::model::encode_payload(&comm, plan, &[(signal, 466.0)], &mut payload);
    assert_eq!(written, 1);
    assert_eq!(payload[0] & 1, 1, "encoding raises the update bit");
    assert_eq!(&payload[1..3], &[0x34, 0x12], "little-endian raw 0x1234");
}

/// AUTOSAR 4.2 shapes taken from the cantools `system-4.2.arxml` fixture:
/// a SECURED-I-PDU whose PAYLOAD-REF names a PDU-TRIGGERING (whose own
/// I-PDU-REF is the protected PDU), and E2E declared on a signal group
/// through a TRANSFORMATION-TECHNOLOGY in a DATA-TRANSFORMATION-SET.
#[test]
fn a_42_secured_payload_triggering_and_group_e2e_are_read() {
    const TEXT: &str = r#"<AUTOSAR><AR-PACKAGES><AR-PACKAGE><SHORT-NAME>P</SHORT-NAME><ELEMENTS>
 <CAN-CLUSTER><SHORT-NAME>C</SHORT-NAME></CAN-CLUSTER>
 <PDU-TRIGGERING><SHORT-NAME>Trig</SHORT-NAME>
  <I-PDU-REF DEST="I-SIGNAL-I-PDU">/P/Inner</I-PDU-REF>
 </PDU-TRIGGERING>
 <SECURED-I-PDU><SHORT-NAME>Sec</SHORT-NAME><LENGTH>8</LENGTH>
  <PAYLOAD-REF DEST="PDU-TRIGGERING">/P/Trig</PAYLOAD-REF>
  <SECURE-COMMUNICATION-PROPS><DATA-ID>1337</DATA-ID></SECURE-COMMUNICATION-PROPS>
 </SECURED-I-PDU>
 <I-SIGNAL-I-PDU><SHORT-NAME>Inner</SHORT-NAME><LENGTH>8</LENGTH>
  <I-SIGNAL-TO-I-PDU-MAPPINGS><I-SIGNAL-TO-I-PDU-MAPPING><SHORT-NAME>M</SHORT-NAME>
   <START-POSITION>0</START-POSITION>
   <PACKING-BYTE-ORDER>MOST-SIGNIFICANT-BYTE-LAST</PACKING-BYTE-ORDER>
   <I-SIGNAL-REF DEST="I-SIGNAL">/P/SeqCounter</I-SIGNAL-REF>
  </I-SIGNAL-TO-I-PDU-MAPPING></I-SIGNAL-TO-I-PDU-MAPPINGS>
 </I-SIGNAL-I-PDU>
 <I-SIGNAL><SHORT-NAME>SeqCounter</SHORT-NAME><LENGTH>4</LENGTH></I-SIGNAL>
 <I-SIGNAL-GROUP><SHORT-NAME>G</SHORT-NAME>
  <I-SIGNAL-REFS><I-SIGNAL-REF DEST="I-SIGNAL">/P/SeqCounter</I-SIGNAL-REF></I-SIGNAL-REFS>
  <TRANSFORMATION-I-SIGNAL-PROPSS><END-TO-END-TRANSFORMATION-I-SIGNAL-PROPS>
   <END-TO-END-TRANSFORMATION-I-SIGNAL-PROPS-VARIANTS>
    <END-TO-END-TRANSFORMATION-I-SIGNAL-PROPS-CONDITIONAL>
     <TRANSFORMER-REF DEST="TRANSFORMATION-TECHNOLOGY">/P/Set/Tech</TRANSFORMER-REF>
     <DATA-IDS><DATA-ID>123</DATA-ID><DATA-ID>124</DATA-ID></DATA-IDS>
    </END-TO-END-TRANSFORMATION-I-SIGNAL-PROPS-CONDITIONAL>
   </END-TO-END-TRANSFORMATION-I-SIGNAL-PROPS-VARIANTS>
  </END-TO-END-TRANSFORMATION-I-SIGNAL-PROPS></TRANSFORMATION-I-SIGNAL-PROPSS>
 </I-SIGNAL-GROUP>
 <DATA-TRANSFORMATION-SET><SHORT-NAME>Set</SHORT-NAME>
  <TRANSFORMATION-TECHNOLOGYS><TRANSFORMATION-TECHNOLOGY><SHORT-NAME>Tech</SHORT-NAME>
   <PROTOCOL>E2E</PROTOCOL>
   <TRANSFORMATION-DESCRIPTIONS><END-TO-END-TRANSFORMATION-DESCRIPTION>
    <PROFILE-NAME>Profile2</PROFILE-NAME>
   </END-TO-END-TRANSFORMATION-DESCRIPTION></TRANSFORMATION-DESCRIPTIONS>
  </TRANSFORMATION-TECHNOLOGY></TRANSFORMATION-TECHNOLOGYS>
 </DATA-TRANSFORMATION-SET>
</ELEMENTS></AR-PACKAGE></AR-PACKAGES></AUTOSAR>"#;
    let comm = import(TEXT, "C", 1);
    let secured = comm
        .pdus
        .values()
        .find(|pdu| pdu.name == "Sec")
        .expect("secured pdu");
    let meta = secured.secured.as_ref().expect("secured meta");
    assert_eq!(meta.data_id, Some(1337));
    assert!(
        meta.payload.is_some(),
        "PDU-TRIGGERING indirection resolves to the protected PDU"
    );
    let group = comm
        .groups
        .values()
        .find(|group| group.name == "G")
        .expect("group");

    let e2e = group.e2e.as_ref().expect("group-level E2E read");
    assert_eq!(e2e.profile_name, "Profile2");
    assert_eq!(e2e.profile, Some(2));
    assert_eq!(e2e.data_ids, vec![123, 124]);
    assert!(
        comm.issues
            .iter()
            .any(|issue| issue.message.contains("counter continuity is not planned")),
        "group-level E2E is recorded but explicitly not planned: {:?}",
        comm.issues
    );
}
