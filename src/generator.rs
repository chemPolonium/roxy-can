//! The interactive generator's message model: base payload, per-signal value
//! sources, and the payload assembly that drives a frame out.

use crate::can::frame::{dlc2len, len2dlc, FrameFlags, MAX_CAN_FD_LEN};
use crate::channel::Channel;
use crate::sim::{eval_phys, ValueSrc};

pub struct TxMsg {
    pub channel: u8,
    pub id: u32,
    pub extended: bool,
    pub name: String,
    /// DBC node this message belongs to, i.e. its transmitter. Empty when the
    /// database assigns no node. Derived by `add_tx` like `extended`, so it is
    /// not saved in the project file.
    pub node: String,
    pub len: u8,
    pub data: [u8; MAX_CAN_FD_LEN],
    pub flags: FrameFlags,
    pub data_text: String,
    pub cycle_us: u64,
    pub active: bool,
    pub next_t_us: u64,
    /// Signals whose value is generated over time rather than held at whatever
    /// `data` says. Applied on top of the base payload at emit time; `data`
    /// itself is never rewritten by them. See [`crate::sim`].
    pub srcs: Vec<ValueSrc>,
}

/// Whitespace-separated hex bytes, as typed in the generator's data box.
/// Returns None on an empty, over-long or non-hex string.
pub(crate) fn parse_hex_bytes(s: &str) -> Option<Vec<u8>> {
    let toks: Vec<&str> = s.split_whitespace().collect();
    if toks.is_empty() || toks.len() > MAX_CAN_FD_LEN {
        return None;
    }
    let mut out = Vec::with_capacity(toks.len());
    for t in toks {
        out.push(u8::from_str_radix(t, 16).ok()?);
    }
    Some(out)
}

pub(crate) fn hex_text(data: &[u8; MAX_CAN_FD_LEN], len: u8) -> String {
    data[..len.min(MAX_CAN_FD_LEN as u8) as usize]
        .iter()
        .map(|b| format!("{b:02X}"))
        .collect::<Vec<_>>()
        .join(" ")
}

/// Ceiling of the period the cycle dialog accepts, in milliseconds.
pub const TX_CYCLE_MAX_MS: u64 = 60_000;

/// The cycle dialog's draft text as microseconds. Whole milliseconds only, and
/// 0 stays meaningful: it is an event-triggered message. Anything that does not
/// read as a period -- half-deleted, fractional, out of range -- is refused so
/// the dialog can disable its confirm button rather than guess one.
pub fn cycle_from_ms_text(s: &str) -> Option<u64> {
    let ms: u64 = s.trim().parse().ok()?;
    (ms <= TX_CYCLE_MAX_MS).then_some(ms * 1_000)
}

/// Payload for one generated frame: `tx`'s base bytes with every driven signal
/// overwritten by its value at `at_us`. Never mutates `tx`, so `data` and
/// `len` stay whatever the user set and a save captures the base rather than
/// wherever the waveform happened to be.
///
/// Takes `channels` explicitly rather than `&self` so the generator can call it
/// while holding a mutable borrow of `tx_list`.
pub(crate) fn tx_payload(
    channels: &[Channel],
    tx: &TxMsg,
    at_us: u64,
) -> ([u8; MAX_CAN_FD_LEN], u8, FrameFlags) {
    let mut data = tx.data;
    let mut len = tx.len;
    let mut flags = tx.flags;
    let Some((table, msg)) = channels
        .get(tx.channel as usize)
        .and_then(|c| c.dbc.as_ref())
        .and_then(|db| db.messages.get(&tx.id).map(|m| (db, m)))
    else {
        return (data, len, flags);
    };
    for src in &tx.srcs {
        let Some(s) = msg.signals.iter().find(|s| s.name == src.name) else {
            continue;
        };
        // The whole 64-byte array goes in: encode_signal skips bits that fall
        // outside its argument, so a narrowed slice would silently drop them.
        if !table.encode_signal(tx.id, &src.name, eval_phys(src, at_us), &mut data) {
            continue;
        }
        // A driven signal reaching past the base length must widen the frame,
        // or decoders -- including our own plots -- read the value as zero.
        let needed = ((s.start_bit + s.size) as usize)
            .div_ceil(8)
            .min(MAX_CAN_FD_LEN);
        len = len.max(needed as u8);
    }
    if len > 8 {
        // Snap to a length a real FD frame can carry, and say so.
        len = dlc2len(len2dlc(len));
        flags = flags.union(FrameFlags::FD);
    }
    (data, len, flags)
}


