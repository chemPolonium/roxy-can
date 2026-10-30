//! Test fixtures generated at runtime. The repository keeps no sample DBC
//! files: tests that need a path get one written under `target/test-assets`
//! from the built-in starter text.

use std::io::Read;
use std::sync::OnceLock;

/// One Ethernet cluster, frame, PDU and 16-bit signal. The frame length and
/// PDU follow the parser's payload-length convention (no L2 header).
const ETH_ARXML: &str = r#"<?xml version="1.0"?>
<AUTOSAR>
 <AR-PACKAGES><AR-PACKAGE><SHORT-NAME>Net</SHORT-NAME><ELEMENTS>
  <ETHERNET-CLUSTER><SHORT-NAME>EthCluster</SHORT-NAME>
   <ETHERNET-PHYSICAL-CHANNEL><SHORT-NAME>Vlan100</SHORT-NAME>
    <VLAN-CONFIG><VLAN-IDENTIFIER>100</VLAN-IDENTIFIER></VLAN-CONFIG>
    <ETHERNET-FRAME-TRIGGERING><SHORT-NAME>Trig</SHORT-NAME>
     <FRAME-REF DEST="ETHERNET-FRAME">/Net/EthFrame</FRAME-REF>
    </ETHERNET-FRAME-TRIGGERING>
   </ETHERNET-PHYSICAL-CHANNEL>
  </ETHERNET-CLUSTER>
  <ETHERNET-FRAME><SHORT-NAME>EthFrame</SHORT-NAME><FRAME-LENGTH>22</FRAME-LENGTH>
   <PDU-TO-FRAME-MAPPING><SHORT-NAME>Map</SHORT-NAME><START-POSITION>0</START-POSITION>
    <PDU-REF DEST="I-SIGNAL-I-PDU">/Net/Pdu</PDU-REF>
   </PDU-TO-FRAME-MAPPING>
  </ETHERNET-FRAME>
  <I-SIGNAL-I-PDU><SHORT-NAME>Pdu</SHORT-NAME><LENGTH>64</LENGTH>
   <I-SIGNAL-TO-I-PDU-MAPPING><SHORT-NAME>Sm</SHORT-NAME>
    <START-POSITION>8</START-POSITION>
    <PACKING-BYTE-ORDER>MOST-SIGNIFICANT-BYTE-LAST</PACKING-BYTE-ORDER>
    <I-SIGNAL-REF DEST="I-SIGNAL">/Net/Sig</I-SIGNAL-REF>
   </I-SIGNAL-TO-I-PDU-MAPPING>
  </I-SIGNAL-I-PDU>
  <I-SIGNAL><SHORT-NAME>Sig</SHORT-NAME><LENGTH>16</LENGTH>
   <COMPU-METHOD-REF DEST="COMPU-METHOD">/Net/Cm</COMPU-METHOD-REF>
  </I-SIGNAL>
  <COMPU-METHOD><SHORT-NAME>Cm</SHORT-NAME>
   <COMPU-INTERNAL-TO-PHYS><COMPU-SCALES><COMPU-SCALE>
    <COMPU-RATIONAL-COEFFS>
     <COMPU-NUMERATOR><V>0</V><V>0.1</V></COMPU-NUMERATOR>
     <COMPU-DENOMINATOR><V>1</V></COMPU-DENOMINATOR>
    </COMPU-RATIONAL-COEFFS>
   </COMPU-SCALE></COMPU-SCALES></COMPU-INTERNAL-TO-PHYS>
  </COMPU-METHOD>
 </ELEMENTS></AR-PACKAGE></AR-PACKAGES>
</AUTOSAR>"#;

pub const ETH_ARXML_SAMPLE: &str = ETH_ARXML;

fn write_gzip(name: &str, compressed: &[u8]) {
    let mut bytes = Vec::new();
    flate2::read::GzDecoder::new(compressed)
        .read_to_end(&mut bytes)
        .expect("test fixture decompresses");
    std::fs::write(std::path::Path::new("target/test-assets").join(name), bytes)
        .expect("write test fixture");
}

fn ensure() {
    static READY: OnceLock<()> = OnceLock::new();
    READY.get_or_init(|| {
        let dir = std::path::Path::new("target/test-assets");
        std::fs::create_dir_all(dir).expect("create test-assets");
        std::fs::write(dir.join("sample.dbc"), crate::sample_data::SAMPLE_DBC)
            .expect("write sample.dbc");
        std::fs::write(dir.join("motbus.dbc"), crate::sample_data::MOTBUS_DBC)
            .expect("write motbus.dbc");
        write_gzip(
            "PowerTrain.arxml",
            crate::test_assets_data::POWERTRAIN_ARXML_GZ,
        );
        write_gzip(
            "PowerTrain_v2.xml",
            crate::test_assets_data::POWERTRAIN_FIBEX_GZ,
        );
        write_gzip(
            "PowerTrain2_v2.xml",
            crate::test_assets_data::POWERTRAIN_FIBEX2_GZ,
        );
        write_gzip(
            "system-4.2.arxml",
            crate::test_assets_data::SYSTEM_42_ARXML_GZ,
        );
        std::fs::write(dir.join("EthSample.arxml"), ETH_ARXML).expect("write EthSample.arxml");
    });
}

/// Path of a generated test fixture. Lives for the process (tests pass it
/// to APIs that borrow `&str`).
pub fn path(name: &str) -> &'static str {
    ensure();
    let path = std::path::Path::new("target/test-assets").join(name);
    Box::leak(path.to_string_lossy().into_owned().into_boxed_str())
}

fn log_name(stem: &str, ext: &str) -> String {
    format!("{stem}_{}.{ext}", std::process::id())
}

fn fr_row(
    bus: u8,
    t_us: u64,
    slot: u16,
    cycle: u8,
    payload: Vec<u8>,
    name: Option<&str>,
) -> crate::trace::FrRow {
    crate::trace::FrRow {
        bus,
        t_us,
        ab: 0,
        slot,
        cycle,
        payload,
        header_crc: 0,
        flags: 0,
        name: name.map(str::to_string),
    }
}

fn ensure_logs() {
    static READY: OnceLock<()> = OnceLock::new();
    READY.get_or_init(|| {
        ensure();
        let dir = std::path::Path::new("target/test-assets");

        // ASC: 30,600 FR rows, first one matching the historic ground-truth
        // coordinates (549 µs, slot 13, cycle 0, named receive, 26 bytes).
        let asc = dir.join(log_name("Logging", "asc"));
        let mut writer = crate::log::AscWriter::new(&asc.to_string_lossy()).expect("ASC fixture");
        for i in 0..30_600u64 {
            let (t, slot, cycle, payload, name) = if i == 0 {
                (549, 13, 0, vec![0u8; 26], Some("Frame_13_0_2"))
            } else {
                (
                    549 + i * 1_000,
                    [13u16, 16, 25, 26][(i % 4) as usize],
                    (i % 64) as u8,
                    vec![(i & 0xFF) as u8; 8],
                    None,
                )
            };
            writer
                .write_fr(&fr_row(0, t, slot, cycle, payload, name))
                .expect("write ASC FR row");
        }
        writer.finish().expect("finish ASC fixture");

        // BLF, single cluster: 39,200 rows over 39.2 s.
        let single = dir.join(log_name("Logging_single", "blf"));
        let mut writer =
            crate::log::blf::BlfWriter::create(&single.to_string_lossy()).expect("BLF fixture");
        for i in 0..39_200u64 {
            writer.write_fr(&fr_row(
                0,
                i * 1_000,
                1 + (i % 63) as u16,
                (i % 64) as u8,
                vec![(i & 0xFF) as u8; 8],
                None,
            ));
        }
        writer.finish().expect("finish single BLF");

        // BLF, two clusters: the historic 29,738 / 30,334 split, shared
        // slots, unique instants, and slot 13/cycle 0 payloads that differ
        // between the clusters.
        let two = dir.join(log_name("Logging_two", "blf"));
        let mut writer =
            crate::log::blf::BlfWriter::create(&two.to_string_lossy()).expect("BLF fixture");
        let counts = [29_738u32, 30_334u32];
        let max = *counts.iter().max().unwrap();
        for i in 0..max {
            for (bus, count) in counts.iter().enumerate() {
                if i >= *count {
                    continue;
                }
                let (slot, cycle) = if i % 50 == 0 {
                    (13u16, 0u8)
                } else {
                    ([13u16, 16, 25, 26][(i as usize) % 4], (i % 64) as u8)
                };
                let mut payload = vec![0u8; 8];
                payload[0] = bus as u8;
                payload[1..5].copy_from_slice(&i.to_le_bytes());
                writer.write_fr(&fr_row(
                    bus as u8,
                    i as u64 * 2_000 + bus as u64 * 1_000,
                    slot,
                    cycle,
                    payload,
                    None,
                ));
            }
        }
        writer.finish().expect("finish two-cluster BLF");
    });
}

pub fn eth_arxml() -> &'static str {
    path("EthSample.arxml")
}

/// The cantools AUTOSAR 4.2 system extract: SecOC with a PDU-TRIGGERING
/// payload and E2E declared through DATA-TRANSFORMATION-SET.
pub fn system42_arxml() -> &'static str {
    path("system-4.2.arxml")
}

pub fn logging_asc() -> &'static str {
    ensure_logs();
    path(&log_name("Logging", "asc"))
}

pub fn logging_blf_single() -> &'static str {
    ensure_logs();
    path(&log_name("Logging_single", "blf"))
}

pub fn logging_blf_two() -> &'static str {
    ensure_logs();
    path(&log_name("Logging_two", "blf"))
}

/// CAN ARXML fixture used by the import tests: two clusters (CanA/CanB).
pub(crate) const CAN_ARXML_SAMPLE: &str = r#"
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
</AUTOSAR>
"#;
