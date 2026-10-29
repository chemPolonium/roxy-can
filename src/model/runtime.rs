//! Plan-driven decoding and encoding.
//!
//! `BusCore` calls these for every subscribed, expanded or generated frame.
//! They take only a flattened [`DecodePlan`] and the payload bytes, so the
//! per-frame path never walks the model tree.

use super::*;
use crate::decode::{extract_raw, from_physical, pack_raw, to_physical};

/// One E2E header reading: the counter the protected PDU carried and the
/// Data ID the document configured. The CRC is deliberately not evaluated.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct E2eReading {
    pub profile: u32,
    pub counter: u8,
    pub data_id: Option<u32>,
}

pub fn decode_e2e(plan: &DecodePlan, payload: &[u8]) -> Vec<E2eReading> {
    let total_bits = (payload.len() as u64) * 8;
    plan.e2e
        .iter()
        .filter_map(|spec| {
            let end = u64::from(spec.counter_bit_offset) + u64::from(spec.counter_bits);
            if end > total_bits || spec.counter_bits == 0 || spec.counter_bits > 8 {
                return None;
            }
            let raw = extract_raw(
                payload,
                u64::from(spec.counter_bit_offset),
                u64::from(spec.counter_bits),
                false,
            );
            Some(E2eReading {
                profile: spec.profile,
                counter: raw as u8,
                data_id: spec.data_id,
            })
        })
        .collect()
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DecodedSignal {
    pub signal: SignalId,
    pub raw: u64,
    /// Physical reading; `None` when the bits fall outside the payload or the
    /// float width is not 32/64.
    pub physical: Option<f64>,
}

/// Whether the container header names the contained PDU this plan entry was
/// expanded from and says its bytes are long enough for the entry that ends
/// at `end_bit`. A document without a header gate reads the child wherever
/// it sits, exactly as before the header existed.
fn container_allows(payload: &[u8], gate: &ContainerGate, end_bit: u64) -> bool {
    let (id_bytes, len_bytes) = match gate.header_type {
        ContainerHeaderType::Short => (3usize, 1usize),
        ContainerHeaderType::Long => (4usize, 4usize),
        ContainerHeaderType::None => return true,
    };
    let start = u64::from(gate.header_start_bit);
    // The container header is a byte stream field, not a CAN Motorola signal:
    // its bytes are one after another, most significant first.
    if !start.is_multiple_of(8) {
        return false;
    }
    let start = (start / 8) as usize;
    let Some(header) = payload.get(start..start + id_bytes + len_bytes) else {
        return false;
    };
    let id = header[..id_bytes]
        .iter()
        .fold(0u64, |acc, byte| (acc << 8) | u64::from(*byte));
    if id != u64::from(gate.header_id) {
        return false;
    }
    let length = header[id_bytes..]
        .iter()
        .fold(0u64, |acc, byte| (acc << 8) | u64::from(*byte));
    let header_bits = u64::from(gate.header_start_bit) + (id_bytes + len_bytes) as u64 * 8;
    end_bit <= header_bits + length * 8
}

pub fn decode_payload(_comm: &Comm, plan: &DecodePlan, payload: &[u8]) -> Vec<DecodedSignal> {
    let total_bits = (payload.len() as u64) * 8;
    let mut selector_values: std::collections::BTreeMap<SignalId, u64> =
        std::collections::BTreeMap::new();
    for read in &plan.reads {
        if let Some(gate) = &read.container
            && !container_allows(payload, gate, read.bit_offset as u64 + read.bit_len as u64)
        {
            continue;
        }
        if read.bit_offset as u64 + read.bit_len as u64 > total_bits {
            continue;
        }
        let raw = extract_raw(
            payload,
            read.bit_offset as u64,
            read.bit_len as u64,
            read.byte_order == ByteOrder::BigEndian,
        );
        if read.mux.is_empty() {
            selector_values.insert(read.signal, raw);
        }
    }
    let selector_value = |condition: &MuxCondition| -> Option<u64> {
        match &condition.selector {
            MuxSelector::Signal { signal } => selector_values.get(signal).copied(),
            MuxSelector::Field {
                start_bit,
                length,
                byte_order,
            } => {
                if u64::from(*start_bit) + u64::from(*length) > total_bits {
                    return None;
                }
                Some(extract_raw(
                    payload,
                    u64::from(*start_bit),
                    u64::from(*length),
                    *byte_order == ByteOrder::BigEndian,
                ))
            }
        }
    };
    let mut out = Vec::with_capacity(plan.reads.len());
    for read in &plan.reads {
        if let Some(gate) = &read.container
            && !container_allows(payload, gate, read.bit_offset as u64 + read.bit_len as u64)
        {
            continue;
        }
        if !read.mux.is_empty() {
            let mut present = true;
            for condition in &read.mux {
                let Some(selector) = selector_value(condition) else {
                    present = false;
                    break;
                };
                let in_range = condition
                    .ranges
                    .iter()
                    .any(|(lo, hi)| selector >= u64::from(*lo) && selector <= u64::from(*hi));
                if !in_range {
                    present = false;
                    break;
                }
            }
            if !present {
                continue;
            }
        }
        let fits = read.bit_offset as u64 + read.bit_len as u64 <= total_bits;
        let raw = if fits {
            extract_raw(
                payload,
                read.bit_offset as u64,
                read.bit_len as u64,
                read.byte_order == ByteOrder::BigEndian,
            )
        } else {
            0
        };
        let physical = if !fits {
            None
        } else if read.float {
            // Float payloads keep their bit pattern; a transformation still
            // maps the value the frame carries into the application's units.
            let value = match read.bit_len {
                32 => Some(f32::from_bits(raw as u32) as f64),
                64 => Some(f64::from_bits(raw)),
                _ => None,
            };
            value.map(|v| read.transform.map_or(v, |t| t.decode(v)))
        } else {
            let value = to_physical(
                raw,
                read.bit_len as u64,
                read.signed,
                read.factor,
                read.offset,
            );
            Some(read.transform.map_or(value, |t| t.decode(value)))
        };
        out.push(DecodedSignal {
            signal: read.signal,
            raw,
            physical,
        });
    }
    out
}

/// Writes the physical values into `payload`. Returns how many signals were
/// written; signals outside every write plan are ignored.
pub fn encode_payload(
    _comm: &Comm,
    plan: &DecodePlan,
    values: &[(SignalId, f64)],
    payload: &mut [u8],
) -> usize {
    let total_bits = (payload.len() as u64) * 8;
    let mut written = 0;
    for write in &plan.writes {
        if let Some(gate) = &write.container
            && !container_allows(
                payload,
                gate,
                write.bit_offset as u64 + write.bit_len as u64,
            )
        {
            continue;
        }
        let Some(&(_, physical)) = values.iter().find(|(signal, _)| *signal == write.signal) else {
            continue;
        };
        if write.bit_offset as u64 + write.bit_len as u64 > total_bits {
            continue;
        }
        let physical = write.transform.map_or(physical, |t| t.encode(physical));
        let raw = if write.float {
            match write.bit_len {
                32 => (physical as f32).to_bits() as u64,
                64 => physical.to_bits(),
                _ => continue,
            }
        } else {
            from_physical(
                physical,
                write.bit_len as u64,
                write.signed,
                write.factor,
                write.offset,
            )
        };
        pack_raw(
            payload,
            write.bit_offset as u64,
            write.bit_len as u64,
            write.byte_order == ByteOrder::BigEndian,
            raw,
        );
        written += 1;
    }
    written
}
