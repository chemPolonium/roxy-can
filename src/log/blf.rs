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
use std::io::Write as _;
use std::path::Path;

use chrono::Datelike as _;
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
/// FlexRay receive objects (the vector-blf ObjectType enum): CANoe 12
/// writes the Ex form.
const OBJ_FR_RCVMESSAGE: u32 = 50;
const OBJ_FR_RCVMESSAGE_EX: u32 = 66;

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
    duration: Option<u64>,
    describe: String,
    pending: VecDeque<CanFrame>,
    /// FlexRay receive objects encountered while reading containers for
    /// CAN frames. Delivered via `poll_fr_rows` as the same containers are
    /// read, in file order.
    fr_pending: VecDeque<crate::trace::FrRow>,
    /// Sticky: did any scan ever surface a CAN frame?
    saw_can: bool,
    /// Set once a scan reaches EOF having seen no CAN frame. The whole file
    /// is then CAN-free, so `peek_t` returns `None` at once instead of
    /// re-decompressing every container on every replay poll -- which is what
    /// pegged the core on a FlexRay-only log.
    no_can_frames: bool,
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
            duration,
            describe,
            pending: VecDeque::new(),
            fr_pending: VecDeque::new(),
            saw_can: false,
            no_can_frames: false,
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
                // CANoe concatenates MULTIPLE zlib members inside one
                // container. A single `read_to_end` stops at the first
                // member's end and silently drops every object after it
                // (the real PowerTrain recording lost 36k of 39k frames
                // this way), so chain members: each decoder reports how
                // much input it consumed, and the next member starts
                // right there. Trailing padding (all zero) ends the loop.
                let mut out = Vec::with_capacity(uncompressed.max(payload.len()));
                let mut rest = payload;
                while !rest.is_empty() {
                    let mut decoder = ZlibDecoder::new(rest);
                    if decoder.read_to_end(&mut out).is_err() {
                        break;
                    }
                    let consumed = decoder.total_in() as usize;
                    if consumed == 0 {
                        break;
                    }
                    rest = &rest[consumed..];
                }
                out
            }
            _ => {
                // Unknown compression: skip the container rather than
                // risk yielding frames we decoded wrongly.
                return true;
            }
        };
        parse_container_objects(&decoded, &mut self.pending, &mut self.fr_pending);
        true
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
        // A prior full scan proved the file carries no CAN frames: report EOF
        // at once rather than decompressing every container again. This is
        // what keeps a FlexRay-only log's replay from re-scanning on every poll.
        if self.no_can_frames {
            return None;
        }
        loop {
            if let Some(f) = self.pending.front() {
                self.saw_can = true;
                let raw = f.t_us;
                return Some(self.rebase(raw));
            }
            if !self.enter_next_container() {
                if !self.saw_can {
                    self.no_can_frames = true;
                }
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

    fn peek_fr_t(&mut self) -> Option<u64> {
        // Read containers until an FR row queues — but stop as soon as
        // CAN frames are pending, so a sparse-FR log does not read the
        // whole file into memory hunting for the next row.
        while self.fr_pending.is_empty() && self.pending.is_empty() && self.enter_next_container()
        {
        }
        let raw = self.fr_pending.front().map(|r| r.t_us);
        raw.map(|t| self.rebase(t))
    }

    fn poll_fr_rows(&mut self, upto_t_us: u64, out: &mut Vec<crate::trace::FrRow>) {
        while let Some(front) = self.fr_pending.front() {
            if self.rebase(front.t_us) > upto_t_us {
                break;
            }
            let mut r = self.fr_pending.pop_front().expect("checked above");
            r.t_us = self.rebase(r.t_us);
            out.push(r);
        }
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

/// Locate the next object. Vector pads records — CANoe 12 sometimes by
/// gaps far larger than the 8 bytes python-can tolerates, both between
/// containers and between the objects inside one decompressed container
/// — so search forward for the next signature without a proximity
/// limit. The caller bounds the search by the slice it passes.
fn next_object(bytes: &[u8], from: usize) -> Option<usize> {
    if from >= bytes.len() {
        return None;
    }
    bytes[from..]
        .windows(4)
        .position(|w| w == OBJECT_SIGNATURE)
        .map(|at| from + at)
}

/// Decode every recognised object inside one container. Objects with
/// unknown types are stepped over using `object_size`, so CANoe files
/// that mix CAN with LIN/Ethernet still yield their CAN traffic.
/// FlexRay receive objects land in `fr_out` — they are display-only and
/// never enter the CAN pipeline.
fn parse_container_objects(
    bytes: &[u8],
    out: &mut VecDeque<CanFrame>,
    fr_out: &mut VecDeque<crate::trace::FrRow>,
) {
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
                OBJ_CAN_MESSAGE | OBJ_CAN_MESSAGE2 => {
                    decode_can_msg(body, t_us).map(FrOrCan::Can)
                }
                OBJ_CAN_FD_MESSAGE => decode_fd_message(body, t_us).map(FrOrCan::Can),
                OBJ_CAN_FD_MESSAGE_64 => {
                    decode_fd_message_64(body, t_us, header_len, size).map(FrOrCan::Can)
                }
                OBJ_CAN_ERROR_EXT => Some(FrOrCan::Can(decode_error_ext(body, t_us))),
                OBJ_FR_RCVMESSAGE | OBJ_FR_RCVMESSAGE_EX => {
                    decode_fr_rcv(h.object_type == OBJ_FR_RCVMESSAGE_EX, body, t_us)
                }
                _ => None,
            };
            match decoded {
                Some(FrOrCan::Can(f)) => out.push_back(f),
                Some(FrOrCan::Fr(r)) => fr_out.push_back(r),
                None => {}
            }
        }
        pos = match next_object(bytes, next) {
            Some(at) => at,
            None => break,
        };
    }
}

/// One decoded object: a CAN frame or a FlexRay row. Keeps the shared
/// object-walk a single match.
enum FrOrCan {
    Can(CanFrame),
    Fr(crate::trace::FrRow),
}

/// `FlexRayVFrReceiveMsg` (50) and `FlexRayVFrReceiveMsgEx` (66): the
/// slot/cycle address, the reception channel mask and the raw payload.
/// Layouts per the vector-blf reference reader; both variants share the
/// first fields and differ in where `dataBytes` starts (fixed 254 bytes
/// in the plain form, `dataCount` bytes past a longer header in Ex).
fn decode_fr_rcv(ex: bool, body: &[u8], t_us: u64) -> Option<FrOrCan> {
    if body.len() < 42 {
        return None;
    }
    // Field layout per Vector's `FlexRayVFrReceiveMsg` / `..Ex` (blf.h),
    // body-relative: channel@0, version@2, channelMask@4, dir@6,
    // clientIndex@8, clusterNo@12, frameId@16, headerCrc1@18, headerCrc2@20,
    // payloadLength@22, payloadLengthValid@24, cycle@26, tag@28, data@32,
    // frameFlags@36, appParameter@40, payload@44 (Ex inserts a further 40 B
    // reserved block, so its payload starts at 84).
    // channelMask: 0 invalid, 1 = A, 2 = B, 3 = both. FrRow's `ab` runs
    // 0 = A / 1 = B / 2 = unknown-or-both (a dual-channel frame decodes
    // identically on either side, so `frame_at` drops the channel preference).
    let channel_mask = u16_at(body, 4);
    // `wClusterNo` is the cluster the object belongs to, and a CANoe
    // recording really does hold more than one: `assets/fibex/Logging.blf`
    // carries 29 738 rows of cluster 0 (Vector channel 1) and 30 334 of
    // cluster 1 (channel 2). So this is the FlexRay bus index -- two clusters
    // share slot numbers, and folding them together names one's frames from
    // the other's description.
    let cluster_no = u16_at(body, 12).min(u16::from(u8::MAX)) as u8;
    let ab = match channel_mask {
        1 => 0,
        2 => 1,
        _ => 2,
    };
    // The object carries a `frameId` but no slot id -- frameId is the only
    // frame coordinate BLF records, so it rides in `slot`. (A live VxlApi
    // FlexRay watch reports a true slotId there instead; replaying a BLF
    // cannot recover the slot from these objects.)
    let frame_id = u16_at(body, 16);
    let header_crc = u16_at(body, 18); // headerCrc1 (channel A's frame header)
    let frame_flags = u32_at(body, 36) as u16;
    let byte_count = u16_at(body, 22) as usize; // payloadLength
    let data_count = u16_at(body, 24) as usize; // payloadLengthValid
    let (cycle, data_at) = if ex {
        // Ex: cycle is u16 at 26, and the payload starts after the
        // 26-byte reserved block (header ends at 84).
        let cycle = u16_at(body, 26).min(63) as u8;
        (cycle, 84)
    } else {
        (body[26], 44)
    };
    let avail = body.len().saturating_sub(data_at);
    let len = byte_count.min(data_count).min(avail).min(254);
    let payload = body[data_at..data_at + len].to_vec();
    Some(FrOrCan::Fr(crate::trace::FrRow {
        bus: cluster_no,
        t_us,
        ab,
        slot: frame_id,
        cycle,
        payload,
        header_crc,
        flags: frame_flags,
        name: None,
    }))
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

// ---- BLF 写入器（录制后端，按扩展名 .blf 启用）--------------------------
//
// 结构与读侧逐字段镜像（同样的 LOGG/LOBJ 头、同样的 zlib 容器、同样的
// CAN_MESSAGE / CAN_FD_MESSAGE_64 事件）。错误帧 v1 不写——读侧把错误帧
// 归一为 id 0 的特殊帧，写回会放大歧义。

/// 每个压缩容器攒多少未压缩事件字节后写入文件。Vector 自家工具用 ~128 KiB。
const CONTAINER_FLUSH_BYTES: usize = 128 * 1024;

/// The eight `SYSTEMTIME` words of a wall-clock instant, in the order the
/// header stores them.
fn fields_at(t: &chrono::DateTime<chrono::Local>) -> [u16; 8] {
    use chrono::Timelike;
    [
        t.year() as u16,
        t.month() as u16,
        t.weekday().num_days_from_sunday() as u16,
        t.day() as u16,
        t.hour() as u16,
        t.minute() as u16,
        t.second() as u16,
        (t.nanosecond() / 1_000_000) as u16,
    ]
}

/// `OBJ_HEADER_V1` + body: signature, header size 32, version pair, object
/// size, type, flags (0 = nanosecond stamps), client index, object version,
/// nanosecond timestamp, body.
fn obj_header_v1_bytes(object_type: u32, ts_raw: u64, flags: u32, body: &[u8]) -> Vec<u8> {
    let header_size: u16 = 32;
    let total = (header_size as usize + body.len()) as u32;
    let mut v = Vec::with_capacity(total as usize);
    v.extend_from_slice(b"LOBJ");
    v.extend_from_slice(&header_size.to_le_bytes());
    v.push(1);
    v.push(0);
    v.extend_from_slice(&total.to_le_bytes());
    v.extend_from_slice(&object_type.to_le_bytes());
    v.extend_from_slice(&flags.to_le_bytes());
    v.extend_from_slice(&0u16.to_le_bytes());
    v.extend_from_slice(&0u16.to_le_bytes());
    v.extend_from_slice(&ts_raw.to_le_bytes());
    v.extend_from_slice(body);
    v
}

fn classic_event(f: &CanFrame, ts_raw: u64) -> Vec<u8> {
    let mut flags = 0u8;
    if matches!(f.dir, Direction::Tx) {
        flags |= CAN_DIR_TX;
    }
    if f.flags.contains(FrameFlags::RTR) {
        flags |= CAN_FLAG_REMOTE;
    }
    let mut b = vec![0u8; 16];
    b[0..2].copy_from_slice(&(u16::from(f.channel) + 1).to_le_bytes());
    b[2] = flags;
    b[3] = f.len;
    b[4..8].copy_from_slice(
        &(f.id | if f.extended { CAN_ID_EXT } else { 0 }).to_le_bytes(),
    );
    let n = f.len.min(8) as usize;
    b[8..8 + n].copy_from_slice(&f.data[..n]);
    obj_header_v1_bytes(OBJ_CAN_MESSAGE, ts_raw, 0, &b)
}

fn fd64_event(f: &CanFrame, ts_raw: u64) -> Vec<u8> {
    let payload = f.payload();
    let mut fd_flags = FD64_EDL;
    if f.flags.contains(FrameFlags::BRS) {
        fd_flags |= FD64_BRS;
    }
    if f.flags.contains(FrameFlags::ESI) {
        fd_flags |= FD64_ESI;
    }
    if f.flags.contains(FrameFlags::RTR) {
        fd_flags |= FD64_RTR;
    }
    let mut b = vec![0u8; FD64_STRUCT_SIZE + payload.len()];
    b[0] = f.channel + 1;
    b[2] = payload.len() as u8;
    b[4..8].copy_from_slice(
        &(f.id | if f.extended { CAN_ID_EXT } else { 0 }).to_le_bytes(),
    );
    b[12..16].copy_from_slice(&fd_flags.to_le_bytes());
    b[34] = u8::from(matches!(f.dir, Direction::Tx));
    b[FD64_STRUCT_SIZE..].copy_from_slice(payload);
    obj_header_v1_bytes(OBJ_CAN_FD_MESSAGE_64, ts_raw, 0, &b)
}

/// `CAN_ERROR_EXT_STRUCT = <HHLBBBxLLH2x8s` — channel one-based at 0, DLC
/// byte at 10, arbitration id at 16, up to eight data bytes at 24.
fn error_ext_event(f: &CanFrame, ts_raw: u64) -> Vec<u8> {
    let mut b = vec![0u8; 32];
    b[0..2].copy_from_slice(&(u16::from(f.channel) + 1).to_le_bytes());
    b[10] = f.len;
    b[16..20].copy_from_slice(&f.id.to_le_bytes());
    let n = (f.len as usize).min(8);
    b[24..24 + n].copy_from_slice(&f.data[..n]);
    obj_header_v1_bytes(OBJ_CAN_ERROR_EXT, ts_raw, 0, &b)
}

/// One FlexRay receive object in the FR_RCVMESSAGE (50) layout, which is what
/// CANoe records static-slot frames with and what the reader decodes. BLF
/// carries no slot field -- `frameId` is the only frame coordinate the object
/// has -- and the real captures put the schedule slot there, so the row's slot
/// rides in it (measured: every row of both bundled CANoe captures resolves
/// against the cluster description this way).
fn fr_rcv_event(r: &crate::trace::FrRow, ts_raw: u64) -> Vec<u8> {
    let mut b = vec![0u8; 44 + r.payload.len()];
    b[4..6].copy_from_slice(&(match r.ab {
        0 => 1u16,  // channel A
        1 => 2,     // channel B
        _ => 3,     // both / unknown
    })
    .to_le_bytes());
    // `wClusterNo`: the FlexRay bus the row arrived on, so a recording of two
    // clusters reads back as two -- the same field [`decode_fr_rcv`] reads.
    b[12..14].copy_from_slice(&u16::from(r.bus).to_le_bytes());
    b[16..18].copy_from_slice(&r.slot.to_le_bytes());
    b[18..20].copy_from_slice(&r.header_crc.to_le_bytes());
    let len = r.payload.len() as u16;
    b[22..24].copy_from_slice(&len.to_le_bytes()); // payloadLength
    b[24..26].copy_from_slice(&len.to_le_bytes()); // payloadLengthValid
    b[26] = r.cycle;
    b[36..38].copy_from_slice(&r.flags.to_le_bytes());
    b[44..].copy_from_slice(&r.payload);
    obj_header_v1_bytes(OBJ_FR_RCVMESSAGE, ts_raw, 0, &b)
}

fn zlib_bytes(data: &[u8]) -> Vec<u8> {
    use flate2::Compression;
    use flate2::write::ZlibEncoder;
    use std::io::Write;
    let mut e = ZlibEncoder::new(Vec::new(), Compression::default());
    let _ = e.write_all(data);
    e.finish().unwrap_or_default()
}

/// Streaming BLF writer for the recorder: LOGG header, then CAN events
/// batched into zlib-compressed LOG_CONTAINERs. `finish` patches the
/// container count and the stop SYSTEMTIME. Error frames are not written
/// (the reader normalises them to id 0, which would lose information).
pub struct BlfWriter {
    file: std::fs::File,
    pending: Vec<u8>,
    containers: u32,
    /// The wall-clock instant written as the header's start time. The stop time
    /// is derived from it plus the recorded traffic span, so the header's
    /// duration describes the frames, not how long the process took to write
    /// them -- a reader (and a replay's progress) needs a span even for a file
    /// produced in one go.
    start: chrono::DateTime<chrono::Local>,
    first_ns: Option<u64>,
    last_ns: u64,
    /// When the last container went to disk. A container is a size decision
    /// (128 KiB of objects), and on a quiet bench that size takes minutes -- so
    /// the same data also has a deadline, or the file looks frozen while the
    /// recording is running.
    last_container: std::time::Instant,
    container_interval: std::time::Duration,
}

/// How old the un-flushed objects may get before they are written out. Only the
/// test overrides it; the number itself is the one a person watching a file
/// size would call live.
const CONTAINER_FLUSH_INTERVAL: std::time::Duration = std::time::Duration::from_secs(1);

impl BlfWriter {
    pub fn create(path: &str) -> std::io::Result<Self> {
        let mut file = std::fs::File::create(path)?;
        let start = chrono::Local::now();
        let mut hdr = vec![0u8; FILE_HEADER_SIZE];
        hdr[0..4].copy_from_slice(FILE_SIGNATURE);
        hdr[4..8].copy_from_slice(&(FILE_HEADER_SIZE as u32).to_le_bytes());
        let st = fields_at(&start);
        for (i, f) in st.iter().enumerate() {
            hdr[HDR_START_TIME + i * 2..HDR_START_TIME + i * 2 + 2]
                .copy_from_slice(&f.to_le_bytes());
        }
        file.write_all(&hdr)?;
        Ok(BlfWriter {
            file,
            pending: Vec::new(),
            containers: 0,
            start,
            first_ns: None,
            last_ns: 0,
            last_container: std::time::Instant::now(),
            container_interval: CONTAINER_FLUSH_INTERVAL,
        })
    }

    #[cfg(test)]
    pub(crate) fn with_flush_interval(mut self, d: std::time::Duration) -> Self {
        self.container_interval = d;
        self
    }

    /// Records the span the objects cover, in the file's own nanosecond units.
    /// A stamp of zero is a real position (a recording started at log time 0),
    /// so it counts rather than being skipped as unstamped.
    fn note_ts(&mut self, ts_raw: u64) {
        self.first_ns = Some(self.first_ns.map_or(ts_raw, |f| f.min(ts_raw)));
        self.last_ns = self.last_ns.max(ts_raw);
    }

    pub fn write(&mut self, f: &CanFrame) {
        let ts_raw = f.t_us.saturating_mul(1_000); // nanoseconds
        self.note_ts(ts_raw);
        let ev = if f.flags.contains(FrameFlags::ERROR) {
            error_ext_event(f, ts_raw)
        } else if f.flags.contains(FrameFlags::FD) {
            fd64_event(f, ts_raw)
        } else {
            classic_event(f, ts_raw)
        };
        self.pending.extend_from_slice(&ev);
        if self.pending.len() >= CONTAINER_FLUSH_BYTES {
            self.flush_container();
        }
    }

    /// FlexRay rows join the same container stream as the CAN frames -- that
    /// is how CANoe records a mixed capture, and it is what makes a recording
    /// of a FlexRay bus replay and re-export like the file it came from.
    pub fn write_fr(&mut self, r: &crate::trace::FrRow) {
        let ts_raw = r.t_us.saturating_mul(1_000); // nanoseconds
        self.note_ts(ts_raw);
        self.pending.extend_from_slice(&fr_rcv_event(r, ts_raw));
        if self.pending.len() >= CONTAINER_FLUSH_BYTES {
            self.flush_container();
        }
    }

    fn flush_container(&mut self) {
        if self.pending.is_empty() {
            return;
        }
        let payload = zlib_bytes(&self.pending);
        let total = (16 + 16 + payload.len()) as u32;
        let mut c = Vec::with_capacity(total as usize);
        c.extend_from_slice(OBJECT_SIGNATURE);
        c.extend_from_slice(&16u16.to_le_bytes());
        c.push(1);
        c.push(0);
        c.extend_from_slice(&total.to_le_bytes());
        c.extend_from_slice(&OBJ_LOG_CONTAINER.to_le_bytes());
        c.extend_from_slice(&METHOD_ZLIB.to_le_bytes());
        c.extend_from_slice(&[0u8; 6]);
        c.extend_from_slice(&(self.pending.len() as u32).to_le_bytes());
        c.extend_from_slice(&[0u8; 4]);
        c.extend_from_slice(&payload);
        if self.file.write_all(&c).is_ok() {
            self.containers += 1;
            self.last_container = std::time::Instant::now();
            // The header is the only place that says how much of this file is
            // there, and a file someone looks at mid-recording is read from it.
            let _ = self.patch_header();
        }
        self.pending.clear();
    }

    /// Writes what has arrived so far out as a container, however young it is.
    ///
    /// The size threshold alone is not enough: a container is 128 KiB of
    /// objects, which on a bench doing ten frames a second takes minutes. The
    /// file then sits at 144 bytes while the recording is plainly running, which
    /// reads as "nothing is being written". The interval keeps containers from
    /// shattering into one-per-step fragments at the same time.
    pub fn flush(&mut self) {
        if !self.pending.is_empty() && self.last_container.elapsed() >= self.container_interval {
            self.flush_container();
        }
    }

    /// The stop time the header carries: the start plus the span the recorded
    /// traffic covers, not the wall clock of this call.
    fn stop_time(&self) -> chrono::DateTime<chrono::Local> {
        let span_ns = self
            .first_ns
            .map_or(0u64, |first| self.last_ns.saturating_sub(first));
        self.start + chrono::Duration::nanoseconds(span_ns as i64)
    }

    /// Patches the container count and the stop SYSTEMTIME into the fixed
    /// 144-byte header, then moves the write position back to the end so the
    /// next container appends.
    fn patch_header(&mut self) -> std::io::Result<()> {
        use std::io::{Seek, SeekFrom, Write};
        let end = self.file.seek(SeekFrom::End(0))?;
        self.file.seek(SeekFrom::Start(HDR_OBJECT_COUNT as u64))?;
        self.file.write_all(&self.containers.to_le_bytes())?;
        self.file.seek(SeekFrom::Start(HDR_STOP_TIME as u64))?;
        let stop = self.stop_time();
        for f in fields_at(&stop) {
            self.file.write_all(&f.to_le_bytes())?;
        }
        self.file.seek(SeekFrom::Start(end))?;
        Ok(())
    }

    /// Flushes the last container, then patches the header one final time.
    pub fn finish(mut self) -> std::io::Result<()> {
        self.flush_container();
        self.patch_header()?;
        self.file.flush()
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::io::Write;

    /// A one-frame log built by the encoders below, for tests elsewhere in the
    /// crate that only need bytes the reader accepts.
    pub(crate) fn minimal_file() -> Vec<u8> {
        let body = can_body(0, 1, 0, 0x100, &[0xAB]);
        assemble(&obj_header_v1(OBJ_CAN_MESSAGE, ns(1_000_000), 0, &body))
    }

    /// The user's real CANoe recording of the FlexRay demo cluster
    /// (39.2 s, ~39.2k frames per CANoe's own statistics). End-to-end
    /// over the production stream: the container walk must recover
    /// every FR_RCVMESSAGE_EX object (large inter-object padding made
    /// the old 8-byte resync window drop whole stretches), timestamps
    /// must ascend over the full capture, and coordinates must stay
    /// sane.
    #[test]
    fn the_real_canoe_flexray_blf_parses() {
        let path = std::path::Path::new("assets/arxml/Logging.blf");
        let Ok(mut stream) = BlfStream::open(path) else {
            panic!("assets/arxml/Logging.blf missing -- it is tracked in the repository, so a broken checkout must fail, not skip");
        };
        let mut fr = Vec::new();
        let mut can = 0usize;
        loop {
            match stream.peek_fr_t() {
                Some(_) => stream.poll_fr_rows(u64::MAX, &mut fr),
                None => match stream.peek_t() {
                    Some(_) => {
                        let _ = stream.next_frame();
                        can += 1;
                    }
                    None => break,
                },
            }
        }
        assert_eq!(can, 0, "the logging session carried no CAN traffic");
        assert!(
            fr.len() >= 39_000,
            "CANoe statistics say 39.2k frames, we decoded {}",
            fr.len()
        );
        // Timestamps ascend across the whole 39-second capture.
        assert!(
            fr.windows(2).all(|w| w[1].t_us >= w[0].t_us),
            "log order is time order"
        );
        let last = fr.last().expect("rows exist").t_us;
        assert!(last >= 39_000_000, "the capture spans 39.2 s: {last}");
        // Sane frame coordinates throughout.
        for r in fr.iter().step_by(97) {
            assert!(r.slot >= 1, "slot {}", r.slot);
            assert!(r.cycle < 64, "cycle {}", r.cycle);
            assert!(!r.payload.is_empty(), "payload present");
        }
    }

    /// A FlexRay-only BLF has no CAN frames; once the first scan proves that,
    /// `peek_t` must keep reporting EOF cheaply (no re-decompress) while the
    /// FlexRay rows stay readable. Without the latch every replay poll walked
    /// the whole file hunting CAN frames that never come, which pegged the core.
    #[test]
    fn flexray_only_stream_stays_can_exhausted() {
        let path = std::path::Path::new("assets/arxml/Logging.blf");
        let Ok(mut stream) = BlfStream::open(path) else {
            panic!("assets/arxml/Logging.blf missing -- it is tracked in the repository, so a broken checkout must fail, not skip");
        };
        // The first CAN peek scans to EOF and finds none; the flag latches.
        assert!(stream.peek_t().is_none(), "no CAN frames in an FR-only log");
        // A repeat CAN peek (what every replay poll does) stays None.
        assert!(stream.peek_t().is_none(), "CAN stays exhausted");
        assert!(stream.peek_fr_t().is_some(), "FlexRay rows still queue");
        let mut fr = Vec::new();
        stream.poll_fr_rows(u64::MAX, &mut fr);
        assert!(!fr.is_empty(), "FlexRay rows still deliver");
        assert!(stream.peek_t().is_none(), "the rows do not resurrect CAN");
    }

    /// The production recorder writer: classic (standard/extended/RTR)
    /// and FD frames survive a write → read round trip with ids, flags,
    /// and payloads intact.
    #[test]
    fn recorder_writer_round_trips_frames() {
        let path = std::env::temp_dir().join("roxy_can_writer_roundtrip.blf");
        let mk = |t_us: u64, id: u32, ext: bool, fd_flags: FrameFlags, len: usize| CanFrame {
            t_us,
            channel: 0,
            id,
            extended: ext,
            len: len as u8,
            data: {
                let mut d = [0u8; MAX_CAN_FD_LEN];
                for (i, b) in d.iter_mut().enumerate().take(len) {
                    *b = (i * 7 + id as usize) as u8;
                }
                d
            },
            dir: Direction::Rx,
            flags: fd_flags,
        };
        let frames = vec![
            mk(1_000, 0x100, false, FrameFlags::NONE, 8),
            mk(2_000, 0x1F3D1E5, true, FrameFlags::NONE, 8),
            mk(3_000, 0x200, false, FrameFlags::RTR, 1),
            mk(4_000, 0x300, true, FrameFlags::FD.union(FrameFlags::BRS), 24),
            mk(5_000, 0x300, false, FrameFlags::FD, 64),
            mk(6_000, 0xABC, false, FrameFlags::ERROR, 2),
        ];
        {
            let path_s = path.to_string_lossy().into_owned();
            let mut w = BlfWriter::create(&path_s).expect("create");
            for f in &frames {
                w.write(f);
            }
            w.finish().expect("finish");
        }
        let mut stream = super::super::blf::BlfStream::open(&path).expect("the writer's file parses");
        let mut got = Vec::new();
        while let Some(f) = stream.next_frame() {
            got.push(f);
        }
        assert_eq!(got.len(), frames.len(), "{:?}", got);
        for (g, want) in got.iter().zip(frames.iter()) {
            if want.flags.contains(FrameFlags::ERROR) {
                // The reader normalises error frames: id and payload are
                // zeroed, only the flag survives.
                assert_eq!(g.id, 0, "error id normalised to zero");
                assert_eq!(g.len, 0, "error payload dropped");
                assert!(g.flags.contains(FrameFlags::ERROR), "stays an error");
                continue;
            }
            assert_eq!((g.id, g.extended, g.len), (want.id, want.extended, want.len));
            assert_eq!(g.flags, want.flags, "flags for 0x{:X}", g.id);
            assert_eq!(&g.data[..g.len as usize], &want.data[..want.len as usize]);
        }
        // The error frame survives as an error frame: the reader
        // normalises its id to zero and keeps the ERROR flag.
        let last = got.last().expect("at least one frame");
        assert!(
            last.flags.contains(FrameFlags::ERROR),
            "the error frame stays flagged"
        );
        std::fs::remove_file(&path).ok();
    }

    /// A recording that has not been closed is already a readable file.
    ///
    /// A container is a *size* decision (128 KiB of objects), which on a bench
    /// doing ten frames a second is minutes away -- so the file sat at its
    /// 144-byte header while traffic was visibly arriving, and a process killed
    /// mid-run left nothing behind. `flush` is what the end of every measurement
    /// step calls; the interval is zeroed here so the test does not sleep.
    #[test]
    fn a_flushed_recording_reads_back_before_it_is_closed() {
        let path = std::env::temp_dir().join("roxy_can_writer_live.blf");
        let frames: Vec<CanFrame> = (0..3u64)
            .map(|i| CanFrame {
                t_us: 1_000 + i * 1_000,
                channel: 0,
                id: 0x100 + i as u32,
                extended: false,
                len: 8,
                data: [0x11; MAX_CAN_FD_LEN],
                dir: Direction::Rx,
                flags: FrameFlags::NONE,
            })
            .collect();
        let mut w = BlfWriter::create(&path.to_string_lossy())
            .expect("create")
            .with_flush_interval(std::time::Duration::ZERO);
        for f in &frames {
            w.write(f);
        }
        w.flush();

        let bytes = std::fs::read(&path).expect("on disk");
        assert!(
            bytes.len() > FILE_HEADER_SIZE,
            "the file grew past its header: {} B",
            bytes.len()
        );
        assert_eq!(
            u32_at(&bytes, HDR_OBJECT_COUNT),
            1,
            "the header counts what is there"
        );
        let mut stream = BlfStream::open(&path).expect("an unfinished file parses");
        let mut got = Vec::new();
        while let Some(f) = stream.next_frame() {
            got.push(f);
        }
        assert_eq!(got.len(), frames.len(), "the running file holds them");

        // Closing writes no second copy of what the flush already put down.
        w.finish().expect("finish");
        let bytes = std::fs::read(&path).expect("on disk");
        assert_eq!(
            u32_at(&bytes, HDR_OBJECT_COUNT),
            1,
            "the flush is not counted twice"
        );
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

    /// A FlexRay receive object body laid out per the vector-blf
    /// reference: channelMask at 4, frameId at 16, byteCount at 22,
    /// dataCount at 24, cycle at 26 (u8 plain / u16 Ex), payload at 44
    /// (plain) or 84 (Ex).
    fn fr_rcv_body(ex: bool, mask: u16, slot: u16, cycle: u8, payload: &[u8]) -> Vec<u8> {
        let data_at = if ex { 84 } else { 44 };
        let mut b = vec![0u8; data_at + payload.len()];
        b[4..6].copy_from_slice(&mask.to_le_bytes());
        b[16..18].copy_from_slice(&slot.to_le_bytes());
        b[22..24].copy_from_slice(&(payload.len() as u16).to_le_bytes());
        b[24..26].copy_from_slice(&(payload.len() as u16).to_le_bytes());
        if ex {
            b[26..28].copy_from_slice(&(cycle as u16).to_le_bytes());
        } else {
            b[26] = cycle;
        }
        b[data_at..data_at + payload.len()].copy_from_slice(payload);
        b
    }

    /// CANoe records FlexRay traffic as FR_RCVMESSAGE (50) /
    /// FR_RCVMESSAGE_EX (66) objects beside the CAN frames; the reader
    /// turns both variants into FR rows with the reception channel from
    /// the channel mask, and honors a truncated dataCount.
    #[test]
    fn flexray_receive_objects_decode_into_rows() {
        for (ex, mask, want_ab) in [
            (false, 1u16, 0u8),
            (false, 2, 1),
            (false, 3, 2),
            (true, 1, 0),
            (true, 2, 1),
        ] {
            let body = fr_rcv_body(ex, mask, 33, 7, &[0xDE, 0xAD, 0xBE, 0xEF]);
            let Some(FrOrCan::Fr(r)) = decode_fr_rcv(ex, &body, 5_000) else {
                panic!("variant ex={ex} should decode");
            };
            assert_eq!(r.ab, want_ab, "channel mask {mask}");
            assert_eq!(r.slot, 33);
            assert_eq!(r.cycle, 7);
            assert_eq!(r.payload, vec![0xDE, 0xAD, 0xBE, 0xEF]);
            assert_eq!(r.t_us, 5_000);
        }

        // A truncated payload (dataCount below byteCount) yields the
        // bytes actually stored, per the format's contract.
        let mut body = fr_rcv_body(false, 3, 33, 7, &[0xDE, 0xAD, 0xBE, 0xEF]);
        body[24..26].copy_from_slice(&2u16.to_le_bytes());
        let Some(FrOrCan::Fr(r)) = decode_fr_rcv(false, &body, 0) else {
            panic!("plain variant decodes");
        };
        assert_eq!(r.payload.len(), 2, "dataCount caps the payload");
    }

    /// The header CRC and frame-flag words the object carries are surfaced,
    /// not dropped: headerCrc1 at 18 and frameFlags at 36 per blf.h.
    #[test]
    fn flexray_receive_object_carries_header_crc_and_flags() {
        let mut body = fr_rcv_body(false, 1, 5, 3, &[0x11, 0x22]);
        body[18..20].copy_from_slice(&0xABCDu16.to_le_bytes()); // headerCrc1
        body[36..40].copy_from_slice(&0x1234_5678u32.to_le_bytes()); // frameFlags
        let Some(FrOrCan::Fr(r)) = decode_fr_rcv(false, &body, 1000) else {
            panic!("plain variant decodes");
        };
        assert_eq!(r.header_crc, 0xABCD, "headerCrc1 at 18");
        assert_eq!(r.flags, 0x5678, "frameFlags at 36, low 16 bits");
        assert_eq!(r.slot, 5, "frameId is the frame coordinate BLF records");
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

    fn times(s: &mut dyn FrameStream) -> Vec<u64> {
        let mut out = Vec::new();
        while let Some(f) = s.next_frame() {
            out.push(f.t_us);
        }
        out
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

    /// A FlexRay row survives the recorder's write -> read round trip: the
    /// slot, cycle, reception channel, payload and the two header words come
    /// back unchanged. Before this the recorder could only save CAN, so a
    /// FlexRay session was unrecoverable.
    #[test]
    fn flexray_rows_survive_a_recording_round_trip() {
        use crate::trace::FrRow;
        let path = std::env::temp_dir().join(format!("roxy_can_blf_fr_{}.blf", std::process::id()));
        let rows = vec![
            FrRow {
                bus: 0,
                t_us: 1_000,
                ab: 0,
                slot: 13,
                cycle: 0,
                payload: vec![0x00, 0xEC, 0x00],
                header_crc: 0xABCD,
                flags: 0x1234,
                name: None,
            },
            FrRow {
                bus: 1,
                t_us: 2_000,
                ab: 1,
                slot: 52,
                cycle: 5,
                payload: vec![1, 2, 3, 4],
                header_crc: 0,
                flags: 0,
                name: None,
            },
            FrRow {
                bus: 0,
                t_us: 3_000,
                ab: 2,
                slot: 141,
                cycle: 12,
                payload: vec![7; 20],
                header_crc: 1,
                flags: 2,
                name: None,
            },
        ];
        {
            let mut w = BlfWriter::create(&path.to_string_lossy()).expect("create");
            for r in &rows {
                w.write_fr(r);
            }
            w.finish().expect("finish");
        }
        let mut stream = BlfStream::open(&path).expect("the FR-only file opens");
        let mut got: Vec<FrRow> = Vec::new();
        while stream.peek_fr_t().is_some() {
            stream.poll_fr_rows(u64::MAX, &mut got);
        }
        assert_eq!(got.len(), rows.len(), "every FR row read back");
        for (g, want) in got.iter().zip(rows.iter()) {
            assert_eq!(
                (g.bus, g.slot, g.cycle, g.ab, &g.payload),
                (want.bus, want.slot, want.cycle, want.ab, &want.payload),
                "the frame coordinates -- and which cluster they belong to -- survive"
            );
            // The reader rebases a log so its first row lands at zero (that is
            // where a replay starts), so what survives is the spacing.
            assert_eq!(
                g.t_us,
                want.t_us - rows[0].t_us,
                "the spacing between rows survives the rebase"
            );
            assert_eq!(g.header_crc, want.header_crc, "headerCrc1 survives");
            assert_eq!(g.flags, want.flags, "frameFlags survive");
        }
        std::fs::remove_file(&path).ok();
    }

    /// The bundled CANoe recording at `assets/fibex/Logging.blf` is the ground
    /// truth for `wClusterNo`: 60 072 FlexRay objects, 29 738 of them cluster 0
    /// and 30 334 cluster 1, and the same slot numbers appear on both. Reading
    /// the field is the only thing that keeps the two clusters apart in Trace,
    /// Messages and every key downstream -- and until now every row said
    /// "cluster 0".
    ///
    /// The shape of the traffic says the same thing: these are two independent
    /// networks running one schedule template, not one cluster captured on its
    /// redundant A/B channels. Both clusters use slots 13/16/25/26/51/52 in the
    /// same even cycles, yet no instant carries a frame from both, and the
    /// payloads of a shared slot barely overlap. A redundant pair would show
    /// the same frames twice at the same time -- and would then have to be
    /// merged into one bus with the cluster number as the channel, which is
    /// the opposite of what this reader does.
    #[test]
    fn a_real_two_cluster_recording_keeps_its_clusters_apart() {
        let p = std::path::Path::new("assets/fibex/Logging.blf");
        if !p.exists() {
            panic!("{p:?} missing -- it is tracked in the repository, so a broken checkout must fail, not skip");
        }
        let mut s = BlfStream::open(p).expect("open");
        let mut rows = Vec::new();
        while s.peek_fr_t().is_some() {
            s.poll_fr_rows(u64::MAX, &mut rows);
        }
        assert_eq!(rows.len(), 60_072, "every FR object decoded");
        let mut per_bus: std::collections::BTreeMap<u8, u64> = Default::default();
        let mut slots: [std::collections::HashSet<u16>; 2] = Default::default();
        for r in &rows {
            *per_bus.entry(r.bus).or_default() += 1;
            if let Some(set) = slots.get_mut(r.bus as usize) {
                set.insert(r.slot);
            }
        }
        assert_eq!(
            per_bus.into_iter().collect::<Vec<_>>(),
            vec![(0, 29_738), (1, 30_334)],
            "the two clusters, as the file states them"
        );
        let shared: Vec<u16> = slots[0].intersection(&slots[1]).copied().collect();
        assert!(
            !shared.is_empty(),
            "the clusters really do reuse slot numbers, which is the point"
        );
        let mut instants: std::collections::BTreeMap<u64, (bool, bool)> = Default::default();
        for r in &rows {
            let e = instants.entry(r.t_us).or_default();
            if r.bus == 0 {
                e.0 = true;
            } else {
                e.1 = true;
            }
        }
        assert_eq!(
            instants.len(),
            rows.len(),
            "each row has an instant to itself, so nothing here is one frame \
             captured twice"
        );
        assert!(
            instants.values().all(|&(a, b)| !(a && b)),
            "no instant carries a frame from both clusters"
        );
        let payloads = |bus: u8| -> std::collections::BTreeSet<Vec<u8>> {
            rows.iter()
                .filter(|r| r.bus == bus && r.slot == 13 && r.cycle == 0)
                .map(|r| r.payload.clone())
                .collect()
        };
        let (a, b) = (payloads(0), payloads(1));
        let same = a.intersection(&b).count();
        assert!(
            a.len() > 100 && b.len() > 100 && same * 4 < a.len().min(b.len()),
            "slot 13 in cycle 0 is different traffic on the two clusters: \
             {} vs {} payloads, {same} shared",
            a.len(),
            b.len()
        );
    }

    /// The recorder writes both kinds into one container stream, the way a
    /// real FlexRay session looks: reading it back keeps each side intact and
    /// the two interleaved in log order.
    #[test]
    fn a_mixed_can_and_flexray_recording_reads_back_in_order() {
        let path = std::env::temp_dir().join(format!(
            "roxy_can_mixed_{}.blf",
            std::process::id()
        ));
        {
            let mut w = BlfWriter::create(&path.to_string_lossy()).expect("create");
            for i in 0..40u64 {
                let mut f = CanFrame {
                    t_us: i * 2_000,
                    channel: 0,
                    id: 0x100,
                    extended: false,
                    len: 2,
                    data: [i as u8, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
                        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
                        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
                        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
                    dir: Direction::Rx,
                    flags: FrameFlags::NONE,
                };
                f.data[1] = i as u8;
                w.write(&f);
                w.write_fr(&crate::trace::FrRow {
                    bus: 0,
                    t_us: i * 2_000 + 1_000,
                    ab: 0,
                    slot: 13,
                    cycle: i as u8 % 16,
                    payload: vec![0x00, i as u8],
                    header_crc: 0,
                    flags: 0,
                    name: None,
                });
            }
            w.finish().expect("finish");
        }
        let mut s = BlfStream::open(&path).expect("open");
        let mut can_ts = Vec::new();
        while let Some(f) = s.next_frame() {
            can_ts.push(f.t_us);
        }
        let mut fr = Vec::new();
        while s.peek_fr_t().is_some() {
            s.poll_fr_rows(u64::MAX, &mut fr);
        }
        assert_eq!(can_ts.len(), 40, "every CAN frame reads back");
        assert!(can_ts.windows(2).all(|w| w[1] > w[0]), "ascending");
        assert_eq!(fr.len(), 40, "every FlexRay row reads back beside them");
        assert_eq!(fr[10].slot, 13);
        assert_eq!(fr[10].payload, vec![0x00, 10]);
        // The FlexRay row of pair i sits between CAN frames i and i+1.
        assert_eq!(fr[10].t_us, can_ts[10] + 1_000, "the interleave survives");
        std::fs::remove_file(&path).ok();
    }
}

#[cfg(test)]
mod record_times {
    use super::*;

    /// A recording states its own traffic span: frames a second apart must
    /// come back a second apart, and the header must say how long the file is
    /// even though the process wrote it in one go. The earlier round trip
    /// compared ids, flags and payloads and never looked at the stamps, so a
    /// collapsed timeline stayed invisible.
    #[test]
    fn a_recording_keeps_its_second_apart_frames_a_second_apart() {
        let path =
            std::env::temp_dir().join(format!("roxy_can_ts_gap_{}.blf", std::process::id()));
        {
            let mut w = BlfWriter::create(&path.to_string_lossy()).expect("create");
            for i in 1..=3u64 {
                w.write(&CanFrame {
                    t_us: i * 1_000_000,
                    channel: 0,
                    id: 0x100,
                    extended: false,
                    len: 0,
                    data: [0u8; MAX_CAN_FD_LEN],
                    dir: Direction::Rx,
                    flags: FrameFlags::NONE,
                });
            }
            w.finish().expect("finish");
        }
        let mut s = BlfStream::open(&path).expect("open");
        let mut got = Vec::new();
        while let Some(f) = s.next_frame() {
            got.push(f.t_us);
        }
        assert_eq!(got, vec![0, 1_000_000, 2_000_000], "the spacing survives");
        assert_eq!(
            s.duration_us(),
            Some(2_000_000),
            "the header states the span the frames cover: {}",
            s.describe()
        );
        std::fs::remove_file(&path).ok();
    }

    /// FlexRay rows count toward that span too -- a FlexRay-only recording has
    /// no CAN frame to carry it.
    #[test]
    fn a_flexray_only_recording_states_its_span_too() {
        let path = std::env::temp_dir().join(format!(
            "roxy_can_ts_fr_{}.blf",
            std::process::id()
        ));
        {
            let mut w = BlfWriter::create(&path.to_string_lossy()).expect("create");
            for i in 0..3u64 {
                w.write_fr(&crate::trace::FrRow {
                    bus: 0,
                    t_us: i * 500_000,
                    ab: 0,
                    slot: 13,
                    cycle: i as u8,
                    payload: vec![1, 2],
                    header_crc: 0,
                    flags: 0,
                    name: None,
                });
            }
            w.finish().expect("finish");
        }
        let s = BlfStream::open(&path).expect("open");
        assert_eq!(
            s.duration_us(),
            Some(1_000_000),
            "FlexRay traffic alone still sizes the file: {}",
            s.describe()
        );
        std::fs::remove_file(&path).ok();
    }
}
