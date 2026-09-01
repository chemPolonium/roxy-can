use std::collections::HashMap;

use can_dbc::{
    AttributeValue, ByteOrder, MessageId, MultiplexIndicator, NumericValue,
    SignalExtendedValueType, ValueDescription, ValueType,
};

use crate::can::frame::CanFrame;
use crate::decode;

fn numeric(v: &NumericValue) -> f64 {
    match v {
        NumericValue::Uint(u) => *u as f64,
        NumericValue::Int(i) => *i as f64,
        NumericValue::Double(d) => *d,
    }
}

/// Attribute names that declare a message's send cycle, best first. Vector and
/// CANoe write `GenMsgCycleTime`; `assets/motbus.dbc` uses the older bare
/// `CycleTime`, so both are honoured.
const CYCLE_ATTRS: [&str; 2] = ["GenMsgCycleTime", "CycleTime"];

fn msg_id_of(id: &MessageId) -> u32 {
    match id {
        MessageId::Standard(i) => *i as u32,
        MessageId::Extended(i) => *i,
    }
}

/// A declared cycle is a number of milliseconds. A non-numeric attribute that
/// happens to share the name yields None rather than a plausible-looking zero.
/// Zero itself is a real value: it means event-triggered, never sent on a timer.
fn cycle_us_of(v: &AttributeValue) -> Option<u64> {
    let ms = match v {
        AttributeValue::Uint(u) => Some(*u),
        AttributeValue::Int(i) => u64::try_from(*i).ok(),
        AttributeValue::Double(d) if *d >= 0.0 && *d < f64::from(u32::MAX) => Some(*d as u64),
        _ => None,
    }?;
    Some(ms.saturating_mul(1_000))
}

/// Every message's declared send cycle in microseconds. An explicit `BA_` wins
/// over the `BA_DEF_DEF_` default, which by definition applies to each message
/// that carries none. A database that says nothing produces no entry at all --
/// the caller then keeps whatever fallback it had rather than an invented value.
fn declared_cycles(db: &can_dbc::Dbc) -> HashMap<u32, u64> {
    let mut out: HashMap<u32, u64> = HashMap::new();
    // One pass per attribute name, always `or_insert`, so `CYCLE_ATTRS` order is
    // the priority order and neither scan can clobber the other's explicit hit.
    for name in CYCLE_ATTRS {
        for v in &db.attribute_values_message {
            if v.name != *name {
                continue;
            }
            if let Some(us) = cycle_us_of(&v.value) {
                out.entry(msg_id_of(&v.message_id)).or_insert(us);
            }
        }
    }
    let default = CYCLE_ATTRS
        .iter()
        .find_map(|n| db.attribute_defaults.iter().find(|d| &d.name == n))
        .and_then(|d| cycle_us_of(&d.value));
    if let Some(us) = default {
        for msg in &db.messages {
            out.entry(msg_id_of(&msg.id)).or_insert(us);
        }
    }
    out
}

#[derive(Clone)]
pub struct SignalInfo {
    pub name: String,
    pub start_bit: u64,
    pub size: u64,
    pub big_endian: bool,
    pub signed: bool,
    pub factor: f64,
    pub offset: f64,
    pub min: f64,
    pub max: f64,
    pub unit: String,
    pub receivers: Vec<String>,
    /// `SIG_VALTYPE_` reinterprets the raw bits as IEEE floats; `false` means
    /// the ordinary integer reading, whose sign lives in `signed`.
    pub is_float: bool,
    /// How the type prints: `u8`/`i16` from the bit layout, `f32`/`f64` from a
    /// `SIG_VALTYPE_` line. Precomputed because every window shows it per row.
    pub type_tag: String,
    /// Conditions that must all hold for the signal to be present in a frame.
    /// Empty covers the switch signals themselves and ordinary signals.
    pub mux_when: Vec<MuxCondition>,
}

/// One gating condition on a multiplexed signal: the named switch signal in
/// the same message must read a value inside one of the inclusive ranges.
/// `m` markers yield a single one-value range; `SG_MUL_VAL_` lines can carry
/// wide ranges, several of them, and nested switches.
#[derive(Clone)]
pub struct MuxCondition {
    pub switch: String,
    pub ranges: Vec<(u64, u64)>,
}

pub struct MessageInfo {
    pub name: String,
    pub dlc: u64,
    pub transmitter: String,
    /// Declared send cycle in microseconds. `Some(0)` means event-triggered --
    /// the generator must never put it on a timer. `None` means the database
    /// declares no cycle, which is not the same claim.
    pub cycle_us: Option<u64>,
    pub signals: Vec<SignalInfo>,
    /// Names of the signals some condition gates on, deduped, so a decode
    /// reads each switch's bits exactly once.
    pub switch_names: Vec<String>,
}

pub struct SymbolTable {
    pub messages: HashMap<u32, MessageInfo>,
    pub order: Vec<u32>,
    pub nodes: Vec<String>,
    /// `VAL_` enum labels, keyed by (message id, signal name) and matching on
    /// the **raw** integer value. Labels referenced by a named table
    /// (`VAL_ 100 Sig TableName;`) are unreachable: the parser cannot read that
    /// form at all.
    pub value_tables: HashMap<(u32, String), HashMap<i64, String>>,
}

/// `SG_MUL_VAL_` gates, grouped as message id → signal name → switch name →
/// the inclusive switch ranges that show the signal.
type ExtMuxRules = HashMap<u32, HashMap<String, HashMap<String, Vec<(u64, u64)>>>>;

impl SymbolTable {
    pub fn from_dbc(db: &can_dbc::Dbc) -> Self {
        let mut messages = HashMap::new();
        let mut order = Vec::new();
        let cycles = declared_cycles(db);
        // `SIG_VALTYPE_` lines name the signals whose bits are IEEE floats.
        let mut floats: HashMap<(u32, &str), u64> = HashMap::new();
        for v in &db.signal_extended_value_type_list {
            let width = match v.signal_extended_value_type {
                SignalExtendedValueType::IEEEfloat32Bit => 32,
                SignalExtendedValueType::IEEEdouble64bit => 64,
                SignalExtendedValueType::SignedOrUnsignedInteger => continue,
            };
            floats.insert((msg_id_of(&v.message_id), v.signal_name.as_str()), width);
        }
        // `SG_MUL_VAL_` lines state a signal's gating outright: per switch
        // signal, a union of inclusive ranges, and all switches must agree.
        let mut ext_mux: ExtMuxRules = HashMap::new();
        for em in &db.extended_multiplex {
            let by_switch = ext_mux.entry(msg_id_of(&em.message_id)).or_default();
            let ranges = by_switch
                .entry(em.signal_name.clone())
                .or_default()
                .entry(em.multiplexor_signal_name.clone())
                .or_default();
            for m in &em.mappings {
                ranges.push((m.min_value, m.max_value));
            }
        }
        for msg in &db.messages {
            let id = msg_id_of(&msg.id);
            // `m` markers gate against the message's single top switch, named
            // by whichever signal carries the `M`. A message with several `M`
            // markers keeps the first -- there is nothing sane to do with
            // more than one.
            let top_switch = msg.signals.iter().find_map(|s| {
                matches!(s.multiplexer_indicator, MultiplexIndicator::Multiplexor)
                    .then_some(s.name.as_str())
            });
            let mut signals = msg
                .signals
                .iter()
                .map(|s| {
                    let mux_when: Vec<MuxCondition> = match ext_mux
                        .get(&id)
                        .and_then(|m| m.get(s.name.as_str()))
                    {
                        // An `SG_MUL_VAL_` line supersedes whatever the
                        // signal's own `m` marker says.
                        Some(by_switch) => by_switch
                            .iter()
                            .map(|(sw, ranges)| MuxCondition {
                                switch: sw.to_string(),
                                ranges: ranges.clone(),
                            })
                            .collect(),
                        None => match s.multiplexer_indicator {
                            MultiplexIndicator::MultiplexedSignal(g)
                            | MultiplexIndicator::MultiplexorAndMultiplexedSignal(g) => top_switch
                                .map(|name| MuxCondition {
                                    switch: name.to_string(),
                                    ranges: vec![(g, g)],
                                })
                                .into_iter()
                                .collect(),
                            _ => Vec::new(),
                        },
                    };
                    // A float declaration whose width disagrees with the
                    // signal's own bit size is broken; the integer reading is
                    // kept rather than decoding from mismatched bits.
                    let float_width = floats
                        .get(&(id, s.name.as_str()))
                        .copied()
                        .filter(|w| *w == s.size);
                    let type_tag = match float_width {
                        Some(32) => "f32".to_string(),
                        Some(64) => "f64".to_string(),
                        Some(_) => unreachable!("only 32 and 64 are stored"),
                        None => {
                            if matches!(s.value_type, ValueType::Signed) {
                                format!("i{}", s.size)
                            } else {
                                format!("u{}", s.size)
                            }
                        }
                    };
                    SignalInfo {
                        name: s.name.clone(),
                        start_bit: s.start_bit,
                        size: s.size,
                        big_endian: matches!(s.byte_order, ByteOrder::BigEndian),
                        signed: matches!(s.value_type, ValueType::Signed),
                        factor: s.factor,
                        offset: s.offset,
                        min: numeric(&s.min),
                        max: numeric(&s.max),
                        unit: s.unit.clone(),
                        receivers: s
                            .receivers
                            .iter()
                            .filter(|r| r.as_str() != "Vector__XXX")
                            .cloned()
                            .collect(),
                        is_float: float_width.is_some(),
                        type_tag,
                        mux_when,
                    }
                })
                .collect::<Vec<_>>();
            // A nested switch's own bytes are only meaningful while the switch
            // above it is active, so a signal gated on a nested switch must
            // inherit that switch's conditions too -- and its ancestors', all
            // the way up. Same switch met twice unions the ranges: a chain
            // that states a range twice narrows nothing. A cycle (a switch
            // ultimately gating on itself) is a broken database; the visited
            // set just stops the walk, leaving the flat conditions already
            // collected.
            let own_conditions: HashMap<String, Vec<MuxCondition>> = signals
                .iter()
                .map(|s| (s.name.clone(), s.mux_when.clone()))
                .collect();
            for s in &mut signals {
                let mut merged: HashMap<String, Vec<(u64, u64)>> = s
                    .mux_when
                    .iter()
                    .map(|c| (c.switch.clone(), c.ranges.clone()))
                    .collect();
                let mut worklist: Vec<String> =
                    s.mux_when.iter().map(|c| c.switch.clone()).collect();
                let mut visited: Vec<String> = Vec::new();
                while let Some(name) = worklist.pop() {
                    if visited.contains(&name) {
                        continue;
                    }
                    visited.push(name.clone());
                    for c in own_conditions
                        .get(&name)
                        .into_iter()
                        .flat_map(|cs| cs.iter())
                    {
                        merged
                            .entry(c.switch.clone())
                            .or_default()
                            .extend(c.ranges.iter().copied());
                        worklist.push(c.switch.clone());
                    }
                }
                s.mux_when = merged
                    .into_iter()
                    .map(|(switch, ranges)| MuxCondition { switch, ranges })
                    .collect();
            }
            let mut switch_names: Vec<String> = signals
                .iter()
                .flat_map(|s| s.mux_when.iter().map(|c| c.switch.clone()))
                .collect();
            switch_names.sort_unstable();
            switch_names.dedup();
            let transmitter = msg
                .transmitter
                .clone()
                .filter(|t| t != "Vector__XXX")
                .unwrap_or_default();
            messages.insert(
                id,
                MessageInfo {
                    name: msg.name.clone(),
                    dlc: msg.size,
                    transmitter,
                    cycle_us: cycles.get(&id).copied(),
                    signals,
                    switch_names,
                },
            );
            order.push(id);
        }
        order.sort_unstable();
        order.dedup();
        let nodes = db.nodes.iter().map(|n| n.0.clone()).collect();
        let mut value_tables: HashMap<(u32, String), HashMap<i64, String>> = HashMap::new();
        for vd in &db.value_descriptions {
            if let ValueDescription::Signal {
                message_id,
                name,
                value_descriptions,
            } = vd
            {
                let table = value_tables
                    .entry((msg_id_of(message_id), name.clone()))
                    .or_default();
                for d in value_descriptions {
                    table.insert(d.id, d.description.clone());
                }
            }
        }
        SymbolTable {
            messages,
            order,
            nodes,
            value_tables,
        }
    }

    /// Message IDs transmitted by the given node.
    pub fn node_tx_ids(&self, node: &str) -> Vec<u32> {
        self.order
            .iter()
            .copied()
            .filter(|id| self.messages.get(id).is_some_and(|m| m.transmitter == node))
            .collect()
    }

    /// Signals received by the given node: (message id, signal name, sender).
    pub fn node_rx_signals(&self, node: &str) -> Vec<(u32, String, String)> {
        let mut out = Vec::new();
        for &id in &self.order {
            let Some(m) = self.messages.get(&id) else {
                continue;
            };
            for s in &m.signals {
                if s.receivers.iter().any(|r| r == node) {
                    out.push((id, s.name.clone(), m.transmitter.clone()));
                }
            }
        }
        out
    }

    pub fn message_name(&self, id: u32) -> Option<&str> {
        self.messages.get(&id).map(|m| m.name.as_str())
    }

    /// Packs a physical signal value into the frame data bytes.
    /// Returns false if the message or signal is unknown.
    pub fn encode_signal(&self, id: u32, name: &str, phys: f64, data: &mut [u8]) -> bool {
        let Some(msg) = self.messages.get(&id) else {
            return false;
        };
        let Some(s) = msg.signals.iter().find(|s| s.name == name) else {
            return false;
        };
        let raw = match (s.is_float, s.size) {
            (true, 32) => (((phys - s.offset) / s.factor) as f32).to_bits() as u64,
            (true, 64) => ((phys - s.offset) / s.factor).to_bits(),
            _ => decode::from_physical(phys, s.size, s.signed, s.factor, s.offset),
        };
        decode::pack_raw(data, s.start_bit, s.size, s.big_endian, raw);
        true
    }

    pub fn decode_signals(&self, frame: &CanFrame) -> Vec<DecodedSignal> {
        let Some(msg) = self.messages.get(&frame.id) else {
            return Vec::new();
        };
        // Every switch a condition gates on, decoded once per call. A switch
        // value is the sign-extended raw seen as bits: a negative reading can
        // then never fall inside the non-negative ranges a DBC can express,
        // which is exactly how an `m` marker behaves too. A frame too short to
        // hold the switch bits reads 0 (extract_raw's out-of-range behaviour),
        // which is taken at face value.
        let mut switches: HashMap<&str, u64> = HashMap::new();
        for name in &msg.switch_names {
            if let Some(sw) = msg.signals.iter().find(|s| s.name == *name) {
                let raw =
                    decode::extract_raw(frame.payload(), sw.start_bit, sw.size, sw.big_endian);
                let v = decode::to_physical(raw, sw.size, sw.signed, 1.0, 0.0) as i64;
                switches.insert(sw.name.as_str(), v as u64);
            }
        }
        msg.signals
            .iter()
            .filter(|s| {
                s.mux_when
                    .iter()
                    .all(|c| match switches.get(c.switch.as_str()) {
                        Some(&v) => c.ranges.iter().any(|&(lo, hi)| v >= lo && v <= hi),
                        // A condition naming a switch the message does not carry
                        // is taken as satisfied, so a broken database still shows
                        // data rather than an empty decode.
                        None => true,
                    })
            })
            .map(|s| {
                let raw = decode::extract_raw(frame.payload(), s.start_bit, s.size, s.big_endian);
                let phys = match (s.is_float, s.size) {
                    (true, 32) => f32::from_bits(raw as u32) as f64 * s.factor + s.offset,
                    (true, 64) => f64::from_bits(raw) * s.factor + s.offset,
                    _ => decode::to_physical(raw, s.size, s.signed, s.factor, s.offset),
                };
                let label = if s.is_float {
                    None
                } else {
                    self.val_label(frame.id, &s.name, raw, s.size, s.signed)
                };
                // The raw integer, sign-extended for signed signals: the Data
                // window's Raw Value column shows the wire value before the
                // factor and offset touch it.
                let raw_val = if s.is_float {
                    raw as i64
                } else {
                    decode::to_physical(raw, s.size, s.signed, 1.0, 0.0) as i64
                };
                DecodedSignal {
                    name: s.name.clone(),
                    phys,
                    raw: raw_val,
                    unit: s.unit.clone(),
                    type_tag: s.type_tag.clone(),
                    label,
                }
            })
            .collect()
    }

    /// The `VAL_` label for a signal's extracted bits, if one names this raw
    /// value. Comparing on the sign-extended integer means a `VAL_` entry with
    /// a negative id can label a signed signal.
    fn val_label(&self, id: u32, name: &str, raw: u64, size: u64, signed: bool) -> Option<String> {
        let key = decode::to_physical(raw, size, signed, 1.0, 0.0) as i64;
        self.value_tables
            .get(&(id, name.to_string()))
            .and_then(|table| table.get(&key))
            .cloned()
    }
}

/// One decoded signal: physical value plus the raw integer the wire carried,
/// and where the database gives one, the enum label for this raw value, and
/// the type tag the windows print with it.
pub struct DecodedSignal {
    pub name: String,
    pub phys: f64,
    pub raw: i64,
    pub unit: String,
    pub type_tag: String,
    pub label: Option<String>,
}

/// A decoded value as its own type reads best: an f32 prints as f32 so the
/// f64 widening never shows binary noise, and an integer drops the trailing
/// `.000` that a fractional factor earns -- but keeps its decimals when the
/// factor makes the value genuinely fractional.
pub fn fmt_decoded(type_tag: &str, phys: f64) -> String {
    match type_tag {
        "f32" => format!("{}", phys as f32),
        "f64" => format!("{phys}"),
        _ if (phys - phys.trunc()).abs() < 1e-9 && phys.abs() < 1e15 => {
            format!("{}", phys as i64)
        }
        _ => {
            let s = format!("{phys:.3}");
            let t = s.trim_end_matches('0');
            if t.ends_with('.') { s } else { t.to_string() }
        }
    }
}

/// One decoded value as the windows print it: the number, then the unit, then
/// the type tag, then the enum label. Empty parts are skipped, so a unitless
/// integer reads `3000 [u16]` rather than `3000  [u16]`.
pub fn fmt_signal_value(phys: f64, unit: &str, type_tag: &str, label: Option<&str>) -> String {
    let mut s = fmt_decoded(type_tag, phys);
    if !unit.is_empty() {
        s.push(' ');
        s.push_str(unit);
    }
    s.push_str(&format!(" [{type_tag}]"));
    if let Some(l) = label {
        s.push_str(&format!(" ({l})"));
    }
    s
}

pub fn load_dbc_str(content: &str) -> Result<SymbolTable, String> {
    let db = can_dbc::Dbc::try_from(content).map_err(|e| format!("parse error: {e:?}"))?;
    Ok(SymbolTable::from_dbc(&db))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::can::frame::{CanFrame, Direction, FrameFlags, MAX_CAN_FD_LEN};

    fn frame_with(id: u32, bytes: &[u8]) -> CanFrame {
        let mut data = [0u8; MAX_CAN_FD_LEN];
        data[..bytes.len()].copy_from_slice(bytes);
        CanFrame {
            t_us: 0,
            channel: 0,
            id,
            extended: false,
            len: bytes.len() as u8,
            data,
            dir: Direction::Rx,
            flags: FrameFlags::NONE,
        }
    }

    /// Four messages exercising every branch of the cycle lookup: both
    /// attribute names on one message, only the legacy name on another, an
    /// explicit zero, and one that falls through to the `BA_DEF_DEF_` default.
    #[cfg(test)]
    const CYCLE_DBC: &str = r#"VERSION "roxy-can cycle test"

NS_ :

BU_: ECU

BO_ 256 BothAttrs: 8 ECU
 SG_ S : 0|8@1+ (1,0) [0|0] "" ECU

BO_ 257 OnlyLegacy: 8 ECU
 SG_ S : 0|8@1+ (1,0) [0|0] "" ECU

BO_ 258 EventOnly: 8 ECU
 SG_ S : 0|8@1+ (1,0) [0|0] "" ECU

BO_ 260 UsesDefault: 8 ECU
 SG_ S : 0|8@1+ (1,0) [0|0] "" ECU

BA_DEF_ BO_  "GenMsgCycleTime" INT 0 10000;
BA_DEF_ BO_  "CycleTime" INT 0 10000;
BA_DEF_DEF_  "GenMsgCycleTime" 77;
BA_DEF_DEF_  "CycleTime" 88;
BA_ "GenMsgCycleTime" BO_ 256 111;
BA_ "CycleTime" BO_ 256 222;
BA_ "CycleTime" BO_ 257 333;
BA_ "GenMsgCycleTime" BO_ 258 0;
"#;

    #[test]
    fn declared_cycle_time_from_motbus() {
        let motbus = std::fs::read_to_string("assets/motbus.dbc").unwrap();
        let db = load_dbc_str(&motbus).unwrap();
        // assets/motbus.dbc:62-63 give these two explicit values...
        assert_eq!(db.messages[&0x64].cycle_us, Some(133_000), "EngineData");
        assert_eq!(db.messages[&0xC9].cycle_us, Some(50_000), "ABSdata");
        // ...and every other message inherits BA_DEF_DEF_ "CycleTime" 100.
        assert_eq!(
            db.messages[&0xC7].cycle_us,
            Some(100_000),
            "unlisted takes the default"
        );

        // sample.dbc declares no attributes at all, which is not a claim of 0.
        let sample = std::fs::read_to_string("assets/sample.dbc").unwrap();
        assert_eq!(
            load_dbc_str(&sample).unwrap().messages[&0x100].cycle_us,
            None
        );
    }

    #[test]
    fn gen_msg_cycle_time_beats_cycle_time() {
        let db = load_dbc_str(CYCLE_DBC).unwrap();
        assert_eq!(
            db.messages[&256].cycle_us,
            Some(111_000),
            "the Vector name wins"
        );
        assert_eq!(
            db.messages[&257].cycle_us,
            Some(333_000),
            "legacy name still honoured"
        );
        assert_eq!(
            db.messages[&260].cycle_us,
            Some(77_000),
            "default follows the winner"
        );
    }

    #[test]
    fn a_zero_declared_cycle_means_event_triggered() {
        let db = load_dbc_str(CYCLE_DBC).unwrap();
        assert_eq!(
            db.messages[&258].cycle_us,
            Some(0),
            "0 is a real declaration, not the absence of one"
        );
        // A non-numeric attribute sharing the name must not masquerade as 0.
        assert_eq!(
            cycle_us_of(&AttributeValue::String("Cyclic".to_string())),
            None
        );
        assert_eq!(
            cycle_us_of(&AttributeValue::Int(-1)),
            None,
            "a negative cycle is nonsense"
        );
    }

    #[test]
    fn sample_dbc_parses() {
        let content = std::fs::read_to_string("assets/sample.dbc").unwrap();
        let table = load_dbc_str(&content).unwrap();
        assert_eq!(table.order.len(), 3);
        let engine = table.messages.get(&0x100).unwrap();
        assert_eq!(engine.name, "EngineStatus");
        assert_eq!(engine.signals.len(), 3);
        assert_eq!(engine.signals[0].name, "EngineSpeed");
        assert_eq!(table.message_name(0x320), Some("BrakeInfo"));
        assert_eq!(table.message_name(0x999), None);
        assert_eq!(table.nodes, vec!["EngineECU", "ChassisECU", "Dashboard"]);
        assert_eq!(engine.transmitter, "EngineECU");
        assert!(engine.signals[0].receivers.iter().any(|r| r == "Dashboard"));
        assert!(
            engine.signals[0]
                .receivers
                .iter()
                .any(|r| r == "ChassisECU")
        );
    }

    #[test]
    fn node_tx_and_rx_lookups() {
        let content = std::fs::read_to_string("assets/sample.dbc").unwrap();
        let table = load_dbc_str(&content).unwrap();
        assert_eq!(table.node_tx_ids("EngineECU"), vec![0x100]);
        assert_eq!(table.node_tx_ids("ChassisECU"), vec![0x200, 0x320]);
        assert!(table.node_tx_ids("Dashboard").is_empty());
        let rx = table.node_rx_signals("Dashboard");
        assert!(rx.iter().any(|(id, sig, sender)| *id == 0x100
            && sig == "EngineSpeed"
            && sender == "EngineECU"));
        assert!(
            rx.iter()
                .any(|(id, sig, _)| *id == 0x320 && sig == "BrakePressure")
        );
        assert_eq!(rx.len(), 5, "Dashboard receives all five signals");
        assert!(
            table
                .node_rx_signals("ChassisECU")
                .iter()
                .any(|(id, sig, _)| *id == 0x100 && sig == "ThrottlePos")
        );
    }

    #[test]
    fn decode_engine_speed() {
        let content = std::fs::read_to_string("assets/sample.dbc").unwrap();
        let table = load_dbc_str(&content).unwrap();
        // raw 12000 * 0.25 = 3000 rpm, little-endian 16 bit at bit 0
        let frame = frame_with(0x100, &[0xE0, 0x2E]);
        let sigs = table.decode_signals(&frame);
        assert_eq!(sigs[0].name, "EngineSpeed");
        assert!((sigs[0].phys - 3000.0).abs() < 1e-9);
    }

    #[test]
    fn encode_signal_roundtrip() {
        let content = std::fs::read_to_string("assets/sample.dbc").unwrap();
        let table = load_dbc_str(&content).unwrap();
        let mut data = [0u8; MAX_CAN_FD_LEN];
        assert!(table.encode_signal(0x100, "EngineSpeed", 3000.0, &mut data));
        let frame = frame_with(0x100, &data[..8]);
        let sigs = table.decode_signals(&frame);
        let speed = sigs.iter().find(|d| d.name == "EngineSpeed").unwrap();
        assert!((speed.phys - 3000.0).abs() < 0.5, "decoded {}", speed.phys);
        assert!(!table.encode_signal(0x100, "NoSuchSignal", 1.0, &mut data));
        assert!(!table.encode_signal(0x999, "EngineSpeed", 1.0, &mut data));
    }

    /// A multiplexed message with two groups sharing the same byte range, one
    /// static signal, and an `m2M` nested-switch marker, plus a plain message
    /// that must keep decoding everything.
    const MUX_DBC: &str = r#"VERSION "roxy-can mux test"

NS_ :

BU_: ECU

BO_ 400 Muxed: 8 ECU
 SG_ Switch M : 0|8@1+ (1,0) [0|0] "" ECU
 SG_ Always : 8|8@1+ (1,0) [0|0] "" ECU
 SG_ G1_A m1 : 16|16@1+ (0.1,0) [0|0] "" ECU
 SG_ G2_C m2 : 16|16@1+ (0.5,0) [0|0] "" ECU
 SG_ Nested m2M : 32|8@1+ (1,0) [0|0] "" ECU

BO_ 401 Plain: 8 ECU
 SG_ P1 : 0|8@1+ (1,0) [0|0] "" ECU
 SG_ P2 : 8|8@1+ (1,0) [0|0] "" ECU
"#;

    fn names(sigs: &[DecodedSignal]) -> Vec<&str> {
        sigs.iter().map(|d| d.name.as_str()).collect()
    }

    #[test]
    fn a_muxed_frame_decodes_only_the_active_group() {
        let table = load_dbc_str(MUX_DBC).unwrap();
        // Switch = 1 selects group 1: raw 100 * 0.1 = 10.0 in bytes 2-3.
        let sigs = table.decode_signals(&frame_with(400, &[1, 7, 100, 0, 0, 0, 0, 0]));
        assert_eq!(names(&sigs), ["Switch", "Always", "G1_A"]);
        let g1 = sigs.iter().find(|d| d.name == "G1_A").unwrap();
        assert!((g1.phys - 10.0).abs() < 1e-9);
    }

    #[test]
    fn switching_the_group_swaps_the_decoded_signals() {
        let table = load_dbc_str(MUX_DBC).unwrap();
        // Switch = 2 selects group 2; the same bytes now mean G2_C, not G1_A.
        let sigs = table.decode_signals(&frame_with(400, &[2, 7, 40, 0, 5, 0, 0, 0]));
        assert_eq!(names(&sigs), ["Switch", "Always", "G2_C", "Nested"]);
        let g2 = sigs.iter().find(|d| d.name == "G2_C").unwrap();
        assert!((g2.phys - 20.0).abs() < 1e-9, "40 * 0.5");
        assert!(
            !names(&sigs).contains(&"G1_A"),
            "the inactive group must vanish, not decode from the same bytes"
        );
    }

    #[test]
    fn an_unknown_switch_value_leaves_only_the_static_signals() {
        let table = load_dbc_str(MUX_DBC).unwrap();
        let sigs = table.decode_signals(&frame_with(400, &[9, 7, 1, 2, 3, 4, 5, 6]));
        assert_eq!(
            names(&sigs),
            ["Switch", "Always"],
            "no group matches, so only the switch and static signals remain"
        );
    }

    #[test]
    fn a_non_muxed_message_decodes_everything_unchanged() {
        let table = load_dbc_str(MUX_DBC).unwrap();
        assert!(table.messages[&401].switch_names.is_empty());
        assert!(!table.messages[&400].switch_names.is_empty());
        let sigs = table.decode_signals(&frame_with(401, &[1, 2, 3, 4, 5, 6, 7, 8]));
        assert_eq!(names(&sigs), ["P1", "P2"]);
    }

    #[test]
    fn a_nested_switch_marker_decodes_with_its_group() {
        let table = load_dbc_str(MUX_DBC).unwrap();
        let nested = table.messages[&400]
            .signals
            .iter()
            .find(|s| s.name == "Nested")
            .unwrap();
        assert_eq!(
            nested.mux_when.len(),
            1,
            "m2M is a member of group 2 as well as a switch"
        );
        assert_eq!(nested.mux_when[0].switch, "Switch");
        assert_eq!(nested.mux_when[0].ranges, [(2, 2)]);
        let off = table.decode_signals(&frame_with(400, &[1, 0, 0, 0, 9, 0, 0, 0]));
        assert!(!names(&off).contains(&"Nested"));
    }

    /// `SG_MUL_VAL_` gating: one switch with wide and split ranges, a nested
    /// chain like Vector's own extended-multiplexing sample, and a signal
    /// whose `m` marker contradicts its `SG_MUL_VAL_` line.
    const SG_MUL_VAL_DBC: &str = r#"VERSION "roxy-can extended mux test"

NS_ :

BU_: ECU

BO_ 430 Ext: 8 ECU
 SG_ Top M : 0|8@1+ (1,0) [0|0] "" ECU
 SG_ Range m1 : 8|8@1+ (1,0) [0|0] "" ECU
 SG_ Split m2 : 8|8@1+ (1,0) [0|0] "" ECU
 SG_ Override m1 : 8|8@1+ (1,0) [0|0] "" ECU
 SG_ Mid m1M : 16|8@1+ (1,0) [0|0] "" ECU
 SG_ Leaf m1 : 24|8@1+ (1,0) [0|0] "" ECU

SG_MUL_VAL_ 430 Range Top 1-3;
SG_MUL_VAL_ 430 Split Top 3-4, 9-10;
SG_MUL_VAL_ 430 Override Top 7-7;
SG_MUL_VAL_ 430 Leaf Mid 5-5;
"#;

    #[test]
    fn a_range_mapping_activates_over_a_span_of_switch_values() {
        let table = load_dbc_str(SG_MUL_VAL_DBC).unwrap();
        // Range is gated to 1-3 by its line, so 2 is in and 4 is out.
        let on = table.decode_signals(&frame_with(430, &[2, 0, 0, 0, 0, 0, 0, 0]));
        assert!(names(&on).contains(&"Range"));
        let off = table.decode_signals(&frame_with(430, &[4, 0, 0, 0, 0, 0, 0, 0]));
        assert!(!names(&off).contains(&"Range"));
    }

    #[test]
    fn a_signal_can_belong_to_several_switch_ranges() {
        let table = load_dbc_str(SG_MUL_VAL_DBC).unwrap();
        let split = table.messages[&430]
            .signals
            .iter()
            .find(|s| s.name == "Split")
            .unwrap();
        assert_eq!(split.mux_when[0].ranges, [(3, 4), (9, 10)]);
        for v in [3, 4, 9, 10] {
            let sigs = table.decode_signals(&frame_with(430, &[v, 0, 0, 0, 0, 0, 0, 0]));
            assert!(names(&sigs).contains(&"Split"), "switch {v} shows Split");
        }
        for v in [2, 5, 8] {
            let sigs = table.decode_signals(&frame_with(430, &[v, 0, 0, 0, 0, 0, 0, 0]));
            assert!(!names(&sigs).contains(&"Split"), "switch {v} hides Split");
        }
    }

    #[test]
    fn a_mul_val_line_supersedes_the_m_marker() {
        let table = load_dbc_str(SG_MUL_VAL_DBC).unwrap();
        // Override carries m1 but its SG_MUL_VAL_ line says 7-7 only.
        let sigs = table.decode_signals(&frame_with(430, &[1, 0, 0, 0, 0, 0, 0, 0]));
        assert!(
            !names(&sigs).contains(&"Override"),
            "the m1 marker must not resurrect the signal"
        );
        let sigs = table.decode_signals(&frame_with(430, &[7, 0, 0, 0, 0, 0, 0, 0]));
        assert!(names(&sigs).contains(&"Override"));
    }

    #[test]
    fn a_nested_chain_gates_on_both_switches() {
        let table = load_dbc_str(SG_MUL_VAL_DBC).unwrap();
        // Leaf needs Top=1 (via Mid's m1M marker) and Mid=5 (via its line).
        let both = table.decode_signals(&frame_with(430, &[1, 0, 5, 0, 0, 0, 0, 0]));
        assert!(names(&both).contains(&"Mid") && names(&both).contains(&"Leaf"));
        let wrong_nested = table.decode_signals(&frame_with(430, &[1, 0, 6, 0, 0, 0, 0, 0]));
        assert!(
            names(&wrong_nested).contains(&"Mid"),
            "Mid itself is active"
        );
        assert!(
            !names(&wrong_nested).contains(&"Leaf"),
            "the nested switch reads 6, so Leaf must hide"
        );
        let wrong_top = table.decode_signals(&frame_with(430, &[2, 0, 5, 0, 0, 0, 0, 0]));
        assert!(!names(&wrong_top).contains(&"Leaf"));
    }

    #[test]
    fn a_condition_naming_an_unknown_switch_still_shows_the_signal() {
        let table = load_dbc_str(
            r#"VERSION "roxy-can broken mux test"

NS_ :

BU_: ECU

BO_ 431 Broken: 8 ECU
 SG_ Orphan m1 : 0|8@1+ (1,0) [0|0] "" ECU

SG_MUL_VAL_ 431 Orphan NoSuchSwitch 1-1;
"#,
        )
        .unwrap();
        let sigs = table.decode_signals(&frame_with(431, &[9, 0, 0, 0, 0, 0, 0, 0]));
        assert_eq!(
            names(&sigs),
            ["Orphan"],
            "an ungatable signal falls back to always present"
        );
    }

    /// Vector databases put several independent switches in one message;
    /// can-dbc 10 parses that, and each `m` marker must gate against its own
    /// letter's switch.
    #[test]
    fn several_independent_switches_parse_and_gate_separately() {
        let table = load_dbc_str(
            r#"VERSION "roxy-can two switches test"

NS_ :

BU_: ECU

BO_ 432 Twin: 8 ECU
 SG_ MuxA M : 0|8@1+ (1,0) [0|0] "" ECU
 SG_ MuxB M : 8|8@1+ (1,0) [0|0] "" ECU
 SG_ A1 m1 : 16|8@1+ (1,0) [0|0] "" ECU
 SG_ B2 m2 : 24|8@1+ (1,0) [0|0] "" ECU

SG_MUL_VAL_ 432 A1 MuxA 1-1;
SG_MUL_VAL_ 432 B2 MuxB 2-2;
"#,
        )
        .unwrap();
        let both = table.decode_signals(&frame_with(432, &[1, 2, 0, 0, 0, 0, 0, 0]));
        assert_eq!(names(&both), ["MuxA", "MuxB", "A1", "B2"]);
        let neither = table.decode_signals(&frame_with(432, &[5, 5, 0, 0, 0, 0, 0, 0]));
        assert_eq!(
            names(&neither),
            ["MuxA", "MuxB"],
            "switches are always shown"
        );
    }

    /// A float declaration makes the raw bits an IEEE value instead of an
    /// integer, and the type tag says so.
    const FLOAT_DBC: &str = r#"VERSION "roxy-can float test"

NS_ :

BU_: ECU

BO_ 440 Floats: 8 ECU
 SG_ F32 : 0|32@1+ (1,0) [0|0] "" ECU
 SG_ F64 : 32|64@1+ (1,0) [0|0] "" ECU
 SG_ Bad : 0|16@1+ (1,0) [0|0] "" ECU

SIG_VALTYPE_ 440 F32 : 1;
SIG_VALTYPE_ 440 F64 : 2;
SIG_VALTYPE_ 440 Bad : 1;
"#;

    // can-dbc parses the SIG_VALTYPE_ value as 0 = integer, 1 = f32, 2 = f64.
    // Vector's own format documentation instead calls 0 a float and 1 a
    // double; this crate is the parser, so its convention is the one that
    // decides what we can read.

    #[test]
    fn a_float_signal_decodes_from_its_bit_pattern() {
        let table = load_dbc_str(FLOAT_DBC).unwrap();
        let mut data = [0u8; 4];
        data.copy_from_slice(&12.5f32.to_bits().to_le_bytes());
        let sigs = table.decode_signals(&frame_with(440, &data));
        let hit = sigs.iter().find(|d| d.name == "F32").unwrap();
        assert!((hit.phys - 12.5).abs() < 1e-9);
        assert_eq!(hit.type_tag, "f32");
        assert!(hit.label.is_none(), "floats have no enum labels");
    }

    #[test]
    fn a_double_signal_decodes_from_its_bit_pattern() {
        let table = load_dbc_str(FLOAT_DBC).unwrap();
        let mut data = [0u8; 12];
        data[4..12].copy_from_slice(&(-0.5f64).to_bits().to_le_bytes());
        let sigs = table.decode_signals(&frame_with(440, &data));
        let hit = sigs.iter().find(|d| d.name == "F64").unwrap();
        assert!((hit.phys - (-0.5)).abs() < 1e-9);
        assert_eq!(hit.type_tag, "f64");
    }

    #[test]
    fn a_float_declaration_on_a_mismatched_size_is_ignored() {
        let table = load_dbc_str(FLOAT_DBC).unwrap();
        let bad = table.messages[&440]
            .signals
            .iter()
            .find(|s| s.name == "Bad")
            .unwrap();
        assert!(!bad.is_float, "16 bits cannot hold an f32");
        assert_eq!(bad.type_tag, "u16");
    }

    #[test]
    fn a_float_signal_encodes_and_roundtrips() {
        let table = load_dbc_str(FLOAT_DBC).unwrap();
        let mut data = [0u8; 8];
        assert!(table.encode_signal(440, "F32", 3000.0, &mut data));
        let sigs = table.decode_signals(&frame_with(440, &data));
        let hit = sigs.iter().find(|d| d.name == "F32").unwrap();
        assert!((hit.phys - 3000.0).abs() < 1e-6);
    }

    #[test]
    fn a_signed_marker_still_names_the_integer_tag() {
        let table = load_dbc_str(
            r#"VERSION "roxy-can tags test"

NS_ :

BU_: ECU

BO_ 441 Tags: 8 ECU
 SG_ S8 : 0|8@1- (1,0) [0|0] "" ECU
 SG_ U24 : 8|24@1+ (1,0) [0|0] "" ECU
"#,
        )
        .unwrap();
        let tags: Vec<&str> = table.messages[&441]
            .signals
            .iter()
            .map(|s| s.type_tag.as_str())
            .collect();
        assert_eq!(tags, ["i8", "u24"]);
    }

    #[test]
    fn an_integer_value_prints_without_trailing_zeros() {
        assert_eq!(fmt_decoded("u16", 3000.0), "3000");
        assert_eq!(fmt_decoded("u16", 3000.25), "3000.25");
        assert_eq!(fmt_decoded("i8", -1.0), "-1");
        assert_eq!(fmt_decoded("u16", 0.25), "0.25");
    }

    #[test]
    fn a_float_value_prints_in_its_own_width() {
        assert_eq!(
            fmt_decoded("f32", 0.1),
            "0.1",
            "the f32 reading, not binary noise"
        );
        assert_eq!(fmt_decoded("f64", 12.5), "12.5");
    }

    #[test]
    fn the_value_cell_joins_only_the_parts_that_exist() {
        assert_eq!(
            fmt_signal_value(3000.0, "rpm", "u16", None),
            "3000 rpm [u16]"
        );
        assert_eq!(fmt_signal_value(3000.0, "", "u16", None), "3000 [u16]");
        assert_eq!(
            fmt_signal_value(2.0, "", "u8", Some("Gear_2")),
            "2 [u8] (Gear_2)"
        );
    }

    const VAL_DBC: &str = r#"VERSION "roxy-can val test"

NS_ :

BU_: ECU

BO_ 410 Enums: 8 ECU
 SG_ Gear : 0|8@1+ (1,0) [0|0] "" ECU
 SG_ Mode : 8|8@1- (1,0) [0|0] "" ECU
 SG_ Free : 16|8@1+ (1,0) [0|0] "" ECU

VAL_ 410 Gear 2 "Gear_2" 1 "Gear_1" 0 "Neutral";
VAL_ 410 Mode -1 "Reverse" 0 "Forward";
"#;

    #[test]
    fn an_enum_label_matches_the_raw_value() {
        let table = load_dbc_str(VAL_DBC).unwrap();
        let sigs = table.decode_signals(&frame_with(410, &[2, 0, 3, 0, 0, 0, 0, 0]));
        let gear = sigs.iter().find(|d| d.name == "Gear").unwrap();
        assert_eq!(gear.label.as_deref(), Some("Gear_2"));
        let mode = sigs.iter().find(|d| d.name == "Mode").unwrap();
        assert_eq!(mode.label.as_deref(), Some("Forward"));
        let free = sigs.iter().find(|d| d.name == "Free").unwrap();
        assert_eq!(free.label, None, "no VAL_ line, no label");
    }

    #[test]
    fn a_value_without_a_matching_entry_stays_unlabelled() {
        let table = load_dbc_str(VAL_DBC).unwrap();
        let sigs = table.decode_signals(&frame_with(410, &[5, 0, 0, 0, 0, 0, 0, 0]));
        let gear = sigs.iter().find(|d| d.name == "Gear").unwrap();
        assert_eq!(gear.label, None);
        assert!((gear.phys - 5.0).abs() < 1e-9, "the number still shows");
    }

    #[test]
    fn a_signed_signal_can_carry_a_negative_label() {
        let table = load_dbc_str(VAL_DBC).unwrap();
        // 0xFF in an 8-bit signed signal is raw -1, which VAL_ names "Reverse".
        let sigs = table.decode_signals(&frame_with(410, &[0, 0xFF, 0, 0, 0, 0, 0, 0]));
        let mode = sigs.iter().find(|d| d.name == "Mode").unwrap();
        assert_eq!(mode.label.as_deref(), Some("Reverse"));
        assert!((mode.phys - (-1.0)).abs() < 1e-9);
    }

    /// `VAL_ <id> <signal> <TableName>;` references a `VAL_TABLE_` entry, but
    /// can-dbc-pest has no production for that form, so such files cannot be
    /// opened at all. Pin the behaviour so a future crate upgrade is noticed.
    #[test]
    fn a_named_table_reference_is_a_parse_error_in_can_dbc() {
        let db = load_dbc_str(
            r#"VERSION "roxy-can named table test"

NS_ :

BU_: ECU

BO_ 420 Refs: 8 ECU
 SG_ S : 0|8@1+ (1,0) [0|0] "" ECU

VAL_TABLE_ States 0 "Off" 1 "On";
VAL_ 420 S States;
"#,
        );
        assert!(db.is_err(), "the named-table VAL_ form is unparseable");
    }
}
