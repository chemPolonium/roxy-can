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
    /// NM timing when the document states it (milliseconds).
    pub nm_timing: Option<EthNmTiming>,
    /// PDUs referenced by `PDU-TRIGGERING` even when no frame maps them
    /// (Ethernet NM and PDU-only deployments).
    pub pdus: Vec<EthPdu>,
    /// Socket addresses / application endpoints from the same document.
    pub endpoints: Vec<EthEndpoint>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct EthNmTiming {
    pub repeat_ms: u64,
    pub ready_sleep_ms: u64,
    pub prepare_sleep_ms: u64,
    pub timeout_ms: u64,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct EthEndpoint {
    pub name: String,
    pub cluster: String,
    pub ip: String,
    pub port: u16,
    /// "UDP" or "TCP".
    pub protocol: String,
    pub connector: String,
    /// PDU carried by this endpoint when a PDU triggering names it.
    pub pdu: String,
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

#[cfg(test)]
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DecodedEth {
    pub frame_name: String,
    pub signals: Vec<DecodedSignal>,
}

#[cfg(test)]
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
    /// Merges another matrix describing the same physical bus. A cluster
    /// name or a frame name inside one cluster keeps the first definition.
    #[allow(dead_code)]
    pub fn merge(&mut self, other: EthDb) -> Vec<String> {
        let mut warnings = Vec::new();
        for cluster in other.clusters {
            if self.clusters.iter().any(|known| known.name == cluster.name) {
                warnings.push(format!(
                    "Ethernet cluster `{}` is declared twice; first wins",
                    cluster.name
                ));
            } else {
                self.clusters.push(cluster);
            }
        }
        for frame in other.frames {
            if self
                .frames
                .iter()
                .any(|known| known.name == frame.name && known.cluster == frame.cluster)
            {
                warnings.push(format!(
                    "Ethernet frame `{}` in `{}` is declared twice; first wins",
                    frame.name, frame.cluster
                ));
            } else {
                self.frames.push(frame);
            }
        }
        for pdu in other.pdus {
            if self.pdus.iter().any(|known| known.name == pdu.name) {
                warnings.push(format!(
                    "Ethernet PDU `{}` is declared twice; first wins",
                    pdu.name
                ));
            } else {
                self.pdus.push(pdu);
            }
        }
        if self.nm_timing.is_none() {
            self.nm_timing = other.nm_timing;
        }
        for endpoint in other.endpoints {
            if self
                .endpoints
                .iter()
                .any(|known| known.name == endpoint.name && known.ip == endpoint.ip)
            {
                continue;
            }
            self.endpoints.push(endpoint);
        }
        warnings
    }

    /// Tries to bind one UDP payload to a PDU-only endpoint. Matching is by
    /// port and, when the ARXML gave an address, by source/destination IP.
    pub fn decode_nm(
        &self,
        src_ip: &str,
        dst_ip: &str,
        src_port: u16,
        dst_port: u16,
        payload: &[u8],
    ) -> Option<(String, Vec<(String, f64)>)> {
        let endpoint = self.endpoints.iter().find(|endpoint| {
            endpoint.protocol == "UDP"
                && !endpoint.pdu.is_empty()
                && (endpoint.port == 0 || endpoint.port == src_port || endpoint.port == dst_port)
                && (endpoint.ip.is_empty() || endpoint.ip == src_ip || endpoint.ip == dst_ip)
        })?;
        let pdu = self.pdus.iter().find(|pdu| pdu.name == endpoint.pdu)?;
        Some((pdu.name.clone(), decode_pdu_signals(pdu, payload)))
    }

    pub fn parse(text: &str) -> Result<Self, String> {
        let doc = Document::parse(text).map_err(|e| e.to_string())?;
        let root = doc.root_element();
        if root.tag_name().name().eq_ignore_ascii_case("FIBEX") {
            return Self::parse_fibex(&doc);
        }
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

        // Optional NM timing: CP documents name these in milliseconds.
        let mut timing = EthNmTiming::default();
        for (tag, slot) in [
            ("NM-REPEAT-MESSAGE-TIME", 0),
            ("NM-READY-SLEEP-TIME", 1),
            ("NM-PREPARE-BUS-SLEEP-TIME", 2),
            ("NM-TIMEOUT-TIME", 3),
        ] {
            if let Some(value) = root
                .descendants()
                .find(|n| n.has_tag_name(tag))
                .and_then(|n| n.text())
                .and_then(|text| text.trim().parse::<u64>().ok())
            {
                match slot {
                    0 => timing.repeat_ms = value,
                    1 => timing.ready_sleep_ms = value,
                    2 => timing.prepare_sleep_ms = value,
                    _ => timing.timeout_ms = value,
                }
            }
        }
        if timing.repeat_ms + timing.ready_sleep_ms + timing.prepare_sleep_ms + timing.timeout_ms
            > 0
        {
            db.nm_timing = Some(timing);
        }

        // PDUs without ever declaring an Ethernet frame. Those PDUs are still
        // useful database objects, so keep them separately from frame PDUs.
        let pdu_nodes: HashMap<String, Node> = root
            .descendants()
            .filter(|n| {
                n.is_element()
                    && matches!(
                        n.tag_name().name(),
                        "NM-PDU" | "I-SIGNAL-I-PDU" | "I-PDU" | "PDU"
                    )
            })
            .filter_map(|n| short_name_opt(n).map(|name| (name, n)))
            .collect();
        for triggering in root
            .descendants()
            .filter(|n| n.is_element() && n.has_tag_name("PDU-TRIGGERING"))
        {
            let Some(reference) = child_text(triggering, "I-PDU-REF") else {
                continue;
            };
            let name = ref_tail(&reference);
            if db.pdus.iter().any(|pdu| pdu.name == name) {
                continue;
            }
            let Some(node) = pdu_nodes.get(&name) else {
                continue;
            };
            if let Some(pdu) = read_pdu(*node, &signals, &compus, &units, &base_types) {
                db.pdus.push(pdu);
            } else {
                db.pdus.push(EthPdu {
                    name,
                    length: number(*node, "LENGTH")
                        .or_else(|| number(*node, "BYTE-LENGTH"))
                        .unwrap_or(0.0) as usize,
                    ..Default::default()
                });
            }
        }

        // Socket/application endpoints are the service-oriented counterpart
        // of frame triggerings; they often exist in documents with no frames.
        let mut connector_pdus: HashMap<String, String> = HashMap::new();
        for triggering in root
            .descendants()
            .filter(|n| n.is_element() && n.has_tag_name("PDU-TRIGGERING"))
        {
            let Some(reference) = child_text(triggering, "I-PDU-REF") else {
                continue;
            };
            let pdu = ref_tail(&reference);
            for port in triggering
                .descendants()
                .filter(|n| n.has_tag_name("I-PDU-PORT-REF"))
            {
                if let Some(connector) = port.text().and_then(|path| {
                    path.split('/').rev().find(|segment| {
                        segment.contains("connector") && !segment.contains("pdu_port")
                    })
                }) {
                    connector_pdus.insert(connector.to_string(), pdu.clone());
                }
            }
        }
        let network_ips: HashMap<String, String> = root
            .descendants()
            .filter(|n| n.is_element() && n.has_tag_name("NETWORK-ENDPOINT"))
            .filter_map(|node| {
                let name = short_name_opt(node)?;
                let ip = node
                    .descendants()
                    .find(|n| n.has_tag_name("IPV-4-ADDRESS"))
                    .and_then(|n| n.text())
                    .map(str::trim)
                    .filter(|text| !text.is_empty())
                    .unwrap_or_default()
                    .to_string();
                Some((name, ip))
            })
            .collect();
        for socket in root
            .descendants()
            .filter(|n| n.is_element() && n.has_tag_name("SOCKET-ADDRESS"))
        {
            let name = short_name(socket);
            if name.is_empty() {
                continue;
            }
            let Some(application) = socket
                .children()
                .find(|n| n.has_tag_name("APPLICATION-ENDPOINT"))
            else {
                continue;
            };
            let port = application
                .descendants()
                .find(|n| n.has_tag_name("PORT-NUMBER"))
                .and_then(|n| n.text())
                .and_then(|text| text.trim().parse::<u16>().ok())
                .unwrap_or(0);
            let protocol = if application.descendants().any(|n| n.has_tag_name("TCP-TP")) {
                "TCP"
            } else {
                "UDP"
            };
            let ip = application
                .descendants()
                .find(|n| n.has_tag_name("NETWORK-ENDPOINT-REF"))
                .and_then(|n| n.text())
                .map(ref_tail)
                .and_then(|name| network_ips.get(&name).cloned())
                .unwrap_or_default();
            let connector = child_text(socket, "CONNECTOR-REF")
                .map(|path| ref_tail(&path))
                .unwrap_or_default();
            let pdu = connector_pdus.get(&connector).cloned().unwrap_or_default();
            let cluster = socket
                .ancestors()
                .find(|n| n.has_tag_name("ETHERNET-CLUSTER"))
                .map(short_name)
                .unwrap_or_default();
            db.endpoints.push(EthEndpoint {
                name,
                cluster,
                ip,
                port,
                protocol: protocol.to_string(),
                connector,
                pdu,
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

    /// Minimal FIBEX 4 Ethernet reader: channels become clusters, FRAME
    /// elements become frame definitions, PDU/SIGNAL instances become signal
    /// layouts. Vendor extensions that do not follow this shape are reported
    /// as an empty database rather than guessed at.
    fn parse_fibex(doc: &Document) -> Result<Self, String> {
        let root = doc.root_element();
        let mut db = EthDb::default();
        let mut cluster_names: Vec<String> = root
            .descendants()
            .filter(|n| {
                n.is_element()
                    && matches!(
                        n.tag_name().name(),
                        "CHANNEL" | "PHYSICAL-CHANNEL" | "CLUSTER"
                    )
            })
            .filter_map(short_name_opt)
            .collect();
        cluster_names.sort();
        cluster_names.dedup();
        if cluster_names.is_empty() {
            cluster_names.push("FIBEX".to_string());
        }
        let cluster = cluster_names[0].clone();
        for name in cluster_names {
            db.clusters.push(EthCluster {
                name,
                vlans: Vec::new(),
            });
        }

        let signal_nodes: HashMap<String, Node> = root
            .descendants()
            .filter(|n| n.is_element() && n.tag_name().name() == "SIGNAL")
            .filter_map(|n| short_name_opt(n).map(|name| (name, n)))
            .collect();
        let pdu_nodes: HashMap<String, Node> = root
            .descendants()
            .filter(|n| n.is_element() && n.tag_name().name() == "PDU")
            .filter_map(|n| short_name_opt(n).map(|name| (name, n)))
            .collect();

        for frame in root
            .descendants()
            .filter(|n| n.is_element() && n.tag_name().name() == "FRAME")
        {
            let Some(name) = short_name_opt(frame) else {
                continue;
            };
            let length = child_text(frame, "BYTE-LENGTH")
                .or_else(|| child_text(frame, "FRAME-LENGTH"))
                .and_then(|value| value.parse::<usize>().ok())
                .unwrap_or(0);
            let mut pdus = Vec::new();
            for instance in frame.descendants().filter(|n| {
                n.is_element()
                    && matches!(
                        n.tag_name().name(),
                        "PDU-INSTANCE" | "PDU-TRIGGERING" | "PDU-REF"
                    )
            }) {
                let Some(reference) = child_text(instance, "PDU-REF").or_else(|| {
                    (instance.tag_name().name() == "PDU-REF")
                        .then(|| instance.text())
                        .flatten()
                        .map(str::to_string)
                }) else {
                    continue;
                };
                let pdu_name = ref_tail(&reference);
                let Some(pdu_node) = pdu_nodes.get(&pdu_name) else {
                    continue;
                };
                let start_bit = number(instance, "BIT-POSITION")
                    .or_else(|| number(instance, "START-POSITION"))
                    .unwrap_or(0.0) as usize;
                let mut pdu = EthPdu {
                    name: pdu_name,
                    length: child_text(*pdu_node, "BYTE-LENGTH")
                        .or_else(|| child_text(*pdu_node, "LENGTH"))
                        .and_then(|value| value.parse::<usize>().ok())
                        .unwrap_or(8),
                    start_bit,
                    signals: Vec::new(),
                };
                for signal_instance in pdu_node
                    .descendants()
                    .filter(|n| n.is_element() && n.tag_name().name() == "SIGNAL-INSTANCE")
                {
                    let Some(reference) = child_text(signal_instance, "SIGNAL-REF") else {
                        continue;
                    };
                    let signal_name = ref_tail(&reference);
                    let Some(signal_node) = signal_nodes.get(&signal_name) else {
                        continue;
                    };
                    let size = number(signal_instance, "BIT-LENGTH")
                        .or_else(|| number(*signal_node, "BIT-LENGTH"))
                        .or_else(|| number(*signal_node, "LENGTH"))
                        .unwrap_or(1.0) as usize;
                    pdu.signals.push(EthSignal {
                        name: signal_name,
                        start_bit: number(signal_instance, "BIT-POSITION").unwrap_or(0.0) as usize,
                        size,
                        big_endian: child_text(*signal_node, "IS-BIG-ENDIAN")
                            .is_some_and(|value| value.eq_ignore_ascii_case("true")),
                        signed: child_text(*signal_node, "IS-SIGNED")
                            .is_some_and(|value| value.eq_ignore_ascii_case("true")),
                        ..Default::default()
                    });
                }
                pdus.push(pdu);
            }
            db.frames.push(EthFrameDef {
                name,
                cluster: cluster.clone(),
                vlan: None,
                length,
                ethertype_hint: None,
                pdus,
            });
        }
        if db.frames.is_empty() {
            let ethernet = root.descendants().any(|node| {
                node.has_tag_name("PROTOCOL")
                    && node
                        .text()
                        .is_some_and(|text| text.trim().eq_ignore_ascii_case("ETHERNET"))
            }) || root.descendants().any(|node| {
                matches!(
                    node.tag_name().name(),
                    "SERVICE-INTERFACE" | "PROVIDED-SERVICE-INSTANCE"
                )
            });
            if db.clusters.is_empty() || !ethernet {
                return Err("FIBEX has no Ethernet FRAME elements".into());
            }
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

    #[cfg(test)]
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

/// Decodes the signal table of a PDU-only object against one UDP payload.
pub fn decode_pdu_signals(pdu: &EthPdu, payload: &[u8]) -> Vec<(String, f64)> {
    let mut out = Vec::new();
    for signal in &pdu.signals {
        let Some(raw) = extract_bits(payload, signal.start_bit, signal.size, signal.big_endian)
        else {
            continue;
        };
        let physical = if signal.signed {
            sign_extend(raw, signal.size) as f64 * signal.factor + signal.offset
        } else {
            raw as f64 * signal.factor + signal.offset
        };
        out.push((signal.name.clone(), physical));
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

fn short_name_opt(node: Node) -> Option<String> {
    let name = short_name(node);
    (!name.is_empty()).then_some(name)
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

    #[test]
    fn nm_decode_matches_endpoint_and_decodes_signals() {
        let db = EthDb {
            pdus: vec![EthPdu {
                name: "Nm".into(),
                length: 8,
                start_bit: 0,
                signals: vec![EthSignal {
                    name: "Repeat".into(),
                    start_bit: 0,
                    size: 1,
                    factor: 1.0,
                    ..Default::default()
                }],
            }],
            endpoints: vec![EthEndpoint {
                name: "Sock".into(),
                cluster: "EthA".into(),
                ip: "192.168.1.10".into(),
                port: 50001,
                protocol: "UDP".into(),
                connector: String::new(),
                pdu: "Nm".into(),
            }],
            ..Default::default()
        };
        let (pdu, signals) = db
            .decode_nm("192.168.1.10", "192.168.1.255", 50001, 50002, &[1])
            .expect("NM match");
        assert_eq!(pdu, "Nm");
        assert_eq!(signals, [("Repeat".to_string(), 1.0)]);
    }

    #[test]
    fn pdu_only_arxml_keeps_pdus_and_endpoints() {
        let text = r#"<?xml version="1.0"?>
<AUTOSAR xmlns="http://autosar.org/schema/r4.0">
 <AR-PACKAGES><AR-PACKAGE><SHORT-NAME>P</SHORT-NAME><ELEMENTS>
  <ETHERNET-CLUSTER><SHORT-NAME>EthA</SHORT-NAME>
   <NM-REPEAT-MESSAGE-TIME>1000</NM-REPEAT-MESSAGE-TIME>
   <NM-READY-SLEEP-TIME>2000</NM-READY-SLEEP-TIME>
   <NM-PREPARE-BUS-SLEEP-TIME>3000</NM-PREPARE-BUS-SLEEP-TIME>
   <NM-TIMEOUT-TIME>4000</NM-TIMEOUT-TIME>
   <PDU-TRIGGERINGS>
    <PDU-TRIGGERING><SHORT-NAME>trig</SHORT-NAME>
     <I-PDU-REF>/P/Nm</I-PDU-REF>
    </PDU-TRIGGERING>
   </PDU-TRIGGERINGS>
  </ETHERNET-CLUSTER>
  <NM-PDU><SHORT-NAME>Nm</SHORT-NAME><LENGTH>8</LENGTH>
   <I-SIGNAL-TO-I-PDU-MAPPINGS>
    <I-SIGNAL-TO-I-PDU-MAPPING><SHORT-NAME>m</SHORT-NAME>
     <I-SIGNAL-REF>/P/Repeat</I-SIGNAL-REF>
     <START-POSITION>0</START-POSITION>
     <PACKING-BYTE-ORDER>MOST-SIGNIFICANT-BYTE-LAST</PACKING-BYTE-ORDER>
    </I-SIGNAL-TO-I-PDU-MAPPING>
   </I-SIGNAL-TO-I-PDU-MAPPINGS>
  </NM-PDU>
  <I-SIGNAL><SHORT-NAME>Repeat</SHORT-NAME><LENGTH>1</LENGTH></I-SIGNAL>
  <NETWORK-ENDPOINT><SHORT-NAME>Nep</SHORT-NAME>
   <NETWORK-ENDPOINT-ADDRESSES><IPV-4-CONFIGURATION>
    <IPV-4-ADDRESS>192.168.1.10</IPV-4-ADDRESS>
   </IPV-4-CONFIGURATION></NETWORK-ENDPOINT-ADDRESSES>
  </NETWORK-ENDPOINT>
  <SOCKET-ADDRESS><SHORT-NAME>Sock</SHORT-NAME>
   <APPLICATION-ENDPOINT>
    <NETWORK-ENDPOINT-REF>/P/Nep</NETWORK-ENDPOINT-REF>
    <TP-CONFIGURATION><UDP-TP><UDP-TP-PORT>
     <PORT-NUMBER>50001</PORT-NUMBER>
    </UDP-TP-PORT></UDP-TP></TP-CONFIGURATION>
   </APPLICATION-ENDPOINT>
  </SOCKET-ADDRESS>
 </ELEMENTS></AR-PACKAGE></AR-PACKAGES>
</AUTOSAR>"#;
        let db = EthDb::parse(text).expect("PDU-only ARXML");
        assert_eq!(db.frames.len(), 0);
        assert_eq!(db.clusters[0].name, "EthA");
        assert_eq!(db.pdus[0].name, "Nm");
        assert_eq!(
            db.nm_timing,
            Some(EthNmTiming {
                repeat_ms: 1000,
                ready_sleep_ms: 2000,
                prepare_sleep_ms: 3000,
                timeout_ms: 4000,
            })
        );
        assert_eq!(db.pdus[0].signals[0].name, "Repeat");
        assert_eq!(db.endpoints[0].name, "Sock");
        assert_eq!(db.endpoints[0].ip, "192.168.1.10");
        assert_eq!(db.endpoints[0].port, 50001);
        assert_eq!(db.endpoints[0].protocol, "UDP");
    }

    #[test]
    fn fibex_frames_pdus_and_signals_are_read() {
        let text = r#"<?xml version="1.0"?>
<FIBEX>
  <CHANNELS><CHANNEL><SHORT-NAME>EthA</SHORT-NAME></CHANNEL></CHANNELS>
  <SIGNALS><SIGNAL><SHORT-NAME>Speed</SHORT-NAME><BIT-LENGTH>16</BIT-LENGTH></SIGNAL></SIGNALS>
  <PDUS><PDU><SHORT-NAME>P1</SHORT-NAME><BYTE-LENGTH>8</BYTE-LENGTH>
    <SIGNAL-INSTANCES><SIGNAL-INSTANCE><SIGNAL-REF>/Speed</SIGNAL-REF>
      <BIT-POSITION>8</BIT-POSITION><BIT-LENGTH>16</BIT-LENGTH>
    </SIGNAL-INSTANCE></SIGNAL-INSTANCES>
  </PDU></PDUS>
  <FRAMES><FRAME><SHORT-NAME>F1</SHORT-NAME><BYTE-LENGTH>16</BYTE-LENGTH>
    <PDU-INSTANCES><PDU-INSTANCE><PDU-REF>/P1</PDU-REF><BIT-POSITION>0</BIT-POSITION>
    </PDU-INSTANCE></PDU-INSTANCES>
  </FRAME></FRAMES>
</FIBEX>"#;
        let db = EthDb::parse(text).expect("FIBEX");
        assert_eq!(db.clusters[0].name, "EthA");
        assert_eq!(db.frames.len(), 1);
        assert_eq!(db.frames[0].length, 16);
        assert_eq!(db.frames[0].pdus[0].name, "P1");
        assert_eq!(db.frames[0].pdus[0].signals[0].name, "Speed");
        assert_eq!(db.frames[0].pdus[0].signals[0].start_bit, 8);
        assert_eq!(db.frames[0].pdus[0].signals[0].size, 16);
    }
}
