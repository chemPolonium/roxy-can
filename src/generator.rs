//! The interactive generator's message model: base payload, per-signal value
//! sources, and the payload assembly that drives a frame out.

use crate::can::frame::{FrameFlags, MAX_CAN_FD_LEN, dlc2len, len2dlc};
use crate::channel::Channel;
use crate::sim::{ValueSrc, eval_phys};

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

/// Encodes `value` into `data` for signal `name` of message `id` and widens
/// `len` to fit, snapping past-8 lengths to an FD length exactly like
/// [`tx_payload`] does. Returns false -- leaving payload and length alone --
/// when the message has no such signal or the value is not representable.
pub(crate) fn encode_mirror(
    table: &crate::dbc::SymbolTable,
    id: u32,
    name: &str,
    value: f64,
    data: &mut [u8; MAX_CAN_FD_LEN],
    len: &mut u8,
    flags: &mut FrameFlags,
) -> bool {
    let Some(s) = table
        .messages
        .get(&id)
        .and_then(|m| m.signals.iter().find(|s| s.name == name))
    else {
        return false;
    };
    if !table.encode_signal(id, name, value, data) {
        return false;
    }
    let needed = ((s.start_bit + s.size) as usize)
        .div_ceil(8)
        .min(MAX_CAN_FD_LEN);
    *len = (*len).max(needed as u8);
    if *len > 8 {
        *len = dlc2len(len2dlc(*len));
        *flags = flags.union(FrameFlags::FD);
    }
    true
}

use crate::app::App;

impl App {
    /// Enables or disables every generator message of one bus; freshly
    /// enabled messages restart their cycle immediately.
    pub fn set_bus_tx(&mut self, ch: u8, on: bool) {
        self.send(crate::bus::BusCommand::SetBusTx { ch, on });
    }

    /// Ticks one entry's On checkbox. The anchoring semantics live with the
    /// command (`SetEntryActive`); this wrapper keeps the index-based call
    /// sites and tests working.
    #[cfg(test)]
    pub fn set_tx_active(&mut self, i: usize, on: bool) {
        let Some(tx) = self.tx_list.get(i) else {
            return;
        };
        let (ch, id) = (tx.channel, tx.id);
        self.send(crate::bus::BusCommand::SetEntryActive { ch, id, on });
    }

    /// Ticks or unticks a DBC node as one this tool transmits as. The
    /// whole membership/activation semantics live with the command.
    pub fn set_node_sim(&mut self, channel: u8, node: &str, on: bool) {
        // `""` is what the parser writes for "no transmitter assigned", and
        // `node_tx_ids` matches it against every unassigned message at once.
        if node.is_empty() || channel as usize >= self.snap.channel_count {
            return;
        }
        self.send(crate::bus::BusCommand::SetNodeSim {
            ch: channel,
            node: node.to_string(),
            on,
        });
    }

    /// Whether this bus was told to transmit as `node`.
    pub fn is_node_simulated(&self, ch: u8, node: &str) -> bool {
        self.snap
            .channels
            .get(ch as usize)
            .is_some_and(|c| c.sim_nodes.iter().any(|n| n == node))
    }

    /// Adds the generator entry unless it exists (command `AddEntry`).
    pub fn add_tx(&mut self, channel: u8, id: u32) {
        self.send(crate::bus::BusCommand::AddEntry { ch: channel, id });
    }

    /// Adds or replaces the source driving `src.name` on generator `i`.
    pub fn set_source(&mut self, i: usize, src: ValueSrc) {
        let Some(tx) = self.snap.tx.get(i) else {
            return;
        };
        let (ch, id) = (tx.channel, tx.id);
        self.send(crate::bus::BusCommand::SetEntrySource { ch, id, src });
    }

    /// Stops driving `name`, which leaves the base bytes in charge again.
    /// Test convenience: the UI sends [`crate::bus::BusCommand::ClearEntrySource`].
    #[cfg(test)]
    pub fn clear_source(&mut self, i: usize, name: &str) {
        let Some(tx) = self.tx_list.get(i) else {
            return;
        };
        let (ch, id) = (tx.channel, tx.id);
        self.send(crate::bus::BusCommand::ClearEntrySource {
            ch,
            id,
            name: name.to_string(),
        });
    }

    /// Writes a physical value into the base payload and pins that signal by
    /// dropping only its source: grabbing a moving slider means "hold here".
    /// The encode is validated read-only first so the command is only sent
    /// when it will succeed; the bus re-checks authoritatively.
    /// Test convenience: the UI sends [`crate::bus::BusCommand::PinEntrySignal`].
    #[cfg(test)]
    pub fn pin_signal(&mut self, i: usize, name: &str, phys: f64) -> bool {
        let Some(tx) = self.tx_list.get(i) else {
            return false;
        };
        let (ch, id) = (tx.channel, tx.id);
        let mut probe = tx.data;
        let encodable = self
            .channel_dbc(ch)
            .is_some_and(|table| table.encode_signal(id, name, phys, &mut probe));
        if encodable {
            self.send(crate::bus::BusCommand::PinEntrySignal {
                ch,
                id,
                name: name.to_string(),
                phys,
            });
        }
        encodable
    }

    /// Replaces the base payload from the generator's hex box. Active sources
    /// deliberately survive: correcting one byte must not throw away a whole
    /// stimulus setup. Returns false if the text is not whole hex bytes.
    /// Test convenience: the UI sends [`crate::bus::BusCommand::SetEntryHex`].
    #[cfg(test)]
    pub fn set_tx_hex(&mut self, i: usize, text: &str) -> bool {
        let parsed = parse_hex_bytes(text).is_some();
        if let Some(tx) = self.tx_list.get(i) {
            let (ch, id) = (tx.channel, tx.id);
            self.send(crate::bus::BusCommand::SetEntryHex {
                ch,
                id,
                text: text.to_string(),
            });
        }
        parsed
    }
}

/// Installs base bytes and keeps length, the FD flag and the hex text in
/// step with them.
pub(crate) fn set_tx_base(tx: &mut TxMsg, data: [u8; MAX_CAN_FD_LEN], len: u8) {
    let len = len.min(MAX_CAN_FD_LEN as u8);
    tx.data = data;
    tx.len = len;
    if len > 8 {
        tx.flags = tx.flags.union(FrameFlags::FD);
    }
    tx.data_text = hex_text(&data, len);
}
