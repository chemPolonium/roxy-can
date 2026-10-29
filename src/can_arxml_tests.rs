use crate::can_arxml::{parse_cluster, scan};

pub(crate) const SAMPLE: &str = r#"<?xml version="1.0"?>
<AUTOSAR>
 <AR-PACKAGES><AR-PACKAGE><SHORT-NAME>P</SHORT-NAME><ELEMENTS>
  <CAN-CLUSTER><SHORT-NAME>CanA</SHORT-NAME>
   <CAN-CLUSTER-VARIANTS><CAN-CLUSTER-CONDITIONAL><BAUDRATE>500000</BAUDRATE>
    <PHYSICAL-CHANNELS><CAN-PHYSICAL-CHANNEL><SHORT-NAME>CH</SHORT-NAME>
     <FRAME-TRIGGERINGS>
      <CAN-FRAME-TRIGGERING><SHORT-NAME>FT_A</SHORT-NAME>
       <FRAME-PORT-REFS><FRAME-PORT-REF>/ECU/Engine/CN/FP_A_Tx</FRAME-PORT-REF></FRAME-PORT-REFS>
       <FRAME-REF>/Frames/A</FRAME-REF>
       <CAN-ADDRESSING-MODE>STANDARD</CAN-ADDRESSING-MODE>
       <IDENTIFIER>291</IDENTIFIER>
      </CAN-FRAME-TRIGGERING>
     </FRAME-TRIGGERINGS>
    </CAN-PHYSICAL-CHANNEL></PHYSICAL-CHANNELS>
   </CAN-CLUSTER-CONDITIONAL></CAN-CLUSTER-VARIANTS>
  </CAN-CLUSTER>
  <CAN-CLUSTER><SHORT-NAME>CanB</SHORT-NAME>
   <CAN-CLUSTER-VARIANTS><CAN-CLUSTER-CONDITIONAL><BAUDRATE>250000</BAUDRATE>
    <PHYSICAL-CHANNELS><CAN-PHYSICAL-CHANNEL><SHORT-NAME>CH</SHORT-NAME>
     <FRAME-TRIGGERINGS>
      <CAN-FRAME-TRIGGERING><SHORT-NAME>FT_B</SHORT-NAME>
       <FRAME-PORT-REFS><FRAME-PORT-REF>/ECU/Gateway/CN/FP_B_Tx</FRAME-PORT-REF></FRAME-PORT-REFS>
       <FRAME-REF>/Frames/B</FRAME-REF>
       <CAN-ADDRESSING-MODE>EXTENDED</CAN-ADDRESSING-MODE>
       <IDENTIFIER>1110</IDENTIFIER>
      </CAN-FRAME-TRIGGERING>
     </FRAME-TRIGGERINGS>
    </CAN-PHYSICAL-CHANNEL></PHYSICAL-CHANNELS>
   </CAN-CLUSTER-CONDITIONAL></CAN-CLUSTER-VARIANTS>
  </CAN-CLUSTER>
  <CAN-FRAME><SHORT-NAME>A</SHORT-NAME><FRAME-LENGTH>8</FRAME-LENGTH>
   <PDU-TO-FRAME-MAPPINGS><PDU-TO-FRAME-MAPPING>
    <PDU-REF>/P/PDU_A</PDU-REF><START-POSITION>8</START-POSITION>
    <PACKING-BYTE-ORDER>MOST-SIGNIFICANT-BYTE-LAST</PACKING-BYTE-ORDER>
   </PDU-TO-FRAME-MAPPING></PDU-TO-FRAME-MAPPINGS>
  </CAN-FRAME>
  <CAN-FRAME><SHORT-NAME>B</SHORT-NAME><FRAME-LENGTH>4</FRAME-LENGTH>
   <PDU-TO-FRAME-MAPPINGS><PDU-TO-FRAME-MAPPING>
    <PDU-REF>/P/PDU_B</PDU-REF><START-POSITION>0</START-POSITION>
    <PACKING-BYTE-ORDER>MOST-SIGNIFICANT-BYTE-LAST</PACKING-BYTE-ORDER>
   </PDU-TO-FRAME-MAPPING></PDU-TO-FRAME-MAPPINGS>
  </CAN-FRAME>
  <I-SIGNAL-I-PDU><SHORT-NAME>PDU_A</SHORT-NAME><LENGTH>8</LENGTH>
   <I-PDU-TIMING-SPECIFICATIONS><I-PDU-TIMING><TRANSMISSION-MODE-DECLARATION>
    <TRANSMISSION-MODE-TRUE-TIMING><CYCLIC-TIMING><TIME-PERIOD><VALUE>0.02</VALUE></TIME-PERIOD></CYCLIC-TIMING></TRANSMISSION-MODE-TRUE-TIMING>
   </TRANSMISSION-MODE-DECLARATION></I-PDU-TIMING></I-PDU-TIMING-SPECIFICATIONS>
   <I-SIGNAL-TO-I-PDU-MAPPINGS><I-SIGNAL-TO-I-PDU-MAPPING>
    <I-SIGNAL-REF>/Signals/Speed</I-SIGNAL-REF><START-POSITION>0</START-POSITION>
    <PACKING-BYTE-ORDER>MOST-SIGNIFICANT-BYTE-LAST</PACKING-BYTE-ORDER>
   </I-SIGNAL-TO-I-PDU-MAPPING></I-SIGNAL-TO-I-PDU-MAPPINGS>
  </I-SIGNAL-I-PDU>
  <I-SIGNAL-I-PDU><SHORT-NAME>PDU_B</SHORT-NAME><LENGTH>4</LENGTH>
   <I-SIGNAL-TO-I-PDU-MAPPINGS><I-SIGNAL-TO-I-PDU-MAPPING>
    <I-SIGNAL-REF>/Signals/Torque</I-SIGNAL-REF><START-POSITION>0</START-POSITION>
    <PACKING-BYTE-ORDER>MOST-SIGNIFICANT-BYTE-FIRST</PACKING-BYTE-ORDER>
   </I-SIGNAL-TO-I-PDU-MAPPING></I-SIGNAL-TO-I-PDU-MAPPINGS>
  </I-SIGNAL-I-PDU>
  <I-SIGNAL><SHORT-NAME>Speed</SHORT-NAME><LENGTH>16</LENGTH>
   <NETWORK-REPRESENTATION-PROPS><SW-DATA-DEF-PROPS-VARIANTS><SW-DATA-DEF-PROPS-CONDITIONAL>
    <BASE-TYPE-REF>/Types/uint16</BASE-TYPE-REF>
    <COMPU-METHOD-REF>/Compu/CM_Speed</COMPU-METHOD-REF>
   </SW-DATA-DEF-PROPS-CONDITIONAL></SW-DATA-DEF-PROPS-VARIANTS></NETWORK-REPRESENTATION-PROPS>
  </I-SIGNAL>
  <I-SIGNAL><SHORT-NAME>Torque</SHORT-NAME><LENGTH>12</LENGTH>
   <NETWORK-REPRESENTATION-PROPS><SW-DATA-DEF-PROPS-VARIANTS><SW-DATA-DEF-PROPS-CONDITIONAL>
    <BASE-TYPE-REF>/Types/sint16</BASE-TYPE-REF>
    <COMPU-METHOD-REF>/Compu/CM_Torque</COMPU-METHOD-REF>
   </SW-DATA-DEF-PROPS-CONDITIONAL></SW-DATA-DEF-PROPS-VARIANTS></NETWORK-REPRESENTATION-PROPS>
  </I-SIGNAL>
  <COMPU-METHOD><SHORT-NAME>CM_Speed</SHORT-NAME>
   <COMPU-INTERNAL-TO-PHYS><COMPU-SCALES><COMPU-SCALE>
    <LOWER-LIMIT>0</LOWER-LIMIT><UPPER-LIMIT>2500</UPPER-LIMIT>
    <COMPU-RATIONAL-COEFFS><COMPU-NUMERATOR><V>0</V><V>0.01</V></COMPU-NUMERATOR>
     <COMPU-DENOMINATOR><V>1</V></COMPU-DENOMINATOR></COMPU-RATIONAL-COEFFS>
   </COMPU-SCALE></COMPU-SCALES></COMPU-INTERNAL-TO-PHYS>
   <UNIT-REF>/Units/kmh</UNIT-REF>
  </COMPU-METHOD>
  <COMPU-METHOD><SHORT-NAME>CM_Torque</SHORT-NAME>
   <COMPU-INTERNAL-TO-PHYS><COMPU-SCALES>
    <COMPU-SCALE><LOWER-LIMIT>0</LOWER-LIMIT><COMPU-CONST><VT>idle</VT></COMPU-CONST></COMPU-SCALE>
    <COMPU-SCALE><LOWER-LIMIT>1</LOWER-LIMIT><COMPU-CONST><VT>drive</VT></COMPU-CONST></COMPU-SCALE>
   </COMPU-SCALES></COMPU-INTERNAL-TO-PHYS>
   <UNIT-REF>/Units/nm</UNIT-REF>
  </COMPU-METHOD>
  <UNIT><SHORT-NAME>kmh</SHORT-NAME><DISPLAY-NAME>km/h</DISPLAY-NAME></UNIT>
  <UNIT><SHORT-NAME>nm</SHORT-NAME><DISPLAY-NAME>Nm</DISPLAY-NAME></UNIT>
  <ECU-INSTANCE><SHORT-NAME>Engine</SHORT-NAME><CONNECTORS><CAN-COMMUNICATION-CONNECTOR><SHORT-NAME>CN</SHORT-NAME>
   <FRAME-PORTS><FRAME-PORT><SHORT-NAME>FP_A_Tx</SHORT-NAME><COMMUNICATION-DIRECTION>OUT</COMMUNICATION-DIRECTION></FRAME-PORT></FRAME-PORTS>
  </CAN-COMMUNICATION-CONNECTOR></CONNECTORS></ECU-INSTANCE>
  <ECU-INSTANCE><SHORT-NAME>Gateway</SHORT-NAME><CONNECTORS><CAN-COMMUNICATION-CONNECTOR><SHORT-NAME>CN</SHORT-NAME>
   <FRAME-PORTS><FRAME-PORT><SHORT-NAME>FP_B_Tx</SHORT-NAME><COMMUNICATION-DIRECTION>OUT</COMMUNICATION-DIRECTION></FRAME-PORT></FRAME-PORTS>
  </CAN-COMMUNICATION-CONNECTOR></CONNECTORS></ECU-INSTANCE>
 </ELEMENTS></AR-PACKAGE></AR-PACKAGES>
</AUTOSAR>"#;

#[test]
fn scans_every_cluster_kind_in_document_order() {
    let extra = SAMPLE.replace(
        "</ELEMENTS>",
        "<FLEXRAY-CLUSTER><SHORT-NAME>FR_A</SHORT-NAME></FLEXRAY-CLUSTER>\
         <ETHERNET-CLUSTER><SHORT-NAME>ETH_A</SHORT-NAME></ETHERNET-CLUSTER>\
         </ELEMENTS>",
    );
    let clusters = scan(&extra).expect("scan");
    let kinds: Vec<(&str, &str)> = clusters
        .iter()
        .map(|c| (c.kind.label(), c.name.as_str()))
        .collect();
    assert_eq!(
        kinds,
        [
            ("CAN", "CanA"),
            ("CAN", "CanB"),
            ("FlexRay", "FR_A"),
            ("Ethernet", "ETH_A")
        ]
    );
}

#[test]
fn one_named_can_cluster_becomes_the_can_table() {
    let a = parse_cluster(SAMPLE, "CanA").expect("CanA");
    assert_eq!(a.cluster, "CanA");
    assert_eq!(a.bitrate_kbps, Some(500));
    assert_eq!(a.db.nodes, ["Engine"], "only this cluster's sender");
    assert_eq!(a.db.order, [(0x123, false)]);
    let msg = &a.db.messages[&(0x123, false)];
    assert_eq!(msg.name, "A");
    assert_eq!(msg.dlc, 8);
    assert_eq!(msg.transmitter, "Engine");
    assert_eq!(msg.cycle_us, Some(20_000));
    assert_eq!(msg.signals.len(), 1);
    let sig = &msg.signals[0];
    assert_eq!(sig.name, "Speed");
    assert_eq!(sig.start_bit, 8, "PDU offset + signal offset");
    assert_eq!(sig.size, 16);
    assert!(!sig.big_endian);
    assert!((sig.factor - 0.01).abs() < 1e-12);
    assert_eq!(sig.unit, "km/h");
    assert_eq!(sig.type_tag, "u16");
    assert_eq!(sig.min, 0.0);
    assert!((sig.max - 25.0).abs() < 1e-12);

    let b = parse_cluster(SAMPLE, "CanB").expect("CanB");
    assert_eq!(b.bitrate_kbps, Some(250));
    assert_eq!(b.db.order, [(0x456, true)], "extended address mode");
    let msg = &b.db.messages[&(0x456, true)];
    assert_eq!(msg.transmitter, "Gateway");
    assert_eq!(msg.signals[0].name, "Torque");
    assert!(msg.signals[0].big_endian);
    assert_eq!(msg.signals[0].type_tag, "i12");
    assert_eq!(
        b.db.value_tables[&((0x456, true), "Torque".to_string())],
        [(0, "idle".to_string()), (1, "drive".to_string())]
            .into_iter()
            .collect()
    );
}

#[test]
fn signal_init_value_is_read_as_a_raw_start_value() {
    let xml = SAMPLE.replace(
        "<I-SIGNAL><SHORT-NAME>Speed</SHORT-NAME><LENGTH>16</LENGTH>",
        "<I-SIGNAL><SHORT-NAME>Speed</SHORT-NAME><INIT-VALUE>\
         <NUMERICAL-VALUE-SPECIFICATION><VALUE>500</VALUE></NUMERICAL-VALUE-SPECIFICATION>\
         </INIT-VALUE><LENGTH>16</LENGTH>",
    );
    let a = parse_cluster(&xml, "CanA").expect("CanA");
    let sig = &a.db.messages[&(0x123, false)].signals[0];
    assert_eq!(sig.initial, Some(5.0), "raw 500 × 0.01");
}

/// One file, every kind: the unified importer adds the CAN clusters as CAN
/// buses and the Ethernet cluster as an Ethernet bus. Saving the project
/// stores each source/cluster pair and restores the same tables.
#[test]
fn unified_import_adds_each_cluster_and_survives_the_project() {
    use crate::app::App;
    use crate::config::Config;

    let xml = SAMPLE.replace(
        "</ELEMENTS>",
        "<ETHERNET-CLUSTER><SHORT-NAME>EthA</SHORT-NAME>\
          <ETHERNET-PHYSICAL-CHANNEL><SHORT-NAME>CH</SHORT-NAME>\
           <VLAN-CONFIG><VLAN-IDENTIFIER>100</VLAN-IDENTIFIER></VLAN-CONFIG>\
           <ETHERNET-FRAME-TRIGGERING><SHORT-NAME>TE</SHORT-NAME>\
            <FRAME-REF>/Frames/E</FRAME-REF></ETHERNET-FRAME-TRIGGERING>\
          </ETHERNET-PHYSICAL-CHANNEL></ETHERNET-CLUSTER>\
         <ETHERNET-FRAME><SHORT-NAME>E</SHORT-NAME><FRAME-LENGTH>8</FRAME-LENGTH></ETHERNET-FRAME>\
         </ELEMENTS>",
    );
    let path = std::env::temp_dir().join(format!("roxy_can_unified_{}.arxml", std::process::id()));
    std::fs::write(&path, xml).expect("write matrix");

    let mut app = App::headless();
    let before = app.snap.channel_count;
    app.import_arxml_path(path.to_string_lossy().as_ref());
    app.settle();
    assert_eq!(
        app.snap.channel_count,
        before + 2,
        "the two CAN clusters created two buses"
    );
    assert_eq!(app.can_arxml.len(), 2);
    assert_eq!(app.eth_buses.len(), 1);
    let can_a = app
        .can_arxml
        .iter()
        .find(|(_, (_, cluster))| cluster == "CanA")
        .map(|(bus, _)| *bus)
        .expect("CanA source");
    let db = app.snap.channels[can_a as usize]
        .dbc
        .as_ref()
        .expect("CanA database");
    assert_eq!(db.messages[&(0x123, false)].name, "A");
    assert_eq!(app.snap.channels[can_a as usize].name, "CanA");
    assert_eq!(app.eth_clusters.get(&0).map(String::as_str), Some("EthA"));

    let cfg = Config::from_app(&app, None);
    let mut restored = App::headless();
    cfg.apply(&mut restored);
    restored.settle();
    assert_eq!(
        restored.can_arxml.len(),
        2,
        "CAN sources survive the project"
    );
    assert_eq!(restored.eth_buses.len(), 1);
    assert!(
        restored
            .snap
            .channels
            .iter()
            .any(|c| c.name == "CanA" && c.dbc.is_some()),
        "the CAN table is reloaded from its cluster"
    );
    assert_eq!(
        restored.eth_clusters.get(&0).map(String::as_str),
        Some("EthA")
    );

    let count = restored.snap.channel_count;
    restored.import_arxml_path(path.to_string_lossy().as_ref());
    restored.settle();
    assert_eq!(restored.snap.channel_count, count, "nothing imported twice");
    assert!(
        restored.status.contains("updated"),
        "the summary says the clusters were refreshed: {}",
        restored.status
    );
    std::fs::remove_file(path).ok();
}

/// The same unified action drives the FlexRay branch: the committed cluster
/// description becomes its own FlexRay bus, with the cluster name remembered.
#[test]
fn unified_import_adds_a_flexray_cluster_from_the_committed_asset() {
    use crate::app::App;
    let path = crate::test_assets::path("PowerTrain.arxml");
    let mut app = App::headless();
    app.import_arxml_path(path);
    app.settle();
    let bus = app
        .fr_buses
        .keys()
        .copied()
        .find(|b| app.fr_clusters.contains_key(b))
        .expect("the file's FlexRay cluster landed on a bus");
    let db = app.fr_db(bus).expect("cluster database");
    assert!(!db.frames.is_empty());
    assert!(!app.fr_clusters[&bus].is_empty());
    assert!(
        app.status.contains("added") || app.status.contains("updated"),
        "the import reports what it did: {}",
        app.status
    );
}

const ONE_CLUSTER: &str = r#"<?xml version="1.0"?>
<AUTOSAR><ELEMENTS>
 <CAN-CLUSTER><SHORT-NAME>Solo</SHORT-NAME><CAN-CLUSTER-VARIANTS><CAN-CLUSTER-CONDITIONAL>
  <BAUDRATE>500000</BAUDRATE><PHYSICAL-CHANNELS><CAN-PHYSICAL-CHANNEL><SHORT-NAME>CH</SHORT-NAME>
   <FRAME-TRIGGERINGS><CAN-FRAME-TRIGGERING><SHORT-NAME>FT</SHORT-NAME>
    <FRAME-REF>/F</FRAME-REF><CAN-ADDRESSING-MODE>STANDARD</CAN-ADDRESSING-MODE><IDENTIFIER>256</IDENTIFIER>
   </CAN-FRAME-TRIGGERING></FRAME-TRIGGERINGS>
  </CAN-PHYSICAL-CHANNEL></PHYSICAL-CHANNELS>
 </CAN-CLUSTER-CONDITIONAL></CAN-CLUSTER-VARIANTS></CAN-CLUSTER>
 <CAN-FRAME><SHORT-NAME>F</SHORT-NAME><FRAME-LENGTH>8</FRAME-LENGTH></CAN-FRAME>
</ELEMENTS></AUTOSAR>"#;

/// "Open..." on one CAN row accepts ARXML when the file has one CAN cluster;
/// it replaces that row's database instead of creating a new bus.
#[test]
fn a_single_cluster_arxml_replaces_one_existing_can_database() {
    use crate::app::App;
    let path = std::env::temp_dir().join(format!(
        "roxy_can_single_cluster_{}.arxml",
        std::process::id()
    ));
    std::fs::write(&path, ONE_CLUSTER).expect("write matrix");
    let mut app = App::headless();
    app.open_can_database_for(0, path.to_string_lossy().into_owned());
    app.settle();
    assert_eq!(app.can_arxml.get(&0).map(|(_, c)| c.as_str()), Some("Solo"));
    let db = app.snap.channels[0]
        .dbc
        .as_ref()
        .expect("replaced database");
    assert_eq!(db.messages[&(0x100, false)].name, "F");
    std::fs::remove_file(path).ok();
}

const SHARED_FRAMES: &str = r#"<?xml version="1.0"?>
<AUTOSAR><ELEMENTS>
 <CAN-CLUSTER><SHORT-NAME>CanA</SHORT-NAME><CAN-CLUSTER-VARIANTS><CAN-CLUSTER-CONDITIONAL>
  <PHYSICAL-CHANNELS><CAN-PHYSICAL-CHANNEL><SHORT-NAME>CHA</SHORT-NAME>
   <FRAME-TRIGGERINGS><CAN-FRAME-TRIGGERING><SHORT-NAME>FTA</SHORT-NAME>
    <FRAME-REF>/F</FRAME-REF><CAN-ADDRESSING-MODE>STANDARD</CAN-ADDRESSING-MODE><IDENTIFIER>256</IDENTIFIER>
   </CAN-FRAME-TRIGGERING></FRAME-TRIGGERINGS>
  </CAN-PHYSICAL-CHANNEL></PHYSICAL-CHANNELS>
 </CAN-CLUSTER-CONDITIONAL></CAN-CLUSTER-VARIANTS></CAN-CLUSTER>
 <CAN-CLUSTER><SHORT-NAME>CanB</SHORT-NAME><CAN-CLUSTER-VARIANTS><CAN-CLUSTER-CONDITIONAL>
  <PHYSICAL-CHANNELS><CAN-PHYSICAL-CHANNEL><SHORT-NAME>CHB</SHORT-NAME>
   <FRAME-TRIGGERINGS><CAN-FRAME-TRIGGERING><SHORT-NAME>FTB</SHORT-NAME>
    <FRAME-REF>/F</FRAME-REF><CAN-ADDRESSING-MODE>STANDARD</CAN-ADDRESSING-MODE><IDENTIFIER>512</IDENTIFIER>
   </CAN-FRAME-TRIGGERING></FRAME-TRIGGERINGS>
  </CAN-PHYSICAL-CHANNEL></PHYSICAL-CHANNELS>
 </CAN-CLUSTER-CONDITIONAL></CAN-CLUSTER-VARIANTS></CAN-CLUSTER>
 <CAN-FRAME><SHORT-NAME>F</SHORT-NAME><FRAME-LENGTH>8</FRAME-LENGTH>
  <PDU-TO-FRAME-MAPPING><PDU-REF>/P</PDU-REF><START-POSITION>0</START-POSITION></PDU-TO-FRAME-MAPPING>
 </CAN-FRAME>
 <I-SIGNAL-I-PDU><SHORT-NAME>P</SHORT-NAME><LENGTH>8</LENGTH>
  <I-SIGNAL-TO-I-PDU-MAPPING><I-SIGNAL-REF>/S</I-SIGNAL-REF><START-POSITION>0</START-POSITION></I-SIGNAL-TO-I-PDU-MAPPING>
 </I-SIGNAL-I-PDU>
 <I-SIGNAL><SHORT-NAME>S</SHORT-NAME><LENGTH>8</LENGTH></I-SIGNAL>
</ELEMENTS></AUTOSAR>"#;

/// Two clusters may share every frame, PDU and signal definition and differ
/// only by the identifier in their own triggering. Each parse keeps its own
/// id; the unified importer creates two buses rather than overwriting one.
#[test]
fn two_can_clusters_can_share_frame_definitions_with_different_ids() {
    use crate::app::App;
    let a = parse_cluster(SHARED_FRAMES, "CanA").expect("CanA");
    assert_eq!(a.db.order, [(0x100, false)]);
    assert_eq!(a.db.messages[&(0x100, false)].name, "F");
    let b = parse_cluster(SHARED_FRAMES, "CanB").expect("CanB");
    assert_eq!(b.db.order, [(0x200, false)]);
    assert_eq!(b.db.messages[&(0x200, false)].name, "F");

    let path = std::env::temp_dir().join(format!(
        "roxy_can_shared_clusters_{}.arxml",
        std::process::id()
    ));
    std::fs::write(&path, SHARED_FRAMES).expect("write matrix");
    let mut app = App::headless();
    let before = app.snap.channel_count;
    app.import_arxml_path(path.to_string_lossy().as_ref());
    app.settle();
    assert_eq!(app.snap.channel_count, before + 2);
    let names: Vec<&str> = app.can_arxml.values().map(|(_, c)| c.as_str()).collect();
    assert!(names.contains(&"CanA"));
    assert!(names.contains(&"CanB"));
    std::fs::remove_file(path).ok();
}

const TOP_LEVEL_TRIGGERS: &str = r#"<?xml version="1.0"?>
<AUTOSAR><ELEMENTS>
 <CAN-CLUSTER><SHORT-NAME>C1</SHORT-NAME></CAN-CLUSTER>
 <CAN-CLUSTER><SHORT-NAME>C2</SHORT-NAME></CAN-CLUSTER>
 <CAN-FRAME-TRIGGERING><SHORT-NAME>T1</SHORT-NAME>
  <FRAME-REF>/F</FRAME-REF><IDENTIFIER>1</IDENTIFIER>
  <PDU-TRIGGERING-REF>/Cluster/C1/CH/PT</PDU-TRIGGERING-REF>
 </CAN-FRAME-TRIGGERING>
 <CAN-FRAME-TRIGGERING><SHORT-NAME>T2</SHORT-NAME>
  <FRAME-REF>/F</FRAME-REF><IDENTIFIER>2</IDENTIFIER>
  <PDU-TRIGGERING-REF>/Cluster/C2/CH/PT</PDU-TRIGGERING-REF>
 </CAN-FRAME-TRIGGERING>
 <CAN-FRAME><SHORT-NAME>F</SHORT-NAME><FRAME-LENGTH>8</FRAME-LENGTH></CAN-FRAME>
</ELEMENTS></AUTOSAR>"#;

/// Triggerings kept outside the cluster package name their cluster through
/// the PDU-triggering path; the same two-cluster split still holds.
#[test]
fn top_level_triggerings_are_attributed_by_their_cluster_path() {
    let a = parse_cluster(TOP_LEVEL_TRIGGERS, "C1").expect("C1");
    assert_eq!(a.db.order, [(1, false)]);
    let b = parse_cluster(TOP_LEVEL_TRIGGERS, "C2").expect("C2");
    assert_eq!(b.db.order, [(2, false)]);
}

/// The base type's declared encoding decides signedness, not the short name:
/// a `uint16` declared two's complement is signed, and a `sint16` declared
/// NONE is not.
#[test]
fn base_type_encoding_beats_the_type_name() {
    let text = SAMPLE.replace(
        "</ELEMENTS>",
        "<SW-BASE-TYPE><SHORT-NAME>uint16</SHORT-NAME><BASE-TYPE-SIZE>16</BASE-TYPE-SIZE>\
           <BASE-TYPE-ENCODING>2C</BASE-TYPE-ENCODING></SW-BASE-TYPE>\
         <SW-BASE-TYPE><SHORT-NAME>sint16</SHORT-NAME><BASE-TYPE-SIZE>12</BASE-TYPE-SIZE>\
           <BASE-TYPE-ENCODING>NONE</BASE-TYPE-ENCODING></SW-BASE-TYPE>\
         </ELEMENTS>",
    );
    let a = parse_cluster(&text, "CanA").expect("CanA");
    let speed = &a.db.messages[&(0x123, false)].signals[0];
    assert_eq!(speed.name, "Speed");
    assert!(speed.signed, "2C wins over the uint16 name");
    let b = parse_cluster(&text, "CanB").expect("CanB");
    let torque = &b.db.messages[&(0x456, true)].signals[0];
    assert_eq!(torque.name, "Torque");
    assert!(!torque.signed, "NONE wins over the sint16 name");
}
