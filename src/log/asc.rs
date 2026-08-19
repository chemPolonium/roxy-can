use std::fs::File;
use std::io::{BufWriter, Write};

use crate::can::frame::{CanFrame, Direction};

pub struct AscWriter {
    w: BufWriter<File>,
}

impl AscWriter {
    pub fn new(path: &str) -> std::io::Result<Self> {
        let file = File::create(path)?;
        let mut w = BufWriter::new(file);
        let date = chrono::Local::now().format("%a %b %d %I:%M:%S %p %Y");
        writeln!(w, "date {date}")?;
        writeln!(w, "base hex  timestamps hex")?;
        writeln!(w, "internal events logged")?;
        writeln!(w, "0.000000 Start of measurement")?;
        Ok(AscWriter { w })
    }

    pub fn write(&mut self, f: &CanFrame) -> std::io::Result<()> {
        let t = f.t_us as f64 / 1e6;
        let dir = match f.dir {
            Direction::Rx => "Rx",
            Direction::Tx => "Tx",
        };
        let ext = if f.extended { "x" } else { "" };
        let data: Vec<String> = f.data[..f.dlc.min(8) as usize]
            .iter()
            .map(|b| format!("{b:02X}"))
            .collect();
        writeln!(
            self.w,
            "{:.6}  {}  {:X}{}  {}  {:X}  {}",
            t,
            f.channel + 1,
            f.id,
            ext,
            dir,
            f.dlc,
            data.join(" ")
        )
    }

    pub fn finish(mut self) -> std::io::Result<()> {
        self.w.flush()
    }
}

pub fn parse_asc(content: &str) -> Vec<CanFrame> {
    let mut frames = Vec::new();
    for line in content.lines() {
        let toks: Vec<&str> = line.split_whitespace().collect();
        if toks.len() < 5 {
            continue;
        }
        let Some(t) = toks[0].parse::<f64>().ok() else {
            continue;
        };
        let Some(bus) = toks[1].parse::<u32>().ok() else {
            continue;
        };
        let (id, ext) = if let Some(h) = toks[2].strip_suffix(['x', 'X']) {
            let Some(v) = u32::from_str_radix(h, 16).ok() else {
                continue;
            };
            (v, true)
        } else {
            let Some(v) = u32::from_str_radix(toks[2], 16).ok() else {
                continue;
            };
            (v, false)
        };
        let dir = match toks[3] {
            "Tx" | "tx" => Direction::Tx,
            "Rx" | "rx" => Direction::Rx,
            _ => continue,
        };
        let Some(dlc) = u8::from_str_radix(toks[4], 16).ok() else {
            continue;
        };
        let mut data = [0u8; 8];
        let mut ok = true;
        for (i, tok) in toks[5..].iter().take(8).enumerate() {
            match u8::from_str_radix(tok, 16).ok() {
                Some(b) => data[i] = b,
                None => {
                    ok = false;
                    break;
                }
            }
        }
        if !ok {
            continue;
        }
        frames.push(CanFrame {
            t_us: (t * 1e6).round() as u64,
            channel: (bus.saturating_sub(1)) as u8,
            id,
            extended: ext,
            dlc,
            data,
            dir,
        });
    }
    frames
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let frames = vec![
            CanFrame {
                t_us: 1_234_567,
                channel: 0,
                id: 0x123,
                extended: false,
                dlc: 8,
                data: [1, 2, 3, 4, 5, 6, 7, 8],
                dir: Direction::Rx,
            },
            CanFrame {
                t_us: 2_000_000,
                channel: 1,
                id: 0x1FFFFFFF,
                extended: true,
                dlc: 3,
                data: [0xAA, 0xBB, 0, 0, 0, 0, 0, 0],
                dir: Direction::Tx,
            },
        ];
        let dir = std::env::temp_dir();
        let path = dir.join("roxy_can_roundtrip.asc");
        let path_str = path.to_string_lossy().to_string();
        let mut w = AscWriter::new(&path_str).unwrap();
        for f in &frames {
            w.write(f).unwrap();
        }
        w.finish().unwrap();
        let content = std::fs::read_to_string(&path).unwrap();
        let parsed = parse_asc(&content);
        assert_eq!(parsed.len(), 2);
        let a = &parsed[0];
        assert_eq!(a.t_us, 1_234_567);
        assert_eq!(a.id, 0x123);
        assert!(!a.extended);
        assert_eq!(a.dlc, 8);
        assert_eq!(a.data, [1, 2, 3, 4, 5, 6, 7, 8]);
        assert_eq!(a.dir, Direction::Rx);
        assert_eq!(a.channel, 0);
        let b = &parsed[1];
        assert_eq!(b.id, 0x1FFFFFFF);
        assert!(b.extended);
        assert_eq!(b.dir, Direction::Tx);
        assert_eq!(b.channel, 1);
        assert_eq!(b.data[0], 0xAA);
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn skips_header_and_junk() {
        let s = "date Mon Jan 01\nbase hex  timestamps hex\ninternal events logged\n0.000000 Start of measurement\nfoo bar\n1.000000  1  200  Rx  8  11 22 33 44 55 66 77 88\n";
        let parsed = parse_asc(s);
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].id, 0x200);
    }
}
