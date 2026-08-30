use std::collections::HashMap;

use can_dbc::{AttributeValue, ByteOrder, MessageId, NumericValue, ValueType};

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
}

pub struct SymbolTable {
    pub messages: HashMap<u32, MessageInfo>,
    pub order: Vec<u32>,
    pub nodes: Vec<String>,
}

impl SymbolTable {
    pub fn from_dbc(db: &can_dbc::Dbc) -> Self {
        let mut messages = HashMap::new();
        let mut order = Vec::new();
        let cycles = declared_cycles(db);
        for msg in &db.messages {
            let id = msg_id_of(&msg.id);
            let signals = msg
                .signals
                .iter()
                .map(|s| SignalInfo {
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
                })
                .collect();
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
                },
            );
            order.push(id);
        }
        order.sort_unstable();
        order.dedup();
        let nodes = db.nodes.iter().map(|n| n.0.clone()).collect();
        SymbolTable {
            messages,
            order,
            nodes,
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
        let raw = decode::from_physical(phys, s.size, s.signed, s.factor, s.offset);
        decode::pack_raw(data, s.start_bit, s.size, s.big_endian, raw);
        true
    }

    pub fn decode_signals(&self, frame: &CanFrame) -> Vec<(String, f64, String)> {
        let Some(msg) = self.messages.get(&frame.id) else {
            return Vec::new();
        };
        msg.signals
            .iter()
            .map(|s| {
                let raw = decode::extract_raw(frame.payload(), s.start_bit, s.size, s.big_endian);
                let phys = decode::to_physical(raw, s.size, s.signed, s.factor, s.offset);
                (s.name.clone(), phys, s.unit.clone())
            })
            .collect()
    }
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
        assert_eq!(sigs[0].0, "EngineSpeed");
        assert!((sigs[0].1 - 3000.0).abs() < 1e-9);
    }

    #[test]
    fn encode_signal_roundtrip() {
        let content = std::fs::read_to_string("assets/sample.dbc").unwrap();
        let table = load_dbc_str(&content).unwrap();
        let mut data = [0u8; MAX_CAN_FD_LEN];
        assert!(table.encode_signal(0x100, "EngineSpeed", 3000.0, &mut data));
        let frame = frame_with(0x100, &data[..8]);
        let sigs = table.decode_signals(&frame);
        let (_, speed, _) = sigs.iter().find(|(n, _, _)| n == "EngineSpeed").unwrap();
        assert!((*speed - 3000.0).abs() < 0.5, "decoded {speed}");
        assert!(!table.encode_signal(0x100, "NoSuchSignal", 1.0, &mut data));
        assert!(!table.encode_signal(0x999, "EngineSpeed", 1.0, &mut data));
    }
}
