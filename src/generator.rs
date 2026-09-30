//! The interactive generator's message model: base payload, per-signal value
//! sources, and the payload assembly that drives a frame out.

use crate::can::frame::{FrameFlags, MAX_CAN_FD_LEN, dlc2len, len2dlc};
use crate::channel::{Channel, NodeRole};
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
    /// 最近一次发射的载荷与长度：发射 slot 时计算并存储，快照直接读
    /// 它——显示值随消息周期跳变，而非逐帧重算。`last_len == 0` 表示
    /// 尚未发射过（显示层回退到 base）。
    pub last_sent: [u8; MAX_CAN_FD_LEN],
    pub last_len: u8,
}

/// One FlexRay generator entry: the slot it fills, the bytes it puts there and
/// the schedule it emits on. The twin of [`TxMsg`] for a bus whose frames are
/// addressed by slot rather than id, and deliberately without the CAN entry's
/// `fd`/`extended` fields.
///
/// `node` is the ECU the description names as the frame's sender: an entry is
/// edited under that node in the Network window, exactly like a CAN entry under
/// its DBC transmitter. It is derived at add time and never typed, and there is
/// no entry without it -- which node owns which frame is the document's to say,
/// and a generator that invented one would be papering over a description that
/// does not answer the question.
pub struct FrTxMsg {
    pub bus: u8,
    pub slot: u16,
    /// The frame name the bus's description gives this slot, for the row header.
    pub name: String,
    /// The ECU declared as this frame's sender; what the row is grouped under.
    pub node: String,
    /// Payload length in bytes; the base buffer is exactly this long.
    pub len: usize,
    pub data: Vec<u8>,
    pub data_text: String,
    /// Emission period in µs. 0 is meaningful: the frame goes out only when the
    /// row's "Send now" is pressed.
    pub cycle_us: u64,
    pub active: bool,
    pub next_t_us: u64,
    /// Signals generated over time rather than held at what `data` says, applied
    /// on top of the base payload at emit time -- see [`crate::sim`].
    pub srcs: Vec<ValueSrc>,
    /// The payload of the last frame that went out, for the row's readout.
    /// Empty until the first emission.
    pub last_sent: Vec<u8>,
}

/// Whitespace-separated hex bytes, as typed in the generator's data box.
/// Returns None on an empty, over-long or non-hex string.
pub(crate) fn parse_hex_bytes(s: &str) -> Option<Vec<u8>> {
    parse_hex_limited(s, MAX_CAN_FD_LEN)
}

/// The same box for a payload of any length: a FlexRay slot carries up to 254
/// bytes, so the cap travels with the caller rather than the parser.
pub(crate) fn parse_hex_limited(s: &str, max: usize) -> Option<Vec<u8>> {
    let toks: Vec<&str> = s.split_whitespace().collect();
    if toks.is_empty() || toks.len() > max {
        return None;
    }
    let mut out = Vec::with_capacity(toks.len());
    for t in toks {
        out.push(u8::from_str_radix(t, 16).ok()?);
    }
    Some(out)
}

pub(crate) fn hex_text(data: &[u8; MAX_CAN_FD_LEN], len: u8) -> String {
    hex_of(&data[..len.min(MAX_CAN_FD_LEN as u8) as usize])
}

/// The generator's display form of any payload.
pub(crate) fn hex_of(data: &[u8]) -> String {
    data.iter()
        .map(|b| format!("{b:02X}"))
        .collect::<Vec<_>>()
        .join(" ")
}

/// How many bytes a payload's hex text holds -- the length a row shows when the
/// schedule does not state one.
pub fn hex_len_of(text: &str) -> usize {
    text.split_whitespace().count()
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

/// Which cycle count a period means on this cluster: `None` when it is not a
/// whole number of cycles, which is what keeps a row honest about a period it
/// inherited from a project whose cycle time has since changed.
pub fn cycles_of(period_us: u64, cycle_us: u64) -> Option<u64> {
    (cycle_us > 0 && period_us > 0 && period_us.is_multiple_of(cycle_us))
        .then(|| period_us / cycle_us)
}

/// The FlexRay cycle dialog's parse: a count of **cluster cycles**, at least one,
/// and the period that makes. A static slot has no event-triggered mode -- the
/// schedule decides when it transmits -- so 0 is refused with the reason rather
/// than silently meaning "never" behind CAN's word for it. `Err` carries the
/// sentence the dialog prints in place of the confirmation.
pub fn fr_cycle_draft(text: &str, cycle_us: u64) -> Result<(u64, u64), &'static str> {
    if cycle_us == 0 {
        return Err("这一路没有声明周期时间：没有可数的周期");
    }
    let Ok(n) = text.trim().parse::<u64>() else {
        return Err("整数周期数");
    };
    if n == 0 {
        return Err("静态槽没有事件触发：至少 1 个周期；要它不发，用行上的 On 勾选框");
    }
    let max = TX_CYCLE_MAX_MS * 1_000 / cycle_us;
    if n > max {
        return Err("周期数太大");
    }
    Ok((n, n * cycle_us))
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
        .and_then(|db| db.message_of(tx.id).map(|m| (db, m)))
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
        .message_of(id)
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

    /// Declares the role of a DBC node on this bus. The whole
    /// membership/activation semantics live with the command.
    pub fn set_node_role(&mut self, channel: u8, node: &str, role: NodeRole) {
        // `""` is what the parser writes for "no transmitter assigned", and
        // `node_tx_ids` matches it against every unassigned message at once.
        if node.is_empty() || channel as usize >= self.snap.channel_count {
            return;
        }
        self.send(crate::bus::BusCommand::SetNodeRole {
            ch: channel,
            node: node.to_string(),
            role,
        });
    }

    /// Simulates every DBC node on the bus (CANoe's "Switch All Blocks to
    /// Simulation").
    #[cfg(test)]
    pub fn simulate_all_nodes(&mut self, ch: u8) {
        let Some(db) = self.channel_dbc(ch) else {
            return;
        };
        let names: Vec<String> = db.nodes.clone();
        for name in names {
            self.set_node_role(ch, &name, NodeRole::Simulated);
        }
    }

    /// Takes every DBC node on the bus out of the simulation.
    #[cfg(test)]
    pub fn stop_all_nodes(&mut self, ch: u8) {
        let Some(db) = self.channel_dbc(ch) else {
            return;
        };
        let names: Vec<String> = db.nodes.clone();
        for name in names {
            self.set_node_role(ch, &name, NodeRole::Absent);
        }
    }

    /// The role declared for `node` on this bus; nodes without a
    /// declaration are `Absent`.
    pub fn node_role(&self, ch: u8, node: &str) -> NodeRole {
        self.snap
            .channels
            .get(ch as usize)
            .map(|c| c.role_of(node))
            .unwrap_or(NodeRole::Absent)
    }

    /// Adds a replay block: recorded traffic from `path`, filtered,
    /// injected onto this bus when a simulation runs. New blocks start
    /// disabled; the whole membership/activation semantics live with the
    /// command. `attached` names the DBC node the block belongs to.
    pub fn add_replay_block(
        &mut self,
        channel: u8,
        name: String,
        path: String,
        node_filter: Option<String>,
        attached: Option<(u8, String)>,
    ) {
        if channel as usize >= self.snap.channel_count {
            return;
        }
        self.send(crate::bus::BusCommand::AddReplayBlock {
            name,
            channel,
            path,
            node_filter,
            attached,
            ids: Vec::new(),
        });
    }

    /// Ticks a replay block. Enabling loads its queue from the log right
    /// away, so a broken path surfaces in the status line.
    pub fn set_replay_block_enabled(&mut self, id: u64, on: bool) {
        self.send(crate::bus::BusCommand::SetReplayBlockEnabled { id, on });
    }

    /// Removes a replay block wholesale.
    pub fn remove_replay_block(&mut self, id: u64) {
        self.send(crate::bus::BusCommand::RemoveReplayBlock { id });
    }

    /// Sets the record filter: only the CAN frames and FlexRay slots it names
    /// land in the recorded file; both halves empty records everything.
    pub fn set_record_filter(&mut self, filter: crate::recorder::RecordFilter) {
        self.send(crate::bus::BusCommand::SetRecordFilter { filter });
    }

    /// Sets the trace ring's retention in frames.
    pub fn set_trace_limit(&mut self, frames: usize) {
        self.send(crate::bus::BusCommand::SetTraceLimit { frames });
    }

    /// Sets the trigger-recording context sizes. Any subset may be set;
    /// the core clamps all three.
    pub fn set_run_limits(&mut self, pre_frames: usize, post_frames: u32, marker_cap: usize) {
        self.send(crate::bus::BusCommand::SetRunLimits {
            pre_frames: Some(pre_frames),
            post_frames: Some(post_frames),
            marker_cap: Some(marker_cap),
        });
    }

    /// Attaches a hardware adapter to a bus. `fd_data_kbps` opts the
    /// channel into CAN FD (data-phase preset must exist for it).
    pub fn set_hardware_channel(
        &mut self,
        bus: u8,
        driver: crate::hw::HwDriver,
        adapter: i32,
        kbps: u32,
        fd_data_kbps: Option<u32>,
    ) {
        self.send(crate::bus::BusCommand::SetHardwareChannel {
            bus,
            driver,
            adapter,
            kbps,
            fd_data_kbps,
        });
    }

    /// Detaches a bus's hardware adapter.
    pub fn detach_hardware(&mut self, bus: u8) {
        self.send(crate::bus::BusCommand::DetachHardware { bus });
    }

    /// Attaches the FlexRay RX-only watch to a Vector channel, configured
    /// from the FIBEX description file. `None` detaches.
    pub fn set_fr_watch(&mut self, bus: u8, channel_index: Option<i32>, fibex_path: &str) {
        self.send(crate::bus::BusCommand::SetFrWatch {
            bus,
            channel_index,
            fibex_path: fibex_path.to_string(),
        });
    }

    /// Adds the generator entry unless it exists (command `AddEntry`).
    pub fn add_tx(&mut self, channel: u8, id: u32) {
        self.send(crate::bus::BusCommand::AddEntry { ch: channel, id });
    }

    /// Adds the FlexRay generator entry `(bus, slot)` unless it exists. The
    /// entry belongs to the ECU the description names as that slot's frame's
    /// sender, so a slot whose sender the document does not state gets no entry
    /// and the status line says which fact is missing: which node sends which
    /// frame is the FIBEX/ARXML's to declare, and an owner invented here would
    /// be a guess the Network tree then presents as fact.
    pub fn add_fr_tx(&mut self, bus: u8, slot: u16) {
        let label = self.fr_bus_label(bus);
        let Some(db) = self.fr_db(bus) else {
            self.status = format!("{label} 没有集群描述：条目按描述声明的发送者归属，先加载描述");
            return;
        };
        let Some(ix) = db.frame_ix_of_slot(slot) else {
            self.status = format!("{label} 的描述未调度 slot {slot}：没有帧可归属，加不了条目");
            return;
        };
        let Some(node) = db.frame_sender(ix) else {
            self.status =
                format!("{label} 的描述没有说明 slot {slot} 的帧由哪个 ECU 发送：加不了条目");
            return;
        };
        let node = node.to_string();
        self.send(crate::bus::BusCommand::AddFrEntry { bus, slot, node });
    }

    /// Replaces a FlexRay entry's base payload from hex text.
    /// Test convenience: the UI sends [`crate::bus::BusCommand::SetFrEntryHex`].
    #[cfg(test)]
    pub fn set_fr_tx_hex(&mut self, bus: u8, slot: u16, text: &str) {
        self.send(crate::bus::BusCommand::SetFrEntryHex {
            bus,
            slot,
            text: text.to_string(),
        });
    }

    /// Ticks one FlexRay entry's On checkbox. Test convenience: the UI sends
    /// [`crate::bus::BusCommand::SetFrEntryActive`].
    #[cfg(test)]
    pub fn set_fr_tx_active(&mut self, bus: u8, slot: u16, on: bool) {
        self.send(crate::bus::BusCommand::SetFrEntryActive { bus, slot, on });
    }

    /// Sets a FlexRay entry's send period in µs (0 = only on demand). Test
    /// convenience: the UI sends [`crate::bus::BusCommand::SetFrEntryCycle`].
    #[cfg(test)]
    pub fn set_fr_tx_cycle(&mut self, bus: u8, slot: u16, cycle_us: u64) {
        self.send(crate::bus::BusCommand::SetFrEntryCycle {
            bus,
            slot,
            cycle_us,
        });
    }

    /// Adds or replaces the source driving `src.name` on generator `i`.
    pub fn set_source(&mut self, i: usize, src: ValueSrc) {
        let Some(tx) = self.snap.tx.get(i) else {
            return;
        };
        let (ch, id) = (tx.channel, tx.id);
        self.send(crate::bus::BusCommand::SetEntrySource { ch, id, src });
    }

    /// The source driving `sig` on this generator row, from this frame's
    /// snapshot. `None` means the base bytes are in charge of that signal.
    pub fn gen_source(&self, row: crate::app::GenRow, sig: &str) -> Option<ValueSrc> {
        let srcs = match row {
            crate::app::GenRow::Can(i) => self.snap.tx.get(i).map(|t| t.srcs.clone()),
            crate::app::GenRow::Fr(i) => self.snap.fr_tx.get(i).map(|t| t.srcs.clone()),
        };
        srcs?
            .into_iter()
            .find(|s| s.name == sig)
    }

    /// Applies a drafted source to whichever row it belongs to.
    pub fn set_gen_source(&mut self, row: crate::app::GenRow, src: ValueSrc) {
        match row {
            crate::app::GenRow::Can(i) => self.set_source(i, src),
            crate::app::GenRow::Fr(i) => {
                let Some(tx) = self.snap.fr_tx.get(i) else {
                    return;
                };
                let (bus, slot) = (tx.bus, tx.slot);
                self.send(crate::bus::BusCommand::SetFrEntrySource { bus, slot, src });
            }
        }
    }

    /// Stops driving `sig`; the base bytes take over again.
    pub fn clear_gen_source(&mut self, row: crate::app::GenRow, sig: &str) {
        match row {
            crate::app::GenRow::Can(i) => {
                let Some(tx) = self.snap.tx.get(i) else {
                    return;
                };
                let (ch, id) = (tx.channel, tx.id);
                self.send(crate::bus::BusCommand::ClearEntrySource {
                    ch,
                    id,
                    name: sig.to_string(),
                });
            }
            crate::app::GenRow::Fr(i) => {
                let Some(tx) = self.snap.fr_tx.get(i) else {
                    return;
                };
                let (bus, slot) = (tx.bus, tx.slot);
                self.send(crate::bus::BusCommand::ClearFrEntrySource {
                    bus,
                    slot,
                    name: sig.to_string(),
                });
            }
        }
    }

    /// Writes a physical value into the row's base payload and drops only that
    /// signal's source. Whether the database can encode it is the bus's call; a
    /// failed pin simply changes nothing.
    pub fn pin_gen_signal(&mut self, row: crate::app::GenRow, sig: &str, phys: f64) {
        match row {
            crate::app::GenRow::Can(i) => {
                let Some(tx) = self.snap.tx.get(i) else {
                    return;
                };
                let (ch, id) = (tx.channel, tx.id);
                self.send(crate::bus::BusCommand::PinEntrySignal {
                    ch,
                    id,
                    name: sig.to_string(),
                    phys,
                });
            }
            crate::app::GenRow::Fr(i) => {
                let Some(tx) = self.snap.fr_tx.get(i) else {
                    return;
                };
                let (bus, slot) = (tx.bus, tx.slot);
                self.send(crate::bus::BusCommand::PinFrEntrySignal {
                    bus,
                    slot,
                    name: sig.to_string(),
                    phys,
                });
            }
        }
    }

    /// A generator row's title and the bus it lives on, for the dialogs that
    /// serve both halves of the window. `None` when the row is gone.
    pub fn gen_title(&self, row: crate::app::GenRow) -> Option<(String, String)> {
        match row {
            crate::app::GenRow::Can(i) => {
                let t = self.snap.tx.get(i)?;
                Some((
                    format!("{}  {:X}", t.name, t.id),
                    self.channel_name(t.channel),
                ))
            }
            crate::app::GenRow::Fr(i) => {
                let t = self.snap.fr_tx.get(i)?;
                Some((
                    format!("{}  slot {}", t.name, t.slot),
                    self.fr_bus_name(t.bus),
                ))
            }
        }
    }

    /// The unit `sig` is declared with on this row's frame, for the dialog
    /// header. Empty when neither the row nor its database knows the signal.
    pub fn gen_signal_unit(&self, row: crate::app::GenRow, sig: &str) -> String {
        match row {
            crate::app::GenRow::Can(i) => self
                .snap
                .tx
                .get(i)
                .and_then(|t| self.channel_dbc(t.channel)?.message_of(t.id))
                .and_then(|m| m.signals.iter().find(|s| s.name == sig))
                .map(|s| s.unit.clone())
                .unwrap_or_default(),
            crate::app::GenRow::Fr(i) => {
                let Some(t) = self.snap.fr_tx.get(i) else {
                    return String::new();
                };
                let (bus, slot) = (t.bus, t.slot);
                let Some(db) = self.fr_db(bus) else {
                    return String::new();
                };
                let Some(ix) = db.frame_ix_of_slot(slot) else {
                    return String::new();
                };
                db.edit_signals(ix)
                    .into_iter()
                    .find(|s| s.name == sig)
                    .map(|s| s.unit)
                    .unwrap_or_default()
            }
        }
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

/// Largest payload a FlexRay frame can carry (the protocol's own ceiling).
pub const MAX_FR_PAYLOAD_LEN: usize = 254;

/// The communication cycle a timestamp falls in, as a FlexRay cycle number.
/// The cluster's own cycle length comes from its description; without one there
/// is nothing to count against, and cycle 0 is the honest answer rather than a
/// made-up phase.
pub(crate) fn fr_cycle_at(db: &crate::fr_db::FrDb, at_us: u64) -> u8 {
    let cycle_us = (db.params.cycle_time_ms * 1000.0).round() as u64;
    if cycle_us == 0 {
        return 0;
    }
    ((at_us / cycle_us) % 64) as u8
}

/// The payload one generated FlexRay frame carries: the base bytes with every
/// driven signal laid over them at `at_us`, encoded against the frame the
/// schedule actually puts in this slot at this cycle. Never mutates `tx`.
///
/// The frame is resolved rather than assumed because a static slot can hold
/// several frames by cycle phase -- writing `Amp` where the cycle carries `Bmp`
/// would put a value the decoder then reads as something else.
pub(crate) fn fr_tx_payload(
    fr_dbs: &std::collections::BTreeMap<u8, std::sync::Arc<crate::fr_db::FrDb>>,
    tx: &FrTxMsg,
    at_us: u64,
) -> Vec<u8> {
    let mut data = tx.data.clone();
    let Some(db) = fr_dbs.get(&tx.bus) else {
        return data;
    };
    let Some(frame_ix) = db.frame_ix_at(tx.slot, fr_cycle_at(db, at_us), 2) else {
        return data;
    };
    for src in &tx.srcs {
        // A signal this frame does not carry, or a payload too short to hold it,
        // leaves the bytes alone -- the same refusal as the CAN side, where a
        // value that cannot be encoded must not reach the wire as a partial one.
        let _ = db.encode_signal(frame_ix, &src.name, eval_phys(src, at_us), &mut data);
    }
    data
}

/// Installs a FlexRay entry's base payload at the length its slot carries: short
/// input is zero-padded, and the entry's length never moves -- a FlexRay slot
/// holds exactly what the schedule says it holds, unlike a CAN frame whose DLC
/// follows whatever the operator typed.
pub(crate) fn set_fr_tx_base(tx: &mut FrTxMsg, mut data: Vec<u8>, len: usize) {
    data.resize(len, 0);
    tx.len = len;
    tx.data_text = hex_of(&data);
    tx.data = data;
}

/// Writes one signal's physical value into a FlexRay entry's base payload,
/// encoded through the cluster description that owns the slot -- the script's
/// version of typing a value into the Network ECU panel. The entry keeps the
/// width it carries, so a signal write can neither grow a slot nor leave a short
/// frame behind. `Err` names the missing fact (this slot schedules no frame, the
/// frame declares no such signal, or the payload does not reach it); nothing is
/// written when it fails.
pub(crate) fn fr_tx_set_signal(
    tx: &mut FrTxMsg,
    db: &crate::fr_db::FrDb,
    name: &str,
    phys: f64,
) -> Result<(), String> {
    let ix = db.frame_ix_of_slot(tx.slot).ok_or_else(|| {
        format!("这一路的描述未在 slot {} 调度帧：没有信号可写", tx.slot)
    })?;
    let mut data = tx.data.clone();
    if !db.encode_signal(ix, name, phys, &mut data) {
        return Err(format!(
            "slot {} 的帧不声明信号 {name:?}（或载荷长度不够它）",
            tx.slot
        ));
    }
    let len = tx.len;
    set_fr_tx_base(tx, data, len);
    Ok(())
}

/// Why a FlexRay payload edit is refused, if it is. Both refusals are loud
/// rather than forgiving: silently dropping the extra bytes, or silently padding
/// a typo'd token away, would put different bytes on the schedule than the ones
/// the operator typed.
pub fn fr_hex_refusal(text: &str, len: usize) -> Option<String> {
    let toks: Vec<&str> = text.split_whitespace().collect();
    if toks.is_empty() {
        return Some(format!("载荷 {len} 字节：空的不能写入"));
    }
    if toks.len() > len {
        return Some(format!(
            "这个槽的载荷是 {len} 字节（调度表规定的），{n} 字节写不下；多出来的部分不会发出",
            n = toks.len()
        ));
    }
    if toks.iter().any(|t| u8::from_str_radix(t, 16).is_err()) {
        return Some("载荷要写成 hex 字节对，例如 00 1A FF".to_string());
    }
    None
}
