//! Plan-driven decoding and encoding.
//!
//! `BusCore` calls these for every subscribed, expanded or generated frame.
//! They take only a flattened [`DecodePlan`] and the payload bytes, so the
//! per-frame path never walks the model tree.

use super::*;
use crate::decode::{extract_raw, from_physical, pack_raw, to_physical};

/// One E2E header reading: counter, Data ID and CRC verdict when the profile
/// and CRC position are known.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct E2eReading {
    pub profile: u32,
    pub counter: u8,
    pub data_id: Option<u32>,
    pub crc: Option<u64>,
    pub crc_ok: Option<bool>,
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
            let (crc, crc_ok) = match (spec.crc_bit_offset, spec.crc_bits) {
                (Some(offset), Some(bits)) if bits % 8 == 0 && offset % 8 == 0 => {
                    let width = usize::try_from(bits / 8).ok().unwrap_or(0);
                    let start = usize::try_from(offset / 8).ok().unwrap_or(usize::MAX);
                    let end = start.saturating_add(width);
                    if width == 0 || end > payload.len() {
                        (None, None)
                    } else {
                        let actual =
                            extract_raw(payload, u64::from(offset), u64::from(bits), false);
                        let mut data = Vec::with_capacity(payload.len() - width);
                        data.extend_from_slice(&payload[..start]);
                        data.extend_from_slice(&payload[end..]);
                        let expected = e2e_crc(spec.profile, &data).filter(|_| {
                            matches!(
                                (spec.profile, bits),
                                (1 | 2, 8)
                                    | (4, 32)
                                    | (5, 16)
                                    | (7, 64)
                                    | (11, 8)
                                    | (22, 32)
                                    | (44, 64)
                            )
                        });
                        (
                            Some(actual),
                            Some(expected.is_some_and(|value| value == actual)),
                        )
                    }
                }
                _ => (None, None),
            };
            Some(E2eReading {
                profile: spec.profile,
                counter: raw as u8,
                data_id: spec.data_id,
                crc,
                crc_ok,
            })
        })
        .collect()
}

fn e2e_crc(profile: u32, data: &[u8]) -> Option<u64> {
    Some(match profile {
        1 => u64::from(crc8(data, 0x1D, 0xFF, 0xFF)),
        2 => u64::from(crc8(data, 0x2F, 0xFF, 0xFF)),
        4 => u64::from(crc32_p4(data)),
        5 => u64::from(crc16_ccitt(data)),
        7 | 44 => crc64_ecma(data),
        11 => u64::from(crc8(data, 0x1D, 0xFF, 0xFF)),
        22 => u64::from(crc32_p4(data)),
        _ => return None,
    })
}

fn crc8(data: &[u8], poly: u8, init: u8, xorout: u8) -> u8 {
    let mut crc = init;
    for byte in data {
        crc ^= *byte;
        for _ in 0..8 {
            crc = if crc & 0x80 != 0 {
                (crc << 1) ^ poly
            } else {
                crc << 1
            };
        }
    }
    crc ^ xorout
}

fn crc16_ccitt(data: &[u8]) -> u16 {
    let mut crc = 0xFFFFu16;
    for byte in data {
        crc ^= u16::from(*byte) << 8;
        for _ in 0..8 {
            crc = if crc & 0x8000 != 0 {
                (crc << 1) ^ 0x1021
            } else {
                crc << 1
            };
        }
    }
    crc
}

fn crc32_p4(data: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for byte in data {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            crc = if crc & 1 != 0 {
                (crc >> 1) ^ 0xF4AC_FB13
            } else {
                crc >> 1
            };
        }
    }
    crc ^ 0xFFFF_FFFF
}

fn crc64_ecma(data: &[u8]) -> u64 {
    let mut crc = 0u64;
    for byte in data {
        crc ^= u64::from(*byte) << 56;
        for _ in 0..8 {
            crc = if crc & 0x8000_0000_0000_0000 != 0 {
                (crc << 1) ^ 0x42F0_E1EB_A9EA_3693
            } else {
                crc << 1
            };
        }
    }
    crc
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
        if let Some(bit) = read.update_bit
            && (u64::from(bit) >= total_bits || payload[bit as usize / 8] & (1 << (bit % 8)) == 0)
        {
            // The sender did not mark this signal as updated in this frame, so
            // it carries no new value even though its bits are readable.
            continue;
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
        if let Some(bit) = write.update_bit
            && u64::from(bit) < total_bits
        {
            // A written signal flags itself as updated; bit 0 of its byte is
            // the LSB, the same convention the signal start positions use.
            pack_raw(payload, u64::from(bit), 1, false, 1);
        }
        written += 1;
    }
    written
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn e2e_crc_known_vector() {
        assert_eq!(crc8(b"123456789", 0x1D, 0xFF, 0xFF), 0x4B);
        assert_eq!(crc64_ecma(b"123456789"), 0x6C40_DF5F_0B49_7347);
    }

    #[test]
    fn decode_e2e_checks_profile1_crc() {
        let mut plan = DecodePlan::default();
        plan.e2e.push(E2ePlan {
            profile: 1,
            counter_bit_offset: 8,
            counter_bits: 8,
            data_id: Some(1),
            crc_bit_offset: Some(0),
            crc_bits: Some(8),
        });
        let mut payload = vec![0u8, 3, 0x11, 0x22];
        payload[0] = crc8(&payload[1..], 0x1D, 0xFF, 0xFF);
        let readings = decode_e2e(&plan, &payload);
        assert_eq!(readings.len(), 1);
        assert_eq!(readings[0].counter, 3);
        assert_eq!(readings[0].crc_ok, Some(true));
        payload[2] ^= 0x01;
        assert_eq!(decode_e2e(&plan, &payload)[0].crc_ok, Some(false));
    }
}
