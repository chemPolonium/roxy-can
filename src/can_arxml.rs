//! Minimal AUTOSAR CAN matrix reader: one `CAN-CLUSTER` at a time into the
//! same [`SymbolTable`] the DBC path produces.
//!
//! Frames, PDUs and signals are global in AUTOSAR; the cluster owns the
//! physical channels and their frame triggerings. That is why the reader takes
//! a cluster name: a file with several CAN clusters becomes several buses
//! instead of one merged database. Container/Multiplexed/Secured PDUs are not
//! decoded here, the same boundary as the Ethernet reader.

use roxmltree::{Document, Node};
use std::collections::HashMap;

use crate::dbc::{MessageInfo, SignalInfo, SymbolTable};

/// One parsed CAN cluster and the facts the bus row can show.
pub struct CanArxmlDb {
    pub cluster: String,
    pub bitrate_kbps: Option<u32>,
    pub db: SymbolTable,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ClusterKind {
    Can,
    FlexRay,
    Ethernet,
}

impl ClusterKind {
    pub fn label(&self) -> &'static str {
        match self {
            ClusterKind::Can => "CAN",
            ClusterKind::FlexRay => "FlexRay",
            ClusterKind::Ethernet => "Ethernet",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClusterInfo {
    pub kind: ClusterKind,
    pub name: String,
}

/// Lists every communication cluster in an ARXML document, in document order.
/// The unified importer uses this to make one bus per cluster without parsing
/// each kind first.
pub fn scan(text: &str) -> Result<Vec<ClusterInfo>, String> {
    let text = text.strip_prefix('\u{FEFF}').unwrap_or(text);
    let doc = Document::parse(text).map_err(|e| format!("XML parse error: {e}"))?;
    let mut out = Vec::new();
    for node in doc.descendants().filter(|n| n.is_element()) {
        let kind = match node.tag_name().name() {
            "CAN-CLUSTER" => ClusterKind::Can,
            "FLEXRAY-CLUSTER" => ClusterKind::FlexRay,
            "ETHERNET-CLUSTER" => ClusterKind::Ethernet,
            _ => continue,
        };
        let name = short_name(node);
        if !name.is_empty() {
            out.push(ClusterInfo { kind, name });
        }
    }
    Ok(out)
}

/// Parses one named CAN cluster. The returned table is the same shape a DBC
/// produces, so every downstream window (Trace, Messages, Network, generator,
/// scripts) applies without a second CAN path.
pub fn parse_cluster(text: &str, cluster: &str) -> Result<CanArxmlDb, String> {
    let text = text.strip_prefix('\u{FEFF}').unwrap_or(text);
    let doc = Document::parse(text).map_err(|e| format!("XML parse error: {e}"))?;
    let root = doc.root_element();

    let can_clusters: Vec<Node> = root
        .descendants()
        .filter(|n| n.is_element() && n.has_tag_name("CAN-CLUSTER"))
        .collect();
    let cluster_node = can_clusters
        .iter()
        .copied()
        .find(|n| short_name(*n) == cluster)
        .ok_or_else(|| format!("CAN cluster `{cluster}` not found in the file"))?;
    let bitrate_kbps = number(cluster_node, "BAUDRATE")
        .filter(|v| *v > 0.0)
        .map(|v| (v / 1000.0).round() as u32);

    // ECU names and their frame ports: a triggering's FRAME-PORT refs are the
    // only place that says who transmits or receives a frame.
    let mut nodes: Vec<String> = Vec::new();
    for ecu in root
        .descendants()
        .filter(|n| n.is_element() && n.has_tag_name("ECU-INSTANCE"))
    {
        let name = short_name(ecu);
        if !name.is_empty() && !nodes.contains(&name) {
            nodes.push(name);
        }
    }
    let mut ports: HashMap<String, (String, bool)> = HashMap::new();
    for port in root
        .descendants()
        .filter(|n| n.is_element() && n.has_tag_name("FRAME-PORT"))
    {
        let name = short_name(port);
        let Some(ecu) = ancestor_ecu(port) else {
            continue;
        };
        let out = child_text(port, "COMMUNICATION-DIRECTION")
            .is_some_and(|d| d.eq_ignore_ascii_case("OUT"));
        ports.insert(name, (ecu, out));
    }

    // COMPU-METHOD / UNIT lookups are document-global.
    let mut units: HashMap<String, String> = HashMap::new();
    for unit in root
        .descendants()
        .filter(|n| n.is_element() && n.has_tag_name("UNIT"))
    {
        let name = short_name(unit);
        if !name.is_empty() {
            let display = child_text(unit, "DISPLAY-NAME").unwrap_or_else(|| name.clone());
            units.insert(name, display);
        }
    }
    let mut compus: HashMap<String, Compu> = HashMap::new();
    for cm in root
        .descendants()
        .filter(|n| n.is_element() && n.has_tag_name("COMPU-METHOD"))
    {
        let name = short_name(cm);
        if !name.is_empty() {
            compus.insert(name, read_compu(cm, &units));
        }
    }
    // A Vector matrix may state the scaling on SYSTEM-SIGNAL; I-SIGNAL refers
    // to it. Keep the fallback by system-signal short name.
    let mut system_compu: HashMap<String, String> = HashMap::new();
    for sys in root
        .descendants()
        .filter(|n| n.is_element() && n.has_tag_name("SYSTEM-SIGNAL"))
    {
        if let Some(r) = text_of(sys, "COMPU-METHOD-REF") {
            system_compu.insert(short_name(sys), ref_tail(&r).to_string());
        }
    }

    // SW-BASE-TYPE declarations: the encoding says whether the raw bits are
    // two's complement or IEEE float; the short name is only a label.
    let mut base_types: HashMap<String, (bool, bool)> = HashMap::new();
    for base in root
        .descendants()
        .filter(|n| n.is_element() && n.has_tag_name("SW-BASE-TYPE"))
    {
        let name = short_name(base);
        if name.is_empty() {
            continue;
        }
        match child_text(base, "BASE-TYPE-ENCODING").as_deref() {
            Some("2C") => {
                base_types.insert(name, (true, false));
            }
            Some("IEEE754") | Some("IEE754") => {
                base_types.insert(name, (false, true));
            }
            Some("NONE") | Some("BOOLEAN") => {
                base_types.insert(name, (false, false));
            }
            _ => {}
        }
    }

    let mut signal_map: HashMap<String, SignalDef> = HashMap::new();
    for sig in root
        .descendants()
        .filter(|n| n.is_element() && n.has_tag_name("I-SIGNAL"))
    {
        let name = short_name(sig);
        if name.is_empty() {
            continue;
        }
        let base = text_of(sig, "BASE-TYPE-REF").unwrap_or_default();
        let base_name = base.rsplit('/').next().unwrap_or(&base).to_string();
        // A declared encoding wins; the name heuristic only covers signals
        // whose base type the file does not define.
        let (signed, is_float) = base_types.get(&base_name).copied().unwrap_or_else(|| {
            let lower = base_name.to_ascii_lowercase();
            (
                lower.contains("sint") || lower.starts_with('i'),
                lower.contains("float"),
            )
        });
        let length = number(sig, "LENGTH").unwrap_or(0.0).max(0.0) as u64;
        let compu_name = text_of(sig, "COMPU-METHOD-REF")
            .map(|r| ref_tail(&r).to_string())
            .or_else(|| {
                text_of(sig, "SYSTEM-SIGNAL-REF")
                    .map(|r| ref_tail(&r).to_string())
                    .and_then(|s| system_compu.get(&s).cloned())
            });
        let compu = compu_name
            .as_deref()
            .and_then(|n| compus.get(n))
            .cloned()
            .unwrap_or_default();
        signal_map.insert(
            name,
            SignalDef {
                length,
                signed,
                is_float,
                factor: compu.factor,
                offset: compu.offset,
                min: compu.min,
                max: compu.max,
                initial: if is_float {
                    None
                } else {
                    // The XML value is the raw start value; the core works
                    // in physical readings.
                    initial_value(sig).map(|raw| raw * compu.factor + compu.offset)
                },
                unit: compu.unit,
                values: compu.values,
            },
        );
    }

    // PDUs and their signals. The PDU start bit is applied when a frame maps
    // the PDU; the signal start bit is relative to the PDU.
    let mut pdus: HashMap<String, PduDef> = HashMap::new();
    for pdu in root
        .descendants()
        .filter(|n| n.is_element() && n.has_tag_name("I-SIGNAL-I-PDU"))
    {
        let name = short_name(pdu);
        if name.is_empty() {
            continue;
        }
        let mut signals = Vec::new();
        for map in pdu
            .descendants()
            .filter(|n| n.has_tag_name("I-SIGNAL-TO-I-PDU-MAPPING"))
        {
            let Some(sig_ref) = child_text(map, "I-SIGNAL-REF") else {
                continue;
            };
            let sig_name = ref_tail(&sig_ref).to_string();
            let Some(def) = signal_map.get(&sig_name) else {
                continue;
            };
            let start_bit = number(map, "START-POSITION").unwrap_or(0.0).max(0.0) as u64;
            let big_endian = child_text(map, "PACKING-BYTE-ORDER")
                .is_some_and(|o| o.eq_ignore_ascii_case("MOST-SIGNIFICANT-BYTE-FIRST"));
            signals.push(PduSignal {
                name: sig_name,
                start_bit,
                length: def.length,
                big_endian,
                signed: def.signed,
                is_float: def.is_float,
                factor: def.factor,
                offset: def.offset,
                min: def.min,
                max: def.max,
                initial: def.initial,
                unit: def.unit.clone(),
                values: def.values.clone(),
            });
        }
        let cycle_us = pdu_cycle_us(pdu);
        pdus.insert(name, PduDef { signals, cycle_us });
    }
    // Some matrices put the cycle on the PDU-TRIGGERING instead.
    let mut trigger_cycles: HashMap<String, u64> = HashMap::new();
    for trig in cluster_node
        .descendants()
        .filter(|n| n.is_element() && n.has_tag_name("PDU-TRIGGERING"))
    {
        let Some(pdu_ref) = text_of(trig, "I-PDU-REF") else {
            continue;
        };
        if let Some(us) = seconds_to_us(number(trig, "VALUE")) {
            trigger_cycles.insert(ref_tail(&pdu_ref).to_string(), us);
        }
    }

    // Frame definitions by short name.
    let mut frames: HashMap<String, FrameDef> = HashMap::new();
    for frame in root
        .descendants()
        .filter(|n| n.is_element() && matches!(n.tag_name().name(), "CAN-FRAME" | "CAN-FD-FRAME"))
    {
        let name = short_name(frame);
        if name.is_empty() {
            continue;
        }
        let length = number(frame, "FRAME-LENGTH").unwrap_or(8.0).max(0.0) as u64;
        let mut maps = Vec::new();
        for map in frame
            .descendants()
            .filter(|n| n.has_tag_name("PDU-TO-FRAME-MAPPING"))
        {
            let Some(pdu_ref) = child_text(map, "PDU-REF").or_else(|| child_text(map, "I-PDU-REF"))
            else {
                continue;
            };
            maps.push((
                ref_tail(&pdu_ref).to_string(),
                number(map, "START-POSITION").unwrap_or(0.0).max(0.0) as u64,
            ));
        }
        frames.insert(name, FrameDef { length, maps });
    }

    let mut messages: HashMap<(u32, bool), MessageInfo> = HashMap::new();
    let mut order: Vec<(u32, bool)> = Vec::new();
    let mut value_tables: HashMap<((u32, bool), String), HashMap<i64, String>> = HashMap::new();
    for trig in root
        .descendants()
        .filter(|n| n.is_element() && n.has_tag_name("CAN-FRAME-TRIGGERING"))
        .filter(|t| triggering_cluster(*t, &can_clusters).as_deref() == Some(cluster))
    {
        let Some(frame_ref) = child_text(trig, "FRAME-REF") else {
            continue;
        };
        let frame_name = ref_tail(&frame_ref).to_string();
        let Some(frame) = frames.get(&frame_name) else {
            continue;
        };
        let id = number(trig, "IDENTIFIER").unwrap_or(0.0).max(0.0) as u32;
        let extended = child_text(trig, "CAN-ADDRESSING-MODE")
            .is_some_and(|m| m.eq_ignore_ascii_case("EXTENDED"))
            || id > 0x7FF;
        let key = (id, extended);
        if messages.contains_key(&key) {
            // Two frame triggerings on one id cannot both be named; keep the
            // first and leave the other out instead of silently overwriting.
            continue;
        }

        let (transmitter, receivers) = frame_ports(trig, &ports);
        let mut signals: Vec<SignalInfo> = Vec::new();
        let mut cycle_us: Option<u64> = None;
        for (pdu_name, pdu_start) in &frame.maps {
            let Some(pdu) = pdus.get(pdu_name) else {
                continue;
            };
            let pdu_cycle = pdu
                .cycle_us
                .or_else(|| trigger_cycles.get(pdu_name).copied());
            cycle_us = match (cycle_us, pdu_cycle) {
                (Some(a), Some(b)) => Some(a.min(b)),
                (None, b) => b,
                (a, None) => a,
            };
            for sig in &pdu.signals {
                let start_bit = pdu_start + sig.start_bit;
                let type_tag = if sig.is_float {
                    if sig.length == 64 { "f64" } else { "f32" }.to_string()
                } else {
                    format!("{}{}", if sig.signed { "i" } else { "u" }, sig.length)
                };
                if !sig.values.is_empty() {
                    value_tables.insert(
                        (key, sig.name.clone()),
                        sig.values.iter().cloned().collect(),
                    );
                }
                signals.push(SignalInfo {
                    name: sig.name.clone(),
                    start_bit,
                    size: sig.length,
                    big_endian: sig.big_endian,
                    signed: sig.signed,
                    factor: sig.factor,
                    offset: sig.offset,
                    min: sig.min.unwrap_or(0.0),
                    max: sig.max.unwrap_or(0.0),
                    initial: sig.initial,
                    unit: sig.unit.clone(),
                    receivers: receivers.clone(),
                    is_float: sig.is_float,
                    type_tag,
                    mux_when: Vec::new(),
                    comment: String::new(),
                });
            }
        }
        messages.insert(
            key,
            MessageInfo {
                name: frame_name,
                dlc: frame.length,
                transmitter,
                cycle_us,
                signals,
                switch_names: Vec::new(),
                comment: String::new(),
                fd: false,
            },
        );
        order.push(key);
    }

    // Only ECUs that appear on this cluster can own a role or a row in the
    // Network tree; a multi-cluster file would otherwise list every other
    // cluster's ECUs on each bus as well.
    let used: std::collections::HashSet<String> = messages
        .values()
        .flat_map(|m| {
            std::iter::once(m.transmitter.clone())
                .chain(m.signals.iter().flat_map(|s| s.receivers.iter().cloned()))
        })
        .filter(|n| !n.is_empty())
        .collect();
    if !used.is_empty() {
        nodes.retain(|n| used.contains(n));
    }

    Ok(CanArxmlDb {
        cluster: cluster.to_string(),
        bitrate_kbps,
        db: SymbolTable {
            messages,
            order,
            nodes,
            value_tables,
        },
    })
}

#[derive(Clone, Default)]
struct Compu {
    factor: f64,
    offset: f64,
    unit: String,
    min: Option<f64>,
    max: Option<f64>,
    values: Vec<(i64, String)>,
}

#[derive(Clone)]
struct SignalDef {
    length: u64,
    signed: bool,
    is_float: bool,
    factor: f64,
    offset: f64,
    min: Option<f64>,
    max: Option<f64>,
    initial: Option<f64>,
    unit: String,
    values: Vec<(i64, String)>,
}

struct PduSignal {
    name: String,
    start_bit: u64,
    length: u64,
    big_endian: bool,
    signed: bool,
    is_float: bool,
    factor: f64,
    offset: f64,
    min: Option<f64>,
    max: Option<f64>,
    initial: Option<f64>,
    unit: String,
    values: Vec<(i64, String)>,
}

struct PduDef {
    signals: Vec<PduSignal>,
    cycle_us: Option<u64>,
}

struct FrameDef {
    length: u64,
    maps: Vec<(String, u64)>,
}

fn read_compu(node: Node, units: &HashMap<String, String>) -> Compu {
    let unit = child_text(node, "UNIT-REF")
        .map(|r| ref_tail(&r).to_string())
        .and_then(|name| units.get(&name).cloned())
        .unwrap_or_default();
    let mut out = Compu {
        factor: 1.0,
        offset: 0.0,
        unit,
        ..Default::default()
    };
    for scale in node.descendants().filter(|n| n.has_tag_name("COMPU-SCALE")) {
        if let Some(vt) = text_of(scale, "VT")
            && let Some(raw) = number(scale, "LOWER-LIMIT")
        {
            let text = vt.trim();
            if !text.is_empty() {
                out.values.push((raw as i64, text.to_string()));
            }
            continue;
        }
        let Some(num) = scale
            .descendants()
            .find(|n| n.has_tag_name("COMPU-NUMERATOR"))
        else {
            continue;
        };
        let nums: Vec<f64> = num
            .children()
            .filter(|n| n.is_element())
            .filter_map(|n| n.text()?.trim().parse().ok())
            .collect();
        let den = scale
            .descendants()
            .find(|n| n.has_tag_name("COMPU-DENOMINATOR"))
            .and_then(|n| n.children().find(|c| c.is_element()))
            .and_then(|n| n.text()?.trim().parse::<f64>().ok())
            .filter(|v| *v != 0.0)
            .unwrap_or(1.0);
        out.offset = nums.first().copied().unwrap_or(0.0) / den;
        out.factor = nums.get(1).copied().unwrap_or(1.0) / den;
        if let (Some(lo), Some(hi)) = (number(scale, "LOWER-LIMIT"), number(scale, "UPPER-LIMIT")) {
            let a = out.offset + out.factor * lo;
            let b = out.offset + out.factor * hi;
            out.min = Some(a.min(b));
            out.max = Some(a.max(b));
        }
        break;
    }
    out
}

fn pdu_cycle_us(pdu: Node) -> Option<u64> {
    for period in pdu.descendants().filter(|n| n.has_tag_name("TIME-PERIOD")) {
        if let Some(us) = seconds_to_us(number(period, "VALUE")) {
            return Some(us);
        }
    }
    None
}

fn seconds_to_us(seconds: Option<f64>) -> Option<u64> {
    let s = seconds?;
    (s > 0.0).then(|| (s * 1_000_000.0).round() as u64)
}

fn frame_ports(trig: Node, ports: &HashMap<String, (String, bool)>) -> (String, Vec<String>) {
    let mut transmitter = String::new();
    let mut receivers: Vec<String> = Vec::new();
    for r in trig
        .descendants()
        .filter(|n| n.is_element() && n.has_tag_name("FRAME-PORT-REF"))
    {
        let Some(text) = r.text() else {
            continue;
        };
        let path = text.trim();
        let port = ref_tail(path);
        let (ecu, out) = match ports.get(port) {
            Some((ecu, out)) => (ecu.clone(), *out),
            None => {
                // A port that is not declared still names its ECU in the path,
                // and the `_Tx`/`_Rx` suffix is the convention both sample
                // matrices use. A path without either stays receiver-neutral.
                let ecu = path
                    .split("/ECU/")
                    .nth(1)
                    .and_then(|rest| rest.split('/').next())
                    .unwrap_or("")
                    .to_string();
                let out = port.ends_with("_Tx");
                (ecu, out)
            }
        };
        if ecu.is_empty() {
            continue;
        }
        if out {
            if transmitter.is_empty() {
                transmitter = ecu;
            }
        } else if !receivers.contains(&ecu) {
            receivers.push(ecu);
        }
    }
    (transmitter, receivers)
}

fn ancestor_ecu(node: Node) -> Option<String> {
    node.ancestors()
        .find(|n| n.has_tag_name("ECU-INSTANCE"))
        .map(short_name)
        .filter(|n| !n.is_empty())
}

/// Which CAN cluster owns a frame triggering. Most matrices nest the
/// triggering under the cluster's physical channel; some keep it in a shared
/// package and name the cluster through a PDU-triggering path
/// (`/Cluster/<name>/...`). A single-cluster file is unambiguous either way.
fn triggering_cluster(trig: Node, clusters: &[Node]) -> Option<String> {
    if let Some(cluster) = trig.ancestors().find(|n| n.has_tag_name("CAN-CLUSTER")) {
        let name = short_name(cluster);
        if !name.is_empty() {
            return Some(name);
        }
    }
    if let Some(path) = text_of(trig, "PDU-TRIGGERING-REF")
        && let Some(name) = cluster_of_path(&path)
    {
        return Some(name);
    }
    if clusters.len() == 1 {
        let name = short_name(clusters[0]);
        if !name.is_empty() {
            return Some(name);
        }
    }
    None
}

/// The cluster segment of an AUTOSAR reference path:
/// `/Cluster/CanA/CH/PT_x` -> `CanA`.
fn cluster_of_path(path: &str) -> Option<String> {
    let mut parts = path.split('/').filter(|p| !p.is_empty());
    match (parts.next(), parts.next()) {
        (Some("Cluster"), Some(name)) => Some(name.to_string()),
        _ => None,
    }
}

fn short_name(node: Node) -> String {
    node.children()
        .find(|n| n.is_element() && n.has_tag_name("SHORT-NAME"))
        .and_then(|n| n.text())
        .map(|t| t.trim().to_string())
        .unwrap_or_default()
}

fn child_text(node: Node, tag: &str) -> Option<String> {
    node.children()
        .find(|n| n.is_element() && n.has_tag_name(tag))
        .and_then(|n| n.text())
        .map(|t| t.trim().to_string())
}

fn text_of(node: Node, tag: &str) -> Option<String> {
    node.descendants()
        .find(|n| n.is_element() && n.has_tag_name(tag))
        .and_then(|n| n.text())
        .map(|t| t.trim().to_string())
}

fn number(node: Node, tag: &str) -> Option<f64> {
    let text = text_of(node, tag)?;
    if let Some(hex) = text.strip_prefix("0x").or_else(|| text.strip_prefix("0X")) {
        return u64::from_str_radix(hex, 16).ok().map(|v| v as f64);
    }
    text.parse().ok()
}

/// `I-SIGNAL/INIT-VALUE` as a raw start value. Arrays and references have no
/// single number and stay `None`.
fn initial_value(node: Node) -> Option<f64> {
    node.children()
        .find(|n| n.is_element() && n.has_tag_name("INIT-VALUE"))
        .and_then(|init| {
            init.descendants()
                .find(|n| n.has_tag_name("NUMERICAL-VALUE-SPECIFICATION"))
        })
        .and_then(|spec| number(spec, "VALUE"))
}

fn ref_tail(path: &str) -> &str {
    path.rsplit('/').find(|s| !s.is_empty()).unwrap_or(path)
}
