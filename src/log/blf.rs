//! Vector's binary CAN log format, as written by CANoe. Every offset here is
//! python-can's `can/io/blf.py` struct field-for-field; that reader has parsed
//! real Vector exports for years, so a CANoe file opens without re-encoding and
//! anything genuinely unparseable shows up as `BadSignature` rather than as
//! frames that look plausible and are wrong.
//!
//! A file is a fixed header -- whose declared size says where the records
//! start -- followed by containers of raw or zlib-deflate objects. Object
//! headers come in v1 (32 B) and v2 (40 B) sharing a 16 B base.

use std::collections::VecDeque;
use std::io::Read;
use std::path::Path;

use flate2::read::ZlibDecoder;

use crate::can::frame::{CanFrame, Direction, FrameFlags, MAX_CAN_FD_LEN, dlc2len};
use crate::log::backing::Backing;
use crate::log::error::LogError;
use crate::source::FrameStream;

const FILE_HEADER_SIZE: usize = 144;
/// Vector's `LOGG_FileHeader` starts with the literal `LOGG`. Real files carry
/// no "BLF4" string anywhere -- an earlier revision of this reader required it
/// and so rejected every genuine export while happily accepting our own
/// synthetic fixtures.
const FILE_SIGNATURE: &[u8; 4] = b"LOGG";
const OBJECT_SIGNATURE: &[u8; 4] = b"LOBJ";

/// Byte offsets inside the file header, per Vector's
/// `struct.Struct("<4sLBBBBBBBBQQLL8H8H")` (4s@0, L@4, 8*B@8, Q@16, Q@24,
/// L@32, L@36, 8*H@40, 8*H@56).
const HDR_OBJECT_COUNT: usize = 32;
const HDR_START_TIME: usize = 40;
const HDR_STOP_TIME: usize = 56;
/// Declared length of the padded file header, at offset 4.
const HDR_HEADER_SIZE: usize = 4;
/// `FILE_HEADER_STRUCT` occupies 72 of the 144 bytes; the declared header size
/// says where the first object starts.
const FILE_HEADER_STRUCT_SIZE: usize = 72;
/// Smallest record an LOBJ header can describe. Stepping by less than this
/// would point the walk at the signature it just read.
const MIN_OBJECT_SIZE: usize = 32;

const METHOD_RAW: u16 = 0;
const METHOD_ZLIB: u16 = 2;

const OBJ_CAN_MESSAGE: u32 = 1;
const OBJ_LOG_CONTAINER: u32 = 10;
const OBJ_CAN_MESSAGE2: u32 = 86;
const OBJ_CAN_FD_MESSAGE: u32 = 100;
const OBJ_CAN_FD_MESSAGE_64: u32 = 101;
const OBJ_CAN_ERROR_EXT: u32 = 73;

/// Object-header flag values steering the timestamp interpretation. Vector's
/// reference reader treats `flags == 1` as 10 µs ticks and *anything else* as
/// nanoseconds -- there is no "already microseconds" case.
const TS_TEN_MICRO: u32 = 0x0000_0001;

/// Arbitration id bit 31 marks an extended frame; the id itself is the low 29
/// bits.
const CAN_ID_EXT: u32 = 0x8000_0000;
const CAN_ID_MASK: u32 = 0x1FFF_FFFF;

/// Message flags byte: bit 0 is direction (set = Tx), bit 7 is RTR.
const CAN_DIR_TX: u8 = 0x01;
const CAN_FLAG_REMOTE: u8 = 0x80;

/// `CAN_FD_MESSAGE` (100) carries its FD bits in one byte.
const FD_FLAG_EDL: u8 = 0x01;
const FD_FLAG_BRS: u8 = 0x02;
const FD_FLAG_ESI: u8 = 0x04;

/// `CAN_FD_MESSAGE_64` (101) is a different dialect again: the FD bits live in
/// a u32 at bit 12 and up, RTR is bit 4, and direction is a separate field.
const FD64_RTR: u32 = 0x0010;
const FD64_EDL: u32 = 0x1000;
const FD64_BRS: u32 = 0x2000;
const FD64_ESI: u32 = 0x4000;

/// Size of the `CAN_FD_MESSAGE_64` fixed record; the payload follows it.
const FD64_STRUCT_SIZE: usize = 40;

pub struct BlfStream {
    data: Backing,
    pos: usize,
    /// Offset just past the file header, as declared by the header itself.
    objects_at: usize,
    duration: Option<u64>,
    describe: String,
    pending: VecDeque<CanFrame>,
    /// Ascending `(rebased t_us, container byte offset)` index, grown as
    /// containers are read. See [`Self::note_checkpoint`].
    checkpoints: Vec<(u64, usize)>,
    /// Absolute t_us of the first frame we emitted; every subsequent
    /// frame is rebased by this so `ReplaySource::pos_us = 0` matches the
    /// log's opening record.
    t_base: Option<u64>,
    /// Rebased max t_us seen so far; only used when the file header's
    /// stop SYSTEMTIME is missing (files still being written).
    t_last: u64,
}

impl BlfStream {
    pub fn open(path: &Path) -> Result<Self, LogError> {
        let data = Backing::map_path(path)?;
        let name = path
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        Self::build(data, Some(name))
    }

    // Test-only entry that avoids the filesystem.
    #[cfg(test)]
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, LogError> {
        Self::build(Backing::owned(bytes), None)
    }

    fn build(data: Backing, name_hint: Option<String>) -> Result<Self, LogError> {
        let header = read_file_header(data.as_slice())?;
        let (duration, count) = (header.duration, header.count);
        let kind = data.kind();
        let describe = {
            let head = match duration {
                Some(d) => format!("BLF4 [{kind}], {:.1} s, {} objects", d as f64 / 1e6, count),
                None => format!("BLF4 [{kind}], {} objects", count),
            };
            match name_hint {
                Some(n) if !n.is_empty() => format!("{head} {n}"),
                _ => head,
            }
        };
        Ok(BlfStream {
            data,
            pos: header.objects_at,
            objects_at: header.objects_at,
            duration,
            describe,
            pending: VecDeque::new(),
            checkpoints: Vec::new(),
            t_base: None,
            t_last: 0,
        })
    }

    /// Refill the pending queue from the next container. Returns false on
    /// EOF or when the trailing bytes cannot form another valid header.
    fn enter_next_container(&mut self) -> bool {
        let bytes = self.data.as_slice();
        let container_start = match next_object(bytes, self.pos) {
            Some(at) => at,
            None => {
                self.pos = bytes.len();
                return false;
            }
        };
        let hdr = match parse_object_header(&bytes[container_start..]) {
            Ok(h) => h,
            Err(_) => {
                self.pos = bytes.len();
                return false;
            }
        };
        let header_len = hdr.header_size as usize;
        let object_total = hdr.object_size as usize;
        let body_end = container_start + object_total;
        if !(MIN_OBJECT_SIZE..).contains(&object_total)
            || object_total < header_len
            || body_end > bytes.len()
        {
            self.pos = bytes.len();
            return false;
        }
        let body_start = container_start + header_len;
        self.pos = body_end;
        if hdr.object_type != OBJ_LOG_CONTAINER || body_end - body_start < 16 {
            return true; // not a container, or an empty one: step over it
        }
        let body = &bytes[body_start..body_end];
        // `LOG_CONTAINER_STRUCT = <H6xL4x`: method at 0, uncompressed size at 8,
        // payload from 16. The stored length is not a field -- it is whatever
        // remains of the object, so reading a "compressed" u32 at offset 8
        // mistook the uncompressed size for it.
        let method = u16_at(body, 0);
        let uncompressed = u32_at(body, 8) as usize;
        let payload = body.get(16..).unwrap_or(&[]);
        let decoded: Vec<u8> = match method {
            METHOD_RAW => payload.to_vec(),
            METHOD_ZLIB => {
                let mut out = Vec::with_capacity(uncompressed.max(payload.len()));
                if ZlibDecoder::new(payload).read_to_end(&mut out).is_err() {
                    return true;
                }
                out
            }
            _ => {
                // Unknown compression: skip the container rather than
                // risk yielding frames we decoded wrongly.
                return true;
            }
        };
        parse_container_objects(&decoded, &mut self.pending);
        if let Some(first) = self.pending.front() {
            let raw = first.t_us;
            self.note_checkpoint(raw, container_start);
        }
        true
    }

    /// Keeps a `(rebased t_us -> byte offset)` entry per container we walk
    /// into. BLF carries no timestamps in the container *header*, so this is
    /// the finest index available without a full decompress pass up front.
    fn note_checkpoint(&mut self, raw_us: u64, container_start: usize) {
        let t = self.rebase(raw_us);
        let at = self.checkpoints.partition_point(|(ct, _)| *ct < t);
        if self
            .checkpoints
            .get(at)
            .is_some_and(|(ct, p)| *ct == t && *p == container_start)
        {
            return;
        }
        self.checkpoints.insert(at, (t, container_start));
    }

    fn rebase(&mut self, raw_us: u64) -> u64 {
        let base = *self.t_base.get_or_insert(raw_us);
        let t = raw_us.saturating_sub(base);
        if t > self.t_last {
            self.t_last = t;
        }
        t
    }
}

impl FrameStream for BlfStream {
    fn peek_t(&mut self) -> Option<u64> {
        loop {
            if let Some(f) = self.pending.front() {
                let raw = f.t_us;
                return Some(self.rebase(raw));
            }
            if !self.enter_next_container() {
                return None;
            }
        }
    }

    fn next_frame(&mut self) -> Option<CanFrame> {
        loop {
            if let Some(mut f) = self.pending.pop_front() {
                f.t_us = self.rebase(f.t_us);
                return Some(f);
            }
            if !self.enter_next_container() {
                return None;
            }
        }
    }

    fn seek_to_us(&mut self, target: u64) -> Option<u64> {
        match self.checkpoints.partition_point(|(t, _)| *t <= target) {
            0 => self.pos = self.objects_at,
            k => self.pos = self.checkpoints[k - 1].1,
        }
        // Queue state is pure; `t_base` deliberately survives so a scrub does
        // not move the log's zero point under the playhead.
        self.pending.clear();
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
        if self.duration.is_some() {
            return self.duration;
        }
        if self.t_last > 0 {
            Some(self.t_last)
        } else {
            None
        }
    }

    fn describe(&self) -> String {
        self.describe.clone()
    }
}

struct ObjectHeader {
    header_size: u16,
    header_version: u8,
    object_size: u32,
    object_type: u32,
    timestamp: u64,
    flags: u32,
}

/// `OBJ_HEADER_BASE_STRUCT = <4sHHLL` followed by `<LHHQ` (v1) or `<LBxHQ8x`
/// (v2). Both dialects agree where the fields we need live -- object type at 12,
/// header flags at 16, timestamp at 24 -- and only the total header length
/// differs, which `header_size` states outright.
///
/// An unknown `header_version` is *not* an error here: Vector's reader skips
/// such objects rather than abandoning the file, and a container's layout never
/// depends on the version.
fn parse_object_header(bytes: &[u8]) -> Result<ObjectHeader, LogError> {
    if bytes.len() < 32 {
        return Err(LogError::Truncated);
    }
    if &bytes[0..4] != OBJECT_SIGNATURE {
        return Err(LogError::BadSignature);
    }
    Ok(ObjectHeader {
        header_size: u16_at(bytes, 4),
        header_version: bytes[6],
        object_size: u32_at(bytes, 8),
        object_type: u32_at(bytes, 12),
        flags: u32_at(bytes, 16),
        timestamp: u64_at(bytes, 24),
    })
}

/// Vector's reference reader treats `flags == 1` as 10 µs ticks and **any other
/// value** as nanoseconds -- there is no "already microseconds" case, so
/// assuming one inflates every stamp by a factor of 1000.
fn object_time(raw: u64, flags: u32) -> u64 {
    if flags == TS_TEN_MICRO {
        raw.saturating_mul(10)
    } else {
        raw / 1_000
    }
}

/// Locate the next object. Vector pads records so `object_size` alone does not
/// always land on the following signature, and python-can's reader likewise
/// searches for the next `LOBJ` within the first eight bytes.
fn next_object(bytes: &[u8], from: usize) -> Option<usize> {
    let rest = bytes.get(from..)?;
    let offset = rest[..rest.len().min(8)]
        .windows(4)
        .position(|w| w == OBJECT_SIGNATURE)?;
    Some(from + offset)
}

/// Decode every recognised object inside one container. Objects with
/// unknown types are stepped over using `object_size`, so CANoe files
/// that mix CAN with LIN/Ethernet still yield their CAN traffic.
fn parse_container_objects(bytes: &[u8], out: &mut VecDeque<CanFrame>) {
    let mut pos = match next_object(bytes, 0) {
        Some(at) => at,
        None => return,
    };
    while let Ok(h) = parse_object_header(&bytes[pos..]) {
        let size = h.object_size as usize;
        let header_len = h.header_size as usize;
        let next = pos + size;
        if !(MIN_OBJECT_SIZE..).contains(&size) || size < header_len || next > bytes.len() {
            break;
        }
        // Only versions 1 and 2 have a known timestamp field; anything else is
        // stepped over whole rather than decoded from guessed offsets.
        if matches!(h.header_version, 1 | 2) {
            let body = &bytes[pos + header_len..next];
            let t_us = object_time(h.timestamp, h.flags);
            let decoded = match h.object_type {
                OBJ_CAN_MESSAGE | OBJ_CAN_MESSAGE2 => decode_can_msg(body, t_us),
                OBJ_CAN_FD_MESSAGE => decode_fd_message(body, t_us),
                OBJ_CAN_FD_MESSAGE_64 => decode_fd_message_64(body, t_us, header_len, size),
                OBJ_CAN_ERROR_EXT => Some(decode_error_ext(body, t_us)),
                _ => None,
            };
            if let Some(f) = decoded {
                out.push_back(f);
            }
        }
        pos = match next_object(bytes, next) {
            Some(at) => at,
            None => break,
        };
    }
}

#[inline]
fn u32_at(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]])
}
#[inline]
fn u16_at(b: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([b[at], b[at + 1]])
}
#[inline]
fn u64_at(b: &[u8], at: usize) -> u64 {
    u64::from_le_bytes([
        b[at],
        b[at + 1],
        b[at + 2],
        b[at + 3],
        b[at + 4],
        b[at + 5],
        b[at + 6],
        b[at + 7],
    ])
}

/// `CAN_MSG_STRUCT = <HBBL8s`, shared by CAN_MESSAGE (1) and CAN_MESSAGE2 (86):
/// u16 1-based channel, flags byte (bit 0 = direction, bit 7 = RTR), dlc byte,
/// u32 arbitration id (bit 31 = extended), then eight data bytes.
fn decode_can_msg(body: &[u8], t_us: u64) -> Option<CanFrame> {
    if body.len() < 16 {
        return None;
    }
    let channel = u16_at(body, 0);
    let msg_flags = body[2];
    let dlc = body[3];
    let raw_id = u32_at(body, 4);
    let remote = msg_flags & CAN_FLAG_REMOTE != 0;
    // Vector stores a plain byte count here and slices the eight-byte field
    // with it, so a non-canonical value clamps to eight rather than going
    // through the FD ladder. One Vector unittest file carries dlc 0x33.
    let len = if remote {
        dlc2len(dlc & 0x0F)
    } else {
        (dlc as usize).min(8) as u8
    };
    let mut data = [0u8; MAX_CAN_FD_LEN];
    if !remote {
        data[..len as usize].copy_from_slice(&body[8..8 + len as usize]);
    }
    let mut flags = FrameFlags::NONE;
    if remote {
        flags = flags.union(FrameFlags::RTR);
    }
    Some(CanFrame {
        t_us,
        channel: channel.saturating_sub(1) as u8,
        id: raw_id & CAN_ID_MASK,
        extended: raw_id & CAN_ID_EXT != 0,
        len,
        data,
        dir: if msg_flags & CAN_DIR_TX != 0 {
            Direction::Tx
        } else {
            Direction::Rx
        },
        flags,
    })
}

/// `CAN_FD_MSG_STRUCT = <HBBLLBBB5x64s`: u16 1-based channel, flags byte (bit 0
/// direction, bit 7 RTR), dlc byte, u32 id, u32 frame length, bit count at 12,
/// FD flags at 13 (EDL 0x1 / BRS 0x2 / ESI 0x4), valid byte count at 14, then 64
/// data bytes from offset 20.
fn decode_fd_message(body: &[u8], t_us: u64) -> Option<CanFrame> {
    if body.len() < 20 {
        return None;
    }
    let channel = u16_at(body, 0);
    let msg_flags = body[2];
    let dlc = body[3];
    let raw_id = u32_at(body, 4);
    let fd_flags = body[13];
    let valid = body[14];
    let len = valid.min(MAX_CAN_FD_LEN as u8);
    let mut data = [0u8; MAX_CAN_FD_LEN];
    let avail = body.len().saturating_sub(20);
    let copied = len as usize;
    if copied <= avail {
        data[..copied].copy_from_slice(&body[20..20 + copied]);
    }
    let mut flags = FrameFlags::NONE;
    if fd_flags & FD_FLAG_EDL != 0 {
        flags = flags.union(FrameFlags::FD);
    }
    if fd_flags & FD_FLAG_BRS != 0 {
        flags = flags.union(FrameFlags::BRS);
    }
    if fd_flags & FD_FLAG_ESI != 0 {
        flags = flags.union(FrameFlags::ESI);
    }
    if msg_flags & CAN_FLAG_REMOTE != 0 {
        flags = flags.union(FrameFlags::RTR);
    }
    Some(CanFrame {
        t_us,
        channel: channel.saturating_sub(1) as u8,
        id: raw_id & CAN_ID_MASK,
        extended: raw_id & CAN_ID_EXT != 0,
        len: dlc2len(dlc).min(len.max(1)),
        data,
        dir: if msg_flags & CAN_DIR_TX != 0 {
            Direction::Tx
        } else {
            Direction::Rx
        },
        flags,
    })
}

/// `CAN_FD_MSG_64_STRUCT = <BBBBLLLLLLLHBBL` (40 bytes, payload follows):
/// u8 1-based channel, dlc, valid byte count, tx count, u32 id, frame length,
/// **u32** FD flags (EDL 0x1000 / BRS 0x2000 / ESI 0x4000 / RTR 0x0010 -- a
/// different dialect from `CAN_FD_MESSAGE`), bit rates and offsets, a direction
/// byte at 34 and an `extDataOffset` byte at 35.
fn decode_fd_message_64(
    body: &[u8],
    t_us: u64,
    header_size: usize,
    object_size: usize,
) -> Option<CanFrame> {
    if body.len() < FD64_STRUCT_SIZE {
        return None;
    }
    let channel = body[0];
    let valid = body[2];
    let raw_id = u32_at(body, 4);
    let fd_flags = u32_at(body, 12);
    let direction = body[34];
    let ext_data_offset = body[35] as usize;
    let mut data = [0u8; MAX_CAN_FD_LEN];
    // `valid_bytes` can exceed what the record actually carries -- Vector's
    // issue 1905 file declares 64 and stores 48. The data field stops at
    // `extDataOffset` when it is set and at the end of the object otherwise,
    // and CANoe shows the shortfall as zero padding.
    let field_end = if ext_data_offset > 0 {
        ext_data_offset
    } else {
        object_size
    };
    let limit = field_end
        .saturating_sub(header_size)
        .saturating_sub(FD64_STRUCT_SIZE)
        .min(body.len() - FD64_STRUCT_SIZE);
    let copied = (valid as usize).min(limit).min(MAX_CAN_FD_LEN);
    data[..copied].copy_from_slice(&body[FD64_STRUCT_SIZE..FD64_STRUCT_SIZE + copied]);
    let mut flags = FrameFlags::NONE;
    if fd_flags & FD64_EDL != 0 {
        flags = flags.union(FrameFlags::FD);
    }
    if fd_flags & FD64_BRS != 0 {
        flags = flags.union(FrameFlags::BRS);
    }
    if fd_flags & FD64_ESI != 0 {
        flags = flags.union(FrameFlags::ESI);
    }
    if fd_flags & FD64_RTR != 0 {
        flags = flags.union(FrameFlags::RTR);
    }
    Some(CanFrame {
        t_us,
        channel: channel.saturating_sub(1),
        id: raw_id & CAN_ID_MASK,
        extended: raw_id & CAN_ID_EXT != 0,
        len: (valid as usize).min(MAX_CAN_FD_LEN) as u8,
        data,
        dir: if direction != 0 {
            Direction::Tx
        } else {
            Direction::Rx
        },
        flags,
    })
}

/// `CAN_ERROR_EXT_STRUCT = <HHLBBBxLLH2x8s`: u16 1-based channel at 0, dlc byte
/// at 10, u32 arbitration id at 16, payload at 24. Error frames have no
/// identifier on the wire, so the id is normalised to zero and the payload
/// dropped -- see the aggregation contract on `FrameFlags::ERROR`.
fn decode_error_ext(body: &[u8], t_us: u64) -> CanFrame {
    let channel = if body.len() >= 2 {
        u16_at(body, 0).saturating_sub(1) as u8
    } else {
        0
    };
    let extended = body.len() >= 20 && u32_at(body, 16) & CAN_ID_EXT != 0;
    CanFrame {
        t_us,
        channel,
        id: 0,
        extended,
        len: 0,
        data: [0u8; MAX_CAN_FD_LEN],
        dir: Direction::Rx,
        flags: FrameFlags::ERROR,
    }
}

/// Validates the file signature and reads `object_count_total` plus the
/// start/stop SYSTEMTIMEs. Missing stop stamps give `duration = None`.
fn read_file_header(bytes: &[u8]) -> Result<FileHeader, LogError> {
    if bytes.len() < FILE_HEADER_SIZE {
        return Err(LogError::Truncated);
    }
    if &bytes[0..4] != FILE_SIGNATURE {
        return Err(LogError::BadSignature);
    }
    let count = u32_at(bytes, HDR_OBJECT_COUNT);
    let start = system_time_us(bytes, HDR_START_TIME);
    let stop = system_time_us(bytes, HDR_STOP_TIME);
    let duration = match (start, stop) {
        (Some(a), Some(b)) if b > a => Some(b - a),
        _ => None,
    };
    // The declared size is the only thing that says how the header was padded.
    let declared = u32_at(bytes, HDR_HEADER_SIZE) as usize;
    let objects_at = if (FILE_HEADER_STRUCT_SIZE..bytes.len()).contains(&declared) {
        declared
    } else {
        FILE_HEADER_SIZE
    };
    Ok(FileHeader {
        duration,
        count,
        objects_at,
    })
}

struct FileHeader {
    duration: Option<u64>,
    count: u32,
    objects_at: usize,
}

/// Vector SYSTEMTIME: 8 consecutive u16 fields
/// (year, month, day-of-week, day, hour, minute, second, millisecond).
fn system_time_us(b: &[u8], at: usize) -> Option<u64> {
    if b.len() < at + 16 {
        return None;
    }
    let year = u16_at(b, at);
    let month = u16_at(b, at + 2);
    let day = u16_at(b, at + 6);
    let hour = u16_at(b, at + 8);
    let min = u16_at(b, at + 10);
    let sec = u16_at(b, at + 12);
    let ms = u16_at(b, at + 14);
    if year == 0 || month == 0 || day == 0 {
        return None;
    }
    use chrono::{NaiveDate, NaiveDateTime, NaiveTime};
    let date = NaiveDate::from_ymd_opt(year as i32, month as u32, day as u32)?;
    let time = NaiveTime::from_hms_milli_opt(hour as u32, min as u32, sec as u32, ms as u32)?;
    let dt = NaiveDateTime::new(date, time);
    let secs = dt.and_utc().timestamp();
    let micros = dt.and_utc().timestamp_subsec_micros();
    Some(
        (secs as u64)
            .saturating_mul(1_000_000)
            .saturating_add(micros as u64),
    )
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::log::vec_stream::VecStream;
    use std::io::Write;

    /// A one-frame log built by the encoders below, for tests elsewhere in the
    /// crate that only need bytes the reader accepts.
    pub(crate) fn minimal_file() -> Vec<u8> {
        let body = can_body(0, 1, 0, 0x100, &[0xAB]);
        assemble(&obj_header_v1(OBJ_CAN_MESSAGE, ns(1_000_000), 0, &body))
    }

    /// Vector's `TIME_ONE_NANS`. The reader never names it -- nanoseconds are
    /// the default branch -- so only the tests need the value.
    const TS_ONE_NANO: u32 = 2;

    // Test-only encoders that mirror the decoder field-for-field. Keeping
    // them in the same file means a reader change is caught the moment it
    // stops matching the writer; the offsets below are taken from Vector's own
    // struct definitions so both sides agree with the format, not just each
    // other.
    /// A `flags == 0` stamp is nanoseconds, so express test timestamps in the
    /// microseconds the reader is asserted against.
    fn ns(micros: u64) -> u64 {
        micros * 1_000
    }

    /// `OBJ_HEADER_BASE_STRUCT = <4sHHLL` (signature, header size, header
    /// version, object size, object type) followed by the `OBJ_HEADER_V1_STRUCT
    /// = <LHHQ` or `OBJ_HEADER_V2_STRUCT = <LBxHQ8x` tail. Both tails put flags
    /// at 16 and the timestamp at 24, so only the header length differs.
    fn obj_header(version: u8, object_type: u32, ts_raw: u64, flags: u32, body: &[u8]) -> Vec<u8> {
        let header_size: u16 = if version == 1 { 32 } else { 40 };
        let total = usize::from(header_size) + body.len();
        let mut v = Vec::with_capacity(total);
        v.extend_from_slice(b"LOBJ");
        v.extend_from_slice(&header_size.to_le_bytes()); // 4..6
        v.push(version); // 6
        v.push(0); // 7 object version
        v.extend_from_slice(&(total as u32).to_le_bytes()); // 8..12
        v.extend_from_slice(&object_type.to_le_bytes()); // 12..16
        v.extend_from_slice(&flags.to_le_bytes()); // 16..20
        if version == 1 {
            v.extend_from_slice(&0u16.to_le_bytes()); // 20..22 client index
        } else {
            v.push(0); // 20 timestamp status
            v.push(0); // 21 pad
        }
        v.extend_from_slice(&0u16.to_le_bytes()); // 22..24 object version
        v.extend_from_slice(&ts_raw.to_le_bytes()); // 24..32
        if version == 2 {
            v.extend_from_slice(&[0u8; 8]); // 32..40 original timestamp
        }
        v.extend_from_slice(body);
        v
    }

    fn obj_header_v1(object_type: u32, ts_raw: u64, flags: u32, body: &[u8]) -> Vec<u8> {
        obj_header(1, object_type, ts_raw, flags, body)
    }

    fn obj_header_v2(object_type: u32, ts_raw: u64, flags: u32, body: &[u8]) -> Vec<u8> {
        obj_header(2, object_type, ts_raw, flags, body)
    }

    /// A `LOG_CONTAINER` object: the 16 B base header is followed straight by
    /// `LOG_CONTAINER_STRUCT = <H6xL4x` (method u16 at 0, uncompressed u32 at
    /// 8), which occupies the bytes a message header's tail would use -- hence
    /// `header_size = 16`. There is no compressed-length field; the payload
    /// runs to the end of the object.
    fn wrap_container(objects: &[u8], method: u16, encoder: fn(&[u8]) -> Vec<u8>) -> Vec<u8> {
        let (uncompressed, payload) = if method == METHOD_ZLIB {
            (objects.len(), encoder(objects))
        } else {
            (objects.len(), objects.to_vec())
        };
        let total = 16 + 16 + payload.len();
        let mut out = Vec::with_capacity(total);
        out.extend_from_slice(b"LOBJ");
        out.extend_from_slice(&16u16.to_le_bytes()); // header_size
        out.push(1); // header_version
        out.push(0); // object_version
        out.extend_from_slice(&(total as u32).to_le_bytes());
        out.extend_from_slice(&OBJ_LOG_CONTAINER.to_le_bytes()); // 12..16 type
        out.extend_from_slice(&method.to_le_bytes()); // 16..18
        out.extend_from_slice(&[0u8; 6]); // 18..24
        out.extend_from_slice(&(uncompressed as u32).to_le_bytes()); // 24..28
        out.extend_from_slice(&[0u8; 4]); // 28..32
        out.extend_from_slice(&payload);
        out
    }

    fn raw_container(objects: &[u8]) -> Vec<u8> {
        wrap_container(objects, METHOD_RAW, |b| b.to_vec())
    }

    fn zlib_container(objects: &[u8]) -> Vec<u8> {
        use flate2::Compression;
        use flate2::write::ZlibEncoder;
        wrap_container(objects, METHOD_ZLIB, |b| {
            let mut e = ZlibEncoder::new(Vec::new(), Compression::default());
            e.write_all(b).unwrap();
            e.finish().unwrap()
        })
    }

    type SystemTimeTuple = (u16, u16, u16, u16, u16, u16, u16, u16);

    fn put_time(v: &mut [u8], at: usize, t: SystemTimeTuple) {
        for (i, field) in [t.0, t.1, t.2, t.3, t.4, t.5, t.6, t.7].iter().enumerate() {
            v[at + i * 2..at + i * 2 + 2].copy_from_slice(&field.to_le_bytes());
        }
    }

    /// Mirrors `LOGG_FileHeader`: "LOGG" at 0, header size at 4, object count at
    /// 32, start SYSTEMTIME at 40, stop at 56. An earlier revision wrote "BLF4"
    /// and the wrong offsets, so the whole suite stayed green against a reader
    /// that could not open a single real file.
    fn file_header(start: Option<SystemTimeTuple>, stop: Option<SystemTimeTuple>) -> Vec<u8> {
        let mut v = vec![0u8; FILE_HEADER_SIZE];
        v[0..4].copy_from_slice(b"LOGG");
        v[4..8].copy_from_slice(&(FILE_HEADER_SIZE as u32).to_le_bytes());
        if let Some(t) = start {
            put_time(&mut v, HDR_START_TIME, t);
        }
        if let Some(t) = stop {
            put_time(&mut v, HDR_STOP_TIME, t);
        }
        v
    }

    /// The standard 144-byte header widened by `extra`, with the declared
    /// header size at offset 4 following suit.
    fn padded_file_header(extra: &[u8]) -> Vec<u8> {
        let mut v = file_header(
            Some((2024, 1, 1, 1, 0, 0, 0, 0)),
            Some((2024, 1, 1, 1, 0, 0, 5, 0)),
        );
        let declared = (v.len() + extra.len()) as u32;
        v[4..8].copy_from_slice(&declared.to_le_bytes());
        v.extend_from_slice(extra);
        v
    }

    fn assemble(objects: &[u8]) -> Vec<u8> {
        let mut v = file_header(
            Some((2024, 1, 1, 1, 0, 0, 0, 0)),
            Some((2024, 1, 1, 1, 0, 0, 5, 0)),
        );
        v.extend_from_slice(&raw_container(objects));
        v
    }

    /// `CAN_MSG_STRUCT = <HBBL8s`, shared by CAN_MESSAGE (1) and CAN_MESSAGE2
    /// (86). Vector stores the channel one-based, so `channel0` is written as
    /// `channel0 + 1` and a reader that forgets to subtract fails.
    fn can_body(channel0: u8, dlc: u8, flags: u8, id: u32, data: &[u8]) -> Vec<u8> {
        let mut b = vec![0u8; 16];
        b[0..2].copy_from_slice(&(u16::from(channel0) + 1).to_le_bytes());
        b[2] = flags;
        b[3] = dlc;
        b[4..8].copy_from_slice(&id.to_le_bytes());
        let n = data.len().min(8);
        b[8..8 + n].copy_from_slice(&data[..n]);
        b
    }

    /// `CAN_FD_MSG_STRUCT = <HBBLLBBB5x64s`: frame length at 8, bit count at 12,
    /// FD flags at 13, valid byte count at 14, payload at 20.
    fn fd_body(
        channel0: u8,
        dlc: u8,
        valid: u8,
        dir_tx: bool,
        fd_flags: u8,
        id: u32,
        data: &[u8],
    ) -> Vec<u8> {
        let mut b = vec![0u8; 20 + MAX_CAN_FD_LEN];
        b[0..2].copy_from_slice(&(u16::from(channel0) + 1).to_le_bytes());
        b[2] = if dir_tx { CAN_DIR_TX } else { 0 };
        b[3] = dlc;
        b[4..8].copy_from_slice(&id.to_le_bytes());
        b[8..12].copy_from_slice(&(data.len() as u32).to_le_bytes());
        b[13] = fd_flags;
        b[14] = valid;
        let n = data.len().min(MAX_CAN_FD_LEN);
        b[20..20 + n].copy_from_slice(&data[..n]);
        b
    }

    /// Field values for [`fd64_body`], which otherwise takes eight positions.
    #[derive(Default)]
    struct Fd64 {
        channel0: u8,
        dlc: u8,
        valid: u8,
        tx: bool,
        fd_flags: u32,
        id: u32,
        /// Absolute offset inside the object that bounds the data field, or 0
        /// to let the object size bound it.
        ext_data_offset: u8,
        data: Vec<u8>,
    }

    /// `CAN_FD_MSG_64_STRUCT = <BBBBLLLLLLLHBBL` (40 B) with the payload right
    /// after it.
    fn fd64_body(f: Fd64) -> Vec<u8> {
        let mut b = vec![0u8; FD64_STRUCT_SIZE + f.data.len()];
        b[0] = f.channel0 + 1;
        b[1] = f.dlc;
        b[2] = f.valid;
        b[4..8].copy_from_slice(&f.id.to_le_bytes());
        b[8..12].copy_from_slice(&(f.data.len() as u32).to_le_bytes());
        b[12..16].copy_from_slice(&f.fd_flags.to_le_bytes());
        b[34] = u8::from(f.tx);
        b[35] = f.ext_data_offset;
        b[FD64_STRUCT_SIZE..].copy_from_slice(&f.data);
        b
    }

    /// `CAN_ERROR_EXT_STRUCT = <HHLBBBxLLH2x8s`: dlc at 10, arbitration id at 16,
    /// eight data bytes at 24.
    fn error_ext_body(channel0: u8, dlc: u8, id: u32, data: &[u8]) -> Vec<u8> {
        let mut b = vec![0u8; 32];
        b[0..2].copy_from_slice(&(u16::from(channel0) + 1).to_le_bytes());
        b[10] = dlc;
        b[16..20].copy_from_slice(&id.to_le_bytes());
        let n = data.len().min(8);
        b[24..24 + n].copy_from_slice(&data[..n]);
        b
    }

    #[test]
    fn rejects_bad_signature() {
        let mut v = vec![0u8; FILE_HEADER_SIZE];
        v[0..4].copy_from_slice(b"XXXX");
        match BlfStream::from_bytes(&v) {
            Err(LogError::BadSignature) => {}
            Err(e) => panic!("expected BadSignature, got {e}"),
            Ok(_) => panic!("expected BadSignature, got Ok"),
        }
    }

    #[test]
    fn rejects_truncated_file() {
        let v = *b"LOGG";
        match BlfStream::from_bytes(&v) {
            Err(LogError::Truncated) => {}
            Err(e) => panic!("expected Truncated, got {e}"),
            Ok(_) => panic!("expected Truncated, got Ok"),
        }
    }

    #[test]
    fn empty_stream_has_no_frames() {
        let v = file_header(None, None);
        let mut s = BlfStream::from_bytes(&v).unwrap();
        assert!(s.peek_t().is_none());
        assert!(s.next_frame().is_none());
    }

    #[test]
    fn decodes_classic_can_message() {
        let body = can_body(1, 8, 0, 0x1A4, &[1, 2, 3, 4, 5, 6, 7, 8]);
        let obj = obj_header_v1(OBJ_CAN_MESSAGE, ns(12_345_000), 0, &body);
        let mut s = BlfStream::from_bytes(&assemble(&obj)).unwrap();
        let f = s.next_frame().expect("frame");
        // First frame is rebased to zero.
        assert_eq!(f.t_us, 0);
        assert_eq!(f.channel, 1);
        assert_eq!(f.id, 0x1A4);
        assert_eq!(f.len, 8);
        assert_eq!(f.payload(), &[1, 2, 3, 4, 5, 6, 7, 8]);
        assert!(!f.extended);
        assert!(!f.is_fd());
        assert!(!f.is_error());
    }

    #[test]
    fn rebase_shifts_every_frame() {
        let a_body = can_body(0, 1, 0, 0x100, &[0xAA]);
        let b_body = can_body(0, 1, 0, 0x101, &[0xBB]);
        let a = obj_header_v1(OBJ_CAN_MESSAGE, ns(5_000_000), 0, &a_body);
        let b = obj_header_v1(OBJ_CAN_MESSAGE, ns(6_000_000), 0, &b_body);
        let mut objs = Vec::new();
        objs.extend_from_slice(&a);
        objs.extend_from_slice(&b);
        let mut s = BlfStream::from_bytes(&assemble(&objs)).unwrap();
        let f1 = s.next_frame().unwrap();
        let f2 = s.next_frame().unwrap();
        assert_eq!(f1.t_us, 0, "first frame rebased to zero");
        assert_eq!(f2.t_us, 1_000_000, "delta preserved across rebase");
    }

    #[test]
    fn decodes_can_message2_with_direction_and_extended() {
        // CAN_MESSAGE2 is the same record as CAN_MESSAGE; bit 31 of the
        // arbitration id is what makes it an extended frame.
        let body = can_body(
            2,
            4,
            CAN_DIR_TX,
            0x1DB3_FFFD | CAN_ID_EXT,
            &[0xDE, 0xAD, 0xBE, 0xEF],
        );
        let obj = obj_header_v1(OBJ_CAN_MESSAGE2, ns(7_000_000), 0, &body);
        let mut s = BlfStream::from_bytes(&assemble(&obj)).unwrap();
        let f = s.next_frame().unwrap();
        assert_eq!(f.channel, 2);
        assert_eq!(f.dir, Direction::Tx);
        assert!(f.extended);
        assert_eq!(f.id, 0x1DB3_FFFD);
        assert_eq!(f.len, 4);
        assert_eq!(f.payload(), &[0xDE, 0xAD, 0xBE, 0xEF]);
    }

    #[test]
    fn decodes_fd_message_with_brs_and_esi() {
        let payload: Vec<u8> = (0..48u8).collect();
        let body = fd_body(
            3,
            14,
            48,
            true,
            FD_FLAG_EDL | FD_FLAG_BRS | FD_FLAG_ESI,
            0x1234_5678 | CAN_ID_EXT,
            &payload,
        );
        let obj = obj_header_v1(OBJ_CAN_FD_MESSAGE, ns(20_000_000), 0, &body);
        let mut s = BlfStream::from_bytes(&assemble(&obj)).unwrap();
        let f = s.next_frame().unwrap();
        assert!(f.is_fd());
        assert!(f.brs());
        assert!(f.esi());
        assert_eq!(f.len, 48);
        assert_eq!(f.dlc_code(), 14);
        assert_eq!(f.payload(), &payload[..]);
        assert_eq!(f.channel, 3);
        assert_eq!(f.dir, Direction::Tx);
        assert!(f.extended);
        assert_eq!(f.id, 0x1234_5678);
    }

    #[test]
    fn decodes_error_ext_frame() {
        // Values taken from Vector's `test_CanErrorFrameExt.blf`: an extended id
        // of 0x19999999 and eight payload bytes. An error frame carries neither
        // on the wire, and the Statistics view aggregates them, so both are
        // dropped deliberately -- see `FrameFlags::ERROR`.
        let body = error_ext_body(
            1,
            0x66,
            0x1999_9999 | CAN_ID_EXT,
            &[0xCC, 0xDD, 0xEE, 0xFF, 0x11, 0x22, 0x33, 0x44],
        );
        let obj = obj_header_v1(OBJ_CAN_ERROR_EXT, ns(30_000_000), 0, &body);
        let mut s = BlfStream::from_bytes(&assemble(&obj)).unwrap();
        let f = s.next_frame().unwrap();
        assert!(f.is_error());
        assert_eq!(f.channel, 1);
        assert!(f.extended, "the IDE bit is still a property of the frame");
        assert_eq!(f.id, 0);
        assert_eq!(f.len, 0);
        assert!(f.payload().is_empty());
    }

    #[test]
    fn decodes_zlib_container() {
        let body = can_body(0, 2, 0, 0x321, &[0xAA, 0xBB]);
        let obj = obj_header_v1(OBJ_CAN_MESSAGE, ns(1_000_000), 0, &body);
        let mut v = file_header(
            Some((2024, 1, 1, 1, 0, 0, 0, 0)),
            Some((2024, 1, 1, 1, 0, 0, 1, 0)),
        );
        v.extend_from_slice(&zlib_container(&obj));
        let mut s = BlfStream::from_bytes(&v).unwrap();
        let f = s.next_frame().unwrap();
        assert_eq!(f.id, 0x321);
        assert_eq!(f.payload(), &[0xAA, 0xBB]);
    }

    #[test]
    fn unknown_object_type_is_skipped() {
        let noise = obj_header_v1(9999, ns(500_000), 0, &[0u8; 8]);
        let good_body = can_body(0, 1, 0, 0x7AA, &[0x11]);
        let good = obj_header_v1(OBJ_CAN_MESSAGE, ns(1_000_000), 0, &good_body);
        let mut objs = Vec::new();
        objs.extend_from_slice(&noise);
        objs.extend_from_slice(&good);
        let mut s = BlfStream::from_bytes(&assemble(&objs)).unwrap();
        let f = s.next_frame().unwrap();
        assert_eq!(f.id, 0x7AA, "skipped object must not block the next one");
    }

    #[test]
    fn v2_header_decodes() {
        let body = can_body(0, 1, 0, 0x55, &[0xEE]);
        let obj = obj_header_v2(OBJ_CAN_MESSAGE, ns(2_000_000), 0, &body);
        let mut s = BlfStream::from_bytes(&assemble(&obj)).unwrap();
        let f = s.next_frame().unwrap();
        assert_eq!(f.id, 0x55);
        // Rebased to zero (single-frame log).
        assert_eq!(f.t_us, 0);
    }

    #[test]
    fn an_unknown_header_version_is_stepped_over() {
        // A newer CANoe writing version 3 must not cost us the rest of the log.
        let body = can_body(0, 1, 0, 0x55, &[0xEE]);
        let mut future = obj_header_v1(OBJ_CAN_MESSAGE, ns(1_000_000), 0, &body);
        future[6] = 3;
        let mut objs = future;
        objs.extend_from_slice(&obj_header_v1(OBJ_CAN_MESSAGE, ns(2_000_000), 0, &body));
        let mut s = BlfStream::from_bytes(&assemble(&objs)).unwrap();
        assert_eq!(s.next_frame().unwrap().id, 0x55);
        assert!(s.next_frame().is_none(), "only the known header decoded");
    }

    #[test]
    fn padded_objects_inside_a_container_are_stepped_over() {
        let mut objs = obj_header_v1(
            OBJ_CAN_MESSAGE,
            ns(1_000_000),
            0,
            &can_body(0, 1, 0, 0x100, &[1]),
        );
        objs.extend_from_slice(&[0u8; 4]);
        objs.extend_from_slice(&obj_header_v1(
            OBJ_CAN_MESSAGE,
            ns(2_000_000),
            0,
            &can_body(0, 1, 0, 0x101, &[2]),
        ));
        let mut s = BlfStream::from_bytes(&assemble(&objs)).unwrap();
        assert_eq!(times(&mut s), vec![0, 1_000_000]);
    }

    #[test]
    fn padding_between_containers_does_not_truncate_the_log() {
        // Vector pads top-level records and `object_size` does not always
        // absorb the slack, so walking by stride alone would stop after the
        // first container and silently lose the rest of the file.
        let mut v = padded_file_header(&[]);
        v.extend_from_slice(&raw_container(&can_run(0, 5)));
        v.extend_from_slice(&[0u8; 4]);
        v.extend_from_slice(&raw_container(&can_run(10_000, 5)));
        let mut s = BlfStream::from_bytes(&v).unwrap();
        assert_eq!(times(&mut s).len(), 10, "both containers were read");
    }

    #[test]
    fn a_wider_file_header_is_honoured() {
        let mut v = padded_file_header(&[0u8; 16]);
        v.extend_from_slice(&raw_container(&can_run(0, 3)));
        let mut s = BlfStream::from_bytes(&v).unwrap();
        assert_eq!(times(&mut s), vec![0, 1_000, 2_000], "objects start at 160");
    }

    #[test]
    fn a_top_level_object_that_is_not_a_container_is_stepped_over() {
        // Markers and application records sit at the top level beside
        // containers. Their bytes must not be mistaken for one, and they must
        // not end the walk either.
        let marker = obj_header_v1(96 /* GLOBAL_MARKER */, ns(500_000), 0, &[0x20u8; 24]);
        let mut v = padded_file_header(&[]);
        v.extend_from_slice(&marker);
        v.extend_from_slice(&raw_container(&can_run(1_000_000, 3)));
        let mut s = BlfStream::from_bytes(&v).unwrap();
        assert_eq!(times(&mut s), vec![0, 1_000, 2_000]);
    }

    /// Rebasing hides the absolute stamp, so the unit a header declares is
    /// asserted through the spacing between two frames written with it.
    fn stamp_pair(flags: u32, first: u64, second: u64) -> (u64, u64) {
        let a = obj_header_v1(
            OBJ_CAN_MESSAGE,
            first,
            flags,
            &can_body(0, 1, 0, 0x100, &[1]),
        );
        let b = obj_header_v1(
            OBJ_CAN_MESSAGE,
            second,
            flags,
            &can_body(0, 1, 0, 0x101, &[2]),
        );
        let mut objs = a;
        objs.extend_from_slice(&b);
        let mut s = BlfStream::from_bytes(&assemble(&objs)).unwrap();
        let first = s.next_frame().unwrap().t_us;
        let second = s.next_frame().unwrap().t_us;
        (first, second)
    }

    #[test]
    fn zero_flag_stamps_are_treated_as_nanoseconds() {
        // `flags == 0` is not a documented unit; Vector's reader treats anything
        // but 1 as nanoseconds, and that is what real files rely on.
        assert_eq!(stamp_pair(0, ns(1_000_000), ns(1_001_000)), (0, 1_000));
    }

    #[test]
    fn nanosecond_stamps_scale_to_microseconds() {
        // 1 ms apart, carried in Vector's TIME_ONE_NANS units.
        assert_eq!(
            stamp_pair(TS_ONE_NANO, 1_000_000_000, 1_001_000_000),
            (0, 1_000)
        );
    }

    #[test]
    fn ten_microsecond_stamps_scale_to_microseconds() {
        // `flags == 1`: 100 ticks × 10 µs = 1 ms.
        assert_eq!(stamp_pair(TS_TEN_MICRO, 100_000, 100_100), (0, 1_000));
    }

    #[test]
    fn header_stop_time_becomes_duration() {
        let mut v = file_header(
            Some((2024, 1, 1, 1, 0, 0, 0, 0)),
            Some((2024, 1, 1, 1, 0, 0, 30, 0)),
        );
        v.extend_from_slice(&raw_container(&[]));
        let s = BlfStream::from_bytes(&v).unwrap();
        assert_eq!(s.duration_us(), Some(30_000_000));
    }

    #[test]
    fn fd_message_64_decodes_the_64_bit_dialect() {
        // A 12-byte FD frame, Tx, with bit-rate switching. Note the FD bits are
        // in a u32 at 0x1000 and up here, not the low bits `CAN_FD_MESSAGE`
        // uses, so decoding either dialect with the other's mask shows up as
        // `is_fd` being false.
        let payload: Vec<u8> = (0..12u8).collect();
        let body = fd64_body(Fd64 {
            channel0: 1,
            dlc: 9, // DLC code 9 -> 12 bytes
            valid: 12,
            tx: true,
            fd_flags: FD64_EDL | FD64_BRS,
            id: 0x00AB_CDEF | CAN_ID_EXT,
            data: payload.clone(),
            ..Default::default()
        });
        let obj = obj_header_v1(OBJ_CAN_FD_MESSAGE_64, ns(500_000), 0, &body);
        let mut s = BlfStream::from_bytes(&assemble(&obj)).unwrap();
        let f = s.next_frame().unwrap();
        assert!(f.is_fd());
        assert!(f.brs());
        assert!(!f.esi());
        assert!(!f.is_remote());
        assert_eq!(f.dir, Direction::Tx);
        assert!(f.extended);
        assert_eq!(f.id, 0x00AB_CDEF);
        assert_eq!(f.channel, 1);
        assert_eq!(f.len, 12);
        assert_eq!(f.payload(), &payload[..]);
    }

    /// A `CAN_FD_MESSAGE_64` record declaring `valid` payload bytes but only
    /// carrying `stored`, the shape Vector's issue-1905 sample has.
    fn over_declared_fd64(valid: u8, stored: usize, ext_data_offset: u8) -> CanFrame {
        let body = fd64_body(Fd64 {
            dlc: 15,
            valid,
            fd_flags: FD64_EDL,
            id: 0x6A9,
            ext_data_offset,
            data: vec![0xFFu8; stored],
            ..Default::default()
        });
        let obj = obj_header_v1(OBJ_CAN_FD_MESSAGE_64, ns(1_000_000), 0, &body);
        let mut s = BlfStream::from_bytes(&assemble(&obj)).unwrap();
        s.next_frame().expect("frame")
    }

    #[test]
    fn fd_message_64_pads_the_payload_the_object_does_not_carry() {
        // 64 bytes declared, 48 actually in the record: CANoe shows the
        // remainder as zero, and reading past the object would show the next
        // record's header instead.
        let f = over_declared_fd64(64, 48, 0);
        assert_eq!(f.len, 64);
        let mut expected = vec![0xFFu8; 48];
        expected.extend_from_slice(&[0u8; 16]);
        assert_eq!(f.payload(), &expected[..]);
    }

    #[test]
    fn fd_message_64_data_field_stops_at_ext_data_offset() {
        // extDataOffset is absolute inside the object: 32 B header + 40 B
        // record + 32 B of data. The object carries 48 bytes, so the last 16
        // are not part of this frame.
        let f = over_declared_fd64(64, 48, 32 + 40 + 32);
        assert_eq!(f.len, 64);
        let mut expected = vec![0xFFu8; 32];
        expected.extend_from_slice(&[0u8; 32]);
        assert_eq!(f.payload(), &expected[..]);
    }

    /// Same logical traffic (one classic, one FD-with-BRS) written to both
    /// ASC and BLF, then decoded by each reader. Compares the outputs on
    /// every field we claim to preserve; drift in either reader trips this.
    #[test]
    fn asc_and_blf_read_the_same_traffic() {
        use crate::can::frame::Direction;
        use crate::log::asc::parse_asc;
        let t0 = 1_000_000u64;

        let mut fd_data = [0u8; MAX_CAN_FD_LEN];
        for (i, b) in fd_data.iter_mut().enumerate() {
            *b = i as u8;
        }

        // ASC side: hand-write the two lines Vector would emit.
        let mut asc = String::new();
        asc.push_str("base hex  timestamps absolute\n");
        asc.push_str("0.000000 Start of measurement\n");
        asc.push_str("1.000000 1 1A4 Rx d 4 11 22 33 44\n");
        let data_hex = (0..48u8)
            .map(|i| format!("{i:02X}"))
            .collect::<Vec<_>>()
            .join(" ");
        asc.push_str(&format!(
            "2.000000 CANFD 2 Tx 1DB3FF01 1 0 e 48 {data_hex} 0 0 00000000 0 0 0 0 0\n"
        ));
        let asc_frames = parse_asc(&asc);
        assert_eq!(asc_frames.len(), 2, "ASC should parse both frames");
        assert_eq!(asc_frames[0].t_us, t0);
        assert_eq!(asc_frames[1].t_us, t0 + 1_000_000);

        // BLF side: encode the same logical traffic and rebase on first frame.
        let a_body = can_body(0, 4, 0, 0x1A4, &[0x11, 0x22, 0x33, 0x44]);
        let a = obj_header_v1(OBJ_CAN_MESSAGE, ns(1_000_000), 0, &a_body);
        let b_body = fd_body(
            1,
            14,
            48,
            true,
            FD_FLAG_EDL | FD_FLAG_BRS,
            0x1DB3_FF01 | CAN_ID_EXT,
            &fd_data[..48],
        );
        let b = obj_header_v1(OBJ_CAN_FD_MESSAGE, ns(2_000_000), 0, &b_body);
        let mut objs = Vec::new();
        objs.extend_from_slice(&a);
        objs.extend_from_slice(&b);
        let mut bs = BlfStream::from_bytes(&assemble(&objs)).unwrap();
        let blf_first = bs.next_frame().unwrap();
        let blf_second = bs.next_frame().unwrap();

        // Rebase zeroes BLF's t_us; ASC keeps the raw absolute stamps. Assert
        // the delta matches and every other meaningful field is identical.
        assert_eq!(blf_first.t_us, asc_frames[0].t_us - t0);
        assert_eq!(blf_second.t_us, asc_frames[1].t_us - t0);
        assert_eq!(blf_first.id, asc_frames[0].id);
        assert_eq!(blf_second.id, asc_frames[1].id);
        assert_eq!(blf_first.len, asc_frames[0].len);
        assert_eq!(blf_second.len, asc_frames[1].len);
        assert_eq!(blf_first.payload(), asc_frames[0].payload());
        assert_eq!(blf_second.payload(), asc_frames[1].payload());
        assert!(blf_second.is_fd() && asc_frames[1].is_fd());
        assert!(blf_second.brs() && asc_frames[1].brs());
        assert_eq!(blf_first.dir, Direction::Rx);
        assert_eq!(asc_frames[0].dir, Direction::Rx);
        assert_eq!(blf_second.dir, Direction::Tx);
        assert_eq!(asc_frames[1].dir, Direction::Tx);
    }

    /// `count` v1 CAN_MESSAGE objects starting at raw timestamp `t0`
    /// (microseconds), 1 ms apart.
    fn can_run(t0: u64, count: u32) -> Vec<u8> {
        let mut v = Vec::new();
        for i in 0..count {
            let body = can_body(1, 2, 0, 0x100 + i, &[0xAA, 0xBB]);
            v.extend_from_slice(&obj_header_v1(
                OBJ_CAN_MESSAGE,
                ns(t0 + u64::from(i) * 1_000),
                0,
                &body,
            ));
        }
        v
    }

    /// One container per run; `runs` holds the raw start timestamps.
    fn multi_container_file(runs: &[u64], zlib: bool) -> Vec<u8> {
        let mut v = file_header(
            Some((2024, 1, 1, 1, 0, 0, 0, 0)),
            Some((2024, 1, 1, 1, 0, 0, 5, 0)),
        );
        for start in runs {
            let objects = can_run(*start, 5);
            let container = if zlib {
                zlib_container(&objects)
            } else {
                raw_container(&objects)
            };
            v.extend_from_slice(&container);
        }
        v
    }

    fn times(s: &mut dyn FrameStream) -> Vec<u64> {
        let mut out = Vec::new();
        while let Some(f) = s.next_frame() {
            out.push(f.t_us);
        }
        out
    }

    fn read_all(bytes: &[u8]) -> Vec<CanFrame> {
        let mut s = BlfStream::from_bytes(bytes).unwrap();
        let mut out = Vec::new();
        while let Some(f) = s.next_frame() {
            out.push(f);
        }
        out
    }

    #[test]
    fn seek_lands_inside_a_later_container() {
        let bytes = multi_container_file(&[0, 10_000, 20_000], false);
        let mut s = BlfStream::from_bytes(&bytes).unwrap();
        assert_eq!(s.seek_to_us(20_000), Some(20_000));
        assert_eq!(times(&mut s), vec![20_000, 21_000, 22_000, 23_000, 24_000]);
    }

    #[test]
    fn seek_rewinds_across_containers() {
        let bytes = multi_container_file(&[0, 10_000, 20_000], false);
        let mut s = BlfStream::from_bytes(&bytes).unwrap();
        assert_eq!(times(&mut s).len(), 15);
        assert_eq!(s.seek_to_us(10_000), Some(10_000));
        assert_eq!(
            times(&mut s),
            vec![
                10_000, 11_000, 12_000, 13_000, 14_000, 20_000, 21_000, 22_000, 23_000, 24_000
            ],
            "rewind replays the remaining containers in order"
        );
    }

    #[test]
    fn one_checkpoint_is_recorded_per_container() {
        let bytes = multi_container_file(&[0, 10_000, 20_000], false);
        let mut s = BlfStream::from_bytes(&bytes).unwrap();
        assert!(s.checkpoints.is_empty(), "nothing walked yet");
        assert_eq!(times(&mut s).len(), 15);
        assert_eq!(s.checkpoints.len(), 3, "one entry per container");
    }

    #[test]
    fn scrubbing_does_not_move_the_rebase_base() {
        // Raw stamps start well past zero, so the rebased timeline has to stay
        // pinned to the first frame ever read -- not to wherever we seek.
        let bytes = multi_container_file(&[5_000_000, 5_010_000], false);
        let mut s = BlfStream::from_bytes(&bytes).unwrap();
        assert_eq!(s.seek_to_us(10_000), Some(10_000), "rebased 10 s");
        assert_eq!(times(&mut s), vec![10_000, 11_000, 12_000, 13_000, 14_000]);
        assert_eq!(s.seek_to_us(0), Some(0));
        assert_eq!(
            times(&mut s).first(),
            Some(&0),
            "the log's zero point must not shift after a scrub"
        );
    }

    #[test]
    fn seek_works_through_zlib_containers() {
        let bytes = multi_container_file(&[0, 10_000, 20_000], true);
        let mut s = BlfStream::from_bytes(&bytes).unwrap();
        assert_eq!(s.seek_to_us(21_000), Some(21_000));
        assert_eq!(times(&mut s), vec![21_000, 22_000, 23_000, 24_000]);
    }

    #[test]
    fn seek_past_the_end_reports_eof() {
        let bytes = multi_container_file(&[0, 10_000], false);
        let mut s = BlfStream::from_bytes(&bytes).unwrap();
        assert_eq!(s.seek_to_us(999_999), None);
        assert_eq!(s.peek_t(), None);
    }

    #[test]
    fn seek_agrees_with_the_in_memory_stream() {
        let bytes = multi_container_file(&[0, 10_000, 20_000], false);
        let all = read_all(&bytes);
        assert_eq!(all.len(), 15);
        for target in [0u64, 1, 3_000, 10_000, 12_500, 24_000, 24_001] {
            let mut a = BlfStream::from_bytes(&bytes).unwrap();
            let mut v = VecStream::new(all.clone());
            assert_eq!(
                a.seek_to_us(target),
                v.seek_to_us(target),
                "landing differs at t={target}"
            );
            assert_eq!(
                times(&mut a),
                times(&mut v),
                "tail after seeking differs at t={target}"
            );
        }
    }

    /// Reads real Vector-authored BLF files to catch dialect drift that our own
    /// encoders would happily paper over. Enable with:
    /// `ROXY_BLF_SAMPLE=<file or directory> cargo test read_real_canoe_blf -- --ignored --nocapture`.
    ///
    /// The upstream reference is python-can's `test/data/*.blf`; those files
    /// were written by Vector's own BLF library (`logformats_test.py`: "log
    /// files created by Toby Lorenz ... events_from_binlog"), so they are an
    /// independent witness rather than something our reader produced. Their
    /// field values are deliberately non-canonical bit patterns, which is what
    /// makes them good at exposing divergences.
    #[test]
    #[ignore]
    fn read_real_canoe_blf() {
        let Ok(arg) = std::env::var("ROXY_BLF_SAMPLE") else {
            eprintln!("set ROXY_BLF_SAMPLE=<path> to enable");
            return;
        };
        let root = Path::new(&arg);
        let mut files = Vec::new();
        if root.is_dir() {
            for e in std::fs::read_dir(root).unwrap().flatten() {
                let p = e.path();
                if p.extension().and_then(|s| s.to_str()) == Some("blf") {
                    files.push(p);
                }
            }
            files.sort();
        } else {
            files.push(root.to_path_buf());
        }
        assert!(!files.is_empty(), "no .blf found at {arg}");

        for path in files {
            eprintln!("\n=== {} ===", path.file_name().unwrap().to_string_lossy());
            let mut s = BlfStream::open(&path).expect("open");
            eprintln!("  describe: {}", s.describe());
            let mut n = 0usize;
            while let Some(f) = s.next_frame() {
                let hex: Vec<String> = f.payload().iter().map(|b| format!("{b:02X}")).collect();
                eprintln!(
                    "  [{n}] t={:<12} ch={:<5} id={:08X} ext={} len={:<3} dir={:?} err={} \
                     rtr={} fd={} brs={} esi={} data={}",
                    f.t_us,
                    f.channel,
                    f.id,
                    f.extended,
                    f.len,
                    f.dir,
                    f.is_error(),
                    f.is_remote(),
                    f.is_fd(),
                    f.brs(),
                    f.esi(),
                    hex.join(" "),
                );
                n += 1;
            }
            assert!(n > 0, "expected frames in {path:?}");
            eprintln!("  total: {n} frames");
        }
    }
}
