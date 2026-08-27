use std::fs::File;
use std::io::{BufWriter, Write};

use crate::can::frame::{
    dlc2len, CanFrame, Direction, FrameFlags, MAX_CAN_FD_LEN,
};

pub struct AscWriter {
    w: BufWriter<File>,
}

impl AscWriter {
    pub fn new(path: &str) -> std::io::Result<Self> {
        let file = File::create(path)?;
        let mut w = BufWriter::new(file);
        // Vector/CANoe header. `base hex` + absolute timestamps; FD frames are
        // marked per-line (the `CANFD` token), so a single header serves both
        // classic and FD traffic.
        let date = chrono::Local::now().format("%a %b %d %H:%M:%S%.3f %Y");
        writeln!(w, "date {date}")?;
        writeln!(w, "base hex  timestamps absolute")?;
        writeln!(w, "internal events logged")?;
        writeln!(w, "Begin Triggerblock {date}")?;
        writeln!(w, "0.000000 Start of measurement")?;
        Ok(AscWriter { w })
    }

    pub fn write(&mut self, f: &CanFrame) -> std::io::Result<()> {
        let t = f.t_us as f64 / 1e6;
        let dir = match f.dir {
            Direction::Rx => "Rx",
            Direction::Tx => "Tx",
        };
        if f.is_error() {
            // Vector emits error frames with a zero ID and no payload; the
            // trailing 0 is a placeholder DLC so naive splitters keep the
            // field count aligned with a data line.
            return writeln!(
                self.w,
                "{t:.6} {:>3} 000             {:<4} e 0",
                f.channel + 1,
                dir,
            );
        }
        let id = format!("{:X}{}", f.id, if f.extended { "x" } else { "" });
        if f.is_remote() {
            // Remote frames request a payload by ID; DLC records the
            // requested length and no data bytes follow.
            return writeln!(
                self.w,
                "{t:.6} {:>3} {:<15} {:<4} r {:x}",
                f.channel + 1,
                id,
                dir,
                f.dlc_code(),
            );
        }
        let data: String = f
            .payload()
            .iter()
            .map(|b| format!("{b:02X}"))
            .collect::<Vec<_>>()
            .join(" ");
        if f.is_fd() {
            // CANoe native FD layout (no symbolic name): trailing duration,
            // frame-length, flags, crc and four bit-timing fields are unknown
            // to us and written as zero placeholders.
            writeln!(
                self.w,
                "{t:.6} CANFD {:>3} {:<4} {:>8} {} {} {:x} {:>2} {data} \
                 {:>8} {:>4} {:>8X} {:>8} {:>8} {:>8} {:>8} {:>8}",
                f.channel + 1,
                dir,
                id,
                f.brs() as u8,
                f.esi() as u8,
                f.dlc_code(),
                f.len,
                0,
                0,
                0u32,
                0,
                0,
                0,
                0,
                0,
            )
        } else {
            // Classic data frame: `<t> <ch> <id>[x] <dir> d <dlc:x> <data>`.
            // For len <= 8 the DLC code equals the byte count, so files written
            // by older versions still parse.
            writeln!(
                self.w,
                "{t:.6} {:>3} {:<15} {:<4} d {:x} {data}",
                f.channel + 1,
                id,
                dir,
                f.dlc_code(),
            )
        }
    }

    pub fn finish(mut self) -> std::io::Result<()> {
        writeln!(self.w, "End TriggerBlock")?;
        self.w.flush()
    }
}

/// Parses a Vector/CANoe `.asc` log. Handles classic frames (with the `d`/`r`
/// frame-type token), CAN FD frames (`CANFD` line marker) and, for
/// backward-compatibility, the older type-token-less layout this tool emitted
/// before the Vector rewrite. Remote and error frames are recognised and
/// skipped cleanly.
pub fn parse_asc(content: &str) -> Vec<CanFrame> {
    let mut frames = Vec::new();
    let mut base: u32 = 16;
    for raw in content.lines() {
        let line = raw.trim();
        if line.is_empty() {
            continue;
        }
        if line.len() > 5 && line[..5].eq_ignore_ascii_case("base ") {
            base = if line.to_ascii_lowercase().contains("dec") {
                10
            } else {
                16
            };
            continue;
        }
        let toks: Vec<&str> = line.split_whitespace().collect();
        if toks.len() < 5 {
            continue;
        }
        let Ok(t) = toks[0].parse::<f64>() else {
            continue;
        };
        let t_us = (t * 1e6).round() as u64;
        let frame = if toks[1].eq_ignore_ascii_case("CANFD") {
            parse_fd(t_us, &toks, base)
        } else {
            parse_classic(t_us, &toks, base)
        };
        if let Some(f) = frame {
            frames.push(f);
        }
    }
    frames
}

fn parse_id(s: &str, base: u32) -> Option<(u32, bool)> {
    match s.strip_suffix(['x', 'X']) {
        Some(h) => Some((u32::from_str_radix(h, base).ok()?, true)),
        None => Some((u32::from_str_radix(s, base).ok()?, false)),
    }
}

fn parse_dir(s: &str) -> Option<Direction> {
    match s.to_ascii_lowercase().as_str() {
        "tx" => Some(Direction::Tx),
        "rx" => Some(Direction::Rx),
        _ => None,
    }
}

fn read_data(toks: &[&str], base: u32, count: usize) -> ([u8; MAX_CAN_FD_LEN], usize) {
    let mut data = [0u8; MAX_CAN_FD_LEN];
    let mut n = 0;
    for tok in toks.iter().take(count.min(MAX_CAN_FD_LEN)) {
        match u8::from_str_radix(tok, base).ok() {
            Some(b) => {
                data[n] = b;
                n += 1;
            }
            None => break,
        }
    }
    (data, n)
}

fn parse_classic(t_us: u64, toks: &[&str], base: u32) -> Option<CanFrame> {
    let channel: u32 = toks[1].parse().ok()?;
    let (id, extended) = parse_id(toks[2], base)?;
    let dir = parse_dir(toks[3])?;
    // Vector inserts a frame-type token (`d` data / `r` remote / `e` error)
    // before the DLC; the legacy roxy-can layout omits it.
    let (frametype, dlc_idx) = match toks[4].to_ascii_lowercase().as_str() {
        "d" | "r" | "e" => (toks[4].to_ascii_lowercase(), 5),
        _ => ("d".to_string(), 4),
    };
    let (flags, len, data) = match frametype.as_str() {
        "e" => (FrameFlags::ERROR, 0u8, [0u8; MAX_CAN_FD_LEN]),
        "r" => {
            let code = u32::from_str_radix(toks.get(dlc_idx)?, base).ok()?;
            (FrameFlags::RTR, dlc2len(code as u8), [0u8; MAX_CAN_FD_LEN])
        }
        _ => {
            let code = u32::from_str_radix(toks.get(dlc_idx)?, base).ok()?;
            let n = dlc2len(code as u8);
            let (d, _m) = read_data(toks.get(dlc_idx + 1..)?, base, n as usize);
            (FrameFlags::NONE, n, d)
        }
    };
    // Error frames have no identifier on the wire; normalize to 0 so
    // downstream aggregation and DBC lookup stay well-defined.
    let id = if flags.contains(FrameFlags::ERROR) {
        0
    } else {
        id
    };
    Some(CanFrame {
        t_us,
        channel: channel.saturating_sub(1) as u8,
        id,
        extended,
        len,
        data,
        dir,
        flags,
    })
}

fn parse_fd(t_us: u64, toks: &[&str], base: u32) -> Option<CanFrame> {
    let channel: u32 = toks[2].parse().ok()?;
    let dir = parse_dir(toks[3])?;
    let (id, extended) = parse_id(toks[4], base)?;
    // CANoe's native FD line has no symbolic-name column; python-can's may
    // insert one. A non-numeric token right after the id is that name.
    let mut p = 5;
    if p < toks.len() && toks[p].parse::<u32>().is_err() {
        p += 1;
    }
    let brs = toks.get(p)?.parse::<u32>().ok()?;
    let esi = toks.get(p + 1)?.parse::<u32>().ok()?;
    let code = u32::from_str_radix(toks.get(p + 2)?, base).ok()?;
    let data_len = toks.get(p + 3)?.parse::<usize>().ok()?;
    // The recorded data_length is authoritative over the DLC code.
    let len = dlc2len(code as u8).max(data_len as u8).min(MAX_CAN_FD_LEN as u8);
    let (data, _n) = read_data(toks.get(p + 4..)?, base, data_len);
    let mut flags = FrameFlags::FD;
    if brs == 1 {
        flags = flags.union(FrameFlags::BRS);
    }
    if esi == 1 {
        flags = flags.union(FrameFlags::ESI);
    }
    Some(CanFrame {
        t_us,
        channel: channel.saturating_sub(1) as u8,
        id,
        extended,
        len,
        data,
        dir,
        flags,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn classic(id: u32, bytes: &[u8]) -> CanFrame {
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

    #[test]
    fn classic_roundtrip() {
        let frames = vec![
            CanFrame {
                t_us: 1_234_567,
                channel: 0,
                id: 0x123,
                extended: false,
                len: 8,
                data: {
                    let mut d = [0u8; MAX_CAN_FD_LEN];
                    d[..8].copy_from_slice(&[1, 2, 3, 4, 5, 6, 7, 8]);
                    d
                },
                dir: Direction::Rx,
                flags: FrameFlags::NONE,
            },
            CanFrame {
                t_us: 2_000_000,
                channel: 1,
                id: 0x1FFFFFFF,
                extended: true,
                len: 3,
                data: {
                    let mut d = [0u8; MAX_CAN_FD_LEN];
                    d[..3].copy_from_slice(&[0xAA, 0xBB, 0xCC]);
                    d
                },
                dir: Direction::Tx,
                flags: FrameFlags::NONE,
            },
        ];
        let path = std::env::temp_dir().join("roxy_can_classic.asc");
        let path_str = path.to_string_lossy().to_string();
        let mut w = AscWriter::new(&path_str).unwrap();
        for f in &frames {
            w.write(f).unwrap();
        }
        w.finish().unwrap();
        let parsed = parse_asc(&std::fs::read_to_string(&path).unwrap());
        assert_eq!(parsed.len(), 2);
        let a = &parsed[0];
        assert_eq!(a.t_us, 1_234_567);
        assert_eq!(a.id, 0x123);
        assert!(!a.extended);
        assert_eq!(a.len, 8);
        assert_eq!(a.payload(), &[1, 2, 3, 4, 5, 6, 7, 8]);
        assert_eq!(a.dir, Direction::Rx);
        assert_eq!(a.channel, 0);
        let b = &parsed[1];
        assert_eq!(b.id, 0x1FFFFFFF);
        assert!(b.extended);
        assert_eq!(b.dir, Direction::Tx);
        assert_eq!(b.channel, 1);
        assert_eq!(b.payload(), &[0xAA, 0xBB, 0xCC]);
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn fd_roundtrip_preserves_length_flags_and_brs() {
        let mut data = [0u8; MAX_CAN_FD_LEN];
        for (i, b) in data.iter_mut().enumerate() {
            *b = i as u8;
        }
        let frame = CanFrame {
            t_us: 3_500_000,
            channel: 2,
            id: 0x1DB3_FF01,
            extended: true,
            len: 48,
            data,
            dir: Direction::Tx,
            flags: FrameFlags::FD.union(FrameFlags::BRS),
        };
        let path = std::env::temp_dir().join("roxy_can_fd.asc");
        let path_str = path.to_string_lossy().to_string();
        let mut w = AscWriter::new(&path_str).unwrap();
        w.write(&frame).unwrap();
        w.finish().unwrap();
        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains("CANFD"), "FD frame must emit a CANFD line");
        let parsed = parse_asc(&content);
        assert_eq!(parsed.len(), 1);
        let a = &parsed[0];
        assert!(a.is_fd());
        assert!(a.brs());
        assert!(!a.esi());
        assert_eq!(a.len, 48);
        assert_eq!(a.dlc_code(), 14);
        assert_eq!(a.payload(), &data[..48]);
        assert_eq!(a.id, 0x1DB3_FF01);
        assert!(a.extended);
        assert_eq!(a.channel, 2);
        assert_eq!(a.dir, Direction::Tx);
        assert_eq!(a.t_us, 3_500_000);
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn parses_vector_classic_sample() {
        let s = "date Sat Jan 01 00:00:00.000 2022\n\
                 base hex  timestamps absolute\n\
                 internal events logged\n\
                 Begin Triggerblock Sat Jan 01 00:00:00.000 2022\n\
                 0.000000 Start of measurement\n\
                 0.000123   1  1A4            Rx       d 8  11 22 33 44 55 66 77 88\n\
                 0.000456   1  1DB3FFFDx      Tx       d 8  DE AD BE EF 00 11 22 33\n\
                 0.000789   2  100            Rx       r 8\n\
                 0.000800   2  000            Rx       e 0\n\
                 End TriggerBlock\n";
        let parsed = parse_asc(s);
        assert_eq!(parsed.len(), 4);
        assert_eq!(parsed[0].id, 0x1A4);
        assert_eq!(parsed[0].len, 8);
        assert_eq!(
            parsed[0].payload(),
            &[0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88]
        );
        assert_eq!(parsed[1].id, 0x1DB3FFFD);
        assert!(parsed[1].extended);
        assert_eq!(parsed[1].dir, Direction::Tx);
        assert_eq!(parsed[1].channel, 0);
        // Row 3 is a remote frame: RTR set, DLC records the requested length,
        // payload() is empty.
        assert!(parsed[2].is_remote());
        assert_eq!(parsed[2].id, 0x100);
        assert_eq!(parsed[2].channel, 1);
        assert_eq!(parsed[2].len, 8);
        assert!(parsed[2].payload().is_empty());
        // Row 4 is an error frame: ERROR set, id forced to 0.
        assert!(parsed[3].is_error());
        assert_eq!(parsed[3].id, 0);
        assert!(parsed[3].payload().is_empty());
    }

    #[test]
    fn parses_vector_fd_sample_with_name() {
        // A CANoe/python-can FD export: 12 data bytes (DLC code 9), BRS on,
        // ESI off, symbolic name column present, and 8 trailing fields.
        let s = "base hex  timestamps absolute\n\
                 0.000000 Start of measurement\n\
                 0.005756 CANFD   1  Rx   1A4   EngineStatus 1 0 9 12 01 02 03 04 05 06 07 08 09 0A 0B 0C 0 0 00000000 0 0 0 0 0\n";
        let parsed = parse_asc(s);
        assert_eq!(parsed.len(), 1);
        let a = &parsed[0];
        assert!(a.is_fd());
        assert!(a.brs());
        assert!(!a.esi());
        assert_eq!(a.id, 0x1A4);
        assert_eq!(a.len, 12);
        assert_eq!(a.dlc_code(), 9);
        assert_eq!(
            a.payload(),
            &[1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12]
        );
        assert_eq!(a.channel, 0);
        assert_eq!(a.dir, Direction::Rx);
    }

    #[test]
    fn parses_vector_fd_sample_without_name() {
        // CANoe native FD line (no symbolic name): brs=0 esi=0, DLC code 8,
        // 8 data bytes.
        let s = "base hex  timestamps absolute\n\
                 0.000000 Start of measurement\n\
                 0.001000 CANFD 2 Tx 321 0 0 8 8 AA BB CC DD EE FF 00 11 0 0 00000000 0 0 0 0 0\n";
        let parsed = parse_asc(s);
        assert_eq!(parsed.len(), 1);
        let a = &parsed[0];
        assert!(a.is_fd());
        assert!(!a.brs());
        assert_eq!(a.channel, 1);
        assert_eq!(a.id, 0x321);
        assert_eq!(a.len, 8);
        assert_eq!(a.payload(), &[0xAA, 0xBB, 0xCC, 0xDD, 0xEE, 0xFF, 0x00, 0x11]);
    }

    #[test]
    fn skips_header_and_junk() {
        let s = "date Mon Jan 01\nbase hex  timestamps hex\ninternal events logged\n0.000000 Start of measurement\nfoo bar\n1.000000  1  200  Rx  8  11 22 33 44 55 66 77 88\n";
        let parsed = parse_asc(s);
        assert_eq!(parsed.len(), 1, "legacy type-token-less line still parses");
        assert_eq!(parsed[0].id, 0x200);
        assert_eq!(parsed[0].len, 8);
    }

    #[test]
    fn decimal_base_header_changes_radix() {
        let s = "base dec  timestamps absolute\n\
                 0.000000 Start of measurement\n\
                 1.0 1 416 Rx d 8 17 34 51 68 85 102 119 136\n";
        let parsed = parse_asc(s);
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].id, 416, "id 416 decimal == 0x1A4");
        assert_eq!(parsed[0].payload(), &[17, 34, 51, 68, 85, 102, 119, 136]);
    }

    #[test]
    fn classic_uses_dlc_code_not_byte_count() {
        // A hand-written classic line can carry a DLC code > 8 (an FD-only
        // length that classic frames never use), but roxy-can only ever writes
        // classic frames with len <= 8, so this guards the reader's mapping.
        let a = classic(0x100, &[1, 2, 3]);
        assert_eq!(a.dlc_code(), 3);
        let big = classic(0x100, &[0u8; 12]);
        assert_eq!(big.dlc_code(), 9);
    }

    #[test]
    fn error_remote_round_trip() {
        let err = CanFrame {
            t_us: 500_000,
            channel: 0,
            id: 0,
            extended: false,
            len: 0,
            data: [0u8; MAX_CAN_FD_LEN],
            dir: Direction::Rx,
            flags: FrameFlags::ERROR,
        };
        let rtr = CanFrame {
            t_us: 800_000,
            channel: 1,
            id: 0x100,
            extended: false,
            len: 8,
            data: [0u8; MAX_CAN_FD_LEN],
            dir: Direction::Tx,
            flags: FrameFlags::RTR,
        };
        let path = std::env::temp_dir().join("roxy_can_err_rtr.asc");
        let path_str = path.to_string_lossy().to_string();
        let mut w = AscWriter::new(&path_str).unwrap();
        w.write(&err).unwrap();
        w.write(&rtr).unwrap();
        w.finish().unwrap();
        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains(" e 0"), "error frame writes an `e` line");
        assert!(content.contains(" r 8"), "remote frame writes an `r` line");
        let parsed = parse_asc(&content);
        assert_eq!(parsed.len(), 2);
        assert!(parsed[0].is_error());
        assert_eq!(parsed[0].t_us, 500_000);
        assert_eq!(parsed[0].channel, 0);
        assert_eq!(parsed[0].id, 0);
        assert!(parsed[0].payload().is_empty());
        assert!(parsed[1].is_remote());
        assert_eq!(parsed[1].t_us, 800_000);
        assert_eq!(parsed[1].channel, 1);
        assert_eq!(parsed[1].id, 0x100);
        assert_eq!(parsed[1].dir, Direction::Tx);
        assert_eq!(parsed[1].len, 8);
        assert!(parsed[1].payload().is_empty());
        std::fs::remove_file(&path).ok();
    }
}
