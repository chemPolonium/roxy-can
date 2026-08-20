use std::collections::HashMap;

use can_dbc::{ByteOrder, MessageId, NumericValue, ValueType};

use crate::can::frame::CanFrame;
use crate::decode;

fn numeric(v: &NumericValue) -> f64 {
    match v {
        NumericValue::Uint(u) => *u as f64,
        NumericValue::Int(i) => *i as f64,
        NumericValue::Double(d) => *d,
    }
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
        for msg in &db.messages {
            let id = match msg.id {
                MessageId::Standard(i) => i as u32,
                MessageId::Extended(i) => i,
            };
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
    pub fn encode_signal(&self, id: u32, name: &str, phys: f64, data: &mut [u8; 8]) -> bool {
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
                let raw = decode::extract_raw(&frame.data, s.start_bit, s.size, s.big_endian);
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
    use crate::can::frame::{CanFrame, Direction};

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
        let frame = CanFrame {
            t_us: 0,
            channel: 0,
            id: 0x100,
            extended: false,
            dlc: 8,
            data: [0xE0, 0x2E, 0, 0, 0, 0, 0, 0],
            dir: Direction::Rx,
        };
        let sigs = table.decode_signals(&frame);
        assert_eq!(sigs[0].0, "EngineSpeed");
        assert!((sigs[0].1 - 3000.0).abs() < 1e-9);
    }

    #[test]
    fn encode_signal_roundtrip() {
        let content = std::fs::read_to_string("assets/sample.dbc").unwrap();
        let table = load_dbc_str(&content).unwrap();
        let mut data = [0u8; 8];
        assert!(table.encode_signal(0x100, "EngineSpeed", 3000.0, &mut data));
        let frame = CanFrame {
            t_us: 0,
            channel: 0,
            id: 0x100,
            extended: false,
            dlc: 8,
            data,
            dir: Direction::Rx,
        };
        let sigs = table.decode_signals(&frame);
        let (_, speed, _) = sigs.iter().find(|(n, _, _)| n == "EngineSpeed").unwrap();
        assert!((*speed - 3000.0).abs() < 0.5, "decoded {speed}");
        assert!(!table.encode_signal(0x100, "NoSuchSignal", 1.0, &mut data));
        assert!(!table.encode_signal(0x999, "EngineSpeed", 1.0, &mut data));
    }
}
