use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::Path;

use crate::can::frame::{CanFrame, Direction, FrameFlags, MAX_CAN_FD_LEN, dlc2len};
use crate::log::backing::Backing;
use crate::log::error::LogError;
use crate::source::FrameStream;

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
        if let Some(f) = parse_asc_line(raw, &mut base) {
            frames.push(f);
        }
    }
    frames
}

/// Parses one ASC line, mutating `base` when the header `base hex|dec`
/// token appears. Returns `None` for headers, comments, malformed rows, and
/// trailing fields. Shared with [`AscStream`] so the mmap path and the
/// string path can never drift on line semantics.
fn parse_asc_line(raw: &str, base: &mut u32) -> Option<CanFrame> {
    let line = raw.trim();
    if line.is_empty() {
        return None;
    }
    if line.len() > 5 && line[..5].eq_ignore_ascii_case("base ") {
        *base = if line.to_ascii_lowercase().contains("dec") {
            10
        } else {
            16
        };
        return None;
    }
    let toks: Vec<&str> = line.split_whitespace().collect();
    if toks.len() < 5 {
        return None;
    }
    let Ok(t) = toks[0].parse::<f64>() else {
        return None;
    };
    let t_us = (t * 1e6).round() as u64;
    if toks[1].eq_ignore_ascii_case("CANFD") {
        parse_fd(t_us, &toks, *base)
    } else {
        parse_classic(t_us, &toks, *base)
    }
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
    let len = dlc2len(code as u8)
        .max(data_len as u8)
        .min(MAX_CAN_FD_LEN as u8);
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

/// Frames to walk between seek checkpoints. One per ~130 KB of a typical
/// log: a backward jump then rescans at most this many lines, while the
/// table itself costs ~1 KB per million frames.
const ASC_CHECKPOINT_EVERY: u32 = 4096;

/// Streaming ASC reader backed by `mmap`, so a 500 MB log costs no heap.
/// Reuses [`parse_asc_line`] to guarantee byte-identical semantics with the
/// in-memory [`parse_asc`] path.
pub struct AscStream {
    data: Backing,
    pos: usize,
    base: u32,
    front: Option<CanFrame>,
    /// Byte offset [`AscStream::front`] was parsed from, so a checkpoint can
    /// name the exact line boundary to resume from.
    front_start: usize,
    eof: bool,
    duration: Option<u64>,
    /// `(t_us, byte offset, radix in force there)` for a handful of positions
    /// we have already walked past, ascending by `t_us`. Recording happens as
    /// we read, so opening a log stays O(1); seeking restores the nearest
    /// entry and rescans forward from it.
    checkpoints: Vec<(u64, usize, u32)>,
    since_ckpt: u32,
}

impl AscStream {
    pub fn open(path: &Path) -> Result<Self, LogError> {
        Ok(Self::from_backing(Backing::map_path(path)?))
    }

    /// Test seam: run the byte-cursor path over an in-memory log so the mmap
    /// reader stays covered even though `open_stream` only reaches it above
    /// the 100 MB threshold.
    #[cfg(test)]
    pub fn from_bytes(bytes: &[u8]) -> Self {
        Self::from_backing(Backing::owned(bytes))
    }

    fn from_backing(data: Backing) -> Self {
        let duration = asc_tail_duration_us(data.as_slice());
        AscStream {
            data,
            pos: 0,
            base: 16,
            front: None,
            front_start: 0,
            eof: false,
            duration,
            checkpoints: Vec::new(),
            since_ckpt: 0,
        }
    }

    /// Reads forward until `front` holds a frame or the file is exhausted.
    /// Kept at one frame of lookahead so `peek_t` never over-consumes.
    fn fill(&mut self) {
        while self.front.is_none() && !self.eof {
            let start = self.pos;
            let len = self.data.len();
            let end = match self.data.as_slice()[start..]
                .iter()
                .position(|&b| b == b'\n')
            {
                Some(rel) => start + rel,
                None => len,
            };
            self.pos = end.saturating_add(1).min(len);
            let raw = &self.data.as_slice()[start..end];
            let Some(frame) = Self::parse_line_bytes(raw, &mut self.base) else {
                if self.pos >= len {
                    self.eof = true;
                }
                continue;
            };
            self.front_start = start;
            self.front = Some(frame);
        }
    }

    /// Indexes the line `t_us` was read from, keeping the table ascending so
    /// [`Self::seek_to_us`] can binary search it. Re-walking an already
    /// indexed stretch must not pile up duplicates.
    fn note_checkpoint(&mut self, t_us: u64, pos: usize, base: u32) {
        let at = self.checkpoints.partition_point(|(t, _, _)| *t < t_us);
        if self
            .checkpoints
            .get(at)
            .is_some_and(|(t, p, _)| *t == t_us && *p == pos)
        {
            return;
        }
        self.checkpoints.insert(at, (t_us, pos, base));
    }

    /// Restores the cursor to the start of the file, forgetting nothing:
    /// the checkpoint table survives, since it describes the bytes, not us.
    fn rewind(&mut self) {
        self.pos = 0;
        self.base = 16;
        self.front = None;
        self.front_start = 0;
        self.eof = false;
        self.since_ckpt = 0;
    }

    fn parse_line_bytes(bytes: &[u8], base: &mut u32) -> Option<CanFrame> {
        // Non-UTF-8 bytes are rare (Vector writes ASCII), but a stray byte
        // must not abort a whole capture; drop the line and keep going.
        let Ok(s) = std::str::from_utf8(bytes) else {
            return None;
        };
        parse_asc_line(s, base)
    }
}

impl FrameStream for AscStream {
    fn peek_t(&mut self) -> Option<u64> {
        if self.front.is_none() {
            self.fill();
        }
        self.front.as_ref().map(|f| f.t_us)
    }

    fn next_frame(&mut self) -> Option<CanFrame> {
        if self.front.is_none() {
            self.fill();
        }
        let f = self.front.take()?;
        if self.since_ckpt >= ASC_CHECKPOINT_EVERY {
            // Frame rows never change the radix, so the current `base` is also
            // the one in force when this row was read.
            self.note_checkpoint(f.t_us, self.front_start, self.base);
            self.since_ckpt = 0;
        }
        self.since_ckpt += 1;
        if self.front.is_none() && self.pos >= self.data.len() {
            self.eof = true;
        }
        Some(f)
    }

    fn seek_to_us(&mut self, target: u64) -> Option<u64> {
        match self.checkpoints.partition_point(|(t, _, _)| *t <= target) {
            0 => self.rewind(),
            k => {
                let (t, pos, base) = self.checkpoints[k - 1];
                let _ = t;
                self.pos = pos;
                self.base = base;
                self.front = None;
                self.front_start = pos;
                self.eof = false;
                self.since_ckpt = 0;
            }
        }
        // Walk forward to the target. Bounded by one checkpoint interval for
        // stretches we have already indexed; the first jump into fresh
        // territory pays for its whole prefix and leaves checkpoints behind,
        // so the same jump is fast afterwards.
        loop {
            match self.peek_t() {
                Some(t) if t >= target => return Some(t),
                Some(_) => {
                    self.next_frame();
                }
                None => return None,
            }
        }
    }

    fn duration_us(&self) -> Option<u64> {
        self.duration
    }

    fn describe(&self) -> String {
        match self.duration {
            Some(d) => format!("ASC, {:.1} s", d as f64 / 1e6),
            None => "ASC".to_string(),
        }
    }
}

/// Best-effort tail read: parse the last 8 KB and report the timestamp of
/// the final frame line. Avoids a full-file scan during open; the caller
/// gets a slightly-off duration only when the last 8 KB is entirely
/// comments or the file has fewer than one frame.
pub fn asc_tail_duration_us(bytes: &[u8]) -> Option<u64> {
    const WINDOW: usize = 8 * 1024;
    let start = bytes.len().saturating_sub(WINDOW);
    let tail = &bytes[start..];
    let Ok(s) = std::str::from_utf8(tail) else {
        return None;
    };
    let mut base = 16u32;
    let mut last: Option<CanFrame> = None;
    for line in s.lines() {
        if let Some(f) = parse_asc_line(line, &mut base) {
            last = Some(f);
        }
    }
    last.map(|f| f.t_us)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::log::vec_stream::VecStream;

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
        assert_eq!(a.payload(), &[1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12]);
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
        assert_eq!(
            a.payload(),
            &[0xAA, 0xBB, 0xCC, 0xDD, 0xEE, 0xFF, 0x00, 0x11]
        );
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

    // ---- mmap byte-cursor path ------------------------------------------

    // `CanFrame` deliberately has no `PartialEq` (the fixed `data` buffer is
    // padding-sensitive), so compare the observable fields instead.
    type FrameKey = (u64, u8, u32, bool, u8, Vec<u8>, Direction, FrameFlags);

    fn frame_key(f: &CanFrame) -> FrameKey {
        (
            f.t_us,
            f.channel,
            f.id,
            f.extended,
            f.len,
            f.payload().to_vec(),
            f.dir,
            f.flags,
        )
    }

    fn keys(frames: &[CanFrame]) -> Vec<FrameKey> {
        frames.iter().map(frame_key).collect()
    }

    fn drain(mut s: impl FrameStream) -> Vec<CanFrame> {
        let mut out = Vec::new();
        while let Some(f) = s.next_frame() {
            out.push(f);
        }
        out
    }

    /// One line per frame kind the reader claims to handle, wrapped in the
    /// header/footer junk Vector emits. Timestamps ascend so the ordering the
    /// replay clock relies on is also covered.
    fn mixed_log() -> String {
        "date Sat Jan 01 00:00:00.000 2022\n\
         base hex  timestamps absolute\n\
         internal events logged\n\
         Begin Triggerblock Sat Jan 01 00:00:00.000 2022\n\
         0.000000 Start of measurement\n\
         0.000123   1  1A4            Rx       d 8  11 22 33 44 55 66 77 88\n\
         0.000456   1  1DB3FFFDx      Tx       d 8  DE AD BE EF 00 11 22 33\n\
         0.000789   2  100            Rx       r 8\n\
         0.000800   2  000            Rx       e 0\n\
         0.005756 CANFD   1  Rx   1A4   EngineStatus 1 0 9 12 01 02 03 04 05 06 07 08 09 0A 0B 0C 0 0 00000000 0 0 0 0 0\n\
         End TriggerBlock\n"
            .to_string()
    }

    #[test]
    fn stream_matches_parse_asc_on_mixed_log() {
        let text = mixed_log();
        let want = parse_asc(&text);
        assert_eq!(want.len(), 5, "fixture should yield one frame per row");
        let got = drain(AscStream::from_bytes(text.as_bytes()));
        assert_eq!(
            keys(&got),
            keys(&want),
            "mmap path drifted from the string path"
        );
    }

    #[test]
    fn stream_survives_a_non_utf8_line() {
        let mut bytes = b"0.000100   1  1A4  Rx  d 2  AA BB\n".to_vec();
        bytes.extend_from_slice(&[0xFF, 0xFE, 0x00, 0x80, b'\n']);
        bytes.extend_from_slice(b"0.000200   1  1A5  Rx  d 1  CC\n");
        let got = drain(AscStream::from_bytes(&bytes));
        assert_eq!(got.len(), 2, "one corrupt row must not abort a capture");
        assert_eq!(got[0].id, 0x1A4);
        assert_eq!(got[1].id, 0x1A5);
        assert_eq!(got[1].t_us, 200);
    }

    #[test]
    fn stream_handles_crlf_and_a_final_unterminated_line() {
        let unix = keys(&parse_asc(&mixed_log()));
        let crlf = mixed_log().replace('\n', "\r\n");
        assert_eq!(keys(&drain(AscStream::from_bytes(crlf.as_bytes()))), unix);

        // Vector can be killed mid-write, leaving no trailing newline.
        let mut truncated = crlf.clone();
        truncated.truncate(truncated.len() - 2);
        assert!(
            !truncated.ends_with('\n'),
            "fixture must drop the terminator"
        );
        assert_eq!(
            keys(&drain(AscStream::from_bytes(truncated.as_bytes()))),
            unix
        );
    }

    #[test]
    fn stream_peek_does_not_consume() {
        let mut s = AscStream::from_bytes(mixed_log().as_bytes());
        let first = s.peek_t();
        assert_eq!(first, Some(123));
        assert_eq!(s.peek_t(), first, "repeated peek must be stable");
        assert_eq!(s.next_frame().map(|f| f.t_us), first);
        assert_eq!(s.peek_t(), Some(456));
    }

    #[test]
    fn stream_reports_no_frames_for_a_header_only_log() {
        let s = "date Sat Jan 01 00:00:00.000 2022\nbase hex  timestamps absolute\n";
        let mut stream = AscStream::from_bytes(s.as_bytes());
        assert_eq!(stream.peek_t(), None);
        assert!(stream.next_frame().is_none());
        assert_eq!(stream.duration_us(), None);
    }

    #[test]
    fn tail_duration_reports_the_last_frame_timestamp() {
        assert_eq!(asc_tail_duration_us(mixed_log().as_bytes()), Some(5_756));
    }

    #[test]
    fn tail_duration_is_none_without_frame_lines() {
        let s = "date Sat Jan 01 00:00:00.000 2022\nbase hex\ninternal events logged\n";
        assert_eq!(asc_tail_duration_us(s.as_bytes()), None);
    }

    #[test]
    fn tail_duration_window_may_start_mid_line() {
        // Longer than the 8 KB window, so the slice begins partway through a
        // comment row; the final frame still has to be found.
        let mut s = String::from("base hex  timestamps absolute\n");
        for i in 0..600 {
            s.push_str(&format!(
                " filler comment row {i} padded out so the window has to clip a line\n"
            ));
        }
        s.push_str("0.042000   1  321  Rx  d 1  A5\n");
        assert!(
            s.len() > 8 * 1024,
            "fixture must exceed the window: {}",
            s.len()
        );
        assert_eq!(asc_tail_duration_us(s.as_bytes()), Some(42_000));
    }

    #[test]
    fn stream_drains_a_many_thousand_frame_log() {
        const N: u32 = 20_000;
        let mut s = String::from("base hex  timestamps absolute\n");
        for i in 0..N {
            s.push_str(&format!(
                "{:.6}   1  {:03X}  Rx  d 2  AA BB\n",
                f64::from(i) / 1000.0,
                i % 0x400,
            ));
        }
        let want = parse_asc(&s);
        let got = drain(AscStream::from_bytes(s.as_bytes()));
        assert_eq!(got.len(), N as usize);
        assert_eq!(keys(&got), keys(&want));
        assert_eq!(
            AscStream::from_bytes(s.as_bytes()).duration_us(),
            want.last().map(|f| f.t_us),
            "tail scan must agree with the full parse"
        );
    }

    /// A log with `n` distinct one-microsecond-spaced frames, long enough to
    /// cross the checkpoint interval.
    fn timed_log(n: u32) -> String {
        let mut s = String::from("base hex  timestamps absolute\n");
        for i in 0..n {
            s.push_str(&format!(
                "{:.6}   1  {:03X}  Rx  d 2  AA BB\n",
                f64::from(i) / 1e6,
                0x100 + i % 0x2FF,
            ));
        }
        s
    }

    fn remaining(s: &mut dyn FrameStream) -> Vec<FrameKey> {
        let mut out = Vec::new();
        while let Some(f) = s.next_frame() {
            out.push(frame_key(&f));
        }
        out
    }

    #[test]
    fn seek_agrees_with_the_in_memory_stream() {
        let text = timed_log(10_000);
        for target in [0u64, 1, 4_095, 4_096, 4_097, 9_998, 9_999] {
            let mut a = AscStream::from_bytes(text.as_bytes());
            let mut v = VecStream::new(parse_asc(&text));
            assert_eq!(
                a.seek_to_us(target),
                v.seek_to_us(target),
                "landing differs at t={target}"
            );
            assert_eq!(
                remaining(&mut a),
                remaining(&mut v),
                "tail after seeking differs at t={target}"
            );
        }
    }

    #[test]
    fn seek_rewinds_after_a_full_drain() {
        let text = timed_log(9_000);
        let mut s = AscStream::from_bytes(text.as_bytes());
        let first_pass = remaining(&mut s);
        assert_eq!(first_pass.len(), 9_000);
        assert_eq!(s.seek_to_us(0), Some(0));
        assert_eq!(
            remaining(&mut s),
            first_pass,
            "rewind must replay the identical stream"
        );
    }

    #[test]
    fn checkpoint_restore_keeps_the_decimal_radix() {
        // `base dec` appears once at the top, so a seek that resumes from a
        // mid-file byte offset has to bring the radix with it.
        let mut s = String::from("base dec  timestamps absolute\n");
        for i in 0..5_000u32 {
            s.push_str(&format!(
                "{:.6}   1  {}  Rx  d 3  10 20 30\n",
                f64::from(i) / 1e6,
                1000 + i,
            ));
        }
        let want = parse_asc(&s);
        let mut stream = AscStream::from_bytes(s.as_bytes());
        let target = want[4_500].t_us;
        assert_eq!(stream.seek_to_us(target), Some(target));
        let f = stream.next_frame().unwrap();
        assert_eq!(f.id, 5_500, "id parsed as decimal, not hex");
        assert_eq!(f.payload(), &[10, 20, 30]);
    }

    #[test]
    fn seek_past_the_end_stops_at_eof() {
        let mut s = AscStream::from_bytes(timed_log(100).as_bytes());
        assert_eq!(s.seek_to_us(9_999_999), None);
        assert_eq!(s.peek_t(), None);
        assert!(s.next_frame().is_none());
    }

    #[test]
    fn repeated_scrubbing_does_not_blow_up_the_index() {
        let text = timed_log(9_000);
        let mut s = AscStream::from_bytes(text.as_bytes());
        assert_eq!(s.seek_to_us(8_999), Some(8_999));
        for t in [0u64, 4_000, 8_999, 100, 6_000, 3] {
            assert!(s.seek_to_us(t).is_some());
        }
        // One entry per interval walked, plus the scrub revisits, against a
        // ceiling far below the frame count.
        let per_walk = 9_000 / ASC_CHECKPOINT_EVERY as usize + 2;
        assert!(
            s.checkpoints.len() <= per_walk * 3,
            "checkpoint table grew to {} for a 9000-frame log",
            s.checkpoints.len()
        );
        assert!(
            s.checkpoints.windows(2).all(|w| w[0].0 <= w[1].0),
            "checkpoint table must stay ascending for partition_point"
        );
    }

    /// Manual smoke over a real >100 MB file through the production entry
    /// point, so the mmap threshold, `Backing::map_path` and the byte cursor
    /// are all covered together. Run with
    /// `cargo test large_asc_mmap_smoke -- --ignored --nocapture` and watch RSS
    /// in Task Manager — it should stay flat while the drain runs.
    #[test]
    #[ignore]
    fn large_asc_mmap_smoke() {
        const N: u32 = 4_000_000;
        let path = std::env::temp_dir().join("roxy_can_large.asc");
        {
            use std::io::Write as _;
            let mut f = std::io::BufWriter::with_capacity(1 << 20, File::create(&path).unwrap());
            writeln!(f, "base hex  timestamps absolute").unwrap();
            for i in 0..N {
                writeln!(
                    f,
                    "{:.6}   1  {:03X}  Rx  d 2  AA BB",
                    f64::from(i) / 1000.0,
                    i % 0x400,
                )
                .unwrap();
            }
            f.flush().unwrap();
        }
        let size = std::fs::metadata(&path).unwrap().len();
        assert!(
            size > crate::log::ASC_MMAP_THRESHOLD,
            "fixture must clear the mmap threshold: {size} B"
        );

        let t0 = std::time::Instant::now();
        let mut stream = crate::log::open_stream(&path).unwrap();
        let describe = stream.describe();
        assert!(
            describe.starts_with("ASC, "),
            "production dispatch should reach the mmap reader, got {describe}"
        );
        let open_ms = t0.elapsed().as_millis();

        let t1 = std::time::Instant::now();
        let mut n = 0usize;
        while stream.next_frame().is_some() {
            n += 1;
        }
        let drain_ms = t1.elapsed().as_millis();

        assert_eq!(n, N as usize);
        assert_eq!(stream.duration_us(), Some((N as u64 - 1) * 1000));

        // The scrub bar's premise: with the checkpoint table now populated by
        // the pass above, a rewind must cost a rescan of one checkpoint
        // interval, not another full sweep.
        let t2 = std::time::Instant::now();
        assert_eq!(stream.seek_to_us(1_000_000), Some(1_000_000));
        let seek_ms = t2.elapsed().as_millis();
        let t3 = std::time::Instant::now();
        assert_eq!(stream.seek_to_us(3_500_000_000), Some(3_500_000_000));
        let seek_fwd_ms = t3.elapsed().as_millis();

        println!(
            "{size} B, open {open_ms} ms, drained {n} frames in {drain_ms} ms \
             ({:.0} frames/s), seek back {seek_ms} ms, seek fwd {seek_fwd_ms} ms",
            n as f64 / (drain_ms as f64 / 1000.0)
        );
        assert!(
            seek_ms.max(seek_fwd_ms) < 50,
            "indexed scrub should be near-instant, saw {seek_ms}/{seek_fwd_ms} ms"
        );
        std::fs::remove_file(&path).ok();
    }
}
