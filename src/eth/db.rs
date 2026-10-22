//! Minimal AUTOSAR Ethernet database: enough to name captured frames and
//! decode one `I-SIGNAL-I-PDU` per frame.
//!
//! The reader follows roxy-arxml's semantics (cluster/physical-channel VLAN,
//! Ethernet frame triggering, PDU-to-frame and I-Signal mappings) without
//! depending on that editor crate. Container/Multiplexed/Secured PDUs are
//! deliberately not decoded here.

use roxmltree::{Document, Node};
use std::collections::HashMap;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct EthCluster {
    pub name: String,
    pub vlans: Vec<u16>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct EthFrameDef {
    pub name: String,
    pub cluster: String,
    pub vlan: Option<u16>,
    /// Declared `FRAME-LENGTH`; the payload length convention (with or
    /// without the 14/18-byte L2 header) is decided by the caller.
    pub length: usize,
    /// IEEE 1722 hint; other frame kinds have no EtherType in AUTOSAR.
    pub ethertype_hint: Option<u16>,
    pub pdus: Vec<EthPdu>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct EthPdu {
    pub name: String,
    pub length: usize,
    pub start_bit: usize,
    pub signals: Vec<EthSignal>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct EthSignal {
    pub name: String,
    /// Bit offset inside the PDU.
    pub start_bit: usize,
    pub size: usize,
    pub big_endian: bool,
    pub signed: bool,
    pub factor: f64,
    pub offset: f64,
    pub unit: String,
    pub values: Vec<(i64, String)>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct EthDb {
    pub clusters: Vec<EthCluster>,
    pub frames: Vec<EthFrameDef>,
}

/// The L2 facts a captured frame offers to the matcher.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EthFlow {
    pub vlan: Option<u16>,
    pub dst: [u8; 6],
    pub src: [u8; 6],
    pub ethertype: u16,
    pub payload_len: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EthMatch {
    Unique(usize),
    Ambiguous(usize),
    None,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct DecodedEth {
    pub frame_name: String,
    pub signals: Vec<DecodedSignal>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct DecodedSignal {
    pub name: String,
    pub raw: i64,
    pub physical: f64,
    pub unit: String,
    pub label: Option<String>,
    pub type_tag: String,
}

impl EthDb {
    pub fn parse(text: &str) -> Result<Self, String> {
        let doc = Document::parse(text).map_err(|e| e.to_string())?;
        let root = doc.root_element();
        let mut db = EthDb::default();

        // Short-name indexes: a matrix may reference an object by path; the
        // last segment is its short name.
        let by_name = |tag: &str| -> HashMap<String, Node> {
            root.descendants()
                .filter(|n| n.is_element() && n.has_tag_name(tag))
                .filter_map(|n| {
                    let name = short_name(n);
                    (!name.is_empty()).then_some((name, n))
                })
                .collect()
        };
        let pdus = by_name("I-SIGNAL-I-PDU");
        let signals = by_name("I-SIGNAL");
        let compus = by_name("COMPU-METHOD");
        let units = by_name("UNIT");
        let base_types = by_name("SW-BASE-TYPE");

        // Frame -> (VLAN, cluster) comes from its triggering, not from the
        // frame object itself.
        let mut attached: HashMap<String, (Option<u16>, String)> = HashMap::new();
        for trig in root
            .descendants()
            .filter(|n| n.is_element() && n.has_tag_name("ETHERNET-FRAME-TRIGGERING"))
        {
            let Some(frame_ref) = child_text(trig, "FRAME-REF") else {
                continue;
            };
            let frame = ref_tail(&frame_ref);
            let vlan = trig
                .ancestors()
                .find(|n| n.has_tag_name("ETHERNET-PHYSICAL-CHANNEL"))
                .and_then(|ch| {
                    ch.descendants()
                        .find(|n| n.has_tag_name("VLAN-IDENTIFIER"))
                        .and_then(|n| n.text())
                        .and_then(|t| t.trim().parse::<u16>().ok())
                })
                .filter(|v| *v <= 4095);
            let cluster = trig
                .ancestors()
                .find(|n| n.has_tag_name("ETHERNET-CLUSTER"))
                .map(short_name)
                .unwrap_or_default();
            attached.insert(frame, (vlan, cluster));
        }

        for cluster_node in root
            .descendants()
            .filter(|n| n.is_element() && n.has_tag_name("ETHERNET-CLUSTER"))
        {
            let mut vlans: Vec<u16> = cluster_node
                .descendants()
                .filter(|n| n.has_tag_name("VLAN-IDENTIFIER"))
                .filter_map(|n| n.text())
                .filter_map(|t| t.trim().parse::<u16>().ok())
                .filter(|v| *v <= 4095)
                .collect();
            vlans.sort_unstable();
            vlans.dedup();
            db.clusters.push(EthCluster {
                name: short_name(cluster_node),
                vlans,
            });
        }

        for frame_node in root.descendants().filter(|n| is_ethernet_frame(*n)) {
            let name = short_name(frame_node);
            if name.is_empty() {
                continue;
            }
            let length = child_text(frame_node, "FRAME-LENGTH")
                .and_then(|t| t.trim().parse::<usize>().ok())
                .unwrap_or(0);
            let (vlan, cluster) = attached
                .get(&name)
                .cloned()
                .unwrap_or((None, String::new()));
            let ethertype_hint = if frame_node.has_tag_name("IEEE-1722-TP-ETHERNET-FRAME")
                || frame_node.has_tag_name("IEEE1722-TP-ETHERNET-FRAME")
            {
                Some(0x22F0)
            } else {
                None
            };
            let mut frame = EthFrameDef {
                name,
                cluster,
                vlan,
                length,
                ethertype_hint,
                pdus: Vec::new(),
            };
            for map in frame_node.descendants().filter(|n| {
                n.has_tag_name("PDU-TO-FRAME-MAPPING") || n.has_tag_name("FRAME-PDU-MAPPING")
            }) {
                let Some(pdu_ref) = child_text(map, "PDU-REF") else {
                    continue;
                };
                let pdu_name = ref_tail(&pdu_ref);
                let Some(pdu_node) = pdus.get(&pdu_name).copied() else {
                    continue;
                };
                let start_bit = number(map, "START-POSITION").unwrap_or(0.0) as usize;
                if let Some(mut pdu) = read_pdu(pdu_node, &signals, &compus, &units, &base_types) {
                    pdu.start_bit = start_bit;
                    frame.pdus.push(pdu);
                }
            }
            db.frames.push(frame);
        }
        Ok(db)
    }

    /// Parses one named `ETHERNET-CLUSTER`. Frames are stamped with the
    /// cluster whose triggering they sit under, so a matrix holding several
    /// clusters (two in-vehicle networks in one file) becomes one bus each
    /// instead of one merged database.
    pub fn parse_cluster(text: &str, cluster: &str) -> Result<Self, String> {
        let mut db = Self::parse(text)?;
        if !db.clusters.iter().any(|c| c.name == cluster) {
            return Err(format!(
                "Ethernet cluster `{cluster}` not found in the file"
            ));
        }
        let only = db.clusters.len() == 1;
        db.clusters.retain(|c| c.name == cluster);
        db.frames
            .retain(|f| f.cluster == cluster || (only && f.cluster.is_empty()));
        Ok(db)
    }

    /// Rank frame definitions against a captured flow. AUTOSAR has no MAC or
    /// EtherType on `ETHERNET-FRAME`; only a unique best score binds.
    pub fn match_flow(&self, flow: &EthFlow, length_includes_header: bool) -> EthMatch {
        let expected = |frame_len: usize| {
            if length_includes_header {
                frame_len.saturating_sub(14)
            } else {
                frame_len
            }
        };
        let mut best = 0i32;
        let mut winner = None;
        let mut ties = 0usize;
        for (i, frame) in self.frames.iter().enumerate() {
            let mut score = 0i32;
            if frame.vlan == flow.vlan {
                score += 2;
            } else {
                continue;
            }
            if expected(frame.length) == flow.payload_len {
                score += 2;
            } else {
                continue;
            }
            if let Some(hint) = frame.ethertype_hint {
                if hint == flow.ethertype {
                    score += 3;
                } else {
                    continue;
                }
            }
            if score > best {
                best = score;
                winner = Some(i);
                ties = 1;
            } else if score == best && score > 0 {
                ties += 1;
            }
        }
        match (winner, ties) {
            (Some(i), 1) => EthMatch::Unique(i),
            (Some(_), n) if n > 1 => EthMatch::Ambiguous(n),
            _ => EthMatch::None,
        }
    }

    pub fn decode(
        &self,
        flow: &EthFlow,
        payload: &[u8],
        length_includes_header: bool,
    ) -> Option<DecodedEth> {
        let EthMatch::Unique(i) = self.match_flow(flow, length_includes_header) else {
            return None;
        };
        let frame = &self.frames[i];
        let mut out = DecodedEth {
            frame_name: frame.name.clone(),
            signals: Vec::new(),
        };
        for pdu in &frame.pdus {
            for sig in &pdu.signals {
                let abs = pdu.start_bit + sig.start_bit;
                let Some(raw) = extract_bits(payload, abs, sig.size, sig.big_endian) else {
                    continue;
                };
                let physical = if sig.signed {
                    sign_extend(raw, sig.size) as f64 * sig.factor + sig.offset
                } else {
                    raw as f64 * sig.factor + sig.offset
                };
                out.signals.push(DecodedSignal {
                    name: sig.name.clone(),
                    raw: raw as i64,
                    physical,
                    unit: sig.unit.clone(),
                    label: sig
                        .values
                        .iter()
                        .find(|(v, _)| *v == raw as i64)
                        .map(|(_, s)| s.clone()),
                    type_tag: format!("{}{}", if sig.signed { "i" } else { "u" }, sig.size),
                });
            }
        }
        Some(out)
    }
}

fn read_pdu(
    node: Node,
    signals: &HashMap<String, Node>,
    compus: &HashMap<String, Node>,
    units: &HashMap<String, Node>,
    base_types: &HashMap<String, Node>,
) -> Option<EthPdu> {
    let name = short_name(node);
    let length = child_text(node, "LENGTH")
        .and_then(|t| t.trim().parse::<usize>().ok())
        .unwrap_or(0);
    let mut pdu = EthPdu {
        name,
        length,
        start_bit: 0,
        signals: Vec::new(),
    };
    for map in node.descendants().filter(|n| {
        n.has_tag_name("I-SIGNAL-TO-I-PDU-MAPPING") || n.has_tag_name("I-SIGNAL-TO-PDU-MAPPING")
    }) {
        let Some(sig_ref) = child_text(map, "I-SIGNAL-REF") else {
            continue;
        };
        let Some(sig_node) = signals.get(&ref_tail(&sig_ref)).copied() else {
            continue;
        };
        let start_bit = number(map, "START-POSITION").unwrap_or(0.0) as usize;
        let big_endian = child_text(map, "PACKING-BYTE-ORDER")
            .is_some_and(|s| s.contains("MOST-SIGNIFICANT-BYTE-FIRST"));
        let size = child_text(sig_node, "LENGTH")
            .and_then(|t| t.trim().parse::<usize>().ok())
            .unwrap_or(0);
        if size == 0 || size > 64 {
            continue;
        }
        let signed = child_text(sig_node, "BASE-TYPE-REF")
            .and_then(|r| base_types.get(&ref_tail(&r)))
            .and_then(|b| child_text(*b, "BASE-TYPE-ENCODING"))
            .is_some_and(|e| e == "2C");
        let compu = child_text(sig_node, "COMPU-METHOD-REF")
            .and_then(|r| compus.get(&ref_tail(&r)).copied());
        let (factor, offset) = compu.map(read_linear).unwrap_or((1.0, 0.0));
        let values = compu.map(read_text_table).unwrap_or_default();
        let unit = compu
            .and_then(|c| child_text(c, "UNIT-REF"))
            .or_else(|| child_text(sig_node, "UNIT-REF"))
            .and_then(|r| units.get(&ref_tail(&r)).copied())
            .map(|u| {
                child_text(u, "DISPLAY-NAME")
                    .filter(|s| !s.is_empty())
                    .unwrap_or_else(|| short_name(u))
            })
            .unwrap_or_default();
        pdu.signals.push(EthSignal {
            name: short_name(sig_node),
            start_bit,
            size,
            big_endian,
            signed,
            factor,
            offset,
            unit,
            values,
        });
    }
    Some(pdu)
}

fn read_linear(compu: Node) -> (f64, f64) {
    let mut factor = 1.0;
    let mut offset = 0.0;
    for scale in compu
        .descendants()
        .filter(|n| n.has_tag_name("COMPU-SCALE"))
    {
        let Some(coeffs) = scale
            .descendants()
            .find(|n| n.has_tag_name("COMPU-RATIONAL-COEFFS"))
        else {
            continue;
        };
        let nums: Vec<f64> = coeffs
            .children()
            .find(|n| n.has_tag_name("COMPU-NUMERATOR"))
            .map(|n| {
                n.children()
                    .filter(|c| c.has_tag_name("V"))
                    .filter_map(|c| c.text())
                    .filter_map(|t| t.trim().parse::<f64>().ok())
                    .collect()
            })
            .unwrap_or_default();
        let den = coeffs
            .children()
            .find(|n| n.has_tag_name("COMPU-DENOMINATOR"))
            .and_then(|n| {
                n.children()
                    .find(|c| c.has_tag_name("V"))
                    .and_then(|c| c.text())
                    .and_then(|t| t.trim().parse::<f64>().ok())
            })
            .unwrap_or(1.0)
            .max(f64::MIN_POSITIVE);
        if let Some(off) = nums.first() {
            offset = off / den;
        }
        if let Some(f) = nums.get(1) {
            factor = f / den;
        }
        break;
    }
    (factor, offset)
}

fn read_text_table(compu: Node) -> Vec<(i64, String)> {
    let mut out = Vec::new();
    for scale in compu
        .descendants()
        .filter(|n| n.has_tag_name("COMPU-SCALE"))
    {
        let Some(raw) = child_text(scale, "LOWER-LIMIT").and_then(|t| t.parse::<i64>().ok()) else {
            continue;
        };
        let Some(label) = scale
            .descendants()
            .find(|n| n.has_tag_name("VT"))
            .and_then(|n| n.text())
        else {
            continue;
        };
        out.push((raw, label.trim().to_string()));
    }
    out
}

/// AUTOSAR bit positions start at the least-significant bit. This mirrors
/// the DBC little-endian walk; big-endian starts at the signal's MSB.
fn extract_bits(data: &[u8], start: usize, size: usize, big_endian: bool) -> Option<u64> {
    if size == 0 || size > 64 {
        return None;
    }
    let mut raw = 0u64;
    if big_endian {
        for i in 0..size {
            let bit = start.checked_sub(i)?;
            let byte = data.get(bit / 8)?;
            raw = (raw << 1) | u64::from((byte >> (bit % 8)) & 1);
        }
    } else {
        for i in 0..size {
            let bit = start + i;
            let byte = data.get(bit / 8)?;
            raw |= u64::from((byte >> (bit % 8)) & 1) << i;
        }
    }
    Some(raw)
}

fn sign_extend(raw: u64, size: usize) -> i64 {
    if size >= 64 {
        raw as i64
    } else {
        let shift = 64 - size;
        ((raw << shift) as i64) >> shift
    }
}

fn is_ethernet_frame(node: Node) -> bool {
    matches!(
        node.tag_name().name(),
        "ETHERNET-FRAME"
            | "GENERIC-ETHERNET-FRAME"
            | "USER-DEFINED-ETHERNET-FRAME"
            | "IEEE-1722-TP-ETHERNET-FRAME"
            | "IEEE1722-TP-ETHERNET-FRAME"
    )
}

fn short_name(node: Node) -> String {
    child_text(node, "SHORT-NAME").unwrap_or_default()
}

fn child_text(node: Node, tag: &str) -> Option<String> {
    node.children()
        .find(|c| c.has_tag_name(tag))
        .and_then(|c| c.text())
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty())
}

fn number(node: Node, tag: &str) -> Option<f64> {
    child_text(node, tag).and_then(|t| t.trim().parse::<f64>().ok())
}

fn ref_tail(path: &str) -> String {
    path.rsplit('/').next().unwrap_or(path).trim().to_string()
}

/// `AA:BB:CC:DD:EE:FF`, the form the Trace/Messages columns print.
pub fn mac_text(mac: &[u8; 6]) -> String {
    mac.iter()
        .map(|b| format!("{b:02X}"))
        .collect::<Vec<_>>()
        .join(":")
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"<?xml version="1.0"?>
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

    fn flow(vlan: Option<u16>, len: usize) -> EthFlow {
        EthFlow {
            vlan,
            dst: [0; 6],
            src: [0; 6],
            ethertype: 0x0800,
            payload_len: len,
        }
    }

    #[test]
    fn parses_cluster_frame_pdu_signal_and_scaling() {
        let db = EthDb::parse(SAMPLE).expect("parse");
        assert_eq!(db.clusters[0].name, "EthCluster");
        assert_eq!(db.clusters[0].vlans, vec![100]);
        let frame = &db.frames[0];
        assert_eq!(frame.name, "EthFrame");
        assert_eq!(frame.vlan, Some(100));
        assert_eq!(frame.length, 22);
        assert_eq!(frame.pdus[0].signals[0].name, "Sig");
        assert_eq!(frame.pdus[0].signals[0].factor, 0.1);
    }

    #[test]
    fn a_unique_flow_decodes_the_signal() {
        let db = EthDb::parse(SAMPLE).expect("parse");
        let payload = [0, 0x34, 0x12, 0, 0, 0, 0, 0];
        let decoded = db
            .decode(&flow(Some(100), 8), &payload, true)
            .expect("decoded");
        assert_eq!(decoded.frame_name, "EthFrame");
        assert_eq!(decoded.signals[0].raw, 0x1234);
        assert!((decoded.signals[0].physical - 466.0).abs() < 1e-9);
    }

    #[test]
    fn a_wrong_vlan_does_not_match() {
        let db = EthDb::parse(SAMPLE).expect("parse");
        assert_eq!(db.match_flow(&flow(Some(101), 8), true), EthMatch::None);
    }

    #[test]
    fn the_header_length_convention_is_explicit() {
        let db = EthDb::parse(SAMPLE).expect("parse");
        assert_eq!(
            db.match_flow(&flow(Some(100), 8), true),
            EthMatch::Unique(0)
        );
        assert_eq!(db.match_flow(&flow(Some(100), 8), false), EthMatch::None);
    }
}

#[cfg(test)]
mod multi_cluster_tests {
    use super::*;

    const TWO: &str = r#"<?xml version="1.0"?>
<AUTOSAR><AR-PACKAGES><AR-PACKAGE><SHORT-NAME>N</SHORT-NAME><ELEMENTS>
 <ETHERNET-CLUSTER><SHORT-NAME>EthA</SHORT-NAME>
  <ETHERNET-PHYSICAL-CHANNEL><SHORT-NAME>CHA</SHORT-NAME>
   <VLAN-CONFIG><VLAN-IDENTIFIER>100</VLAN-IDENTIFIER></VLAN-CONFIG>
   <ETHERNET-FRAME-TRIGGERING><SHORT-NAME>TA</SHORT-NAME><FRAME-REF>/Frames/A</FRAME-REF></ETHERNET-FRAME-TRIGGERING>
  </ETHERNET-PHYSICAL-CHANNEL>
 </ETHERNET-CLUSTER>
 <ETHERNET-CLUSTER><SHORT-NAME>EthB</SHORT-NAME>
  <ETHERNET-PHYSICAL-CHANNEL><SHORT-NAME>CHB</SHORT-NAME>
   <VLAN-CONFIG><VLAN-IDENTIFIER>200</VLAN-IDENTIFIER></VLAN-CONFIG>
   <ETHERNET-FRAME-TRIGGERING><SHORT-NAME>TB</SHORT-NAME><FRAME-REF>/Frames/B</FRAME-REF></ETHERNET-FRAME-TRIGGERING>
  </ETHERNET-PHYSICAL-CHANNEL>
 </ETHERNET-CLUSTER>
 <ETHERNET-FRAME><SHORT-NAME>A</SHORT-NAME><FRAME-LENGTH>8</FRAME-LENGTH></ETHERNET-FRAME>
 <ETHERNET-FRAME><SHORT-NAME>B</SHORT-NAME><FRAME-LENGTH>8</FRAME-LENGTH></ETHERNET-FRAME>
</ELEMENTS></AR-PACKAGE></AR-PACKAGES></AUTOSAR>"#;

    #[test]
    fn one_named_ethernet_cluster_keeps_its_own_frames() {
        let a = EthDb::parse_cluster(TWO, "EthA").expect("EthA");
        assert_eq!(a.clusters.len(), 1);
        assert_eq!(a.clusters[0].name, "EthA");
        assert_eq!(a.frames.len(), 1);
        assert_eq!(a.frames[0].name, "A");
        assert_eq!(a.frames[0].vlan, Some(100));

        let b = EthDb::parse_cluster(TWO, "EthB").expect("EthB");
        assert_eq!(b.frames.len(), 1);
        assert_eq!(b.frames[0].name, "B");
        assert_eq!(b.frames[0].vlan, Some(200));

        assert!(EthDb::parse_cluster(TWO, "Missing").is_err());
    }
}
