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
}

pub struct MessageInfo {
    pub name: String,
    pub dlc: u64,
    pub signals: Vec<SignalInfo>,
}

pub struct SymbolTable {
    pub messages: HashMap<u32, MessageInfo>,
    pub order: Vec<u32>,
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
                })
                .collect();
            messages.insert(
                id,
                MessageInfo {
                    name: msg.name.clone(),
                    dlc: msg.size,
                    signals,
                },
            );
            order.push(id);
        }
        order.sort_unstable();
        order.dedup();
        SymbolTable { messages, order }
    }

    pub fn message_name(&self, id: u32) -> Option<&str> {
        self.messages.get(&id).map(|m| m.name.as_str())
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
}
