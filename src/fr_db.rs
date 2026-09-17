//! FlexRay database: FIBEX (2.x/3.x) and AUTOSAR ARXML (R4.x) parsing.
//!
//! Ported from the roxy-fibex project's importers, trimmed to a read-only
//! database: cluster parameters, ECUs, PDUs with signals, frames with
//! PDU mappings and the slot/cycle schedule. Both dialects are parsed
//! tolerantly (namespaces ignored, attribute-carrying tags accepted);
//! the format is detected from the document root and, failing that,
//! from characteristic element names.
//!
//! The parsed model is a plain data container -- many fields exist to
//! describe the file faithfully before any view consumes them -- so
//! dead-code analysis is silenced module-wide.

#![allow(dead_code)]

use std::collections::HashMap;

/// FlexRay transmission channel. A / B / both at once.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum FrChannel {
    #[default]
    A,
    B,
    Both,
}

impl FrChannel {
    pub fn label(self) -> &'static str {
        match self {
            FrChannel::A => "A",
            FrChannel::B => "B",
            FrChannel::Both => "A+B",
        }
    }

    /// Whether this channel selection covers channel `ch`.
    pub fn covers(self, ch: FrChannel) -> bool {
        self == FrChannel::Both || self == ch
    }
}

/// One signal of a PDU: position, encoding and display metadata.
#[derive(Clone, Debug, Default)]
pub struct FrSignal {
    pub name: String,
    pub start_bit: u32,
    pub length_bits: u32,
    /// Motorola / big-endian (the FlexRay default) when true.
    pub big_endian: bool,
    pub signed: bool,
    pub factor: f64,
    pub offset: f64,
    pub min: f64,
    pub max: f64,
    pub unit: String,
    pub comment: String,
    /// Enumeration labels: raw value -> text.
    pub value_descriptions: Vec<(i64, String)>,
}

/// One PDU: a payload chunk with its signal layout.
#[derive(Clone, Debug, Default)]
pub struct FrPdu {
    pub name: String,
    /// Byte length.
    pub length: u32,
    /// Dynamic / event PDUs ride the dynamic segment; static otherwise.
    pub dynamic: bool,
    pub comment: String,
    pub signals: Vec<FrSignal>,
}

/// A frame's static-segment schedule entry (FIBEX FRAME-TRIGGERING /
/// AUTOSAR FLEXRAY-FRAME-TRIGGERING).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct FrTriggering {
    pub channel: FrChannel,
    /// Static slot number 1..=2047.
    pub slot_id: u32,
    /// Base cycle 0..=63.
    pub base_cycle: u32,
    /// Cycle repetition 1/2/4/8/16/32/64.
    pub cycle_repetition: u32,
    pub startup: bool,
}

impl FrTriggering {
    /// Whether the trigger fires on communication cycle `cycle`.
    pub fn is_active_at_cycle(&self, cycle: u32) -> bool {
        cycle % self.cycle_repetition.max(1) == self.base_cycle % self.cycle_repetition.max(1)
    }
}

/// One frame: payload size, schedule and the PDUs it carries.
#[derive(Clone, Debug, Default)]
pub struct FrFrameDb {
    pub name: String,
    /// Byte length 0..=254.
    pub length: u32,
    pub payload_preamble: bool,
    pub triggering: FrTriggering,
    /// (PDU name, start byte in the frame).
    pub pdus: Vec<(String, u32)>,
    pub comment: String,
}

/// Cluster-level protocol parameters. The core sixteen mirror the
/// editor's set; the `extra` block collects the raw vxlapi-oriented
/// values (payload length, correction windows, sample clock) that the
/// driver configuration needs and only some files carry.
#[derive(Clone, Debug, Default)]
pub struct FrClusterParams {
    pub name: String,
    /// fx:SPEED, normalised to kbit/s (5000 or 10000).
    pub speed_kbps: u32,
    /// gdCycle in ms.
    pub cycle_time_ms: f64,
    /// Macrotick duration in µs.
    pub macrotick_duration_us: f64,
    pub coldstart_attempts: u32,
    pub action_point_offset: u32,
    pub minislot_action_point_offset: u32,
    pub dynamic_slot_idle_phase: u32,
    pub minor_version: u32,
    pub network_idle_time: u32,
    pub number_of_minislots: u32,
    pub number_of_static_slots: u32,
    pub minislot_duration: u32,
    pub static_slot_duration: u32,
    pub symbol_window: u32,
    pub symbol_window_idle_phase: u32,
    pub offset_correction_start: u32,
    // ---- extras consumed by the vxlapi cluster config ----
    /// gPayloadLengthStatic (bytes).
    pub payload_length_static: u32,
    pub g_macro_per_cycle: Option<u32>,
    pub g_micro_per_cycle: u32,
    pub p_micro_per_macro_nom: u32,
    pub p_samples_per_microtick: u32,
    /// SAMPLE-CLOCK-PERIOD in µs.
    pub sample_clock_period_us: f64,
    pub gd_cas_rx_low_max: u32,
    pub g_sync_node_max: u32,
    pub g_listen_noise: u32,
    pub g_network_management_vector_length: u32,
    pub g_max_without_clock_correction_fatal: u32,
    pub g_max_without_clock_correction_passive: u32,
    pub gd_tss_transmitter: u32,
    pub p_cluster_drift_damping: u32,
    pub p_decoding_correction: u32,
    pub p_delay_compensation_a: u32,
    pub p_delay_compensation_b: u32,
    pub p_extern_offset_correction: u32,
    pub p_extern_rate_correction: u32,
    pub p_latest_tx: u32,
    pub p_macro_initial_offset_a: u32,
    pub p_macro_initial_offset_b: u32,
    pub p_micro_initial_offset_a: u32,
    pub p_micro_initial_offset_b: u32,
    pub p_offset_correction_out: u32,
    pub p_rate_correction_out: u32,
    pub p_allow_halt_due_to_clock: bool,
    pub p_allow_passive_to_active: u32,
    pub p_key_slot_used_for_startup: u32,
    pub p_key_slot_used_for_sync: u32,
    pub p_single_slot_enabled: u32,
    pub p_wakeup_channel: u32,
    pub p_wakeup_pattern: u32,
    pub pd_accepted_startup_range: u32,
    pub pd_listen_timeout: u32,
    pub pd_max_drift: u32,
    pub pd_microtick: u32,
    pub g_channels: u32,
    pub p_channels: u32,
}

/// The parsed FlexRay database.
#[derive(Clone, Debug, Default)]
pub struct FrDb {
    pub params: FrClusterParams,
    pub ecus: Vec<String>,
    pub pdus: Vec<FrPdu>,
    pub frames: Vec<FrFrameDb>,
}

impl FrDb {
    /// Parses FIBEX or ARXML text, detecting the dialect from the root
    /// element and, failing that, from characteristic children.
    pub fn parse(text: &str) -> Result<FrDb, String> {
        let doc = roxmltree::Document::parse(text).map_err(|e| format!("XML 解析错误：{e}"))?;
        let root = doc.root_element();
        let root_name = root.tag_name().name().to_uppercase();
        if root_name.contains("FIBEX") {
            parse_fibex_doc(&doc)
        } else if root_name.contains("AUTOSAR") {
            parse_arxml_doc(&doc)
        } else {
            let mut has_fibex = false;
            let mut has_autosar = false;
            for node in doc.descendants() {
                if !node.is_element() {
                    continue;
                }
                let name = node.tag_name().name().to_uppercase();
                if name == "FLEXRAY-CLUSTER" || name == "I-SIGNAL-I-PDU" || name == "ECU-INSTANCE"
                {
                    has_autosar = true;
                    break;
                }
                if name == "CLUSTER" || name == "SLOT" {
                    has_fibex = true;
                }
            }
            if has_autosar {
                parse_arxml_doc(&doc)
            } else if has_fibex {
                parse_fibex_doc(&doc)
            } else {
                Err("无法识别的 XML 格式：既不是 FIBEX 也不是 AUTOSAR".to_string())
            }
        }
    }

    /// The frame scheduled in `slot` and active on communication cycle
    /// `cycle`, preferring one whose channel matches `ab` (0 = A, 1 = B,
    /// 2 = unknown). Several frames may share a slot across repetitions;
    /// the first whose schedule covers the cycle wins.
    pub fn frame_at(&self, slot: u16, cycle: u8, ab: u8) -> Option<&FrFrameDb> {
        let want = match ab {
            0 => Some(FrChannel::A),
            1 => Some(FrChannel::B),
            _ => None,
        };
        self.frames
            .iter()
            .filter(|f| f.triggering.slot_id == slot as u32)
            .filter(|f| f.triggering.is_active_at_cycle(cycle as u32))
            .find(|f| match want {
                Some(w) => f.triggering.channel.covers(w),
                None => true,
            })
    }

    /// Decodes a frame payload into `(signal, text)` pairs: physical
    /// values with units, enumeration labels where the database has
    /// them. PDU start offsets shift each signal's absolute bit.
    pub fn decode(&self, frame: &FrFrameDb, payload: &[u8]) -> Vec<(String, String)> {
        let mut out = Vec::new();
        let total_bits = payload.len() * 8;
        for (pdu_name, start_byte) in &frame.pdus {
            let Some(pdu) = self.pdus.iter().find(|p| p.name == *pdu_name) else {
                continue;
            };
            for sig in &pdu.signals {
                let abs_bit = *start_byte as usize * 8 + sig.start_bit as usize;
                let len = sig.length_bits as usize;
                if abs_bit + len > total_bits {
                    continue;
                }
                let raw = crate::decode::extract_raw(
                    payload,
                    abs_bit as u64,
                    len as u64,
                    sig.big_endian,
                );
                let phys = crate::decode::to_physical(
                    raw,
                    len as u64,
                    sig.signed,
                    sig.factor,
                    sig.offset,
                );
                let label = sig
                    .value_descriptions
                    .iter()
                    .find(|(v, _)| *v as f64 == phys)
                    .map(|(_, t)| t.clone());
                let mut text = crate::dbc::fmt_signal_value(phys, &sig.unit, "", label.as_deref());
                text.push_str(&format!("  ({raw:X}h)"));
                out.push((sig.name.clone(), text));
            }
        }
        out
    }
}

// ----------------------------------------------------------------------
// XML helpers (namespace-tolerant)
// ----------------------------------------------------------------------

/// Collects all descendants with the given tag name (namespace ignored).
fn collect<'a, 'b>(node: &roxmltree::Node<'a, 'b>, tag: &str, out: &mut Vec<roxmltree::Node<'a, 'b>>) {
    if node.is_element() && node.tag_name().name() == tag {
        out.push(*node);
    }
    for child in node.children() {
        collect(&child, tag, out);
    }
}

/// The element's SHORT-NAME text.
fn short_name(node: &roxmltree::Node) -> String {
    node.children()
        .find(|n| n.is_element() && n.tag_name().name() == "SHORT-NAME")
        .and_then(|n| n.text())
        .unwrap_or("Unnamed")
        .trim()
        .to_string()
}

/// First non-empty text under any of the candidate tag names, inside the
/// node's subtree.
fn text_of(node: &roxmltree::Node, tags: &[&str]) -> Option<String> {
    for tag in tags {
        let mut found = Vec::new();
        collect(node, tag, &mut found);
        for f in found {
            if let Some(t) = f.text() {
                let t = t.trim();
                if !t.is_empty() {
                    return Some(t.to_string());
                }
            }
        }
    }
    None
}

/// A child element's reference: ID-REF attribute first, element text as
/// the fallback.
fn ref_of(node: &roxmltree::Node, tag: &str) -> Option<String> {
    let child = node
        .children()
        .find(|n| n.is_element() && n.tag_name().name() == tag)?;
    if let Some(id_ref) = child.attribute("ID-REF") {
        return Some(id_ref.to_string());
    }
    child.text().map(|t| t.trim().to_string()).filter(|t| !t.is_empty())
}

fn parse_u32(s: &str) -> Option<u32> {
    let s = s.trim();
    if let Some(hex) = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
        u32::from_str_radix(hex, 16).ok()
    } else {
        s.parse::<u32>().ok()
    }
}

fn parse_f64(s: &str) -> Option<f64> {
    s.trim().parse::<f64>().ok()
}

/// AUTOSAR cycle repetition enum (`CYCLE-REPETITION-4` /
/// `FR_CYCLE_REPETITION_4` / plain `4`).
fn parse_cycle_repetition(s: &str) -> Option<u32> {
    if let Some(v) = parse_u32(s) {
        return Some(v);
    }
    let upper = s.to_uppercase();
    let stripped = upper
        .trim_start_matches("FR_CYCLE_REPETITION")
        .trim_start_matches("CYCLE-REPETITION")
        .trim_start_matches(['-', '_']);
    stripped.parse::<u32>().ok()
}

/// An AUTOSAR reference path's last segment (`/Pkg/Sub/Name` -> `Name`).
fn ref_short_name(s: &str) -> &str {
    s.rsplit('/').next().unwrap_or(s)
}

/// "true"/"false" attribute text, absent = false.
fn truthy(node: &roxmltree::Node, tags: &[&str]) -> bool {
    text_of(node, tags)
        .map(|t| t.eq_ignore_ascii_case("true"))
        .unwrap_or(false)
}

/// Twos-complement / signed convention detection; "UNSIGNED" must not
/// match ("SIGNED" is a substring of it).
fn is_signed_text(t: Option<String>) -> bool {
    t.map(|t| {
        let u = t.to_uppercase();
        u.contains("TWOS") || u.contains("SYMMETRIC") || (u.contains("SIGNED") && !u.starts_with("UN"))
    })
    .unwrap_or(false)
}

// ----------------------------------------------------------------------
// FIBEX 2.x/3.x
// ----------------------------------------------------------------------

/// Reads a child reference ignoring namespace quirks: ID-REF first, then
/// text (shared by the dialect-specific readers below).
fn get_ref(node: &roxmltree::Node, tag: &str) -> Option<String> {
    ref_of(node, tag)
}

/// SIGNAL-INSTANCE list under `node`: (signal ref id, start bit, big-endian).
/// Both the PDU-contained and the signal-directly-on-frame dialects go
/// through here.
fn parse_signal_instances(node: &roxmltree::Node) -> Vec<(String, u32, bool)> {
    let mut instances = Vec::new();
    collect(node, "SIGNAL-INSTANCE", &mut instances);
    let mut out = Vec::new();
    for si in &instances {
        let sig_ref = get_ref(si, "SIGNAL-REF").unwrap_or_default();
        if sig_ref.is_empty() {
            continue;
        }
        // FIBEX standard is BIT-POSITION; START-BIT-POSITION is a tool dialect.
        let start_bit = text_of(si, &["BIT-POSITION", "START-BIT-POSITION", "START-POSITION"])
            .and_then(|t| parse_u32(&t))
            .unwrap_or(0);
        let is_high_low = text_of(si, &["IS-HIGH-LOW-BYTE-ORDER"])
            .map(|t| t.eq_ignore_ascii_case("true"))
            .unwrap_or(true);
        out.push((sig_ref, start_bit, is_high_low));
    }
    out
}

/// FRAME's PDU placements: (PDU ref id, start byte). Two dialects:
/// PDU-MAPPING + START-BIT-POSITION (in bytes) and PDU-INSTANCE +
/// BIT-POSITION (in bits, byte-aligned, divided by 8).
fn parse_pdu_placements(frame: &roxmltree::Node) -> Vec<(String, u32)> {
    let mut out = Vec::new();
    for tag in ["PDU-MAPPING", "PDU-INSTANCE"] {
        let mut nodes = Vec::new();
        collect(frame, tag, &mut nodes);
        for m in &nodes {
            let pdu_ref = get_ref(m, "PDU-REF").unwrap_or_default();
            if pdu_ref.is_empty() {
                continue;
            }
            let start = match text_of(m, &["BIT-POSITION"]).and_then(|t| parse_u32(&t)) {
                Some(bits) => bits / 8,
                None => text_of(m, &["START-BIT-POSITION", "START-POSITION"])
                    .and_then(|t| parse_u32(&t))
                    .unwrap_or(0),
            };
            out.push((pdu_ref, start));
        }
        if !out.is_empty() {
            break; // the two dialects never mix in one file
        }
    }
    out
}

/// Channel attribution: explicit identifier element first, SHORT-NAME
/// containing "B" or the second of two channels as the fallback.
fn classify_channel(channel: &roxmltree::Node, ch_idx: usize, total: usize) -> FrChannel {
    for tag in ["FLEXRAY-CHANNEL-NAME", "CHANNEL-IDENTIFIER"] {
        if let Some(t) = text_of(channel, &[tag]) {
            let t = t.trim().to_uppercase();
            if t.contains('B') {
                return FrChannel::B;
            }
            if t.contains('A') {
                return FrChannel::A;
            }
        }
    }
    let name = short_name(channel).to_uppercase();
    if name.contains('B') || (ch_idx == 1 && total == 2) {
        FrChannel::B
    } else {
        FrChannel::A
    }
}

pub fn parse_fibex_doc(doc: &roxmltree::Document) -> Result<FrDb, String> {
    let root = doc.root_element();

    // 1. Signals and codings: the CODING carries the physical encoding.
    let mut signal_nodes = Vec::new();
    collect(&root, "SIGNAL", &mut signal_nodes);
    let mut coding_nodes = Vec::new();
    collect(&root, "CODING", &mut coding_nodes);

    struct CodingInfo {
        bit_length: u32,
        signed: bool,
        factor: f64,
        offset: f64,
        min: f64,
        max: f64,
        unit: String,
        value_descriptions: Vec<(i64, String)>,
    }

    let mut codings: HashMap<String, CodingInfo> = HashMap::new();
    for coding in &coding_nodes {
        let id = coding.attribute("ID").unwrap_or_default().to_string();
        let bit_length = text_of(coding, &["BIT-LENGTH"])
            .and_then(|t| parse_u32(&t))
            .unwrap_or(1);
        let mut signed = is_signed_text(text_of(coding, &["SIGN-CONVENTION"]));
        if !signed {
            // Vector dialects carry no SIGN-CONVENTION; CODED-TYPE's
            // BASE-DATA-TYPE=A_INT* marks signed instead.
            let mut coded_types = Vec::new();
            collect(coding, "CODED-TYPE", &mut coded_types);
            signed = coded_types.iter().any(|ct| {
                ct.attributes()
                    .any(|a| a.name().contains("BASE-DATA-TYPE") && a.value().contains("A_INT"))
            });
        }
        let mut factor = 1.0f64;
        let mut offset = 0.0f64;
        let mut min = 0.0f64;
        let mut max = 0.0f64;
        let mut unit = String::new();
        let mut value_descriptions = Vec::new();

        // COMPU-RATIONAL-COEFFS: numerator = [offset, factor], denominator = [1].
        // Searching the whole CODING subtree covers SCALE and COMPU-SCALE containers.
        let mut coeffs = Vec::new();
        collect(coding, "COMPU-RATIONAL-COEFFS", &mut coeffs);
        if let Some(coeff) = coeffs.first() {
            let mut numerators = Vec::new();
            collect(coeff, "V", &mut numerators);
            let values: Vec<f64> = numerators
                .iter()
                .filter_map(|n| n.text().and_then(parse_f64))
                .collect();
            if values.len() >= 2 {
                offset = values[0];
                factor = values[1];
            } else if values.len() == 1 {
                factor = values[0];
            }
        }

        // Range: SCALE-CONSTR (Vector) first, plain SCALE as fallback;
        // enumeration items (carrying VT) are skipped.
        'limits: for tag in ["SCALE-CONSTR", "SCALE"] {
            let mut scales = Vec::new();
            collect(coding, tag, &mut scales);
            for scale in &scales {
                if text_of(scale, &["VT"]).is_some() {
                    continue;
                }
                let lo = text_of(scale, &["LOWER-LIMIT"]).and_then(|t| parse_f64(&t));
                let hi = text_of(scale, &["UPPER-LIMIT"]).and_then(|t| parse_f64(&t));
                if lo.is_some() || hi.is_some() {
                    if let Some(lo) = lo {
                        min = lo;
                    }
                    if let Some(hi) = hi {
                        max = hi;
                    }
                    break 'limits;
                }
            }
        }

        // Unit: UNIT-REF into a standalone UNIT element, or an inline
        // UNIT / DISPLAY-NAME.
        let mut unit_refs = Vec::new();
        collect(coding, "UNIT-REF", &mut unit_refs);
        if let Some(unit_ref) = unit_refs.first() {
            let unit_id = unit_ref.attribute("ID-REF").unwrap_or_default();
            if !unit_id.is_empty() {
                let mut unit_nodes = Vec::new();
                collect(&root, "UNIT", &mut unit_nodes);
                if let Some(u) = unit_nodes.iter().find(|n| n.attribute("ID") == Some(unit_id))
                    && let Some(dn) = text_of(u, &["DISPLAY-NAME"])
                {
                    unit = dn;
                }
            }
        } else if let Some(dn) = text_of(coding, &["DISPLAY-NAME"]) {
            unit = dn;
        }

        // Enumeration labels: scales carrying a VT.
        let mut seen_vt: std::collections::HashSet<(i64, String)> = std::collections::HashSet::new();
        for tag in ["COMPU-SCALE", "SCALE-CONSTR", "SCALE"] {
            let mut scales = Vec::new();
            collect(coding, tag, &mut scales);
            for cs in &scales {
                let Some(vt) = text_of(cs, &["VT"]) else {
                    continue;
                };
                let Some(lo) = text_of(cs, &["LOWER-LIMIT"]).and_then(|t| parse_u32(&t)) else {
                    continue;
                };
                if seen_vt.insert((lo as i64, vt.clone())) {
                    value_descriptions.push((lo as i64, vt));
                }
            }
        }

        codings.insert(
            id,
            CodingInfo {
                bit_length,
                signed,
                factor,
                offset,
                min,
                max,
                unit,
                value_descriptions,
            },
        );
    }

    // Signal ID -> (name, CODING-REF, comment).
    let mut signal_defs: HashMap<String, (String, String, String)> = HashMap::new();
    for sig in &signal_nodes {
        let Some(id) = sig.attribute("ID") else {
            continue;
        };
        let name = short_name(sig);
        let coding_ref = get_ref(sig, "CODING-REF").unwrap_or_default();
        let comment = text_of(sig, &["L-2", "DESC"]).unwrap_or_default();
        signal_defs.insert(id.to_string(), (name, coding_ref, comment));
    }

    // 2. PDUs.
    let mut pdu_nodes = Vec::new();
    collect(&root, "PDU", &mut pdu_nodes);

    struct PduRaw {
        name: String,
        length: u32,
        dynamic: bool,
        comment: String,
        instances: Vec<(String, u32, bool)>,
    }

    let mut pdu_raws: Vec<(String, PduRaw)> = Vec::new();
    for pdu in &pdu_nodes {
        let Some(id) = pdu.attribute("ID") else {
            continue;
        };
        let name = short_name(pdu);
        let length = text_of(pdu, &["PDU-LENGTH", "BYTE-LENGTH"])
            .and_then(|t| parse_u32(&t))
            .unwrap_or(4);
        let dynamic = matches!(
            text_of(pdu, &["PDU-TYPE"])
                .unwrap_or_default()
                .to_uppercase()
                .as_str(),
            "DYNAMIC-PDU" | "EVENT-PDU"
        );
        let comment = text_of(pdu, &["DESC", "DESCRIPTION"]).unwrap_or_default();
        pdu_raws.push((
            id.to_string(),
            PduRaw {
                name,
                length,
                dynamic,
                comment,
                instances: parse_signal_instances(pdu),
            },
        ));
    }

    // 3. Frames.
    let mut frame_nodes = Vec::new();
    collect(&root, "FRAME", &mut frame_nodes);

    struct FrameRaw {
        name: String,
        length: u32,
        payload_preamble: bool,
        comment: String,
        pdu_mappings: Vec<(String, u32)>,
        instances: Vec<(String, u32, bool)>,
    }

    let mut frame_raws: Vec<(String, FrameRaw)> = Vec::new();
    for frame in &frame_nodes {
        let Some(id) = frame.attribute("ID") else {
            continue;
        };
        let name = short_name(frame);
        let length = text_of(frame, &["FRAME-LENGTH", "BYTE-LENGTH"])
            .and_then(|t| parse_u32(&t))
            .unwrap_or(8);
        let payload_preamble = truthy(frame, &["PAYLOAD-PREAMBLE"]);
        let comment = text_of(frame, &["DESC", "DESCRIPTION"]).unwrap_or_default();
        let pdu_mappings = parse_pdu_placements(frame);
        let instances = parse_signal_instances(frame);
        frame_raws.push((
            id.to_string(),
            FrameRaw {
                name,
                length,
                payload_preamble,
                comment,
                pdu_mappings,
                instances,
            },
        ));
    }

    // 4. Frame triggerings per channel. Two hierarchies:
    //    CHANNEL/SLOT/FRAME-TRIGGERING (SLOT-ID on the SLOT wrapper) and
    //    CHANNEL/FRAME-TRIGGERING (Vector: SLOT-ID inside the timing).
    //    A frame appearing on both channels -> Both.
    let mut triggerings: HashMap<String, FrTriggering> = HashMap::new();

    let mut channel_nodes = Vec::new();
    collect(&root, "CHANNEL", &mut channel_nodes);

    for (ch_idx, channel) in channel_nodes.iter().enumerate() {
        let ch = classify_channel(channel, ch_idx, channel_nodes.len());
        let mut ft_nodes = Vec::new();
        collect(channel, "FRAME-TRIGGERING", &mut ft_nodes);
        for ft in &ft_nodes {
            let frame_ref = get_ref(ft, "FRAME-REF").unwrap_or_default();
            if frame_ref.is_empty() {
                continue;
            }
            let slot_id = text_of(ft, &["SLOT-ID"])
                .and_then(|t| parse_u32(&t))
                .or_else(|| {
                    ft.ancestors()
                        .find(|n| n.is_element() && n.tag_name().name() == "SLOT")
                        .and_then(|slot| text_of(&slot, &["SLOT-ID"]).and_then(|t| parse_u32(&t)))
                })
                .unwrap_or(0);
            let rep = text_of(ft, &["CYCLE-REPETITION"])
                .and_then(|t| parse_cycle_repetition(&t))
                .unwrap_or(1);
            let base = text_of(ft, &["BASE-CYCLE"])
                .and_then(|t| parse_u32(&t))
                .unwrap_or(0);
            let startup = truthy(ft, &["STARTUP-FRAME", "IS-STARTUP-FRAME"]);

            let trig = triggerings.entry(frame_ref).or_insert(FrTriggering {
                channel: ch,
                slot_id,
                base_cycle: base,
                cycle_repetition: rep.max(1),
                startup,
            });
            if trig.channel != ch {
                trig.channel = FrChannel::Both;
            }
            trig.slot_id = slot_id;
            trig.base_cycle = base;
            trig.cycle_repetition = rep.max(1);
            trig.startup = startup;
        }
    }

    // 5. ECUs: fx:ECU first, top-level CONTROLLER as the fallback.
    let mut ecu_nodes = Vec::new();
    collect(&root, "ECU", &mut ecu_nodes);
    if ecu_nodes.is_empty() {
        collect(&root, "CONTROLLER", &mut ecu_nodes);
    }
    let mut ecus: Vec<String> = ecu_nodes.iter().map(short_name).collect();
    ecus.retain(|n| !n.is_empty() && n != "Unnamed");

    // 6. Cluster parameters.
    let mut cluster_nodes = Vec::new();
    collect(&root, "CLUSTER", &mut cluster_nodes);
    let mut params = default_cluster_params();
    if let Some(cluster_node) = cluster_nodes.first() {
        params.name = short_name(cluster_node);
        let get_u32 = |tags: &[&str]| -> Option<u32> {
            text_of(cluster_node, tags).and_then(|t| parse_u32(&t))
        };
        let get_f64 = |tags: &[&str]| -> Option<f64> {
            text_of(cluster_node, tags).and_then(|t| parse_f64(&t))
        };
        if let Some(v) = get_u32(&["SPEED"]) {
            // Units vary: Vector writes bit/s (10000000), others kbit/s.
            params.speed_kbps = if v >= 100_000 { v / 1000 } else { v };
        }
        if let Some(v) = get_f64(&["CYCLE-TIME-ms", "CYCLE-TIME-MS", "CYCLE-TIME"]) {
            params.cycle_time_ms = v;
        } else if let Some(v) = get_f64(&["CYCLE"]) {
            // flexray:CYCLE is in µs (5000 = 5 ms).
            params.cycle_time_ms = v / 1000.0;
        }
        if let Some(v) = get_f64(&["MACROTICK", "MACROTICK-DURATION", "GD-MACROTICK-DURATION"]) {
            // MACROTICK-DURATION is usually ms (0.005); flexray:MACROTICK is µs.
            params.macrotick_duration_us = if v < 0.1 { v * 1000.0 } else { v };
        }
        if let Some(v) = get_u32(&[
            "COLD-START-ATTEMPTS",
            "COLDSTART-ATTEMPTS",
            "GD-COLDSTART-ATTEMPTS",
            "G-COLDSTART-ATTEMPTS",
        ]) {
            params.coldstart_attempts = v;
        }
        if let Some(v) = get_u32(&["ACTION-POINT-OFFSET", "GD-ACTION-POINT-OFFSET"]) {
            params.action_point_offset = v;
        }
        if let Some(v) = get_u32(&[
            "MINISLOT-ACTION-POINT-OFFSET",
            "GD-MINISLOT-ACTION-POINT-OFFSET",
        ]) {
            params.minislot_action_point_offset = v;
        }
        if let Some(v) = get_u32(&["DYNAMIC-SLOT-IDLE-PHASE", "GD-DYNAMIC-SLOT-IDLE-PHASE"]) {
            params.dynamic_slot_idle_phase = v;
        }
        if let Some(v) = get_u32(&["MINOR-VERSION", "GD-MINOR-VERSION"]) {
            params.minor_version = v;
        }
        if let Some(v) = get_u32(&["NETWORK-IDLE-TIME", "N-I-T", "NIT", "GD-NIT"]) {
            params.network_idle_time = v;
        }
        if let Some(v) = get_u32(&["NUMBER-OF-MINISLOTS", "G-NUMBER-OF-MINISLOTS"]) {
            params.number_of_minislots = v;
        }
        if let Some(v) = get_u32(&["NUMBER-OF-STATIC-SLOTS", "G-NUMBER-OF-STATIC-SLOTS"]) {
            params.number_of_static_slots = v;
        }
        if let Some(v) = get_u32(&["MINISLOT-DURATION", "GD-MINISLOT", "MINISLOT"]) {
            params.minislot_duration = v;
        }
        if let Some(v) = get_u32(&["STATIC-SLOT-DURATION", "GD-STATIC-SLOT", "STATIC-SLOT"]) {
            params.static_slot_duration = v;
        }
        if let Some(v) = get_u32(&["SYMBOL-WINDOW", "GD-SYMBOL-WINDOW"]) {
            params.symbol_window = v;
        }
        if let Some(v) = get_u32(&["SYMBOL-WINDOW-IDLE-PHASE", "GD-SYMBOL-WINDOW-IDLE-PHASE"]) {
            params.symbol_window_idle_phase = v;
        }
        if let Some(v) = get_u32(&["OFFSET-CORRECTION-START", "G-OFFSET-CORRECTION-START"]) {
            params.offset_correction_start = v;
        }
        fill_extra_params(&mut params, &get_u32, &get_f64);
    }

    // 7. Assemble PDUs and frames, resolving references. The
    // signal-directly-on-frame dialect (no PDU layer) synthesises a
    // same-named PDU per frame.
    for (id, raw) in frame_raws.iter_mut() {
        if !raw.pdu_mappings.is_empty() || raw.instances.is_empty() {
            continue;
        }
        let synth_id = format!("__synth_{id}");
        pdu_raws.push((
            synth_id.clone(),
            PduRaw {
                name: raw.name.clone(),
                length: raw.length,
                dynamic: false,
                comment: raw.comment.clone(),
                instances: raw.instances.clone(),
            },
        ));
        raw.pdu_mappings.push((synth_id, 0));
    }

    let build_signals = |instances: &[(String, u32, bool)]| -> Vec<FrSignal> {
        let mut signals = Vec::new();
        for (sig_ref, start_bit, is_high_low) in instances {
            let Some((sig_name, coding_ref, sig_comment)) = signal_defs.get(sig_ref) else {
                continue;
            };
            let coding = codings.get(coding_ref);
            signals.push(FrSignal {
                name: sig_name.clone(),
                start_bit: *start_bit,
                length_bits: coding.map(|c| c.bit_length).unwrap_or(1),
                big_endian: *is_high_low,
                signed: coding.map(|c| c.signed).unwrap_or(false),
                factor: coding.map(|c| c.factor).unwrap_or(1.0),
                offset: coding.map(|c| c.offset).unwrap_or(0.0),
                min: coding.map(|c| c.min).unwrap_or(0.0),
                max: coding.map(|c| c.max).unwrap_or(0.0),
                unit: coding.map(|c| c.unit.clone()).unwrap_or_default(),
                comment: sig_comment.clone(),
                value_descriptions: coding
                    .map(|c| c.value_descriptions.clone())
                    .unwrap_or_default(),
            });
        }
        signals
    };

    let mut pdus: Vec<FrPdu> = Vec::new();
    for (_, raw) in &pdu_raws {
        pdus.push(FrPdu {
            name: raw.name.clone(),
            length: raw.length,
            dynamic: raw.dynamic,
            comment: raw.comment.clone(),
            signals: build_signals(&raw.instances),
        });
    }

    let mut frames: Vec<FrFrameDb> = Vec::new();
    for (frame_id, raw) in &frame_raws {
        let mut pdus_in_frame = Vec::new();
        for (pdu_ref, start) in &raw.pdu_mappings {
            let Some((_, pdu_raw)) = pdu_raws.iter().find(|(id, _)| id == pdu_ref) else {
                continue;
            };
            pdus_in_frame.push((pdu_raw.name.clone(), *start));
        }
        frames.push(FrFrameDb {
            name: raw.name.clone(),
            length: raw.length,
            payload_preamble: raw.payload_preamble,
            triggering: triggerings.get(frame_id).copied().unwrap_or_default(),
            pdus: pdus_in_frame,
            comment: raw.comment.clone(),
        });
    }

    Ok(FrDb {
        params,
        ecus,
        pdus,
        frames,
    })
}

/// The extra vxlapi-oriented cluster fields, shared by both parsers.
/// All of them default to zero, so an absent tag just means zero.
fn fill_extra_params(
    p: &mut FrClusterParams,
    get_u32: &impl Fn(&[&str]) -> Option<u32>,
    get_f64: &impl Fn(&[&str]) -> Option<f64>,
) {
    p.payload_length_static = get_u32(&[
        "PAYLOAD-LENGTH-STATIC",
        "G-PAYLOAD-LENGTH-STATIC",
        "FLEXRAY-G-PAYLOAD-LENGTH-STATIC",
    ])
    .unwrap_or(p.payload_length_static);
    if p.g_macro_per_cycle.is_none() {
        p.g_macro_per_cycle = get_u32(&["MACRO-PER-CYCLE", "G-MACRO-PER-CYCLE"]);
    }
    p.g_micro_per_cycle = get_u32(&["MICRO-PER-CYCLE", "G-MICRO-PER-CYCLE"]).unwrap_or(0);
    p.p_samples_per_microtick =
        get_u32(&["SAMPLES-PER-MICROTICK", "P-SAMPLES-PER-MICROTICK"]).unwrap_or(0);
    p.sample_clock_period_us =
        get_f64(&["SAMPLE-CLOCK-PERIOD", "P-SAMPLE-CLOCK-PERIOD"]).unwrap_or(0.0);
    p.gd_cas_rx_low_max = get_u32(&["CAS-RX-LOW-MAX", "GD-CAS-RX-LOW-MAX"]).unwrap_or(0);
    p.g_sync_node_max = get_u32(&["SYNC-NODE-MAX", "G-SYNC-NODE-MAX"]).unwrap_or(0);
    p.g_listen_noise = get_u32(&["LISTEN-NOISE", "G-LISTEN-NOISE"]).unwrap_or(0);
    p.g_network_management_vector_length = get_u32(&[
        "NETWORK-MANAGEMENT-VECTOR-LENGTH",
        "G-NETWORK-MANAGEMENT-VECTOR-LENGTH",
    ])
    .unwrap_or(0);
    p.g_max_without_clock_correction_fatal = get_u32(&[
        "MAX-WITHOUT-CLOCK-CORRECTION-FATAL",
        "G-MAX-WITHOUT-CLOCK-CORRECTION-FATAL",
    ])
    .unwrap_or(0);
    p.g_max_without_clock_correction_passive = get_u32(&[
        "MAX-WITHOUT-CLOCK-CORRECTION-PASSIVE",
        "G-MAX-WITHOUT-CLOCK-CORRECTION-PASSIVE",
    ])
    .unwrap_or(0);
    p.gd_tss_transmitter = get_u32(&["T-S-S-TRANSMITTER", "GD-TSS-TRANSMITTER"]).unwrap_or(0);
    p.p_cluster_drift_damping =
        get_u32(&["CLUSTER-DRIFT-DAMPING", "P-CLUSTER-DRIFT-DAMPING"]).unwrap_or(0);
    p.p_decoding_correction =
        get_u32(&["DECODING-CORRECTION", "P-DECODING-CORRECTION"]).unwrap_or(0);
    p.p_delay_compensation_a =
        get_u32(&["DELAY-COMPENSATION-A", "P-DELAY-COMPENSATION-A"]).unwrap_or(0);
    p.p_delay_compensation_b =
        get_u32(&["DELAY-COMPENSATION-B", "P-DELAY-COMPENSATION-B"]).unwrap_or(0);
    p.p_extern_offset_correction =
        get_u32(&["EXTERN-OFFSET-CORRECTION", "P-EXTERN-OFFSET-CORRECTION"]).unwrap_or(0);
    p.p_extern_rate_correction =
        get_u32(&["EXTERN-RATE-CORRECTION", "P-EXTERN-RATE-CORRECTION"]).unwrap_or(0);
    p.p_latest_tx = get_u32(&["LATEST-TX", "P-LATEST-TX"]).unwrap_or(0);
    p.p_macro_initial_offset_a =
        get_u32(&["MACRO-INITIAL-OFFSET-A", "P-MACRO-INITIAL-OFFSET-A"]).unwrap_or(0);
    p.p_macro_initial_offset_b =
        get_u32(&["MACRO-INITIAL-OFFSET-B", "P-MACRO-INITIAL-OFFSET-B"]).unwrap_or(0);
    p.p_micro_initial_offset_a =
        get_u32(&["MICRO-INITIAL-OFFSET-A", "P-MICRO-INITIAL-OFFSET-A"]).unwrap_or(0);
    p.p_micro_initial_offset_b =
        get_u32(&["MICRO-INITIAL-OFFSET-B", "P-MICRO-INITIAL-OFFSET-B"]).unwrap_or(0);
    p.p_offset_correction_out =
        get_u32(&["OFFSET-CORRECTION-OUT", "P-OFFSET-CORRECTION-OUT"]).unwrap_or(0);
    p.p_rate_correction_out =
        get_u32(&["RATE-CORRECTION-OUT", "P-RATE-CORRECTION-OUT"]).unwrap_or(0);
    p.p_allow_halt_due_to_clock = get_u32(&[
        "ALLOW-HALT-DUE-TO-CLOCK",
        "P-ALLOW-HALT-DUE-TO-CLOCK",
    ]) == Some(1);
    p.p_allow_passive_to_active =
        get_u32(&["ALLOW-PASSIVE-TO-ACTIVE", "P-ALLOW-PASSIVE-TO-ACTIVE"]).unwrap_or(0);
    p.p_key_slot_used_for_startup =
        get_u32(&["KEY-SLOT-USED-FOR-STARTUP", "P-KEY-SLOT-USED-FOR-STARTUP"]).unwrap_or(0);
    p.p_key_slot_used_for_sync =
        get_u32(&["KEY-SLOT-USED-FOR-SYNC", "P-KEY-SLOT-USED-FOR-SYNC"]).unwrap_or(0);
    p.p_single_slot_enabled =
        get_u32(&["SINGLE-SLOT-ENABLED", "P-SINGLE-SLOT-ENABLED"]).unwrap_or(0);
    p.p_wakeup_channel = get_u32(&["WAKEUP-CHANNEL", "P-WAKEUP-CHANNEL"]).unwrap_or(0);
    p.p_wakeup_pattern = get_u32(&["WAKEUP-PATTERN", "P-WAKEUP-PATTERN"]).unwrap_or(0);
    p.pd_accepted_startup_range =
        get_u32(&["ACCEPTED-STARTUP-RANGE", "PD-ACCEPTED-STARTUP-RANGE"]).unwrap_or(0);
    p.pd_listen_timeout = get_u32(&["LISTEN-TIMEOUT", "PD-LISTEN-TIMEOUT"]).unwrap_or(0);
    p.pd_max_drift = get_u32(&["MAX-DRIFT", "PD-MAX-DRIFT"]).unwrap_or(0);
}

fn default_cluster_params() -> FrClusterParams {
    FrClusterParams {
        speed_kbps: 10000,
        cycle_time_ms: 5.0,
        macrotick_duration_us: 5.0,
        coldstart_attempts: 8,
        action_point_offset: 3,
        minislot_action_point_offset: 2,
        dynamic_slot_idle_phase: 1,
        minor_version: 3,
        network_idle_time: 75,
        number_of_minislots: 292,
        number_of_static_slots: 10,
        minislot_duration: 9,
        static_slot_duration: 67,
        symbol_window: 1,
        symbol_window_idle_phase: 1,
        offset_correction_start: 233,
        ..Default::default()
    }
}

// ----------------------------------------------------------------------
// AUTOSAR ARXML (R4.x)
// ----------------------------------------------------------------------

pub fn parse_arxml_doc(doc: &roxmltree::Document) -> Result<FrDb, String> {
    let root = doc.root_element();

    // 1. I-SIGNAL / SYSTEM-SIGNAL definitions.
    let mut signal_nodes = Vec::new();
    for tag in ["I-SIGNAL", "SYSTEM-SIGNAL"] {
        collect(&root, tag, &mut signal_nodes);
    }

    struct SignalDef {
        length_bits: u32,
        signed: bool,
        comment: String,
    }

    let mut signal_defs: HashMap<String, SignalDef> = HashMap::new();
    for sig in &signal_nodes {
        let name = short_name(sig);
        let length_bits = text_of(sig, &["LENGTH"])
            .and_then(|t| parse_u32(&t))
            .unwrap_or(1);
        let signed = is_signed_text(text_of(
            sig,
            &["SIGN-CONVENTION", "I-SIGNAL-TYPE", "NETWORK-REPRESENTATION-PROPS"],
        ));
        let comment = text_of(sig, &["L-2", "DESC"]).unwrap_or_default();
        signal_defs.insert(name, SignalDef { length_bits, signed, comment });
    }

    // 2. PDUs: I-SIGNAL-I-PDU / FLEXRAY-I-PDU.
    let mut pdu_nodes = Vec::new();
    for tag in ["I-SIGNAL-I-PDU", "FLEXRAY-I-PDU"] {
        collect(&root, tag, &mut pdu_nodes);
    }

    struct PduMappingRaw {
        signal_name: Option<String>,
        start_bit: u32,
        length_bits: u32,
        big_endian: bool,
    }

    struct PduRaw {
        name: String,
        length: u32,
        dynamic: bool,
        comment: String,
        mappings: Vec<PduMappingRaw>,
    }

    let mut pdu_raws: Vec<(String, PduRaw)> = Vec::new();
    for pdu in &pdu_nodes {
        let name = short_name(pdu);
        let length = text_of(pdu, &["LENGTH"])
            .and_then(|t| parse_u32(&t))
            .unwrap_or(4);
        let comment = text_of(pdu, &["L-2", "DESC"]).unwrap_or_default();
        let dynamic = {
            let t = text_of(pdu, &["PDU-TYPE", "I-PDU-TYPE"])
                .unwrap_or_default()
                .to_uppercase();
            t.contains("DYNAMIC") || t.contains("EVENT") || t.contains("GENERAL-PURPOSE")
        };

        let mut mappings = Vec::new();
        let mut mapping_nodes = Vec::new();
        collect(pdu, "I-SIGNAL-TO-PDU-MAPPING", &mut mapping_nodes);
        for m in &mapping_nodes {
            let signal_name = text_of(m, &["I-SIGNAL-REF", "SYSTEM-SIGNAL-REF"])
                .map(|r| ref_short_name(&r).to_string());
            let start_bit = text_of(m, &["START-POSITION", "START-BIT-POSITION"])
                .and_then(|t| parse_u32(&t))
                .unwrap_or(0);
            let length_bits = text_of(m, &["LENGTH"])
                .and_then(|t| parse_u32(&t))
                .unwrap_or(1);
            let big_endian = text_of(m, &["PACKING-BYTE-ORDER"])
                .unwrap_or_default()
                .to_uppercase()
                .split(['-', '_'])
                .any(|seg| seg == "MOST" || seg == "BIG");
            mappings.push(PduMappingRaw {
                signal_name,
                start_bit,
                length_bits,
                big_endian,
            });
        }

        pdu_raws.push((
            name.clone(),
            PduRaw {
                name,
                length,
                dynamic,
                comment,
                mappings,
            },
        ));
    }

    // 3. Frames.
    let mut frame_nodes = Vec::new();
    collect(&root, "FLEXRAY-FRAME", &mut frame_nodes);

    struct FrameRaw {
        name: String,
        length: u32,
        payload_preamble: bool,
        comment: String,
        pdu_mappings: Vec<(String, u32)>,
    }

    let mut frames_by_name: HashMap<String, FrameRaw> = HashMap::new();
    for frame in &frame_nodes {
        let name = short_name(frame);
        let length = text_of(frame, &["FRAME-LENGTH"])
            .and_then(|t| parse_u32(&t))
            .unwrap_or(8);
        let payload_preamble = truthy(frame, &["PAYLOAD-PREAMBLE", "PADDING-ACTIVATION"]);
        let comment = text_of(frame, &["L-2", "DESC"]).unwrap_or_default();

        let mut pdu_mappings = Vec::new();
        let mut mapping_nodes = Vec::new();
        for tag in ["FRAME-PDU-MAPPING", "PDU-TO-FRAME-MAPPING"] {
            collect(frame, tag, &mut mapping_nodes);
        }
        for m in &mapping_nodes {
            let Some(pdu_ref) = text_of(m, &["PDU-REF"]) else {
                continue;
            };
            let start = text_of(m, &["START-POSITION", "START-BIT-POSITION"])
                .and_then(|t| parse_u32(&t))
                .unwrap_or(0);
            pdu_mappings.push((ref_short_name(&pdu_ref).to_string(), start));
        }

        frames_by_name.insert(
            name.clone(),
            FrameRaw {
                name,
                length,
                payload_preamble,
                comment,
                pdu_mappings,
            },
        );
    }

    // 4. Frame triggerings per physical channel.
    let mut channel_nodes = Vec::new();
    collect(&root, "FLEXRAY-PHYSICAL-CHANNEL", &mut channel_nodes);

    let mut triggerings: HashMap<String, FrTriggering> = HashMap::new();
    for (ch_idx, channel) in channel_nodes.iter().enumerate() {
        let ch_name = short_name(channel).to_uppercase();
        let ch = if ch_name.contains('B') || (ch_idx == 1 && channel_nodes.len() == 2) {
            FrChannel::B
        } else {
            FrChannel::A
        };

        let mut trigger_nodes = Vec::new();
        collect(channel, "FLEXRAY-FRAME-TRIGGERING", &mut trigger_nodes);
        for ft in &trigger_nodes {
            let Some(frame_ref) = text_of(ft, &["FRAME-REF"]) else {
                continue;
            };
            let frame_name = ref_short_name(&frame_ref).to_string();
            let slot_id = text_of(ft, &["SLOT-ID"])
                .and_then(|t| parse_u32(&t))
                .unwrap_or(1);
            let base_cycle = text_of(ft, &["BASE-CYCLE"])
                .and_then(|t| parse_u32(&t))
                .unwrap_or(0);
            let cycle_repetition = text_of(ft, &["CYCLE-REPETITION"])
                .and_then(|t| parse_cycle_repetition(&t))
                .unwrap_or(1);
            let startup = truthy(ft, &["STARTUP-FRAME"]);

            let trig = triggerings.entry(frame_name).or_insert(FrTriggering {
                channel: ch,
                slot_id,
                base_cycle,
                cycle_repetition: cycle_repetition.max(1),
                startup,
            });
            if trig.channel != ch {
                trig.channel = FrChannel::Both;
            }
            trig.slot_id = slot_id;
            trig.base_cycle = base_cycle;
            trig.cycle_repetition = cycle_repetition.max(1);
            trig.startup = startup;
        }
    }

    // 5. Cluster parameters.
    let mut cluster_nodes = Vec::new();
    collect(&root, "FLEXRAY-CLUSTER", &mut cluster_nodes);
    let mut params = default_cluster_params();
    if let Some(cluster_node) = cluster_nodes.first() {
        params.name = short_name(cluster_node);
        let get_u32 = |tags: &[&str]| -> Option<u32> {
            text_of(cluster_node, tags).and_then(|t| parse_u32(&t))
        };
        let get_f64 = |tags: &[&str]| -> Option<f64> {
            text_of(cluster_node, tags).and_then(|t| parse_f64(&t))
        };

        if let Some(v) = get_u32(&["FLEXRAY-COLDSTART-ATTEMPTS", "GD-COLDSTART-ATTEMPTS"]) {
            params.coldstart_attempts = v;
        }
        let macrotick = get_f64(&["FLEXRAY-GD-MACROTICK-DURATION", "GD-MACROTICK-DURATION"]);
        let cycle_mt = get_u32(&["FLEXRAY-CYCLE", "GD-CYCLE"]);
        if let Some(mt) = macrotick {
            params.macrotick_duration_us = mt;
        }
        if let Some(mt) = cycle_mt {
            params.cycle_time_ms = mt as f64 * params.macrotick_duration_us / 1000.0;
            params.g_macro_per_cycle = Some(mt);
        } else if let Some(ms) = get_f64(&["FLEXRAY-CYCLE-TIME-MS"]) {
            params.cycle_time_ms = ms;
        }
        if let Some(v) = get_u32(&["FLEXRAY-GD-ACTION-POINT-OFFSET"]) {
            params.action_point_offset = v;
        }
        if let Some(v) = get_u32(&["FLEXRAY-GD-MINISLOT-ACTION-POINT-OFFSET"]) {
            params.minislot_action_point_offset = v;
        }
        if let Some(v) = get_u32(&["FLEXRAY-GD-DYNAMIC-SLOT-IDLE-PHASE"]) {
            params.dynamic_slot_idle_phase = v;
        }
        if let Some(v) = get_u32(&["FLEXRAY-GD-MINOR-VERSION"]) {
            params.minor_version = v;
        }
        if let Some(v) = get_u32(&["FLEXRAY-GD-NIT", "FLEXRAY-NETWORK-IDLE-TIME"]) {
            params.network_idle_time = v;
        }
        if let Some(v) = get_u32(&["FLEXRAY-G-NUMBER-OF-MINISLOTS", "G-NUMBER-OF-MINISLOTS"]) {
            params.number_of_minislots = v;
        }
        if let Some(v) = get_u32(&["FLEXRAY-G-NUMBER-OF-STATIC-SLOTS", "G-NUMBER-OF-STATIC-SLOTS"])
        {
            params.number_of_static_slots = v;
        }
        if let Some(v) = get_u32(&["FLEXRAY-GD-MINISLOT"]) {
            params.minislot_duration = v;
        }
        if let Some(v) = get_u32(&["FLEXRAY-GD-STATIC-SLOT"]) {
            params.static_slot_duration = v;
        }
        if let Some(v) = get_u32(&["FLEXRAY-GD-SYMBOL-WINDOW"]) {
            params.symbol_window = v;
        }
        if let Some(v) = get_u32(&["FLEXRAY-GD-SYMBOL-WINDOW-IDLE-PHASE"]) {
            params.symbol_window_idle_phase = v;
        }
        if let Some(v) = get_u32(&["FLEXRAY-OFFSET-CORRECTION-START", "G-OFFSET-CORRECTION-START"])
        {
            params.offset_correction_start = v;
        }
        fill_extra_params(&mut params, &get_u32, &get_f64);
    }

    // 6. ECUs.
    let mut ecu_nodes = Vec::new();
    collect(&root, "ECU-INSTANCE", &mut ecu_nodes);
    let mut ecus: Vec<String> = ecu_nodes.iter().map(short_name).collect();
    ecus.retain(|n| !n.is_empty() && n != "Unnamed");

    // 7. Assemble.
    let mut pdus: Vec<FrPdu> = Vec::new();
    for (_, raw) in &pdu_raws {
        let mut signals = Vec::new();
        for m in &raw.mappings {
            let Some(signal_name) = &m.signal_name else {
                continue;
            };
            let def = signal_defs.get(signal_name);
            signals.push(FrSignal {
                name: signal_name.clone(),
                start_bit: m.start_bit,
                length_bits: def.map(|d| d.length_bits).unwrap_or(m.length_bits.max(1)),
                big_endian: m.big_endian,
                signed: def.map(|d| d.signed).unwrap_or(false),
                factor: 1.0,
                offset: 0.0,
                min: 0.0,
                max: 0.0,
                unit: String::new(),
                comment: def.map(|d| d.comment.clone()).unwrap_or_default(),
                value_descriptions: Vec::new(),
            });
        }
        pdus.push(FrPdu {
            name: raw.name.clone(),
            length: raw.length,
            dynamic: raw.dynamic,
            comment: raw.comment.clone(),
            signals,
        });
    }

    let mut frames: Vec<FrFrameDb> = Vec::new();
    for (name, raw) in &frames_by_name {
        frames.push(FrFrameDb {
            name: raw.name.clone(),
            length: raw.length,
            payload_preamble: raw.payload_preamble,
            triggering: triggerings.get(name).copied().unwrap_or_default(),
            pdus: raw.pdu_mappings.clone(),
            comment: raw.comment.clone(),
        });
    }

    let mut db = FrDb {
        params,
        ecus,
        pdus,
        frames,
    };
    dedup_frame_names(&mut db);
    Ok(db)
}

/// Keeps the first frame of each name; duplicates only confuse the
/// display-side lookups.
fn dedup_frame_names(db: &mut FrDb) {
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    db.frames.retain(|f| seen.insert(f.name.clone()));
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_FIBEX: &str = include_str!("../assets/powertrain.fibex");
    const SAMPLE_ARXML: &str = include_str!("../assets/chassis.arxml");
    const CANOE_DEMO: &str = include_str!("../assets/DemoFile_v3_FIBEX_3_0.xml");

    #[test]
    fn parses_the_asam_style_fibex_sample() {
        let db = FrDb::parse(SAMPLE_FIBEX).expect("sample FIBEX should parse");

        assert_eq!(db.params.name, "PowertrainCluster");
        assert_eq!(db.params.speed_kbps, 10000);
        assert_eq!(db.params.cycle_time_ms, 5.0);
        assert_eq!(db.params.number_of_static_slots, 10);
        assert_eq!(db.params.macrotick_duration_us, 5.0);

        assert_eq!(db.ecus.len(), 4);
        assert!(db.ecus.contains(&"EngECU".to_string()));

        assert_eq!(db.frames.len(), 4);
        assert_eq!(db.pdus.len(), 4);

        // EngineData: slots on A+B, repetition 1.
        let engine = db.frames.iter().find(|f| f.name == "EngineData").unwrap();
        assert_eq!(engine.triggering.slot_id, 1);
        assert_eq!(engine.triggering.channel, FrChannel::Both);
        assert_eq!(engine.triggering.cycle_repetition, 1);
        assert_eq!(engine.pdus.len(), 1);

        // TransmissionData only A; ChassisStatus only B with rep 4.
        let trans = db.frames.iter().find(|f| f.name == "TransmissionData").unwrap();
        assert_eq!(trans.triggering.channel, FrChannel::A);
        assert_eq!(trans.triggering.slot_id, 2);
        let chassis = db.frames.iter().find(|f| f.name == "ChassisStatus").unwrap();
        assert_eq!(chassis.triggering.channel, FrChannel::B);
        assert_eq!(chassis.triggering.slot_id, 3);
        assert_eq!(chassis.triggering.cycle_repetition, 4);

        // Signal encoding from the CODING: factor / offset / unit / labels.
        let eng_pdu = db.pdus.iter().find(|p| p.name == "Eng_PDU").unwrap();
        let speed = eng_pdu.signals.iter().find(|s| s.name == "EngineSpeed").unwrap();
        assert_eq!(speed.length_bits, 16);
        assert!(speed.big_endian);
        assert_eq!(speed.factor, 0.25);
        assert_eq!(speed.unit, "rpm");

        let temp = eng_pdu.signals.iter().find(|s| s.name == "CoolantTemp").unwrap();
        assert_eq!(temp.offset, -40.0);
        assert_eq!(temp.min, -40.0);
        assert_eq!(temp.max, 215.0);
        assert_eq!(temp.unit, "DegC");

        let chassis_pdu = db.pdus.iter().find(|p| p.name == "Chassis_PDU").unwrap();
        let status = chassis_pdu
            .signals
            .iter()
            .find(|s| s.name == "ChassisStatus")
            .unwrap();
        assert_eq!(
            status.value_descriptions,
            &[(0, "OK".to_string()), (1, "Warning".to_string()), (2, "Error".to_string())]
        );
    }

    #[test]
    fn parses_the_autosar_arxml_sample() {
        let db = FrDb::parse(SAMPLE_ARXML).expect("sample ARXML should parse");

        assert_eq!(db.params.name, "PowertrainCluster");
        // FLEXRAY-CYCLE 1000 mt × 5 µs = 5 ms.
        assert_eq!(db.params.cycle_time_ms, 5.0);
        assert_eq!(db.params.macrotick_duration_us, 5.0);

        assert_eq!(db.ecus.len(), 4);
        assert_eq!(db.frames.len(), 4);
        assert_eq!(db.pdus.len(), 4);

        // EngineData: A+B slot 1, startup.
        let engine = db.frames.iter().find(|f| f.name == "EngineData").unwrap();
        assert_eq!(engine.triggering.slot_id, 1);
        assert_eq!(engine.triggering.channel, FrChannel::Both);
        assert_eq!(engine.triggering.cycle_repetition, 1);
        assert!(engine.triggering.startup);

        // ChassisStatus: B only, repetition 4.
        let chassis = db.frames.iter().find(|f| f.name == "ChassisStatus").unwrap();
        assert_eq!(chassis.triggering.channel, FrChannel::B);
        assert_eq!(chassis.triggering.cycle_repetition, 4);
        assert_eq!(chassis.length, 12);

        let eng_pdu = db.pdus.iter().find(|p| p.name == "Eng_PDU").unwrap();
        assert_eq!(eng_pdu.signals.len(), 3);
        let speed = eng_pdu.signals.iter().find(|s| s.name == "EngineSpeed").unwrap();
        assert_eq!(speed.start_bit, 0);
        assert_eq!(speed.length_bits, 16);
        assert!(speed.big_endian);
    }

    /// The Vector CANoe 3.0 demo export: the flat scanner used to lose
    /// its fx:-prefixed frame triggerings; the tree walk gets every one,
    /// with names.
    #[test]
    fn parses_the_canoe_demo_fibex_completely() {
        let db = FrDb::parse(CANOE_DEMO).expect("the demo FIBEX parses");
        assert_eq!(db.params.speed_kbps, 10000);
        assert_eq!(db.params.number_of_static_slots, 60);
        assert_eq!(db.params.payload_length_static, 21);
        assert_eq!(
            db.frames.len(),
            33,
            "every frame triggering becomes a named frame"
        );
        assert!(db.frames.iter().any(|f| f.triggering.slot_id == 7));
        assert!(
            db.frames.iter().all(|f| !f.name.is_empty()),
            "frames carry their SHORT-NAMEs"
        );
    }

    /// Slot lookup honours the cycle repetition: a rep-4 frame on base
    /// cycle 0 is only visible on cycles 0/4/8/....
    #[test]
    fn frame_at_respects_the_schedule() {
        let db = FrDb::parse(SAMPLE_FIBEX).unwrap();
        // ChassisStatus: slot 3, B, base 0, rep 4.
        assert!(db.frame_at(3, 0, 1).is_some(), "cycle 0 fires");
        assert!(db.frame_at(3, 4, 1).is_some(), "cycle 4 fires");
        assert!(db.frame_at(3, 1, 1).is_none(), "cycle 1 silent");
        assert!(db.frame_at(3, 0, 0).is_none(), "channel A does not carry it");
        assert!(db.frame_at(3, 0, 2).is_some(), "unknown channel still finds it");
    }

    /// Signal decode: raw bits -> physical value -> enumeration label.
    /// The bit convention matches roxy-fibex and the DBC extract path:
    /// big-endian start_bit names the MSB, walking down within the byte
    /// and jumping +15 at the byte boundary.
    #[test]
    fn decodes_signals_from_a_payload() {
        let db = FrDb::parse(SAMPLE_FIBEX).unwrap();
        let engine = db.frames.iter().find(|f| f.name == "EngineData").unwrap();
        // EngineSpeed: big-endian 16 bit at start 0. In the sawtooth
        // walk byte1 bit3 is raw bit 10: 0x08 lands raw = 1024, and
        // 1024 × 0.25 = 256.0 rpm.
        let payload = [0x00, 0x08, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00];
        let decoded = db.decode(engine, &payload);
        let speed = decoded.iter().find(|(n, _)| n == "EngineSpeed").unwrap();
        assert!(speed.1.starts_with("256"), "{speed:?}");

        // ChassisStatus (the status signal sits at bit 48, big-endian,
        // 8 coded bits): the walk ends at byte7 bit1, so 0x02 there is
        // raw 1 -> the Warning label.
        let chassis = db.frames.iter().find(|f| f.name == "ChassisStatus").unwrap();
        let decoded = db.decode(chassis, &[0, 0, 0, 0, 0, 0, 0, 0x02, 0, 0, 0, 0]);
        assert!(
            decoded.iter().any(|(_, v)| v.contains("Warning")),
            "raw 1 decodes to its Warning label: {decoded:?}"
        );
    }
}
