pub fn extract_raw(data: &[u8], start_bit: u64, size: u64, big_endian: bool) -> u64 {
    if size == 0 || size > 64 {
        return 0;
    }
    let n = data.len();
    let mut raw: u64 = 0;
    if !big_endian {
        for i in 0..size {
            let bit = start_bit + i;
            let byte = (bit / 8) as usize;
            if byte < n && (data[byte] >> (bit % 8)) & 1 == 1 {
                raw |= 1 << i;
            }
        }
    } else {
        let mut bit = start_bit;
        for i in 0..size {
            let byte = (bit / 8) as usize;
            if byte < n && (data[byte] >> (bit % 8)) & 1 == 1 {
                raw |= 1 << (size - 1 - i);
            }
            if bit.is_multiple_of(8) {
                bit += 15;
            } else {
                bit -= 1;
            }
        }
    }
    raw
}

pub fn pack_raw(data: &mut [u8], start_bit: u64, size: u64, big_endian: bool, raw: u64) {
    if size == 0 || size > 64 {
        return;
    }
    let n = data.len();
    if !big_endian {
        for i in 0..size {
            let bit = start_bit + i;
            let byte = (bit / 8) as usize;
            if byte < n {
                let mask = 1u8 << (bit % 8);
                if (raw >> i) & 1 == 1 {
                    data[byte] |= mask;
                } else {
                    data[byte] &= !mask;
                }
            }
        }
    } else {
        let mut bit = start_bit;
        for i in 0..size {
            let byte = (bit / 8) as usize;
            if byte < n {
                let mask = 1u8 << (bit % 8);
                if (raw >> (size - 1 - i)) & 1 == 1 {
                    data[byte] |= mask;
                } else {
                    data[byte] &= !mask;
                }
            }
            if bit.is_multiple_of(8) {
                bit += 15;
            } else {
                bit -= 1;
            }
        }
    }
}

pub fn to_physical(raw: u64, size: u64, signed: bool, factor: f64, offset: f64) -> f64 {
    let v = if signed && size > 0 && size <= 64 {
        let shift = 64 - size;
        ((raw << shift) as i64 >> shift) as f64
    } else {
        raw as f64
    };
    v * factor + offset
}

pub fn from_physical(phys: f64, size: u64, signed: bool, factor: f64, offset: f64) -> u64 {
    let v = ((phys - offset) / factor).round() as i64;
    if size == 0 || size >= 64 {
        return v as u64;
    }
    let max = if signed {
        (1i64 << (size - 1)) - 1
    } else {
        (1i64 << size) - 1
    };
    let min = if signed { -(1i64 << (size - 1)) } else { 0 };
    v.clamp(min, max) as u64 & ((1u64 << size) - 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn intel_16bit() {
        let data = [0x34, 0x12, 0, 0, 0, 0, 0, 0];
        assert_eq!(extract_raw(&data, 0, 16, false), 0x1234);
    }

    #[test]
    fn motorola_16bit_aligned() {
        let data = [0x12, 0x34, 0, 0, 0, 0, 0, 0];
        assert_eq!(extract_raw(&data, 7, 16, true), 0x1234);
    }

    #[test]
    fn motorola_cross_byte_nibble() {
        let data = [0x05, 0xA0, 0, 0, 0, 0, 0, 0];
        assert_eq!(extract_raw(&data, 4, 8, true), 0x2D);
    }

    #[test]
    fn signed_extend() {
        assert_eq!(to_physical(0xFFF, 12, true, 1.0, 0.0), -1.0);
        assert_eq!(to_physical(0x7FF, 12, true, 1.0, 0.0), 2047.0);
        assert_eq!(to_physical(0xFF, 8, true, 2.0, 1.0), -1.0);
    }

    #[test]
    fn pack_extract_roundtrip() {
        for &big in &[false, true] {
            let start = if big { 7 } else { 0 };
            let mut data = [0u8; 8];
            pack_raw(&mut data, start, 16, big, 0xBEEF);
            assert_eq!(extract_raw(&data, start, 16, big), 0xBEEF);
        }
        let mut data = [0xFF; 8];
        pack_raw(&mut data, 4, 8, true, 0x2D);
        assert_eq!(extract_raw(&data, 4, 8, true), 0x2D);
    }

    #[test]
    fn physical_roundtrip() {
        let raw = from_physical(3000.0, 16, false, 0.25, 0.0);
        assert_eq!(raw, 12000);
        assert!((to_physical(raw, 16, false, 0.25, 0.0) - 3000.0).abs() < 0.25);
    }

    #[test]
    fn fd_signal_past_byte_8() {
        // A 16-bit signal at byte 8 (bit 64) lives only in CAN FD payloads.
        let mut data = [0u8; 16];
        pack_raw(&mut data, 64, 16, false, 0xCAFE);
        assert_eq!(extract_raw(&data, 64, 16, false), 0xCAFE);
        // Reading the same bits from an 8-byte buffer yields 0 (out of range).
        let short = [0u8; 8];
        assert_eq!(extract_raw(&short, 64, 16, false), 0);
    }
}
