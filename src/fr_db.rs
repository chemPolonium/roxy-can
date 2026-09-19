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

use std::collections::{HashMap, HashSet};

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
#[derive(Clone, Debug, Default, PartialEq)]
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
#[derive(Clone, Debug, Default, PartialEq)]
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
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FrFrameDb {
    pub name: String,
    /// Byte length 0..=254.
    pub length: u32,
    pub payload_preamble: bool,
    pub triggering: FrTriggering,
    /// (PDU name, start bit within the frame payload). Both the FIBEX and
    /// ARXML dialects express the frame's PDU placement in bits, so the
    /// decoder adds it to each signal's bit offset directly.
    pub pdus: Vec<(String, u32)>,
    pub comment: String,
}

/// Cluster-level protocol parameters. The core sixteen mirror the
/// editor's set; the `extra` block collects the raw vxlapi-oriented
/// values (payload length, correction windows, sample clock) that the
/// driver configuration needs and only some files carry.
#[derive(Clone, Debug, Default, PartialEq)]
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
    pub gd_wakeup_symbol_rx_idle: u32,
    pub gd_wakeup_symbol_rx_low: u32,
    pub gd_wakeup_symbol_rx_window: u32,
    pub gd_wakeup_symbol_tx_idle: u32,
    pub gd_wakeup_symbol_tx_low: u32,
    pub g_channels: u32,
    pub p_channels: u32,
}

/// The parsed FlexRay database.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FrDb {
    pub params: FrClusterParams,
    pub ecus: Vec<String>,
    pub pdus: Vec<FrPdu>,
    pub frames: Vec<FrFrameDb>,
    /// `PDU name -> index into `pdus``, filled by [`Self::parse`]. Decoding a
    /// frame used to scan the whole PDU list once per PDU it carried, which on
    /// a production cluster is per-arrival work: 5k PDUs x 20 PDUs x 1k rows/s.
    pdu_ix: HashMap<String, usize>,
    /// `slot id -> indices into `frames``, document order. `frame_at` scanned
    /// every frame of the database for every arriving FlexRay row.
    slot_ix: HashMap<u32, Vec<usize>>,
}

impl FrDb {
    /// Assembles a database from its parts, with the decode lookup tables
    /// filled. Every `FrDb` in the program comes through here, so the lookups
    /// can rely on the indexes being there.
    pub fn assemble(
        params: FrClusterParams,
        ecus: Vec<String>,
        pdus: Vec<FrPdu>,
        frames: Vec<FrFrameDb>,
    ) -> Self {
        let mut db = Self {
            params,
            ecus,
            pdus,
            frames,
            pdu_ix: HashMap::new(),
            slot_ix: HashMap::new(),
        };
        for (i, p) in db.pdus.iter().enumerate() {
            // First declaration wins, as `iter().find` did.
            db.pdu_ix.entry(p.name.clone()).or_insert(i);
        }
        for (i, f) in db.frames.iter().enumerate() {
            db.slot_ix.entry(f.triggering.slot_id).or_default().push(i);
        }
        db
    }

    /// The PDU named `name`.
    fn pdu_of(&self, name: &str) -> Option<&FrPdu> {
        self.pdu_ix.get(name).and_then(|&i| self.pdus.get(i))
    }

    /// Parses FIBEX or ARXML text, detecting the dialect from the root
    /// element and, failing that, from characteristic children.
    pub fn parse(text: &str) -> Result<FrDb, String> {
        // A UTF-8 BOM (DaVinci exports carry one) is not XML.
        let text = text.strip_prefix('\u{FEFF}').unwrap_or(text);
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
    /// `cycle`, *preferring* one whose channel declaration matches `ab`
    /// (0 = A, 1 = B, 2 = unknown). Several frames may share a slot across
    /// repetitions; the first whose schedule covers the cycle wins.
    ///
    /// The channel is a preference, not a requirement, for two reasons. What
    /// the cluster schedule actually fixes is `(slot, cycle)`; and a live frame
    /// arrives with `ab` unknown, so requiring the declaration would resolve a
    /// frame read off a port but not the same frame read out of a log -- with a
    /// description whose channel attribute disagrees with reality (a stale
    /// export, a frame logged on both channels) leaving the replay to decode
    /// nothing. The check still breaks the tie between frames that share a slot
    /// and cycle and differ only by channel.
    pub fn frame_at(&self, slot: u16, cycle: u8, ab: u8) -> Option<&FrFrameDb> {
        let want = match ab {
            0 => Some(FrChannel::A),
            1 => Some(FrChannel::B),
            _ => None,
        };
        let cands = self.slot_ix.get(&(slot as u32));
        let scheduled =
            |f: &FrFrameDb| f.triggering.slot_id == u32::from(slot)
                && f.triggering.is_active_at_cycle(u32::from(cycle));
        let lookup = |prefer_channel: bool| {
            cands
                .into_iter()
                .flatten()
                .filter_map(|&i| self.frames.get(i))
                .find(|f| {
                    scheduled(f)
                        && (!prefer_channel
                            || match want {
                                Some(w) => f.triggering.channel.covers(w),
                                None => true,
                            })
                })
        };
        lookup(true).or_else(|| lookup(false))
    }

    /// Decodes a frame payload into `(signal, text)` pairs: physical
    /// values with units, enumeration labels where the database has
    /// them. PDU start offsets shift each signal's absolute bit.
    pub fn decode(&self, frame: &FrFrameDb, payload: &[u8]) -> Vec<(String, String)> {
        let mut out = Vec::new();
        let total_bits = payload.len() * 8;
        // The logged payload begins at the frame's first PDU; AUTOSAR PDU
        // START-POSITIONs are measured from the frame head, so subtract the
        // lowest one to align them with the bytes actually recorded.
        let base = frame.pdus.iter().map(|(_, b)| *b).min().unwrap_or(0);
        for (pdu_name, pdu_start_bit) in &frame.pdus {
            let Some(pdu) = self.pdu_of(pdu_name) else {
                continue;
            };
            for sig in &pdu.signals {
                let abs_bit = (*pdu_start_bit - base) as usize + sig.start_bit as usize;
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

    /// Numeric sibling of [`Self::decode`]: the physical value, the raw
    /// integer, the unit and any enum label, per signal. This is what feeds
    /// a Graphics/Data/State curve -- the plotting path needs an `f64`, not
    /// the display text `decode` returns.
    pub fn decode_signals(
        &self,
        frame: &FrFrameDb,
        payload: &[u8],
    ) -> Vec<crate::dbc::DecodedSignal> {
        let mut out = Vec::new();
        let total_bits = payload.len() * 8;
        // See [`Self::decode`]: align AUTOSAR PDU positions with the logged
        // payload, which begins at the frame's first PDU.
        let base = frame.pdus.iter().map(|(_, b)| *b).min().unwrap_or(0);
        for (pdu_name, pdu_start_bit) in &frame.pdus {
            let Some(pdu) = self.pdu_of(pdu_name) else {
                continue;
            };
            for sig in &pdu.signals {
                let abs_bit = (*pdu_start_bit - base) as usize + sig.start_bit as usize;
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
                let phys =
                    crate::decode::to_physical(raw, len as u64, sig.signed, sig.factor, sig.offset);
                let label = sig
                    .value_descriptions
                    .iter()
                    .find(|(v, _)| *v as f64 == phys)
                    .map(|(_, t)| t.clone());
                out.push(crate::dbc::DecodedSignal {
                    name: sig.name.clone(),
                    phys,
                    raw: raw as i64,
                    unit: sig.unit.clone(),
                    type_tag: String::new(),
                    label,
                });
            }
        }
        out
    }

    /// The signal names a frame carries, in payload order. The selection
    /// tree lists a FlexRay frame's signals from this, so a picker entry and
    /// a decoded value share one source of truth and cannot drift apart.
    pub fn signal_names(&self, frame: &FrFrameDb) -> Vec<String> {
        let mut names = Vec::new();
        for (pdu_name, _) in &frame.pdus {
            if let Some(pdu) = self.pdu_of(pdu_name) {
                names.extend(pdu.signals.iter().map(|s| s.name.clone()));
            }
        }
        names
    }

    /// The declaration of signal `name` in the frame that holds `slot`.
    /// Slots shared by several frames (cycle repetition) resolve to the
    /// first, as [`Self::slot_signals`] lists it, so a curve and the picker
    /// that offered it read the same layout. A signal's declared range does
    /// not depend on which cycle the frame is active in, so unlike
    /// [`Self::frame_at`] this ignores the schedule.
    pub fn slot_signal(&self, slot: u16, name: &str) -> Option<&FrSignal> {
        let &i = self.slot_ix.get(&(slot as u32))?.first()?;
        let frame = self.frames.get(i)?;
        frame.pdus.iter().find_map(|(pdu_name, _)| {
            self.pdu_of(pdu_name)
                .and_then(|pdu| pdu.signals.iter().find(|s| s.name == name))
        })
    }

    /// One entry per distinct static slot: the slot id, a representative
    /// frame name and that frame's signal names. Frames sharing a slot
    /// (cycle repetition) collapse to the first, matching [`Self::frame_at`]
    /// so a curve keyed by the slot sees the layout it will decode against.
    pub fn slot_signals(&self) -> Vec<(u16, String, Vec<String>)> {
        let mut seen: HashSet<u32> = HashSet::new();
        let mut out = Vec::new();
        for f in &self.frames {
            if !seen.insert(f.triggering.slot_id) {
                continue;
            }
            let names = self.signal_names(f);
            if names.is_empty() {
                continue;
            }
            out.push((f.triggering.slot_id as u16, f.name.clone(), names));
        }
        out
    }
}

// ----------------------------------------------------------------------
// XML helpers (namespace-tolerant)
// ----------------------------------------------------------------------

/// Collects all descendants with the given tag name (namespace ignored).
/// For *subtree* searches; whole-document searches go through [`DocIndex`].
fn collect<'a, 'b>(node: &roxmltree::Node<'a, 'b>, tag: &str, out: &mut Vec<roxmltree::Node<'a, 'b>>) {
    if node.is_element() && node.tag_name().name() == tag {
        out.push(*node);
    }
    for child in node.children() {
        collect(&child, tag, out);
    }
}

/// Every element of the document, indexed once by tag name and by `ID`
/// attribute.
///
/// Both parsers used to walk the whole tree once per tag they cared about, and
/// once *per item* wherever a reference had to be resolved -- 42x the element
/// count in node visits on a 426 KB ARXML, and O(items x document) for the
/// UNIT lookups a FIBEX does per CODING. That is the difference between
/// milliseconds on a demo cluster and seconds on a production one. The lists
/// keep document order, so every "first match wins" rule reads the same as it
/// did when the scan was literal.
struct DocIndex<'a, 'b> {
    by_tag: HashMap<String, Vec<roxmltree::Node<'a, 'b>>>,
    by_id: HashMap<String, roxmltree::Node<'a, 'b>>,
}

impl<'a, 'b> DocIndex<'a, 'b> {
    fn build(root: &roxmltree::Node<'a, 'b>) -> Self {
        let mut index = DocIndex {
            by_tag: HashMap::new(),
            by_id: HashMap::new(),
        };
        index.walk(root);
        index
    }

    /// Pre-order walk, so every list keeps document order -- the order the
    /// per-tag scans used to produce, and what "first match wins" means here.
    fn walk(&mut self, node: &roxmltree::Node<'a, 'b>) {
        if node.is_element() {
            self.by_tag
                .entry(node.tag_name().name().to_string())
                .or_default()
                .push(*node);
            if let Some(id) = node.attribute("ID") {
                // First wins, as `find` over a document-order scan did.
                self.by_id.entry(id.to_string()).or_insert(*node);
            }
        }
        for child in node.children() {
            self.walk(&child);
        }
    }

    /// Every element named `tag`, document order.
    fn nodes(&self, tag: &str) -> &[roxmltree::Node<'a, 'b>] {
        self.by_tag.get(tag).map(Vec::as_slice).unwrap_or(&[])
    }

    /// The element carrying `id` as its `ID` attribute.
    fn element(&self, id: &str) -> Option<roxmltree::Node<'a, 'b>> {
        self.by_id.get(id).copied()
    }

    /// Text of the first element anywhere in the document named `tag`. The
    /// whole-document fallbacks used to re-walk the tree looking for a tag
    /// that is usually absent -- the commonest case, and the most expensive.
    fn text(&self, tag: &str) -> Option<String> {
        self.nodes(tag)
            .iter()
            .find_map(|n| {
                n.text()
                    .map(str::trim)
                    .filter(|t| !t.is_empty())
                    .map(str::to_string)
            })
    }
}

/// The first non-empty text under `tag` inside the subtree, document order.
fn first_text<'a, 'b>(node: &roxmltree::Node<'a, 'b>, tag: &str) -> Option<String> {
    if node.is_element()
        && node.tag_name().name() == tag
        && let Some(t) = node.text().map(str::trim).filter(|t| !t.is_empty())
    {
        return Some(t.to_string());
    }
    node.children().find_map(|c| first_text(&c, tag))
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
/// node's subtree. Candidates keep their priority: the first tag with a
/// non-empty match wins, even when a later tag appears earlier in the tree.
fn text_of(node: &roxmltree::Node, tags: &[&str]) -> Option<String> {
    tags.iter().find_map(|t| first_text(node, t))
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

/// FRAME's PDU placements: (PDU ref id, start bit). The FIBEX dialects all
/// express the placement in bits -- BIT-POSITION, START-BIT-POSITION and
/// Vector's START-POSITION alike -- stored as a bit offset the decoder adds
/// to each signal's bit position.
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
            let start = text_of(m, &["BIT-POSITION", "START-BIT-POSITION", "START-POSITION"])
                .and_then(|t| parse_u32(&t))
                .unwrap_or(0);
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
    let index = DocIndex::build(&root);

    // 1. Signals and codings: the CODING carries the physical encoding.
    let signal_nodes = index.nodes("SIGNAL");
    let coding_nodes = index.nodes("CODING");

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
    for coding in coding_nodes {
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
            if !unit_id.is_empty()
                && let Some(u) = index.element(unit_id)
                && let Some(dn) = text_of(&u, &["DISPLAY-NAME"])
            {
                unit = dn;
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
    for sig in signal_nodes {
        let Some(id) = sig.attribute("ID") else {
            continue;
        };
        let name = short_name(sig);
        let coding_ref = get_ref(sig, "CODING-REF").unwrap_or_default();
        let comment = text_of(sig, &["L-2", "DESC"]).unwrap_or_default();
        signal_defs.insert(id.to_string(), (name, coding_ref, comment));
    }

    // 2. PDUs.
    let pdu_nodes = index.nodes("PDU");

    struct PduRaw {
        name: String,
        length: u32,
        dynamic: bool,
        comment: String,
        instances: Vec<(String, u32, bool)>,
    }

    let mut pdu_raws: Vec<(String, PduRaw)> = Vec::new();
    for pdu in pdu_nodes {
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
    let frame_nodes = index.nodes("FRAME");

    struct FrameRaw {
        name: String,
        length: u32,
        payload_preamble: bool,
        comment: String,
        pdu_mappings: Vec<(String, u32)>,
        instances: Vec<(String, u32, bool)>,
    }

    let mut frame_raws: Vec<(String, FrameRaw)> = Vec::new();
    for frame in frame_nodes {
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

    let channel_nodes = index.nodes("CHANNEL");

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
    let ecu_nodes = index.nodes("ECU");
    let ecu_nodes = if ecu_nodes.is_empty() {
        index.nodes("CONTROLLER")
    } else {
        ecu_nodes
    };
    let mut ecus: Vec<String> = ecu_nodes.iter().map(short_name).collect();
    ecus.retain(|n| !n.is_empty() && n != "Unnamed");

    // 6. Cluster parameters.
    let cluster_nodes = index.nodes("CLUSTER");
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
    // SAMPLES-PER-MICROTICK lives per controller, outside the CLUSTER
    // element, in Vector exports: fall back to the first occurrence in
    // the whole document.
    if params.p_samples_per_microtick == 0
        && let Some(v) = index.text("SAMPLES-PER-MICROTICK").and_then(|t| parse_u32(&t))
    {
        params.p_samples_per_microtick = v;
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

    // Reference resolution for frames: the PDU list by id, first wins -- the
    // same rule the linear `find` over `pdu_raws` applied per mapping.
    let mut pdu_by_id: HashMap<&str, &PduRaw> = HashMap::new();
    for (id, raw) in &pdu_raws {
        pdu_by_id.entry(id.as_str()).or_insert(raw);
    }

    let mut frames: Vec<FrFrameDb> = Vec::new();
    for (frame_id, raw) in &frame_raws {
        let mut pdus_in_frame = Vec::new();
        for (pdu_ref, start) in &raw.pdu_mappings {
            let Some(pdu_raw) = pdu_by_id.get(pdu_ref.as_str()).copied() else {
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

    Ok(FrDb::assemble(params, ecus, pdus, frames))
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
    p.gd_wakeup_symbol_rx_idle =
        get_u32(&["WAKE-UP-SYMBOL-RX-IDLE", "GD-WAKE-UP-SYMBOL-RX-IDLE"]).unwrap_or(0);
    p.gd_wakeup_symbol_rx_low =
        get_u32(&["WAKE-UP-SYMBOL-RX-LOW", "GD-WAKE-UP-SYMBOL-RX-LOW"]).unwrap_or(0);
    p.gd_wakeup_symbol_rx_window =
        get_u32(&["WAKE-UP-SYMBOL-RX-WINDOW", "GD-WAKE-UP-SYMBOL-RX-WINDOW"]).unwrap_or(0);
    p.gd_wakeup_symbol_tx_idle =
        get_u32(&["WAKE-UP-SYMBOL-TX-IDLE", "GD-WAKE-UP-SYMBOL-TX-IDLE"]).unwrap_or(0);
    p.gd_wakeup_symbol_tx_low =
        get_u32(&["WAKE-UP-SYMBOL-TX-LOW", "GD-WAKE-UP-SYMBOL-TX-LOW"]).unwrap_or(0);
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

/// A COMPU-METHOD's linear scaling and (for TEXTTABLE-style methods) the
/// enumeration labels -- the physical-value half of an ARXML I-SIGNAL, which
/// the FIBEX path reads from CODING but ARXML keeps in a separate, referenced
/// COMPU-METHOD element.
struct CompuMethod {
    factor: f64,
    offset: f64,
    unit: String,
    value_descriptions: Vec<(i64, String)>,
}

fn parse_compu_method(
    node: &roxmltree::Node,
    unit_display: &HashMap<String, String>,
) -> CompuMethod {
    let mut cm = CompuMethod {
        factor: 1.0,
        offset: 0.0,
        unit: String::new(),
        value_descriptions: Vec::new(),
    };
    // COMPU-RATIONAL-COEFFS numerator = [offset, factor], denominator = [1].
    let mut coeffs = Vec::new();
    collect(node, "COMPU-RATIONAL-COEFFS", &mut coeffs);
    if let Some(coeff) = coeffs.first() {
        let mut vs = Vec::new();
        collect(coeff, "V", &mut vs);
        let values: Vec<f64> = vs.iter().filter_map(|n| n.text().and_then(parse_f64)).collect();
        if values.len() >= 2 {
            cm.offset = values[0];
            cm.factor = values[1];
        } else if values.len() == 1 {
            cm.factor = values[0];
        }
    }
    // Enumeration: a COMPU-SCALE carrying a COMPU-CONST/VT, keyed by its
    // internal LOWER-LIMIT. Linear scales (no COMPU-CONST) are skipped.
    let mut scales = Vec::new();
    collect(node, "COMPU-SCALE", &mut scales);
    for sc in &scales {
        let mut consts = Vec::new();
        collect(sc, "COMPU-CONST", &mut consts);
        if let Some(cc) = consts.first()
            && let Some(vt) = text_of(cc, &["VT"])
        {
            let key = text_of(sc, &["LOWER-LIMIT"])
                .and_then(|t| parse_f64(&t))
                .map(|f| f as i64)
                .unwrap_or(0);
            cm.value_descriptions.push((key, vt));
        }
    }
    // Unit: UNIT-REF resolved against the document's UNIT display names.
    if let Some(uref) = text_of(node, &["UNIT-REF"])
        && let Some(dn) = unit_display.get(ref_short_name(&uref))
    {
        cm.unit = dn.clone();
    }
    cm
}

pub fn parse_arxml_doc(doc: &roxmltree::Document) -> Result<FrDb, String> {
    let root = doc.root_element();
    let index = DocIndex::build(&root);

    // 1. I-SIGNAL / SYSTEM-SIGNAL definitions.
    let mut signal_nodes: Vec<roxmltree::Node> = Vec::new();
    for tag in ["I-SIGNAL", "SYSTEM-SIGNAL"] {
        signal_nodes.extend_from_slice(index.nodes(tag));
    }

    // COMPU-METHODs carry each signal's physical scaling and enum labels; an
    // I-SIGNAL references one by name. The FIBEX path folds these into CODING;
    // ARXML keeps them separate, so resolve them here.
    //
    // UNIT display names are resolved once per document: each COMPU-METHOD used
    // to re-walk the whole tree for its own UNIT reference, which is
    // O(COMPU-METHODs x document).
    let mut unit_display: HashMap<String, String> = HashMap::new();
    for u in index.nodes("UNIT") {
        if let Some(dn) = text_of(u, &["DISPLAY-NAME"]) {
            unit_display.entry(short_name(u)).or_insert(dn);
        }
    }
    let mut compu_methods: HashMap<String, CompuMethod> = HashMap::new();
    for cm in index.nodes("COMPU-METHOD") {
        compu_methods.insert(short_name(cm), parse_compu_method(cm, &unit_display));
    }

    struct SignalDef {
        length_bits: u32,
        signed: bool,
        comment: String,
        factor: f64,
        offset: f64,
        unit: String,
        value_descriptions: Vec<(i64, String)>,
    }

    let mut signal_defs: HashMap<String, SignalDef> = HashMap::new();
    for sig in &signal_nodes {
        let name = short_name(sig);
        // `Option`: only an explicit <LENGTH> is a real bit width. A
        // SYSTEM-SIGNAL shares its I-SIGNAL's SHORT-NAME but carries no
        // LENGTH, so a plain default would clobber the I-SIGNAL's width.
        let length_bits = text_of(sig, &["LENGTH"]).and_then(|t| parse_u32(&t));
        let signed = is_signed_text(text_of(
            sig,
            &["SIGN-CONVENTION", "I-SIGNAL-TYPE", "NETWORK-REPRESENTATION-PROPS"],
        ));
        let comment = text_of(sig, &["L-2", "DESC"]).unwrap_or_default();
        let compu = text_of(sig, &["COMPU-METHOD-REF"])
            .map(|r| ref_short_name(&r).to_string())
            .and_then(|n| compu_methods.get(&n));
        match signal_defs.get_mut(&name) {
            Some(existing) => {
                if let Some(l) = length_bits {
                    existing.length_bits = l;
                }
                if existing.comment.is_empty() {
                    existing.comment = comment;
                }
                // Only a node that references a COMPU-METHOD supplies scaling;
                // a SYSTEM-SIGNAL (none) must not reset the I-SIGNAL's.
                if let Some(cm) = compu {
                    existing.factor = cm.factor;
                    existing.offset = cm.offset;
                    if existing.unit.is_empty() {
                        existing.unit = cm.unit.clone();
                    }
                    if existing.value_descriptions.is_empty() {
                        existing.value_descriptions = cm.value_descriptions.clone();
                    }
                }
            }
            None => {
                signal_defs.insert(
                    name,
                    SignalDef {
                        length_bits: length_bits.unwrap_or(1),
                        signed,
                        comment,
                        factor: compu.map(|c| c.factor).unwrap_or(1.0),
                        offset: compu.map(|c| c.offset).unwrap_or(0.0),
                        unit: compu.map(|c| c.unit.clone()).unwrap_or_default(),
                        value_descriptions: compu
                            .map(|c| c.value_descriptions.clone())
                            .unwrap_or_default(),
                    },
                );
            }
        }
    }

    // 2. PDUs: I-SIGNAL-I-PDU / FLEXRAY-I-PDU.
    let mut pdu_nodes: Vec<roxmltree::Node> = Vec::new();
    for tag in ["I-SIGNAL-I-PDU", "FLEXRAY-I-PDU"] {
        pdu_nodes.extend_from_slice(index.nodes(tag));
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
        for tag in [
            "I-SIGNAL-TO-PDU-MAPPING",
            "I-SIGNAL-TO-I-PDU-MAPPING",
        ] {
            collect(pdu, tag, &mut mapping_nodes);
        }
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
    let frame_nodes = index.nodes("FLEXRAY-FRAME");

    struct FrameRaw {
        name: String,
        length: u32,
        payload_preamble: bool,
        comment: String,
        pdu_mappings: Vec<(String, u32)>,
    }

    let mut frames_by_name: HashMap<String, FrameRaw> = HashMap::new();
    for frame in frame_nodes {
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
    let channel_nodes = index.nodes("FLEXRAY-PHYSICAL-CHANNEL");

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
    let cluster_nodes = index.nodes("FLEXRAY-CLUSTER");
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
    // Same document-level fallback as the FIBEX path.
    if params.p_samples_per_microtick == 0
        && let Some(v) = index.text("SAMPLES-PER-MICROTICK").and_then(|t| parse_u32(&t))
    {
        params.p_samples_per_microtick = v;
    }

    // 6. ECUs.
    let ecu_nodes = index.nodes("ECU-INSTANCE");
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
                factor: def.map(|d| d.factor).unwrap_or(1.0),
                offset: def.map(|d| d.offset).unwrap_or(0.0),
                min: 0.0,
                max: 0.0,
                unit: def.map(|d| d.unit.clone()).unwrap_or_default(),
                comment: def.map(|d| d.comment.clone()).unwrap_or_default(),
                value_descriptions: def
                    .map(|d| d.value_descriptions.clone())
                    .unwrap_or_default(),
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

    dedup_frame_names(&mut frames);
    // Index after the dedup: the tables hold positions in `frames`.
    Ok(FrDb::assemble(params, ecus, pdus, frames))
}

/// Keeps the first frame of each name; duplicates only confuse the
/// display-side lookups.
fn dedup_frame_names(frames: &mut Vec<FrFrameDb>) {
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    frames.retain(|f| seen.insert(f.name.clone()));
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The user's real network files. They are read at runtime (they may
    /// move around), and the tests skip with a message when absent.
    const POWERTRAIN_ARXML: &str = "assets/arxml/PowerTrain.arxml";
    const POWERTRAIN_FIBEX_V2: &str = "assets/fibex/PowerTrain_v2.xml";

    fn read_asset(path: &str) -> Option<String> {
        let bytes = std::fs::read(path).ok()?;
        Some(crate::dbc::text_from_bytes(bytes))
    }

    /// The real AUTOSAR cluster export (GBK, no BOM): after the UTF-8 →
    /// GBK text fallback it parses completely -- 10 Mbit/s, 5 ms cycle,
    /// 10 static slots, 48 scheduled frames with names.
    #[test]
    fn parses_the_real_gbk_powertrain_arxml() {
        let Some(text) = read_asset(POWERTRAIN_ARXML) else {
            println!("assets/arxml/PowerTrain.arxml not present -- skipped");
            return;
        };
        let db = FrDb::parse(&text).expect("PowerTrain.arxml parses");
        assert_eq!(db.params.speed_kbps, 10000);
        assert_eq!(db.params.cycle_time_ms, 5.0);
        assert_eq!(db.params.macrotick_duration_us, 5.0);
        assert_eq!(db.params.number_of_static_slots, 10);
        assert_eq!(db.params.payload_length_static, 13);
        assert_eq!(db.params.number_of_minislots, 292);
        assert_eq!(db.frames.len(), 48, "every triggering becomes a frame");
        assert!(
            db.frames.iter().all(|f| !f.name.is_empty()),
            "frames carry their SHORT-NAMEs"
        );
        assert!(
            db.pdus.iter().any(|p| !p.signals.is_empty()),
            "signal layouts came along"
        );
    }

    /// Two ARXML parsing regressions that made every FlexRay signal read 0:
    /// (1) a SYSTEM-SIGNAL sharing an I-SIGNAL's SHORT-NAME clobbered its
    /// bit width back to the default 1, (2) the frame's PDU START-POSITION
    /// is a *bit* offset relative to the frame head while the logged payload
    /// begins at the first PDU, so the base must be subtracted, and (3) the
    /// ARXML path never read the COMPU-METHOD scale, so every physical value
    /// came out as its raw integer.
    #[test]
    fn arxml_keeps_signal_widths_and_pdu_bit_offsets() {
        let Some(text) = read_asset(POWERTRAIN_ARXML) else {
            println!("assets/arxml/PowerTrain.arxml not present -- skipped");
            return;
        };
        let db = FrDb::parse(&text).unwrap();
        // (1) CarSpeed is a 16-bit I-SIGNAL; the SYSTEM-SIGNAL merge must
        // not shrink it to 1. (3) Its COMPU-METHOD scales by 0.5.
        let car = db
            .pdus
            .iter()
            .flat_map(|p| p.signals.iter())
            .find(|s| s.name == "CarSpeed")
            .expect("CarSpeed signal");
        assert_eq!(car.length_bits, 16, "I-SIGNAL LENGTH survives the merge");
        assert!((car.factor - 0.5).abs() < 1e-9, "COMPU-METHOD factor parsed");
        // (2)+(3): a payload whose CarSpeed bits read 236 must decode to 118
        // at the frame's first PDU (base 7 subtracted).
        let f13 = db.frames.iter().find(|f| f.name == "Frame_13_0_2").unwrap();
        let cs = db
            .decode_signals(f13, &[0x00, 0xEC, 0x00, 0x00, 0x00, 0x00])
            .into_iter()
            .find(|d| d.name == "CarSpeed")
            .expect("CarSpeed decoded");
        assert_eq!(cs.raw, 236, "CarSpeed raw at the corrected bit base");
        assert_eq!(cs.phys, 118.0, "CarSpeed physical = raw * factor");
        // (2) EngineData sits at bit 71 of Frame_25_0_2; its signals must
        // decode rather than be skipped for "exceeding" the payload.
        let frame = db
            .frames
            .iter()
            .find(|f| f.name == "Frame_25_0_2")
            .expect("Frame_25_0_2");
        assert!(
            frame.pdus.iter().any(|(n, b)| n == "EngineData" && *b == 71),
            "EngineData placed at bit 71: {:?}",
            frame.pdus
        );
        let decoded = db.decode_signals(frame, &[0xFF; 26]);
        let names: Vec<String> = decoded.iter().map(|d| d.name.clone()).collect();
        assert!(
            decoded.iter().any(|d| d.name == "EngSpeed" && d.raw != 0),
            "EngineData signals decode, not skipped: {names:?}"
        );
    }

    /// The real FIBEX 2.0.0d export (DaVinci, dual-net forwarding demo):
    /// six static frames, signal-directly-on-frame layout, slot 13 and
    /// 51 among them.
    #[test]
    fn parses_the_real_fibex_20d_export() {
        let Some(text) = read_asset(POWERTRAIN_FIBEX_V2) else {
            println!("assets/fibex/PowerTrain_v2.xml not present -- skipped");
            return;
        };
        let db = FrDb::parse(&text).expect("PowerTrain_v2.xml parses");
        assert_eq!(db.frames.len(), 6, "the demo schedules six static frames");
        let absinfo = db.frames.iter().find(|f| f.name == "ABSInfo");
        assert!(absinfo.is_some(), "ABSInfo is among the frames");
        assert!(
            db.frames.iter().any(|f| f.triggering.slot_id == 13),
            "slot 13 is scheduled"
        );
        // Signal-directly-on-frame: the synthesized PDU carries the
        // frame's signals, so decode works without a PDU layer.
        let frame = absinfo.expect("checked above");
        assert!(
            db.decode(frame, &[0xFF; 16])
                .iter()
                .any(|(_, v)| !v.is_empty()),
            "frame signals decode"
        );
    }

    /// Slot lookup honours the cycle repetition and the reception
    /// channel, exercised on the real AUTOSAR schedule.
    #[test]
    fn frame_at_respects_the_real_schedule() {
        let Some(text) = read_asset(POWERTRAIN_ARXML) else {
            println!("assets/arxml/PowerTrain.arxml not present -- skipped");
            return;
        };
        let db = FrDb::parse(&text).unwrap();
        // Slot 13 hosts a frame that the logging session carried on the
        // very first cycle: it must resolve for an unknown channel.
        let frame = db
            .frame_at(13, 0, 2)
            .expect("slot 13 fires on cycle 0 for an unknown channel");
        assert_eq!(frame.triggering.slot_id, 13);
        // Every db frame resolves for its own base cycle when the
        // channel is unknown.
        for f in db.frames.iter().take(10) {
            assert!(
                db.frame_at(
                    f.triggering.slot_id as u16,
                    f.triggering.base_cycle as u8,
                    2
                )
                .is_some(),
                "slot {} resolves on its base cycle",
                f.triggering.slot_id
            );
        }
    }

    /// Signal decode over the real AUTOSAR layout. An all-ones payload
    /// makes any unsigned signal's raw read `2^len - 1` regardless of
    /// its bit position, so the physical value is exactly
    /// `(2^len - 1) × factor + offset` -- deterministic without knowing
    /// the concrete network's scaling.
    #[test]
    fn decodes_a_signal_from_the_real_arxml() {
        let Some(text) = read_asset(POWERTRAIN_ARXML) else {
            println!("assets/arxml/PowerTrain.arxml not present -- skipped");
            return;
        };
        let db = FrDb::parse(&text).unwrap();
        let (frame, pdu_name, sig) = db
            .frames
            .iter()
            .find_map(|f| {
                f.pdus.iter().find_map(|(pdu, pdu_start)| {
                    let pdu = db.pdus.iter().find(|p| p.name == *pdu)?;
                    let s = pdu.signals.first()?;
                    // The signal's extent must fit the frame the db
                    // declares, or decode() would (correctly) skip it.
                    let end = pdu_start * 8 + s.start_bit + s.length_bits;
                    (end <= f.length * 8).then(|| (f, pdu.name.clone(), s))
                })
            })
            .expect("the cluster declares at least one signal");
        assert!(!sig.signed, "the first signal is unsigned");

        let raw_max = (1u64 << sig.length_bits.min(32)) - 1;
        let payload = vec![0xFFu8; frame.length.max(1) as usize];
        let decoded = db.decode(frame, &payload);
        let (_name, value) = &decoded[0];
        // decode() annotates every value with its raw hex.
        let expected = format!(
            "{}  ({raw_max:X}h)",
            crate::dbc::fmt_signal_value(
                raw_max as f64 * sig.factor + sig.offset,
                &sig.unit,
                "",
                None,
            )
        );
        assert_eq!(
            value, &expected,
            "frame {} pdu {} signal {} (start {} len {})",
            frame.name, pdu_name, sig.name, sig.start_bit, sig.length_bits
        );
    }
}
