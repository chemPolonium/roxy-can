//! Classic PCAP and PCAPNG readers for Ethernet captures. Only link type 1
//! (Ethernet) is consumed; other interfaces are counted and skipped, so a
//! multi-interface capture still opens instead of failing wholesale.
//!
//! The replay clock wants microseconds rebased to the first frame, exactly
//! like the BLF reader: capture epochs are not meaningful on the bench
//! timeline.

use std::collections::VecDeque;
use std::path::Path;
use std::sync::Arc;

use crate::can::frame::Direction;
use crate::eth::EthFrame;
use crate::source::FrameStream;

use super::error::LogError;

pub struct PcapStream {
    frames: VecDeque<EthFrame>,
    duration_us: u64,
    summary: String,
}

impl PcapStream {
    pub fn open(path: &Path) -> Result<Self, LogError> {
        let data = std::fs::read(path)?;
        if data.len() < 4 {
            return Err(LogError::Truncated);
        }
        let (mut raw, iface_count) = if data.starts_with(&[0x0a, 0x0d, 0x0d, 0x0a]) {
            parse_pcapng(&data)?
        } else if is_classic_magic(&data[..4]) {
            (parse_classic(&data)?, 1)
        } else {
            return Err(LogError::BadSignature);
        };
        let base = raw.iter().map(|frame| frame.t_us).min().unwrap_or(0);
        for frame in &mut raw {
            frame.t_us = frame.t_us.saturating_sub(base);
        }
        let duration_us = raw.iter().map(|frame| frame.t_us).max().unwrap_or(0);
        let summary = format!(
            "PCAP, {} frames, {:.1} s, {iface_count} interface{}",
            raw.len(),
            duration_us as f64 / 1e6,
            if iface_count == 1 { "" } else { "s" }
        );
        Ok(Self {
            frames: raw.into(),
            duration_us,
            summary,
        })
    }
}

impl FrameStream for PcapStream {
    fn peek_t(&mut self) -> Option<u64> {
        None
    }

    fn next_frame(&mut self) -> Option<crate::can::frame::CanFrame> {
        None
    }

    fn duration_us(&self) -> Option<u64> {
        Some(self.duration_us)
    }

    fn describe(&self) -> String {
        self.summary.clone()
    }

    fn poll_eth_frames(&mut self, upto_t_us: u64, out: &mut Vec<EthFrame>) {
        while let Some(frame) = self.frames.front() {
            if frame.t_us > upto_t_us {
                break;
            }
            if let Some(frame) = self.frames.pop_front() {
                out.push(frame);
            }
        }
    }

    fn peek_eth_t(&mut self) -> Option<u64> {
        self.frames.front().map(|frame| frame.t_us)
    }
}

fn is_classic_magic(bytes: &[u8]) -> bool {
    matches!(
        bytes,
        &[0xa1, 0xb2, 0xc3, 0xd4]
            | &[0xd4, 0xc3, 0xb2, 0xa1]
            | &[0xa1, 0xb2, 0x3c, 0x4d]
            | &[0x4d, 0x3c, 0xb2, 0xa1]
    )
}

fn read_u16(data: &[u8], at: usize, little: bool) -> Option<u16> {
    let bytes = data.get(at..at + 2)?;
    Some(if little {
        u16::from_le_bytes([bytes[0], bytes[1]])
    } else {
        u16::from_be_bytes([bytes[0], bytes[1]])
    })
}

fn read_u32(data: &[u8], at: usize, little: bool) -> Option<u32> {
    let bytes = data.get(at..at + 4)?;
    Some(if little {
        u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])
    } else {
        u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])
    })
}

/// One packet record, shared by both containers.
struct Packet {
    t_us: u64,
    bus: u8,
    data: Vec<u8>,
    truncated: bool,
}

fn parse_classic(data: &[u8]) -> Result<Vec<EthFrame>, LogError> {
    let header = data.get(..24).ok_or(LogError::Truncated)?;
    let (little, nanos) = match &header[..4] {
        [0xa1, 0xb2, 0xc3, 0xd4] => (false, false),
        [0xd4, 0xc3, 0xb2, 0xa1] => (true, false),
        [0xa1, 0xb2, 0x3c, 0x4d] => (false, true),
        [0x4d, 0x3c, 0xb2, 0xa1] => (true, true),
        _ => return Err(LogError::BadSignature),
    };
    let link_type = read_u32(data, 20, little).ok_or(LogError::Truncated)?;
    let mut packets = Vec::new();
    if link_type == 1 {
        let mut at = 24usize;
        while at + 16 <= data.len() {
            let ts_sec = read_u32(data, at, little).ok_or(LogError::Truncated)?;
            let ts_frac = read_u32(data, at + 4, little).ok_or(LogError::Truncated)?;
            let incl = read_u32(data, at + 8, little).ok_or(LogError::Truncated)? as usize;
            let orig = read_u32(data, at + 12, little).ok_or(LogError::Truncated)? as usize;
            at += 16;
            let Some(body) = data.get(at..at + incl) else {
                return Err(LogError::Truncated);
            };
            at += incl;
            let frac_us = if nanos { ts_frac / 1_000 } else { ts_frac };
            let t_us = u64::from(ts_sec) * 1_000_000 + u64::from(frac_us);
            packets.push(Packet {
                t_us,
                bus: 0,
                data: body.to_vec(),
                truncated: orig > incl,
            });
        }
    }
    Ok(packets
        .into_iter()
        .enumerate()
        .filter_map(|(seq, packet)| {
            let bus = packet.bus;
            ethernet_frame(&packet, seq as u64, bus)
        })
        .collect())
}

fn parse_pcapng(data: &[u8]) -> Result<(Vec<EthFrame>, usize), LogError> {
    let mut frames = Vec::new();
    let mut interfaces: Vec<Option<(u64, bool)>> = Vec::new(); // (units/s, ethernet)
    let mut at = 0usize;
    let mut little = true;
    let mut seq = 0u64;
    while at + 12 <= data.len() {
        let block_type_le = read_u32(data, at, true).ok_or(LogError::Truncated)?;
        let section_header = block_type_le == 0x0a0d0d0a;
        if section_header {
            let bom = data.get(at + 8..at + 12).ok_or(LogError::Truncated)?;
            little = matches!(bom, [0x4d, 0x3c, 0x2b, 0x1a]);
            interfaces.clear();
        }
        let total = read_u32(data, at + 4, little).ok_or(LogError::Truncated)? as usize;
        if total < 12 || at + total > data.len() {
            return Err(LogError::Truncated);
        }
        let body = &data[at + 8..at + total - 4];
        let block_type = read_u32(data, at, little).ok_or(LogError::Truncated)?;
        match block_type {
            0x00000001 => {
                // Interface Description Block: link type, reserved, snaplen.
                let link = read_u16(body, 0, little).ok_or(LogError::Truncated)?;
                let mut units = 1_000_000u64;
                let mut option = 8usize;
                while option + 4 <= body.len() {
                    let code = read_u16(body, option, little).ok_or(LogError::Truncated)?;
                    let length =
                        read_u16(body, option + 2, little).ok_or(LogError::Truncated)? as usize;
                    if code == 0 {
                        break;
                    }
                    if code == 9 && length >= 1 {
                        let value = body[option + 4];
                        units = if value & 0x80 != 0 {
                            1u64.checked_shl(u32::from(value & 0x7f))
                                .unwrap_or(1_000_000)
                        } else {
                            10u64.checked_pow(u32::from(value)).unwrap_or(1_000_000)
                        }
                        .max(1);
                    }
                    option += 4 + ((length + 3) & !3);
                }
                interfaces.push(Some((units, link == 1)));
            }
            0x00000006 => {
                // Enhanced Packet Block.
                let interface = read_u32(body, 0, little).ok_or(LogError::Truncated)? as usize;
                let high = read_u32(body, 4, little).ok_or(LogError::Truncated)?;
                let low = read_u32(body, 8, little).ok_or(LogError::Truncated)?;
                let cap = read_u32(body, 12, little).ok_or(LogError::Truncated)? as usize;
                let orig = read_u32(body, 16, little).ok_or(LogError::Truncated)? as usize;
                let Some(packet) = body.get(20..20 + cap) else {
                    return Err(LogError::Truncated);
                };
                let Some((units, ethernet)) = interfaces.get(interface).copied().flatten() else {
                    at += total;
                    continue;
                };
                if !ethernet {
                    at += total;
                    continue;
                }
                let stamp = (u64::from(high) << 32) | u64::from(low);
                let t_us = (u128::from(stamp) * 1_000_000 / u128::from(units)) as u64;
                let packet = Packet {
                    t_us,
                    bus: interface.min(u8::MAX as usize) as u8,
                    data: packet.to_vec(),
                    truncated: orig > cap,
                };
                if let Some(frame) = ethernet_frame(&packet, seq, packet.bus) {
                    frames.push(frame);
                    seq += 1;
                }
            }
            _ => {}
        }
        at += total;
    }
    Ok((frames, interfaces.len()))
}

fn ethernet_frame(packet: &Packet, seq: u64, bus: u8) -> Option<EthFrame> {
    let data = &packet.data;
    if data.len() < 14 {
        return None;
    }
    let mut dst = [0u8; 6];
    let mut src = [0u8; 6];
    dst.copy_from_slice(&data[..6]);
    src.copy_from_slice(&data[6..12]);
    let mut ethertype = u16::from_be_bytes([data[12], data[13]]);
    let mut vlan = None;
    let mut vlan_tpid = None;
    let mut at = 14usize;
    while matches!(ethertype, 0x8100 | 0x88a8 | 0x9100) {
        if data.len() < at + 4 {
            return None;
        }
        if vlan.is_none() {
            vlan = Some(u16::from_be_bytes([data[at], data[at + 1]]) & 0x0fff);
            vlan_tpid = Some(ethertype);
        }
        ethertype = u16::from_be_bytes([data[at + 2], data[at + 3]]);
        at += 4;
    }
    Some(EthFrame {
        t_us: packet.t_us,
        bus,
        dir: Direction::Rx,
        dst,
        src,
        vlan,
        vlan_tpid,
        ethertype,
        payload: Arc::from(data[at..].to_vec()),
        seq,
        truncated: packet.truncated,
    })
}

/// Classic PCAP writer for Ethernet recording (link type 1). One file carries
/// MAC/VLAN/EtherType/payload; timestamps are rebased from the first frame so
/// a session's relative time stays readable in Wireshark.
pub struct PcapWriter {
    file: std::io::BufWriter<std::fs::File>,
    base_wall_us: u64,
    first_t_us: Option<u64>,
}

impl PcapWriter {
    pub fn create(path: &str) -> std::io::Result<Self> {
        use std::io::Write;
        let file = std::fs::File::create(path)?;
        let mut file = std::io::BufWriter::new(file);
        file.write_all(&0xa1b2_c3d4u32.to_le_bytes())?;
        file.write_all(&2u16.to_le_bytes())?;
        file.write_all(&4u16.to_le_bytes())?;
        file.write_all(&0i32.to_le_bytes())?;
        file.write_all(&0u32.to_le_bytes())?;
        file.write_all(&262_144u32.to_le_bytes())?;
        file.write_all(&1u32.to_le_bytes())?;
        let now_us = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_micros().min(u128::from(u64::MAX)) as u64)
            .unwrap_or(0);
        Ok(Self {
            file,
            base_wall_us: now_us,
            first_t_us: None,
        })
    }

    pub fn write(&mut self, frame: &EthFrame) -> std::io::Result<()> {
        use std::io::Write;
        let first = *self.first_t_us.get_or_insert(frame.t_us);
        let t_us = self
            .base_wall_us
            .saturating_add(frame.t_us.saturating_sub(first));
        let l2 = eth_l2_bytes(frame);
        let len = l2.len() as u32;
        self.file
            .write_all(&((t_us / 1_000_000) as u32).to_le_bytes())?;
        self.file
            .write_all(&((t_us % 1_000_000) as u32).to_le_bytes())?;
        self.file.write_all(&len.to_le_bytes())?;
        self.file.write_all(&len.to_le_bytes())?;
        self.file.write_all(&l2)?;
        Ok(())
    }

    pub fn flush(&mut self) {
        use std::io::Write;
        self.file.flush().ok();
    }

    pub fn finish(mut self) {
        use std::io::Write;
        self.file.flush().ok();
    }
}

fn eth_l2_bytes(frame: &EthFrame) -> Vec<u8> {
    let mut out = Vec::with_capacity(18 + frame.payload.len());
    out.extend_from_slice(&frame.dst);
    out.extend_from_slice(&frame.src);
    if let Some(vlan) = frame.vlan {
        out.extend_from_slice(&frame.vlan_tpid.unwrap_or(0x8100).to_be_bytes());
        out.extend_from_slice(&vlan.to_be_bytes());
    }
    out.extend_from_slice(&frame.ethertype.to_be_bytes());
    out.extend_from_slice(&frame.payload);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn classic_packet() -> Vec<u8> {
        let mut file = Vec::new();
        file.extend_from_slice(&0xa1b2c3d4u32.to_le_bytes());
        file.extend_from_slice(&2u16.to_le_bytes());
        file.extend_from_slice(&4u16.to_le_bytes());
        file.extend_from_slice(&0i32.to_le_bytes());
        file.extend_from_slice(&0u32.to_le_bytes());
        file.extend_from_slice(&65535u32.to_le_bytes());
        file.extend_from_slice(&1u32.to_le_bytes());
        // 10.5 s, 22-byte frame: 14 L2 + VLAN 4 + 4 payload.
        file.extend_from_slice(&10u32.to_le_bytes());
        file.extend_from_slice(&500_000u32.to_le_bytes());
        file.extend_from_slice(&22u32.to_le_bytes());
        file.extend_from_slice(&22u32.to_le_bytes());
        let body = [
            1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 0x81, 0x00, 0x00, 0x64, 0x08, 0x00, 0xaa, 0xbb,
            0xcc, 0xdd,
        ];
        file.extend_from_slice(&body);
        file
    }

    #[test]
    fn a_classic_pcap_opens_with_vlan_and_payload() {
        let path = std::env::temp_dir().join("roxy_can_one.pcap");
        std::fs::write(&path, classic_packet()).unwrap();
        let mut stream = PcapStream::open(&path).unwrap();
        let mut out = Vec::new();
        stream.poll_eth_frames(u64::MAX, &mut out);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].t_us, 0, "timestamps are rebased");
        assert_eq!(out[0].vlan, Some(100));
        assert_eq!(out[0].vlan_tpid, Some(0x8100));
        assert_eq!(out[0].ethertype, 0x0800);
        assert_eq!(out[0].payload.as_ref(), [0xaa, 0xbb, 0xcc, 0xdd]);
        assert_eq!(stream.duration_us(), Some(0));
        std::fs::remove_file(&path).ok();
    }

    fn block(kind: u32, body: &[u8]) -> Vec<u8> {
        let pad = (4 - body.len() % 4) % 4;
        let total = (12 + body.len() + pad) as u32;
        let mut out = Vec::new();
        out.extend_from_slice(&kind.to_le_bytes());
        out.extend_from_slice(&total.to_le_bytes());
        out.extend_from_slice(body);
        out.extend(std::iter::repeat_n(0u8, pad));
        out.extend_from_slice(&total.to_le_bytes());
        out
    }

    fn pcapng_packet() -> Vec<u8> {
        let mut file = Vec::new();
        let mut shb = Vec::new();
        shb.extend_from_slice(&0x1a2b3c4du32.to_le_bytes());
        shb.extend_from_slice(&1u16.to_le_bytes());
        shb.extend_from_slice(&0u16.to_le_bytes());
        shb.extend_from_slice(&(-1i64).to_le_bytes());
        file.extend(block(0x0a0d0d0a, &shb));

        let mut idb = Vec::new();
        idb.extend_from_slice(&1u16.to_le_bytes());
        idb.extend_from_slice(&0u16.to_le_bytes());
        idb.extend_from_slice(&65535u32.to_le_bytes());
        idb.extend_from_slice(&9u16.to_le_bytes());
        idb.extend_from_slice(&1u16.to_le_bytes());
        idb.extend_from_slice(&[9, 0, 0, 0]);
        idb.extend_from_slice(&[0, 0, 0, 0]);
        file.extend(block(0x00000001, &idb));

        let stamp = 10_500_000_000u64; // 10.5 s at 10^-9 resolution
        let mut epb = Vec::new();
        epb.extend_from_slice(&0u32.to_le_bytes());
        epb.extend_from_slice(&((stamp >> 32) as u32).to_le_bytes());
        epb.extend_from_slice(&(stamp as u32).to_le_bytes());
        epb.extend_from_slice(&14u32.to_le_bytes());
        epb.extend_from_slice(&14u32.to_le_bytes());
        epb.extend_from_slice(&[1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 0x08, 0x00]);
        file.extend(block(0x00000006, &epb));
        file
    }

    #[test]
    fn a_pcapng_enhanced_packet_uses_its_timestamp() {
        let path = std::env::temp_dir().join("roxy_can_one.pcapng");
        std::fs::write(&path, pcapng_packet()).unwrap();
        let mut stream = PcapStream::open(&path).unwrap();
        let mut out = Vec::new();
        stream.poll_eth_frames(u64::MAX, &mut out);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].t_us, 0, "the first packet defines zero");
        assert_eq!(out[0].ethertype, 0x0800);
        assert_eq!(out[0].payload.len(), 0);
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn a_written_pcap_replays_with_vlan_and_payload() {
        let path = std::env::temp_dir().join("roxy_can_write_one.pcap");
        let mut writer = PcapWriter::create(path.to_str().unwrap()).unwrap();
        let mut frame = EthFrame::new(
            1_000,
            0,
            Direction::Rx,
            [1, 2, 3, 4, 5, 6],
            [7, 8, 9, 10, 11, 12],
            Some(100),
            0x0800,
            &[0xaa, 0xbb, 0xcc, 0xdd][..],
        );
        frame.vlan_tpid = Some(0x8100);
        writer.write(&frame).unwrap();
        writer.finish();
        let mut stream = PcapStream::open(&path).unwrap();
        let mut out = Vec::new();
        stream.poll_eth_frames(u64::MAX, &mut out);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].vlan, Some(100));
        assert_eq!(out[0].ethertype, 0x0800);
        assert_eq!(out[0].payload.as_ref(), [0xaa, 0xbb, 0xcc, 0xdd]);
        std::fs::remove_file(&path).ok();
    }
}
