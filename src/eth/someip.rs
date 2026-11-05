//! SOME/IP over IPv4/IPv6, decoded from captured Ethernet frames.
//!
//! UDP messages are one datagram each. TCP is reassembled in-order per flow:
//! a captured stream often starts mid-connection, so the first segment seen
//! defines the sequence origin; retransmitted overlap is trimmed and a gap
//! drops the flow rather than guessing bytes. SOME/IP-TP is not reassembled.

use std::collections::{BTreeMap, HashMap};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

use crate::eth::EthFrame;
use serde::Serialize;
use std::sync::Arc;

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SomeIpEvent {
    pub t_us: u64,
    pub bus: u8,
    pub tx: bool,
    /// "UDP" or "TCP".
    pub transport: String,
    pub src: String,
    pub dst: String,
    pub src_port: u16,
    pub dst_port: u16,
    pub service: u16,
    pub method: u16,
    pub message_type: u8,
    pub return_code: u8,
    pub client: u16,
    pub session: u16,
    pub interface_version: u8,
    pub protocol_version: u8,
    pub length: u32,
    /// Payload bytes after the 16-byte SOME/IP header, capped for snapshots.
    pub payload: Vec<u8>,
    /// Fields resolved through the ARXML service catalog, when one is loaded.
    pub decoded: Vec<crate::eth::arxml_service::ServiceValue>,
    /// Why ARXML field decoding produced no values, if it was attempted.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub decode_error: Option<String>,
    /// SOME/IP-SD entries (services offered/subscribed); `None` for ordinary
    /// method/event messages.
    pub sd: Option<Vec<SdEntry>>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SdEntry {
    /// 0x01 service (offer/stop-offer, TTL decides), 0x06 eventgroup
    /// subscribe/stop, 0x07 subscribe ack/nack.
    pub kind: u8,
    pub service: u16,
    pub instance: u16,
    pub major: u8,
    pub ttl: u32,
    pub minor: u32,
    /// Subscribe/subscribe-ack entries carry an eventgroup id in the last
    /// four bytes (12 reserved bits + 16-bit id + 4-bit counter).
    pub eventgroup: Option<u16>,
    /// Endpoint options referenced by this entry.
    pub endpoints: Vec<SdEndpoint>,
    /// Configuration strings referenced by this entry's option run.
    pub configuration: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SdEndpoint {
    pub address: String,
    pub port: u16,
    /// 0x11 UDP, 0x06 TCP (the SD L4 protocol value).
    pub protocol: u8,
}

#[derive(Clone, Debug)]
struct SomeIpHeader {
    service: u16,
    method: u16,
    message_type: u8,
    return_code: u8,
    client: u16,
    session: u16,
    interface_version: u8,
    protocol_version: u8,
    length: u32,
    sd: Option<Vec<SdEntry>>,
}

/// Returns a service event when the frame's UDP payload is a well-formed
/// SOME/IP message. The length field must match the datagram exactly, which
/// keeps arbitrary UDP traffic from being mislabelled.
pub fn decode(frame: &EthFrame) -> Option<SomeIpEvent> {
    let (src, dst, l4) = ip_l4(&frame.payload, 17)?;
    if l4.len() < 8 {
        return None;
    }
    let src_port = u16::from_be_bytes([l4[0], l4[1]]);
    let dst_port = u16::from_be_bytes([l4[2], l4[3]]);
    let udp_len = usize::from(u16::from_be_bytes([l4[4], l4[5]]));
    if udp_len < 8 || udp_len > l4.len() {
        return None;
    }
    let header = parse(&l4[8..udp_len])?;
    Some(event(
        &header,
        frame,
        src,
        dst,
        src_port,
        dst_port,
        "UDP",
        &l4[8..udp_len],
    ))
}

#[allow(clippy::too_many_arguments)]
fn event(
    header: &SomeIpHeader,
    frame: &EthFrame,
    src: IpAddr,
    dst: IpAddr,
    src_port: u16,
    dst_port: u16,
    transport: &str,
    payload: &[u8],
) -> SomeIpEvent {
    SomeIpEvent {
        t_us: frame.t_us,
        bus: frame.bus,
        tx: matches!(frame.dir, crate::can::frame::Direction::Tx),
        transport: transport.to_string(),
        src: src.to_string(),
        dst: dst.to_string(),
        src_port,
        dst_port,
        service: header.service,
        method: header.method,
        message_type: header.message_type,
        return_code: header.return_code,
        client: header.client,
        session: header.session,
        interface_version: header.interface_version,
        protocol_version: header.protocol_version,
        length: header.length,
        payload: payload
            .get(16..)
            .unwrap_or_default()
            .iter()
            .copied()
            .take(4096)
            .collect(),
        decoded: Vec::new(),
        decode_error: None,
        sd: header.sd.clone(),
    }
}

/// The 16-byte SOME/IP header plus its payload length, on an exact message
/// slice (used by both the UDP datagram and the TCP stream).
fn parse(payload: &[u8]) -> Option<SomeIpHeader> {
    if payload.len() < 16 {
        return None;
    }
    let service = u16::from_be_bytes([payload[0], payload[1]]);
    let method = u16::from_be_bytes([payload[2], payload[3]]);
    let length = u32::from_be_bytes([payload[4], payload[5], payload[6], payload[7]]);
    if usize::try_from(length).ok()?.checked_add(8)? != payload.len() {
        return None;
    }
    let client = u16::from_be_bytes([payload[8], payload[9]]);
    let session = u16::from_be_bytes([payload[10], payload[11]]);
    let protocol_version = payload[12];
    let interface_version = payload[13];
    let message_type = payload[14];
    let return_code = payload[15];
    if protocol_version != 1 {
        return None;
    }
    let known_type = matches!(
        message_type,
        0x00 | 0x01 | 0x02 | 0x20 | 0x21 | 0x22 | 0x80 | 0x81 | 0xa0 | 0xa1
    );
    if !known_type {
        return None;
    }
    let sd = if service == 0xffff && method == 0x8100 {
        let body = payload.get(16..)?;
        if body.len() < 4 {
            return None;
        }
        Some(parse_sd(body)?)
    } else {
        None
    };
    Some(SomeIpHeader {
        service,
        method,
        message_type,
        return_code,
        client,
        session,
        interface_version,
        protocol_version,
        length,
        sd,
    })
}

/// SD body, per the wire format: flags(1) + reserved(3), entries length(u32),
/// entries, options length(u32), options. Both lengths are explicit; the
/// entry count is never guessed.
fn parse_sd(body: &[u8]) -> Option<Vec<SdEntry>> {
    if body.len() < 8 {
        return None;
    }
    let entries_len =
        usize::try_from(u32::from_be_bytes([body[4], body[5], body[6], body[7]])).ok()?;
    let entries_start = 8usize;
    let entries_end = entries_start.checked_add(entries_len)?;
    if entries_len % 16 != 0 || entries_end + 4 > body.len() {
        return None;
    }
    let options_len = usize::try_from(u32::from_be_bytes([
        body[entries_end],
        body[entries_end + 1],
        body[entries_end + 2],
        body[entries_end + 3],
    ]))
    .ok()?;
    let options_start = entries_end + 4;
    let options_end = options_start.checked_add(options_len)?;
    if options_end > body.len() {
        return None;
    }
    let options = parse_options(&body[options_start..options_end])?;
    let mut entries = Vec::with_capacity(entries_len / 16);
    for entry in body[entries_start..entries_end].as_chunks::<16>().0 {
        let num_first = entry[3] >> 4;
        let num_second = entry[3] & 0x0f;
        let mut endpoints = endpoints_at(&options, entry[1], num_first)?;
        endpoints.extend(endpoints_at(&options, entry[2], num_second)?);
        let mut configuration = configuration_at(&options, entry[1], num_first);
        configuration.extend(configuration_at(&options, entry[2], num_second));
        let kind = entry[0];
        let eventgroup = if matches!(kind, 0x06 | 0x07) {
            Some(u16::from_be_bytes([entry[14], entry[15]]))
        } else {
            None
        };
        entries.push(SdEntry {
            kind,
            service: u16::from_be_bytes([entry[4], entry[5]]),
            instance: u16::from_be_bytes([entry[6], entry[7]]),
            major: entry[8],
            ttl: u32::from_be_bytes([0, entry[9], entry[10], entry[11]]),
            minor: if eventgroup.is_some() {
                0
            } else {
                u32::from_be_bytes([entry[12], entry[13], entry[14], entry[15]])
            },
            eventgroup,
            endpoints,
            configuration,
        });
    }
    Some(entries)
}

struct SdOption {
    offset: usize,
    endpoint: Option<SdEndpoint>,
    configuration: Vec<String>,
}

fn parse_options(bytes: &[u8]) -> Option<Vec<SdOption>> {
    let mut options = Vec::new();
    let mut at = 0usize;
    while at + 3 <= bytes.len() {
        let length = usize::from(u16::from_be_bytes([bytes[at], bytes[at + 1]]));
        let total = length.checked_add(3)?;
        if total < 4 || at + total > bytes.len() {
            return None;
        }
        let kind = bytes[at + 2];
        let data = &bytes[at + 4..at + total];
        options.push(SdOption {
            offset: at,
            endpoint: endpoint_option(kind, data),
            configuration: configuration_option(kind, data),
        });
        at += total;
    }
    if at != bytes.len() {
        return None;
    }
    Some(options)
}

fn endpoint_option(kind: u8, data: &[u8]) -> Option<SdEndpoint> {
    let (address, protocol, port) = match kind {
        0x04 | 0x14 | 0x24 if data.len() == 8 => {
            let address = format!("{}.{}.{}.{}", data[0], data[1], data[2], data[3]);
            (address, data[5], u16::from_be_bytes([data[6], data[7]]))
        }
        0x06 | 0x16 | 0x26 if data.len() == 20 => {
            let address = Ipv6Addr::from(<[u8; 16]>::try_from(&data[..16]).ok()?).to_string();
            (address, data[17], u16::from_be_bytes([data[18], data[19]]))
        }
        _ => return None,
    };
    Some(SdEndpoint {
        address,
        port,
        protocol,
    })
}

fn configuration_option(kind: u8, data: &[u8]) -> Vec<String> {
    if kind != 0x01 {
        return Vec::new();
    }
    let mut strings = Vec::new();
    let mut at = 0usize;
    while at < data.len() {
        let length = usize::from(data[at]);
        at += 1;
        if length == 0 {
            break;
        }
        let Some(value) = data.get(at..at + length) else {
            break;
        };
        if let Ok(text) = std::str::from_utf8(value) {
            strings.push(text.to_string());
        }
        at += length;
    }
    strings
}

fn endpoints_at(options: &[SdOption], index: u8, count: u8) -> Option<Vec<SdEndpoint>> {
    if count == 0 {
        return Some(Vec::new());
    }
    let start = usize::from(index);
    let first = options.iter().position(|option| option.offset == start)?;
    let mut endpoints = Vec::new();
    for option in options.iter().skip(first).take(usize::from(count)) {
        if let Some(endpoint) = &option.endpoint {
            endpoints.push(endpoint.clone());
        }
    }
    Some(endpoints)
}

fn configuration_at(options: &[SdOption], index: u8, count: u8) -> Vec<String> {
    if count == 0 {
        return Vec::new();
    }
    let start = usize::from(index);
    let Some(first) = options.iter().position(|option| option.offset == start) else {
        return Vec::new();
    };
    let mut values = Vec::new();
    for option in options.iter().skip(first).take(usize::from(count)) {
        values.extend(option.configuration.iter().cloned());
    }
    values
}

/// IPv4 or IPv6, non-fragmented, the requested L4 protocol, plus endpoints.
fn ip_l4(payload: &[u8], protocol: u8) -> Option<(IpAddr, IpAddr, &[u8])> {
    let version = payload.first()? >> 4;
    if version == 4 {
        if payload.len() < 20 {
            return None;
        }
        let ihl = usize::from(payload[0] & 0x0f) * 4;
        if ihl < 20 || payload.len() < ihl {
            return None;
        }
        let total = usize::from(u16::from_be_bytes([payload[2], payload[3]]));
        let fragment = u16::from_be_bytes([payload[6], payload[7]]);
        if fragment & 0x3fff != 0 || payload[9] != protocol {
            return None;
        }
        let src = IpAddr::V4(Ipv4Addr::new(
            payload[12],
            payload[13],
            payload[14],
            payload[15],
        ));
        let dst = IpAddr::V4(Ipv4Addr::new(
            payload[16],
            payload[17],
            payload[18],
            payload[19],
        ));
        let end = total.min(payload.len());
        return Some((src, dst, payload.get(ihl..end)?));
    }
    if version == 6 {
        if payload.len() < 40 {
            return None;
        }
        let total =
            (40 + usize::from(u16::from_be_bytes([payload[4], payload[5]]))).min(payload.len());
        let src = IpAddr::V6(Ipv6Addr::from(<[u8; 16]>::try_from(&payload[8..24]).ok()?));
        let dst = IpAddr::V6(Ipv6Addr::from(<[u8; 16]>::try_from(&payload[24..40]).ok()?));
        let mut next = payload[6];
        let mut at = 40usize;
        for _ in 0..8 {
            if next == protocol {
                return Some((src, dst, payload.get(at..total)?));
            }
            match next {
                // Hop-by-Hop, Routing and Destination Options: next header
                // then an 8-byte-unit length.
                0 | 43 | 60 => {
                    let length = (usize::from(*payload.get(at + 1)?) + 1) * 8;
                    next = *payload.get(at)?;
                    at = at.checked_add(length)?;
                }
                // Fragment header: only the first fragment can carry the
                // transport header; later fragments are skipped.
                44 => {
                    let fragment =
                        u16::from_be_bytes([*payload.get(at + 2)?, *payload.get(at + 3)?]);
                    if fragment & 0xfff8 != 0 {
                        return None;
                    }
                    next = *payload.get(at)?;
                    at = at.checked_add(8)?;
                }
                // Authentication Header: (length + 2) * 4 bytes.
                51 => {
                    let length = (usize::from(*payload.get(at + 1)?) + 2) * 4;
                    next = *payload.get(at)?;
                    at = at.checked_add(length)?;
                }
                // ESP is encrypted; nothing useful to parse here.
                50 | 59 => return None,
                _ => return None,
            }
            if at > total {
                return None;
            }
        }
        return None;
    }
    None
}

#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
struct TcpKey {
    bus: u8,
    src: IpAddr,
    src_port: u16,
    dst: IpAddr,
    dst_port: u16,
}

#[derive(Default)]
struct TcpFlow {
    next_seq: Option<u32>,
    buffer: Vec<u8>,
}

/// In-order TCP reassembly for SOME/IP over TCP. Bounded per flow and in
/// flow count so a malformed capture cannot grow without limit.
#[derive(Default)]
pub struct SomeIpTcp {
    flows: HashMap<TcpKey, TcpFlow>,
}

const MAX_FLOWS: usize = 256;
const MAX_STREAM: usize = 1 << 20;

impl SomeIpTcp {
    pub fn feed(&mut self, frame: &EthFrame) -> Vec<SomeIpEvent> {
        let Some((src, dst, tcp)) = ip_l4(&frame.payload, 6) else {
            return Vec::new();
        };
        if tcp.len() < 20 {
            return Vec::new();
        }
        let src_port = u16::from_be_bytes([tcp[0], tcp[1]]);
        let dst_port = u16::from_be_bytes([tcp[2], tcp[3]]);
        let seq = u32::from_be_bytes([tcp[4], tcp[5], tcp[6], tcp[7]]);
        let flags = tcp[13];
        let offset = usize::from(tcp[12] >> 4) * 4;
        if offset < 20 || tcp.len() < offset {
            return Vec::new();
        }
        let key = TcpKey {
            bus: frame.bus,
            src,
            src_port,
            dst,
            dst_port,
        };
        let payload = &tcp[offset..];

        if flags & 0x02 != 0 {
            // SYN: the next byte is sequence+1.
            self.flows.insert(
                key,
                TcpFlow {
                    next_seq: Some(seq.wrapping_add(1)),
                    buffer: Vec::new(),
                },
            );
            return Vec::new();
        }

        if self.flows.len() >= MAX_FLOWS && !self.flows.contains_key(&key) {
            return Vec::new();
        }
        let flow = self.flows.entry(key).or_default();
        if flow.next_seq.is_none() {
            flow.next_seq = Some(seq);
        }
        let expected = flow.next_seq.unwrap_or(seq);
        if seq == expected {
            flow.buffer.extend_from_slice(payload);
            flow.next_seq = Some(seq.wrapping_add(payload.len() as u32));
        } else if seq.wrapping_sub(expected) > 0x8000_0000 {
            // Retransmission or overlap: keep only the new tail.
            let overlap = expected.wrapping_sub(seq) as usize;
            if overlap < payload.len() {
                flow.buffer.extend_from_slice(&payload[overlap..]);
                flow.next_seq = Some(seq.wrapping_add(payload.len() as u32));
            }
        } else {
            // A forward gap means a lost segment; do not guess the bytes.
            flow.buffer.clear();
            flow.next_seq = Some(seq);
            flow.buffer.extend_from_slice(payload);
            flow.next_seq = Some(seq.wrapping_add(payload.len() as u32));
        }
        if flow.buffer.len() > MAX_STREAM {
            flow.buffer.clear();
            flow.next_seq = None;
        }

        let mut events = Vec::new();
        loop {
            if flow.buffer.len() < 16 {
                break;
            }
            let length = u32::from_be_bytes([
                flow.buffer[4],
                flow.buffer[5],
                flow.buffer[6],
                flow.buffer[7],
            ]);
            let Some(total) = usize::try_from(length).ok().and_then(|n| n.checked_add(8)) else {
                flow.buffer.clear();
                flow.next_seq = None;
                break;
            };
            if !(16..=MAX_STREAM).contains(&total) {
                flow.buffer.clear();
                flow.next_seq = None;
                break;
            }
            if flow.buffer.len() < total {
                break;
            }
            let Some(header) = parse(&flow.buffer[..total]) else {
                // The stream is no longer trustworthy; wait for a retransmit.
                flow.buffer.drain(..total);
                continue;
            };
            events.push(event(
                &header,
                frame,
                src,
                dst,
                src_port,
                dst_port,
                "TCP",
                &flow.buffer[..total],
            ));
            flow.buffer.drain(..total);
        }
        if flags & (0x01 | 0x04) != 0 {
            self.flows.remove(&key);
        }
        events
    }
}

#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
struct TpKey {
    bus: u8,
    tx: bool,
    src: IpAddr,
    dst: IpAddr,
    src_port: u16,
    dst_port: u16,
    service: u16,
    method: u16,
    client: u16,
    session: u16,
}

#[derive(Default)]
struct TpFlow {
    segments: BTreeMap<usize, Vec<u8>>,
}

/// SOME/IP-TP reassembly: the four-byte TP header carries a 28-bit byte
/// offset (16-byte aligned) and a "more segments" flag. Segments are joined
/// only when the run starts at zero and is contiguous; a missing segment
/// clears the flow instead of producing a short message.
#[derive(Default)]
pub struct SomeIpTp {
    flows: HashMap<TpKey, TpFlow>,
}

impl SomeIpTp {
    pub fn feed(&mut self, event: &SomeIpEvent) -> Option<SomeIpEvent> {
        if event.message_type & 0x20 == 0 {
            return Some(event.clone());
        }
        if event.payload.len() < 4 {
            return None;
        }
        let header = u32::from_be_bytes([
            event.payload[0],
            event.payload[1],
            event.payload[2],
            event.payload[3],
        ]);
        let offset = (header & 0xffff_fff0) as usize;
        let more = header & 1 != 0;
        let body = event.payload[4..].to_vec();
        let key = TpKey {
            bus: event.bus,
            tx: event.tx,
            src: event.src.parse().ok()?,
            dst: event.dst.parse().ok()?,
            src_port: event.src_port,
            dst_port: event.dst_port,
            service: event.service,
            method: event.method,
            client: event.client,
            session: event.session,
        };
        let flow = self.flows.entry(key).or_default();
        if offset == 0 {
            flow.segments.clear();
        }
        flow.segments.insert(offset, body);
        if more {
            return None;
        }
        let end = offset.checked_add(flow.payload_len(offset)?)?;
        let mut combined = Vec::with_capacity(end);
        let mut expected = 0usize;
        for (start, data) in &flow.segments {
            if *start != expected {
                self.flows.remove(&key);
                return None;
            }
            combined.extend_from_slice(data);
            expected = start.checked_add(data.len())?;
        }
        if combined.len() != end {
            self.flows.remove(&key);
            return None;
        }
        self.flows.remove(&key);
        let mut full = event.clone();
        full.message_type &= !0x20;
        full.length = u32::try_from(8 + combined.len()).ok()?;
        full.payload = combined;
        full.decoded.clear();
        Some(full)
    }
}

impl TpFlow {
    fn payload_len(&self, offset: usize) -> Option<usize> {
        self.segments.get(&offset).map(Vec::len)
    }
}

#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
struct IpKey {
    bus: u8,
    src: IpAddr,
    dst: IpAddr,
    id: u16,
    protocol: u8,
}

#[derive(Default)]
struct IpFlow {
    header: Option<Vec<u8>>,
    pieces: BTreeMap<usize, Vec<u8>>,
    total: Option<usize>,
}

/// IPv4 fragment reassembly. Non-fragmented frames pass through unchanged;
/// IPv4/IPv6 fragment reassembly. Non-fragmented frames pass through
/// unchanged; fragmented frames are held until offset 0..total is contiguous.
#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
struct Ip6Key {
    bus: u8,
    src: Ipv6Addr,
    dst: Ipv6Addr,
    id: u32,
    next: u8,
}

#[derive(Default)]
struct Ip6Flow {
    prefix: Option<Vec<u8>>,
    last_next_at: Option<usize>,
    pieces: BTreeMap<usize, Vec<u8>>,
    total: Option<usize>,
}

#[derive(Default)]
pub struct SomeIpIp {
    flows: HashMap<IpKey, IpFlow>,
    flows6: HashMap<Ip6Key, Ip6Flow>,
}

impl SomeIpIp {
    pub fn feed(&mut self, frame: &EthFrame) -> Option<EthFrame> {
        match frame.payload.first().map(|byte| byte >> 4) {
            Some(4) => self.feed_v4(frame),
            Some(6) => self.feed_v6(frame),
            _ => Some(frame.clone()),
        }
    }

    fn feed_v4(&mut self, frame: &EthFrame) -> Option<EthFrame> {
        if frame.payload.len() < 20 {
            return Some(frame.clone());
        }
        let ihl = usize::from(frame.payload[0] & 0x0f) * 4;
        if ihl < 20 || frame.payload.len() < ihl {
            return Some(frame.clone());
        }
        let fragment = u16::from_be_bytes([frame.payload[6], frame.payload[7]]);
        if fragment & 0x3fff == 0 {
            return Some(frame.clone());
        }
        let total = usize::from(u16::from_be_bytes([frame.payload[2], frame.payload[3]]))
            .min(frame.payload.len());
        let offset = usize::from(fragment & 0x1fff) * 8;
        let more = fragment & 0x2000 != 0;
        let key = IpKey {
            bus: frame.bus,
            src: IpAddr::V4(Ipv4Addr::new(
                frame.payload[12],
                frame.payload[13],
                frame.payload[14],
                frame.payload[15],
            )),
            dst: IpAddr::V4(Ipv4Addr::new(
                frame.payload[16],
                frame.payload[17],
                frame.payload[18],
                frame.payload[19],
            )),
            id: u16::from_be_bytes([frame.payload[4], frame.payload[5]]),
            protocol: frame.payload[9],
        };
        let flow = self.flows.entry(key).or_default();
        if offset == 0 {
            flow.header = Some(frame.payload[..ihl].to_vec());
        }
        let data = frame.payload[ihl..total].to_vec();
        if !more {
            flow.total = Some(offset + data.len());
        }
        flow.pieces.insert(offset, data);
        let (Some(header), Some(total)) = (flow.header.as_ref(), flow.total) else {
            return None;
        };
        let mut combined = Vec::with_capacity(total);
        let mut expected = 0usize;
        for (start, data) in &flow.pieces {
            if *start > expected {
                return None;
            }
            let skip = expected - start;
            if skip < data.len() {
                combined.extend_from_slice(&data[skip..]);
                expected = start + data.len();
            }
        }
        if combined.len() != total {
            return None;
        }
        let mut ip = header.clone();
        ip[2..4].copy_from_slice(&((ihl + total) as u16).to_be_bytes());
        ip[6..8].copy_from_slice(&0u16.to_be_bytes());
        ip[10..12].copy_from_slice(&0u16.to_be_bytes());
        ip.extend_from_slice(&combined);
        self.flows.remove(&key);
        let mut full = frame.clone();
        full.payload = Arc::from(ip);
        Some(full)
    }

    fn feed_v6(&mut self, frame: &EthFrame) -> Option<EthFrame> {
        let payload = &frame.payload;
        if payload.len() < 40 {
            return Some(frame.clone());
        }
        let total =
            (40 + usize::from(u16::from_be_bytes([payload[4], payload[5]]))).min(payload.len());
        let mut next = payload[6];
        let mut at = 40usize;
        let mut last_next_at = None;
        loop {
            match next {
                44 => break,
                0 | 43 | 60 => {
                    if at + 2 > total {
                        return Some(frame.clone());
                    }
                    last_next_at = Some(at);
                    let length = (usize::from(payload[at + 1]) + 1) * 8;
                    next = payload[at];
                    at = at.checked_add(length)?;
                }
                51 => {
                    if at + 2 > total {
                        return Some(frame.clone());
                    }
                    last_next_at = Some(at);
                    let length = (usize::from(payload[at + 1]) + 2) * 4;
                    next = payload[at];
                    at = at.checked_add(length)?;
                }
                _ => return Some(frame.clone()),
            }
            if at > total {
                return Some(frame.clone());
            }
        }
        if at + 8 > total {
            return Some(frame.clone());
        }
        let fragment_next = payload[at];
        let fragment = u16::from_be_bytes([payload[at + 2], payload[at + 3]]);
        let offset = usize::from(fragment & 0xfff8);
        let more = fragment & 1 != 0;
        let id = u32::from_be_bytes([
            payload[at + 4],
            payload[at + 5],
            payload[at + 6],
            payload[at + 7],
        ]);
        let key = Ip6Key {
            bus: frame.bus,
            src: Ipv6Addr::from(<[u8; 16]>::try_from(&payload[8..24]).ok()?),
            dst: Ipv6Addr::from(<[u8; 16]>::try_from(&payload[24..40]).ok()?),
            id,
            next: fragment_next,
        };
        let flow = self.flows6.entry(key).or_default();
        if offset == 0 {
            flow.prefix = Some(payload[..at].to_vec());
            flow.last_next_at = last_next_at;
        }
        let data = payload[at + 8..total].to_vec();
        if !more {
            flow.total = Some(offset + data.len());
        }
        flow.pieces.insert(offset, data);
        let (Some(prefix), Some(total)) = (flow.prefix.as_ref(), flow.total) else {
            return None;
        };
        let mut combined = Vec::with_capacity(total);
        let mut expected = 0usize;
        for (start, data) in &flow.pieces {
            if *start > expected {
                return None;
            }
            let skip = expected - start;
            if skip < data.len() {
                combined.extend_from_slice(&data[skip..]);
                expected = start + data.len();
            }
        }
        if combined.len() != total {
            return None;
        }
        let mut ip = prefix.clone();
        let payload_len = ip.len() - 40 + total;
        ip[4..6].copy_from_slice(&(payload_len as u16).to_be_bytes());
        if ip.len() == 40 {
            ip[6] = fragment_next;
        } else if let Some(last) = flow.last_next_at {
            ip[last] = fragment_next;
        }
        ip.extend_from_slice(&combined);
        self.flows6.remove(&key);
        let mut full = frame.clone();
        full.payload = Arc::from(ip);
        Some(full)
    }
}

/// One SOME/IP notification/request as a complete session Ethernet frame.
/// UDP ports are the conventional 30501 -> 30490 pair; the message type is
/// caller-selected, so a Panel/SOME-IP send can inject request, response or
/// notification without a separate packet builder.
#[allow(clippy::too_many_arguments)]
/// The 16-byte SOME/IP header plus payload, without a UDP/TCP wrapper.
/// TCP send writes this directly; UDP wraps it in an 8-byte datagram header.
pub fn build_message(
    service: u16,
    method: u16,
    client: u16,
    session: u16,
    message_type: u8,
    payload: &[u8],
) -> Vec<u8> {
    build_message_rc(service, method, client, session, message_type, 0, payload)
}

pub fn build_message_rc(
    service: u16,
    method: u16,
    client: u16,
    session: u16,
    message_type: u8,
    return_code: u8,
    payload: &[u8],
) -> Vec<u8> {
    let mut someip = Vec::with_capacity(16 + payload.len());
    someip.extend_from_slice(&service.to_be_bytes());
    someip.extend_from_slice(&method.to_be_bytes());
    someip.extend_from_slice(&((8 + payload.len()) as u32).to_be_bytes());
    someip.extend_from_slice(&client.to_be_bytes());
    someip.extend_from_slice(&session.to_be_bytes());
    someip.extend_from_slice(&[1, 1, message_type, return_code]);
    someip.extend_from_slice(payload);
    someip
}

#[allow(clippy::too_many_arguments)]
pub fn build_udp_frame(
    t_us: u64,
    bus: u8,
    service: u16,
    method: u16,
    client: u16,
    session: u16,
    message_type: u8,
    payload: &[u8],
) -> EthFrame {
    build_udp_frame_to(
        t_us,
        bus,
        [10, 0, 0, 1],
        [10, 0, 0, 2],
        30501,
        30490,
        service,
        method,
        client,
        session,
        message_type,
        payload,
    )
}

/// Build one SOME/IP UDP datagram with explicit IPv4 endpoints. Service
/// Discovery uses the same shape with ports 30490 -> 30490 and the SD
/// service/method pair.
#[allow(clippy::too_many_arguments)]
pub fn build_udp_frame_to(
    t_us: u64,
    bus: u8,
    src_ip: [u8; 4],
    dst_ip: [u8; 4],
    src_port: u16,
    dst_port: u16,
    service: u16,
    method: u16,
    client: u16,
    session: u16,
    message_type: u8,
    payload: &[u8],
) -> EthFrame {
    let someip = build_message(service, method, client, session, message_type, payload);

    let mut udp = Vec::with_capacity(8 + someip.len());
    udp.extend_from_slice(&src_port.to_be_bytes());
    udp.extend_from_slice(&dst_port.to_be_bytes());
    udp.extend_from_slice(&((8 + someip.len()) as u16).to_be_bytes());
    udp.extend_from_slice(&0u16.to_be_bytes());
    udp.extend_from_slice(&someip);

    let mut ip = vec![0u8; 20];
    ip[0] = 0x45;
    ip[2..4].copy_from_slice(&((20 + udp.len()) as u16).to_be_bytes());
    ip[4..6].copy_from_slice(&1u16.to_be_bytes()); // identification
    ip[8] = 64;
    ip[9] = 17;
    ip[12..16].copy_from_slice(&src_ip);
    ip[16..20].copy_from_slice(&dst_ip);
    let checksum = ipv4_checksum(&ip);
    ip[10..12].copy_from_slice(&checksum.to_be_bytes());
    ip.extend_from_slice(&udp);

    EthFrame {
        t_us,
        bus,
        dir: crate::can::frame::Direction::Tx,
        dst: EthFrame::SESSION_DST,
        src: EthFrame::SESSION_SRC,
        vlan: None,
        vlan_tpid: None,
        ethertype: 0x0800,
        payload: Arc::from(ip),
        seq: 0,
        truncated: false,
    }
}

/// Build one SOME/IP UDP datagram with an explicit return code. The header
/// byte is patched after the regular builder, keeping the common path small.
#[allow(clippy::too_many_arguments)]
pub fn build_udp_frame_to_rc(
    t_us: u64,
    bus: u8,
    src_ip: [u8; 4],
    dst_ip: [u8; 4],
    src_port: u16,
    dst_port: u16,
    service: u16,
    method: u16,
    client: u16,
    session: u16,
    message_type: u8,
    return_code: u8,
    payload: &[u8],
) -> EthFrame {
    let mut frame = build_udp_frame_to(
        t_us,
        bus,
        src_ip,
        dst_ip,
        src_port,
        dst_port,
        service,
        method,
        client,
        session,
        message_type,
        payload,
    );
    let offset = 20 + 8 + 15;
    let mut bytes = frame.payload.to_vec();
    if bytes.len() > offset {
        bytes[offset] = return_code;
        frame.payload = Arc::from(bytes);
    }
    frame
}

/// One SD OfferService or StopOfferService frame. `ttl=0` is StopOffer.
#[allow(clippy::too_many_arguments)]
pub fn build_sd_offer_frame(
    t_us: u64,
    bus: u8,
    service: u16,
    instance: u16,
    major: u8,
    minor: u32,
    ttl: u32,
    endpoint: [u8; 4],
    port: u16,
    session: u16,
) -> EthFrame {
    let mut entry = [0u8; 16];
    entry[0] = 0x01;
    entry[3] = 0x10; // one option in the first run
    entry[4..6].copy_from_slice(&service.to_be_bytes());
    entry[6..8].copy_from_slice(&instance.to_be_bytes());
    entry[8] = major;
    entry[9..12].copy_from_slice(&ttl.to_be_bytes()[1..]);
    entry[12..16].copy_from_slice(&minor.to_be_bytes());

    let mut option = Vec::with_capacity(12);
    option.extend_from_slice(&9u16.to_be_bytes());
    option.push(0x04); // IPv4 endpoint
    option.push(0);
    option.extend_from_slice(&endpoint);
    option.push(0);
    option.push(17); // UDP
    option.extend_from_slice(&port.to_be_bytes());

    let mut body = vec![0u8; 4]; // flags
    body.extend_from_slice(&16u32.to_be_bytes());
    body.extend_from_slice(&entry);
    body.extend_from_slice(&(option.len() as u32).to_be_bytes());
    body.extend_from_slice(&option);

    build_udp_frame_to(
        t_us,
        bus,
        [10, 0, 0, 1],
        [10, 0, 0, 2],
        30490,
        30490,
        0xFFFF,
        0x8100,
        0,
        session,
        0x02,
        &body,
    )
}

/// One SD SubscribeEventgroupAck. Entry type 0x07 carries the eventgroup in
/// the last two bytes; the endpoint option is the same IPv4 form as Offer.
#[allow(clippy::too_many_arguments)]
pub fn build_sd_subscribe_ack_frame(
    t_us: u64,
    bus: u8,
    service: u16,
    instance: u16,
    major: u8,
    eventgroup: u16,
    ttl: u32,
    endpoint: [u8; 4],
    port: u16,
    session: u16,
) -> EthFrame {
    let mut entry = [0u8; 16];
    entry[0] = 0x07;
    entry[3] = 0x10;
    entry[4..6].copy_from_slice(&service.to_be_bytes());
    entry[6..8].copy_from_slice(&instance.to_be_bytes());
    entry[8] = major;
    entry[9..12].copy_from_slice(&ttl.to_be_bytes()[1..]);
    entry[14..16].copy_from_slice(&eventgroup.to_be_bytes());

    let mut option = Vec::with_capacity(12);
    option.extend_from_slice(&9u16.to_be_bytes());
    option.push(0x04);
    option.push(0);
    option.extend_from_slice(&endpoint);
    option.push(0);
    option.push(17);
    option.extend_from_slice(&port.to_be_bytes());

    let mut body = vec![0u8; 4];
    body.extend_from_slice(&16u32.to_be_bytes());
    body.extend_from_slice(&entry);
    body.extend_from_slice(&(option.len() as u32).to_be_bytes());
    body.extend_from_slice(&option);

    build_udp_frame_to(
        t_us,
        bus,
        [10, 0, 0, 1],
        [10, 0, 0, 2],
        30490,
        30490,
        0xFFFF,
        0x8100,
        0,
        session,
        0x02,
        &body,
    )
}

fn ipv4_checksum(header: &[u8]) -> u16 {
    let mut sum = 0u32;
    for chunk in header.as_chunks::<2>().0 {
        sum += u32::from(u16::from_be_bytes([chunk[0], chunk[1]]));
    }
    while sum >> 16 != 0 {
        sum = (sum & 0xffff) + (sum >> 16);
    }
    !(sum as u16)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::can::frame::Direction;
    use std::sync::Arc;

    fn frame(l4: &[u8], protocol: u8, ethertype: u16) -> EthFrame {
        let mut ip = vec![0u8; 20];
        ip[0] = 0x45;
        ip[2..4].copy_from_slice(&((20 + l4.len()) as u16).to_be_bytes());
        ip[8] = 64;
        ip[9] = protocol;
        ip[12..16].copy_from_slice(&[10, 0, 0, 1]);
        ip[16..20].copy_from_slice(&[10, 0, 0, 2]);
        ip.extend_from_slice(l4);
        EthFrame {
            t_us: 10,
            bus: 0,
            dir: Direction::Rx,
            dst: [0; 6],
            src: [0; 6],
            vlan: None,
            vlan_tpid: None,
            ethertype,
            payload: Arc::from(ip),
            seq: 0,
            truncated: false,
        }
    }

    fn udp(port: u16, payload: &[u8]) -> Vec<u8> {
        let mut udp = Vec::new();
        udp.extend_from_slice(&30490u16.to_be_bytes());
        udp.extend_from_slice(&port.to_be_bytes());
        udp.extend_from_slice(&((8 + payload.len()) as u16).to_be_bytes());
        udp.extend_from_slice(&0u16.to_be_bytes());
        udp.extend_from_slice(payload);
        udp
    }

    fn someip_message(service: u16, method: u16, payload: &[u8]) -> Vec<u8> {
        let mut message = Vec::new();
        message.extend_from_slice(&service.to_be_bytes());
        message.extend_from_slice(&method.to_be_bytes());
        message.extend_from_slice(&((8 + payload.len()) as u32).to_be_bytes());
        message.extend_from_slice(&1u16.to_be_bytes());
        message.extend_from_slice(&7u16.to_be_bytes());
        message.push(1);
        message.push(1);
        message.push(0x02);
        message.push(0);
        message.extend_from_slice(payload);
        message
    }

    fn tp_message(service: u16, method: u16, offset: u32, more: bool, body: &[u8]) -> Vec<u8> {
        let tp = (offset & 0xffff_fff0) | u32::from(more);
        let mut payload = tp.to_be_bytes().to_vec();
        payload.extend_from_slice(body);
        let mut message = Vec::new();
        message.extend_from_slice(&service.to_be_bytes());
        message.extend_from_slice(&method.to_be_bytes());
        message.extend_from_slice(&((8 + payload.len()) as u32).to_be_bytes());
        message.extend_from_slice(&1u16.to_be_bytes());
        message.extend_from_slice(&7u16.to_be_bytes());
        message.extend_from_slice(&[1, 1, 0x22, 0]);
        message.extend_from_slice(&payload);
        message
    }

    #[test]
    fn a_notification_carries_service_method_and_session() {
        let event = decode(&frame(
            &udp(30501, &someip_message(0x1234, 0x8001, &[1, 2])),
            17,
            0x0800,
        ))
        .expect("SOME/IP notification");
        assert_eq!((event.service, event.method), (0x1234, 0x8001));
        assert_eq!(event.message_type, 0x02);
        assert_eq!(event.client, 1);
        assert_eq!(event.session, 7);
        assert_eq!(event.src, "10.0.0.1");
        assert_eq!(event.transport, "UDP");
        assert_eq!(event.payload, [1, 2], "payload excludes the SOME/IP header");
        assert!(event.sd.is_none());
    }

    fn sd_body(entries: &[[u8; 16]], options: &[u8]) -> Vec<u8> {
        let mut body = vec![0u8; 4];
        body.extend_from_slice(&((entries.len() * 16) as u32).to_be_bytes());
        for entry in entries {
            body.extend_from_slice(entry);
        }
        body.extend_from_slice(&(options.len() as u32).to_be_bytes());
        body.extend_from_slice(options);
        body
    }

    #[test]
    fn an_sd_subscribe_ack_carries_the_eventgroup() {
        let frame =
            build_sd_subscribe_ack_frame(1_000, 0, 0x1234, 2, 1, 5, 3, [10, 0, 0, 1], 30490, 8);
        let event = decode(&frame).expect("SD ack");
        let entries = event.sd.expect("entries");
        assert_eq!(entries[0].kind, 0x07);
        assert_eq!(entries[0].eventgroup, Some(5));
        assert_eq!(entries[0].ttl, 3);
    }

    #[test]
    fn udp_builder_patches_the_return_code() {
        let frame = build_udp_frame_to_rc(
            1,
            0,
            [10, 0, 0, 1],
            [10, 0, 0, 2],
            30501,
            30490,
            0x1234,
            0x8001,
            1,
            2,
            0x80,
            7,
            &[0xAA],
        );
        assert_eq!(frame.payload[20 + 8 + 15], 7);
    }

    #[test]
    fn an_sd_offer_frame_round_trips_through_the_parser() {
        let frame = build_sd_offer_frame(1_000, 0, 0x1234, 2, 1, 3, 10, [10, 0, 0, 1], 30490, 7);
        let event = decode(&frame).expect("SD decode");
        assert_eq!(event.service, 0xFFFF);
        let entries = event.sd.expect("entries");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].kind, 0x01);
        assert_eq!(entries[0].service, 0x1234);
        assert_eq!(entries[0].instance, 2);
        assert_eq!(entries[0].ttl, 10);
        assert_eq!(entries[0].endpoints[0].port, 30490);
    }

    #[test]
    fn an_sd_message_counts_its_entries() {
        let mut entry = [0u8; 16];
        entry[0] = 0x01;
        let sd = sd_body(&[entry, entry], &[]);
        let event = decode(&frame(
            &udp(30490, &someip_message(0xffff, 0x8100, &sd)),
            17,
            0x0800,
        ))
        .expect("SD message");
        assert_eq!(event.sd.as_ref().map(Vec::len), Some(2));
    }

    #[test]
    fn an_sd_offer_exposes_its_ipv4_endpoint() {
        let mut entry = [0u8; 16];
        entry[0] = 0x01;
        entry[1] = 0;
        entry[3] = 0x10;
        entry[4..6].copy_from_slice(&0x1234u16.to_be_bytes());
        entry[6..8].copy_from_slice(&1u16.to_be_bytes());
        entry[8] = 1;
        entry[11] = 3;
        // IPv4 endpoint option: 2-byte length (9), type, reserved, address,
        // reserved, L4 protocol, port.
        let option = [0, 9, 0x04, 0, 10, 0, 0, 2, 0, 0x11, 0x77, 0x25];
        let sd = sd_body(&[entry], &option);
        let event = decode(&frame(
            &udp(30490, &someip_message(0xffff, 0x8100, &sd)),
            17,
            0x0800,
        ))
        .expect("SD message");
        let entry = &event.sd.expect("entries")[0];
        assert_eq!(entry.service, 0x1234);
        assert_eq!(entry.endpoints.len(), 1);
        assert_eq!(entry.endpoints[0].address, "10.0.0.2");
        assert_eq!(entry.endpoints[0].port, 30501);
        assert_eq!(entry.endpoints[0].protocol, 0x11);
    }

    #[test]
    fn an_sd_entry_reads_configuration_options() {
        let mut entry = [0u8; 16];
        entry[0] = 0x01;
        entry[1] = 0;
        entry[3] = 0x10;
        // Configuration option: 2-byte length (9), type 1, reserved, then
        // length-prefixed UTF-8 strings.
        let option = [0, 9, 0x01, 0, 3, b'a', b'b', b'c', 2, b'd', b'e', 0];
        let sd = sd_body(&[entry], &option);
        let event = decode(&frame(
            &udp(30490, &someip_message(0xffff, 0x8100, &sd)),
            17,
            0x0800,
        ))
        .expect("SD message");
        assert_eq!(
            event.sd.expect("entries")[0].configuration,
            ["abc".to_string(), "de".to_string()]
        );
    }

    #[test]
    fn someip_tp_reassembles_two_segments() {
        let mut tracker = SomeIpTp::default();
        let first = decode(&frame(
            &udp(30501, &tp_message(0x1234, 0x8001, 0, true, &[1; 16])),
            17,
            0x0800,
        ))
        .expect("first TP segment");
        assert!(tracker.feed(&first).is_none(), "first half waits");
        let second = decode(&frame(
            &udp(30501, &tp_message(0x1234, 0x8001, 16, false, &[2; 8])),
            17,
            0x0800,
        ))
        .expect("second TP segment");
        let full = tracker.feed(&second).expect("reassembled message");
        assert_eq!(full.message_type, 0x02);
        assert_eq!(full.payload.len(), 24);
        assert_eq!(&full.payload[..16], &[1; 16]);
        assert_eq!(&full.payload[16..], &[2; 8]);
    }

    #[test]
    fn ipv6_hop_by_hop_reaches_someip() {
        let message = someip_message(0x1234, 0x8001, &[7]);
        let mut udp = Vec::new();
        udp.extend_from_slice(&30501u16.to_be_bytes());
        udp.extend_from_slice(&30490u16.to_be_bytes());
        udp.extend_from_slice(&((8 + message.len()) as u16).to_be_bytes());
        udp.extend_from_slice(&0u16.to_be_bytes());
        udp.extend_from_slice(&message);

        let mut ip = vec![0u8; 40];
        ip[0] = 0x60;
        ip[4..6].copy_from_slice(&((8 + udp.len()) as u16).to_be_bytes());
        ip[6] = 0; // Hop-by-Hop Options
        ip[7] = 64;
        ip[8..24].copy_from_slice(&[0x20, 1, 0x0d, 0xb8, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1]);
        ip[24..40].copy_from_slice(&[0x20, 1, 0x0d, 0xb8, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 2]);
        // Hop-by-Hop: next header UDP, length 0 (8 bytes total).
        ip.extend_from_slice(&[17, 0, 0, 0, 0, 0, 0, 0]);
        ip.extend_from_slice(&udp);

        let frame = EthFrame {
            t_us: 10,
            bus: 0,
            dir: Direction::Rx,
            dst: [0; 6],
            src: [0; 6],
            vlan: None,
            vlan_tpid: None,
            ethertype: 0x86dd,
            payload: Arc::from(ip),
            seq: 0,
            truncated: false,
        };
        let event = decode(&frame).expect("SOME/IP behind Hop-by-Hop");
        assert_eq!(event.src, "2001:db8::1");
        assert_eq!(event.service, 0x1234);
    }

    fn ipv4_fragment(id: u16, offset_units: u16, more: bool, data: &[u8]) -> EthFrame {
        let mut ip = vec![0u8; 20];
        ip[0] = 0x45;
        ip[2..4].copy_from_slice(&((20 + data.len()) as u16).to_be_bytes());
        ip[4..6].copy_from_slice(&id.to_be_bytes());
        ip[6..8].copy_from_slice(
            &((offset_units & 0x1fff) | if more { 0x2000 } else { 0 }).to_be_bytes(),
        );
        ip[8] = 64;
        ip[9] = 17;
        ip[12..16].copy_from_slice(&[10, 0, 0, 1]);
        ip[16..20].copy_from_slice(&[10, 0, 0, 2]);
        ip.extend_from_slice(data);
        EthFrame {
            t_us: 10,
            bus: 0,
            dir: Direction::Rx,
            dst: [0; 6],
            src: [0; 6],
            vlan: None,
            vlan_tpid: None,
            ethertype: 0x0800,
            payload: Arc::from(ip),
            seq: 0,
            truncated: false,
        }
    }

    #[test]
    fn ipv4_fragments_reassemble_before_someip() {
        let message = someip_message(0x1234, 0x8001, &[7; 32]);
        let mut udp = Vec::new();
        udp.extend_from_slice(&30501u16.to_be_bytes());
        udp.extend_from_slice(&30490u16.to_be_bytes());
        udp.extend_from_slice(&((8 + message.len()) as u16).to_be_bytes());
        udp.extend_from_slice(&0u16.to_be_bytes());
        udp.extend_from_slice(&message);

        let mut tracker = SomeIpIp::default();
        let first = ipv4_fragment(7, 0, true, &udp[..24]);
        assert!(tracker.feed(&first).is_none(), "first fragment waits");
        let second = ipv4_fragment(7, 3, false, &udp[24..]);
        let reassembled = tracker.feed(&second).expect("both fragments complete");
        let event = decode(&reassembled).expect("SOME/IP after IP reassembly");
        assert_eq!(event.service, 0x1234);
        assert_eq!(event.payload, [7; 32]);
    }

    fn ipv6_fragment(id: u32, offset_units: u16, more: bool, data: &[u8]) -> EthFrame {
        let mut ip = vec![0u8; 40];
        ip[0] = 0x60;
        ip[4..6].copy_from_slice(&((8 + data.len()) as u16).to_be_bytes());
        ip[6] = 44; // Fragment
        ip[7] = 64;
        ip[8..24].copy_from_slice(&[0x20, 1, 0x0d, 0xb8, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1]);
        ip[24..40].copy_from_slice(&[0x20, 1, 0x0d, 0xb8, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 2]);
        ip.extend_from_slice(&[17, 0]);
        ip.extend_from_slice(
            &(((offset_units << 3) & 0xfff8) | if more { 1 } else { 0 }).to_be_bytes(),
        );
        ip.extend_from_slice(&id.to_be_bytes());
        ip.extend_from_slice(data);
        EthFrame {
            t_us: 10,
            bus: 0,
            dir: Direction::Rx,
            dst: [0; 6],
            src: [0; 6],
            vlan: None,
            vlan_tpid: None,
            ethertype: 0x86dd,
            payload: Arc::from(ip),
            seq: 0,
            truncated: false,
        }
    }

    #[test]
    fn ipv6_fragments_reassemble_before_someip() {
        let message = someip_message(0x1234, 0x8001, &[8; 32]);
        let mut udp = Vec::new();
        udp.extend_from_slice(&30501u16.to_be_bytes());
        udp.extend_from_slice(&30490u16.to_be_bytes());
        udp.extend_from_slice(&((8 + message.len()) as u16).to_be_bytes());
        udp.extend_from_slice(&0u16.to_be_bytes());
        udp.extend_from_slice(&message);

        let mut tracker = SomeIpIp::default();
        assert!(
            tracker
                .feed(&ipv6_fragment(9, 0, true, &udp[..24]))
                .is_none()
        );
        let full = tracker
            .feed(&ipv6_fragment(9, 3, false, &udp[24..]))
            .expect("IPv6 fragments complete");
        let event = decode(&full).expect("SOME/IP after IPv6 reassembly");
        assert_eq!(event.service, 0x1234);
        assert_eq!(event.payload, [8; 32]);
    }

    fn tcp(seq: u32, flags: u8, payload: &[u8]) -> Vec<u8> {
        let mut tcp = Vec::new();
        tcp.extend_from_slice(&30501u16.to_be_bytes());
        tcp.extend_from_slice(&30490u16.to_be_bytes());
        tcp.extend_from_slice(&seq.to_be_bytes());
        tcp.extend_from_slice(&0u32.to_be_bytes());
        tcp.push(5 << 4);
        tcp.push(flags);
        tcp.extend_from_slice(&0u16.to_be_bytes());
        tcp.extend_from_slice(&0u16.to_be_bytes());
        tcp.extend_from_slice(&0u16.to_be_bytes());
        tcp.extend_from_slice(payload);
        tcp
    }

    #[test]
    fn tcp_reassembles_two_messages_across_segments() {
        let first = someip_message(0x1234, 0x0001, &[1, 2, 3]);
        let second = someip_message(0x1234, 0x0002, &[4, 5]);
        let mut tracker = SomeIpTcp::default();
        let one = frame(&tcp(100, 0x18, &first[..12]), 6, 0x0800);
        assert!(tracker.feed(&one).is_empty(), "first half waits");
        let mut rest = first[12..].to_vec();
        rest.extend_from_slice(&second);
        let events = tracker.feed(&frame(&tcp(112, 0x18, &rest), 6, 0x0800));
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].method, 0x0001);
        assert_eq!(events[1].method, 0x0002);
        assert_eq!(events[0].transport, "TCP");
    }

    #[test]
    fn a_length_mismatch_is_not_someip() {
        let mut message = someip_message(0x1234, 0x8001, &[]);
        message[7] = 99;
        assert!(decode(&frame(&udp(30501, &message), 17, 0x0800)).is_none());
    }
}
