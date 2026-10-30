use super::*;

#[test]
fn fibex_imports_frames_and_signals() {
    let path = crate::test_assets::path("PowerTrain_v2.xml");
    let text = String::from_utf8_lossy(&std::fs::read(path).unwrap()).into_owned();
    let mut ids = ElementIds::new();
    let source = ClusterSource::Fibex {
        path: "PowerTrain_v2.xml".to_string(),
    };
    let outcome = import_fibex(&text, "PowerTrain", ClusterId(4), &source, &mut ids);
    assert_eq!(outcome.comm.frames.len(), 6, "{:?}", outcome.issues);
    assert!(
        outcome
            .comm
            .frames
            .values()
            .any(|frame| frame.name == "ABSInfo")
    );
    assert!(!outcome.comm.signals.is_empty());
    assert!(outcome.comm.frames.values().all(|frame| matches!(
        frame.address,
        FrameAddress::FlexRay { slot, .. } if slot > 0
    )));
    let NetworkParams::FlexRay(fr) = &outcome.comm.networks.get(&NetworkId(4)).unwrap().params
    else {
        panic!("expected FlexRay network");
    };
    assert!(fr.speed_kbps > 0);
    assert!(outcome.comm.integrity_issues().is_empty());
}

#[test]
fn ethernet_database_imports_vlan_frame_and_signals() {
    let text = std::fs::read_to_string(crate::test_assets::eth_arxml()).unwrap();
    let mut ids = ElementIds::new();
    let source = ClusterSource::Fibex {
        path: "EthSample.arxml".to_string(),
    };
    let outcome = import_eth_db(&text, "EthCluster", ClusterId(5), &source, &mut ids);
    let frame = outcome.comm.frames.values().next().unwrap();
    assert_eq!(frame.name, "EthFrame");
    assert_eq!(frame.length, 22);
    assert_eq!(frame.address, FrameAddress::Ethernet { vlan: Some(100) });
    assert_eq!(frame.ethernet, Some(EthernetFrameMeta::Base));
    let signal = outcome.comm.signals.values().next().unwrap();
    assert_eq!(signal.start_bit, 8);
    assert_eq!(signal.length, 16);
    assert_eq!(signal.factor, 0.1);
    let NetworkParams::Ethernet(eth) = &outcome.comm.networks.get(&NetworkId(5)).unwrap().params
    else {
        panic!("expected Ethernet network");
    };
    assert_eq!(eth.vlans, vec![100]);
}

#[test]
fn dbc_and_arxml_agree_on_the_same_message() {
    const ARXML: &str = r#"<AUTOSAR><AR-PACKAGES><AR-PACKAGE><SHORT-NAME>P</SHORT-NAME><ELEMENTS>
 <CAN-CLUSTER><SHORT-NAME>CAN1</SHORT-NAME>
  <CAN-CLUSTER-VARIANTS><CAN-CLUSTER-CONDITIONAL>
   <BAUDRATE>500000</BAUDRATE>
   <CAN-FRAME-TRIGGERINGS><CAN-FRAME-TRIGGERING><SHORT-NAME>FT</SHORT-NAME>
    <FRAME-REF DEST="CAN-FRAME">/P/EngineStatus</FRAME-REF>
    <IDENTIFIER>256</IDENTIFIER><CAN-ADDRESSING-MODE>STANDARD</CAN-ADDRESSING-MODE>
   </CAN-FRAME-TRIGGERING></CAN-FRAME-TRIGGERINGS>
  </CAN-CLUSTER-CONDITIONAL></CAN-CLUSTER-VARIANTS>
 </CAN-CLUSTER>
 <CAN-FRAME><SHORT-NAME>EngineStatus</SHORT-NAME><FRAME-LENGTH>8</FRAME-LENGTH>
  <PDU-TO-FRAME-MAPPING><SHORT-NAME>FM</SHORT-NAME>
   <PDU-REF DEST="I-SIGNAL-I-PDU">/P/EngineStatusPdu</PDU-REF>
  </PDU-TO-FRAME-MAPPING>
 </CAN-FRAME>
 <I-SIGNAL-I-PDU><SHORT-NAME>EngineStatusPdu</SHORT-NAME><LENGTH>8</LENGTH>
  <I-SIGNAL-TO-I-PDU-MAPPING><SHORT-NAME>SM</SHORT-NAME>
   <START-POSITION>0</START-POSITION>
   <PACKING-BYTE-ORDER>MOST-SIGNIFICANT-BYTE-LAST</PACKING-BYTE-ORDER>
   <I-SIGNAL-REF DEST="I-SIGNAL">/P/EngineSpeed</I-SIGNAL-REF>
  </I-SIGNAL-TO-I-PDU-MAPPING>
 </I-SIGNAL-I-PDU>
 <I-SIGNAL><SHORT-NAME>EngineSpeed</SHORT-NAME><LENGTH>16</LENGTH>
  <COMPU-METHOD-REF DEST="COMPU-METHOD">/P/SpeedCompu</COMPU-METHOD-REF>
 </I-SIGNAL>
 <COMPU-METHOD><SHORT-NAME>SpeedCompu</SHORT-NAME>
  <COMPU-INTERNAL-TO-PHYS><COMPU-SCALES><COMPU-SCALE>
   <LOWER-LIMIT>0</LOWER-LIMIT><UPPER-LIMIT>8000</UPPER-LIMIT>
   <COMPU-RATIONAL-COEFFS><COMPU-NUMERATOR><V>0</V><V>0.25</V></COMPU-NUMERATOR><COMPU-DENOMINATOR><V>1</V></COMPU-DENOMINATOR></COMPU-RATIONAL-COEFFS>
   <UNIT-REF DEST="UNIT">/P/Rpm</UNIT-REF>
  </COMPU-SCALE></COMPU-SCALES></COMPU-INTERNAL-TO-PHYS>
 </COMPU-METHOD>
 <UNIT><SHORT-NAME>Rpm</SHORT-NAME><DISPLAY-NAME>rpm</DISPLAY-NAME></UNIT>
</ELEMENTS></AR-PACKAGE></AR-PACKAGES></AUTOSAR>"#;
    let mut dbc_ids = ElementIds::new();
    let dbc_source = ClusterSource::Dbc {
        path: "sample.dbc".to_string(),
    };
    let dbc = import_dbc(
        crate::sample_data::SAMPLE_DBC,
        "CAN1",
        ClusterId(1),
        &dbc_source,
        &mut dbc_ids,
    )
    .comm;
    let mut arxml_ids = ElementIds::new();
    let arxml_source = ClusterSource::Arxml {
        path: "inline.arxml".to_string(),
        cluster: "CAN1".to_string(),
    };
    let arxml = import_cluster(ARXML, "CAN1", ClusterId(1), &arxml_source, &mut arxml_ids).comm;
    let dbc_frame = dbc
        .frames
        .values()
        .find(|frame| frame.name == "EngineStatus")
        .unwrap();
    let arxml_frame = arxml
        .frames
        .values()
        .find(|frame| frame.name == "EngineStatus")
        .unwrap();
    assert_eq!(dbc_frame.address, arxml_frame.address);
    assert_eq!(dbc_frame.length, arxml_frame.length);
    let dbc_sig = dbc
        .signals
        .values()
        .find(|signal| signal.name == "EngineSpeed")
        .unwrap();
    let arxml_sig = arxml
        .signals
        .values()
        .find(|signal| signal.name == "EngineSpeed")
        .unwrap();
    assert_eq!(dbc_sig.start_bit, arxml_sig.start_bit);
    assert_eq!(dbc_sig.length, arxml_sig.length);
    assert_eq!(dbc_sig.factor, arxml_sig.factor);
    assert_eq!(dbc_sig.unit, arxml_sig.unit);
    assert_eq!(dbc_sig.max, arxml_sig.max);
    assert_eq!(dbc_sig.byte_order, arxml_sig.byte_order);
}
