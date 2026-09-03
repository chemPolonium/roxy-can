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
    /// The bytes on the wire as of the last throttled text refresh -- the base
    /// payload with every driven source's value laid over it. The generator
    /// row renders this read-only; session state, never saved.
    pub(crate) sent_text: String,
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

use crate::app::{App, DEFAULT_TX_CYCLE_US};

impl App {
    /// Enables or disables every generator message of one bus; freshly
    /// enabled messages restart their cycle immediately.
    pub fn set_bus_tx(&mut self, ch: u8, on: bool) {
        for i in 0..self.tx_list.len() {
            if self.tx_list[i].channel == ch && self.tx_list[i].active != on {
                self.set_tx_active(i, on);
            }
        }
    }

    /// Ticks one entry's On checkbox. The anchoring semantics live with the
    /// command (`SetEntryActive`); this wrapper keeps the index-based call
    /// sites and tests working.
    pub fn set_tx_active(&mut self, i: usize, on: bool) {
        let Some(tx) = self.tx_list.get(i) else {
            return;
        };
        let (ch, id) = (tx.channel, tx.id);
        self.send(crate::bus::BusCommand::SetEntryActive { ch, id, on });
    }

    /// Ticks or unticks a DBC node as one this tool transmits as.
    ///
    /// Ticking adds whatever generator entry the node is missing and switches
    /// them on. The period of an entry that already exists is never rewritten,
    /// so a value tuned by hand outlives the click. Unticking only stops
    /// sending: entries keep their payload and waveforms, so ticking the node
    /// again restores it exactly as it was.
    pub fn set_node_sim(&mut self, channel: u8, node: &str, on: bool) {
        // `""` is what the parser writes for "no transmitter assigned", and
        // `node_tx_ids` matches it against every unassigned message at once.
        if node.is_empty() || channel as usize >= self.channels.len() {
            return;
        }
        // The tick is recorded first and unconditionally: a node that sends
        // nothing still has to remember that we mean to be it.
        let list = &mut self.channels[channel as usize].sim_nodes;
        if on {
            if !list.iter().any(|n| n == node) {
                list.push(node.to_string());
            }
        } else {
            list.retain(|n| n != node);
        }

        // Membership comes from the live database, not from each entry's
        // stamped `node`: loading another DBC does not rebuild the generator,
        // so a stamp can name a message this node no longer owns.
        let ids = self
            .channel_dbc(channel)
            .map(|db| db.node_tx_ids(node))
            .unwrap_or_default();
        if on {
            for id in &ids {
                self.add_tx(channel, *id);
            }
            for i in 0..self.tx_list.len() {
                if self.tx_list[i].channel == channel
                    && ids.contains(&self.tx_list[i].id)
                    && !self.tx_list[i].active
                {
                    self.set_tx_active(i, true);
                }
            }
        } else {
            // The stamped name is included on the way out only, so unchecking
            // still silences a node whose database has since been swapped or
            // unloaded. "I unchecked it and it is still transmitting" is the
            // one outcome a user cannot recover from by guessing.
            for t in &mut self.tx_list {
                if t.channel == channel && t.active && (ids.contains(&t.id) || t.node == node) {
                    t.active = false;
                }
            }
        }
        let bus = self.channel_name(channel);
        self.status = if on {
            format!("simulating {node} on {bus} ({} message(s))", ids.len())
        } else {
            format!("{node} stopped on {bus}")
        };
    }

    /// Whether this bus was told to transmit as `node`.
    pub fn is_node_simulated(&self, ch: u8, node: &str) -> bool {
        self.channels
            .get(ch as usize)
            .is_some_and(|c| c.sim_nodes.iter().any(|n| n == node))
    }

    pub fn add_tx(&mut self, channel: u8, id: u32) {
        if self
            .tx_list
            .iter()
            .any(|t| t.channel == channel && t.id == id)
        {
            return;
        }
        let (name, node, len, cycle_us) = self
            .channel_dbc(channel)
            .and_then(|db| db.messages.get(&id))
            .map(|m| {
                (
                    m.name.clone(),
                    m.transmitter.clone(),
                    m.dlc.min(MAX_CAN_FD_LEN as u64) as u8,
                    // A declared 0 is event-triggered, so `unwrap_or` rather
                    // than `unwrap_or_default` on the Option: only "the DBC
                    // said nothing" gets our invented period.
                    m.cycle_us.unwrap_or(DEFAULT_TX_CYCLE_US),
                )
            })
            .unwrap_or_else(|| (format!("{id:X}"), String::new(), 8, DEFAULT_TX_CYCLE_US));
        let data_text = vec!["00"; len as usize].join(" ");
        self.tx_list.push(TxMsg {
            channel,
            id,
            srcs: Vec::new(),
            extended: id > 0x7FF,
            name,
            node,
            len,
            data: [0; MAX_CAN_FD_LEN],
            flags: if len > 8 {
                FrameFlags::FD
            } else {
                FrameFlags::NONE
            },
            data_text,
            cycle_us,
            active: false,
            next_t_us: 0,
            sent_text: String::new(),
        });
    }

    /// Adds or replaces the source driving `src.name` on generator `i`.
    pub fn set_source(&mut self, i: usize, src: ValueSrc) {
        let Some(tx) = self.tx_list.get_mut(i) else {
            return;
        };
        match tx.srcs.iter_mut().find(|s| s.name == src.name) {
            Some(held) => *held = src,
            None => tx.srcs.push(src),
        }
    }

    /// Stops driving `name`, which leaves the base bytes in charge again.
    pub fn clear_source(&mut self, i: usize, name: &str) {
        if let Some(tx) = self.tx_list.get_mut(i) {
            tx.srcs.retain(|s| s.name != name);
        }
    }

    /// Writes a physical value into the base payload and pins that signal by
    /// dropping only its source: grabbing a moving slider means "hold here".
    pub fn pin_signal(&mut self, i: usize, name: &str, phys: f64) -> bool {
        let Some(tx) = self.tx_list.get(i) else {
            return false;
        };
        let (channel, id, mut data) = (tx.channel, tx.id, tx.data);
        let Some(table) = self.channel_dbc(channel) else {
            return false;
        };
        if !table.encode_signal(id, name, phys, &mut data) {
            return false;
        }
        let msg_size = table
            .messages
            .get(&id)
            .map(|m| m.dlc.min(MAX_CAN_FD_LEN as u64) as u8)
            .unwrap_or(0);
        let tx = &mut self.tx_list[i];
        tx.srcs.retain(|s| s.name != name);
        let len = tx.len.max(msg_size);
        set_tx_base(tx, data, len);
        true
    }

    /// Replaces the base payload from the generator's hex box. Active sources
    /// deliberately survive: correcting one byte must not throw away a whole
    /// stimulus setup. Returns false if the text is not whole hex bytes.
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
