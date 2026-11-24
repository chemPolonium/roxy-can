//! Panel/script/generator writes, entry creation and frame encoding.

use super::*;

impl BusCore {
    /// Model-side physical encoding for a Panel CAN send. `None` means the
    /// model cannot encode it: no model, a multiplexed message, or a frame the
    /// plan does not know.
    pub(super) fn model_can_signal_payload(
        &self,
        channel: u8,
        id: u32,
        extended: bool,
        signals: &std::collections::BTreeMap<String, f64>,
    ) -> Option<Result<Vec<u8>, String>> {
        let comm = self
            .model_comms
            .get(channel as usize)
            .and_then(|entry| entry.as_ref())?;
        let plans = self
            .model_plans
            .get(channel as usize)
            .and_then(|entry| entry.as_ref())?;
        let cluster = self.can_cluster_ids.get(channel as usize).copied()?;
        let frame = comm.frames_in_cluster(cluster).find(|frame| {
            frame.address.can_id() == Some(id)
                && matches!(
                    frame.address,
                    crate::model::FrameAddress::Can {
                        extended: frame_extended,
                        ..
                    } if frame_extended == extended
                )
        })?;
        let plan = plans.get(frame.id)?;
        let signals_in_frame = comm.signals_in_frame(frame);
        for name in signals.keys() {
            if !signals_in_frame.iter().any(|signal| &signal.name == name) {
                return Some(Err(format!(
                    "message \"{}\" has no signal \"{name}\"",
                    frame.name
                )));
            }
        }
        let mut values = Vec::with_capacity(signals_in_frame.len());
        for signal in &signals_in_frame {
            let mut value = signal.initial_physical().unwrap_or(0.0);
            if let Some(&requested) = signals.get(&signal.name) {
                value = match (signal.min, signal.max) {
                    (Some(min), Some(max)) if max > min => requested.clamp(min, max),
                    _ => requested,
                };
            }
            values.push((signal.id, value));
        }
        let mut payload = vec![0u8; frame.length as usize];
        crate::model::encode_payload(comm, plan, &values, &mut payload);
        Some(Ok(payload))
    }

    /// Encodes a Button's physical CAN frame from catalog signal values:
    /// declared start values first, then each named signal clamped to its
    /// range, at the message's declared DLC.
    pub(super) fn panel_can_signal_payload(
        &self,
        channel: u8,
        id: u32,
        extended: bool,
        signals: &std::collections::BTreeMap<String, f64>,
    ) -> Result<Vec<u8>, String> {
        if let Some(result) = self.model_can_signal_payload(channel, id, extended, signals) {
            return result;
        }
        Err(format!(
            "CAN{} has no model: the frame cannot be encoded",
            channel + 1
        ))
    }

    /// Encodes a Button's physical FlexRay frame from catalog signal values,
    /// using the occupant the slot actually carries at this cycle.
    pub(super) fn model_fr_signal_payload(
        &self,
        bus: u8,
        frame_name: &str,
        signals: &std::collections::BTreeMap<String, f64>,
    ) -> Option<Result<Vec<u8>, String>> {
        let runtime = self.fr_models.get(&bus)?;
        let db = self.fr_dbs.get(&bus)?;
        if !Arc::ptr_eq(&runtime.db, db) {
            return None;
        }
        let (comm, plans) = (&runtime.comm, &runtime.plans);
        let cluster = self.fr_cluster_ids.get(&bus).copied()?;
        let frame = comm.frame_by_name(cluster, frame_name)?;
        let plan = plans.get(frame.id)?;
        let signals_in_frame = comm.signals_in_frame(frame);
        for name in signals.keys() {
            if !signals_in_frame.iter().any(|signal| &signal.name == name) {
                return Some(Err(format!(
                    "FR{bus} frame \"{frame_name}\" has no signal \"{name}\""
                )));
            }
        }
        let mut values = Vec::with_capacity(signals_in_frame.len());
        for signal in &signals_in_frame {
            let mut value = signal.initial_physical().unwrap_or(0.0);
            if let Some(&requested) = signals.get(&signal.name) {
                value = match (signal.min, signal.max) {
                    (Some(min), Some(max)) if max > min => requested.clamp(min, max),
                    _ => requested,
                };
            }
            values.push((signal.id, value));
        }
        let mut payload = vec![0u8; frame.length as usize];
        crate::model::encode_payload(comm, plan, &values, &mut payload);
        Some(Ok(payload))
    }

    pub(super) fn panel_fr_signal_payload(
        &self,
        bus: u8,
        slot: u16,
        signals: &std::collections::BTreeMap<String, f64>,
    ) -> Result<(Vec<u8>, u8), String> {
        let Some(db) = self.fr_dbs.get(&bus) else {
            return Err(format!("FR{bus} has no cluster description"));
        };
        let now_cycle = crate::generator::fr_cycle_at(db, self.sim_t_us);
        let Some(ix) = db
            .frame_ix_at(slot, now_cycle, 2)
            .or_else(|| db.frame_ix_of_slot(slot))
        else {
            return Err(format!("FR{bus} has no frame in slot {slot}"));
        };
        let Some(frame) = db.frame_index(ix) else {
            return Err(format!("FR{bus} has no frame in slot {slot}"));
        };
        let payload = if let Some(result) = self.model_fr_signal_payload(bus, &frame.name, signals)
        {
            result?
        } else {
            return Err(format!("FR{bus} has no model: the frame cannot be encoded"));
        };
        let cycle = if db.frame_ix_at(slot, now_cycle, 2).is_some() {
            now_cycle
        } else {
            frame.triggering.base_cycle as u8
        };
        Ok((payload, cycle))
    }

    pub(super) fn model_eth_signal_payload(
        &self,
        bus: u8,
        frame_name: &str,
        signals: &std::collections::BTreeMap<String, f64>,
    ) -> Option<Result<Vec<u8>, String>> {
        let runtime = self.eth_models.get(&bus)?;
        let db = self.eth_dbs.get(&bus)?;
        if !Arc::ptr_eq(&runtime.db, db) {
            return None;
        }
        let (comm, plans) = (&runtime.comm, &runtime.plans);
        let cluster = self.eth_cluster_ids.get(&bus).copied()?;
        let frame = comm.frame_by_name(cluster, frame_name)?;
        let plan = plans.get(frame.id)?;
        let signals_in_frame = comm.signals_in_frame(frame);
        for name in signals.keys() {
            if !signals_in_frame.iter().any(|signal| &signal.name == name) {
                return Some(Err(format!(
                    "frame \"{frame_name}\" has no signal \"{name}\""
                )));
            }
        }
        let mut values = Vec::with_capacity(signals_in_frame.len());
        for signal in &signals_in_frame {
            values.push((signal.id, signals.get(&signal.name).copied().unwrap_or(0.0)));
        }
        let width = if self.eth_length_includes_header {
            frame.length.saturating_sub(14)
        } else {
            frame.length
        };
        let mut payload = vec![0u8; width as usize];
        crate::model::encode_payload(comm, plan, &values, &mut payload);
        Some(Ok(payload))
    }

    /// Encodes a Button's physical Ethernet frame at the declared payload
    /// width. Ethernet signals carry no declared range here.
    pub(super) fn panel_eth_signal_payload(
        &self,
        bus: u8,
        _db: &crate::eth::db::EthDb,
        frame: &str,
        signals: &std::collections::BTreeMap<String, f64>,
    ) -> Result<Vec<u8>, String> {
        if let Some(result) = self.model_eth_signal_payload(bus, frame, signals) {
            return result;
        }
        Err(format!(
            "ETH{bus} has no model: the frame cannot be encoded"
        ))
    }

    /// Panel setpoint write: encode `phys` into the generator entry
    /// `(ch, id)`, creating that entry from the database when absent, drop
    /// the signal's waveform source so the value holds, and let the next
    /// step send one frame. Values are clamped into the DBC range.
    pub(super) fn panel_write_signal(
        &mut self,
        ch: u8,
        id: u32,
        ext: bool,
        name: &str,
        phys: f64,
    ) -> Result<(), String> {
        let (data, msg_size) = if let Some((comm, plans, cluster)) = self.can_codec(ch) {
            // The model names the frame class exactly, so a standard and an
            // extended id sharing one number cannot write into each other.
            let frame = comm
                .frames_in_cluster(cluster)
                .find(|frame| {
                    matches!(
                        frame.address,
                        crate::model::FrameAddress::Can {
                            id: frame_id,
                            extended,
                            ..
                        } if frame_id == id && extended == ext
                    )
                })
                .ok_or_else(|| format!("unknown message {id:#x}"))?;
            let plan = plans
                .get(frame.id)
                .ok_or_else(|| format!("cannot encode \"{name}\""))?;
            let signal = comm
                .signals_in_frame(frame)
                .into_iter()
                .find(|signal| signal.name == name)
                .ok_or_else(|| format!("unknown signal \"{name}\""))?;
            let value = match (signal.min, signal.max) {
                (Some(min), Some(max)) if max > min => phys.clamp(min, max),
                _ => phys,
            };
            let mut data = self
                .entry_mut(ch, id)
                .map(|tx| tx.data)
                .unwrap_or([0; crate::can::frame::MAX_CAN_FD_LEN]);
            let signal_id = signal.id;
            crate::model::encode_payload(&comm, plan, &[(signal_id, value)], &mut data);
            let size = frame.length.min(crate::can::frame::MAX_CAN_FD_LEN as u32) as u8;
            (data, size)
        } else {
            return Err(format!(
                "bus {ch} has no CAN model: the signal cannot be written"
            ));
        };
        if self.entry_mut(ch, id).is_none() {
            self.add_entry(ch, id);
        }
        let Some(tx) = self.entry_mut(ch, id) else {
            return Err(format!("cannot create the generator entry {id:#x}"));
        };
        tx.srcs.retain(|src| src.name != name);
        let len = tx.len.max(msg_size);
        crate::generator::set_tx_base(tx, data, len);
        Ok(())
    }

    pub(super) fn send_node_frames(
        &mut self,
        frames: Vec<(u64, u8, bool, crate::script::FrameInstance)>,
    ) {
        for (node_id, channel, allowed, frame) in frames {
            if let Err(error) = self.send_script_frame(channel, allowed, &frame)
                && let Some(node) = self.nodes.iter_mut().find(|node| node.id == node_id)
            {
                node.note(format!("[send_frame] {error}"));
            }
        }
    }

    /// Sends one `frame(...)` instance from a script node. The path is
    /// `Frame`, `Bus.Frame`; CAN requires the node's own bus (the node's
    /// role gate still applies), FR/ETH stay session-only.
    pub(super) fn send_script_frame(
        &mut self,
        node_channel: u8,
        allowed: bool,
        frame: &crate::script::FrameInstance,
    ) -> Result<(), String> {
        let parts: Vec<&str> = frame.path.split('.').collect();
        if parts.len() > 2 || parts.iter().any(|part| part.is_empty()) {
            return Err(format!("frame({}) needs Frame or Bus.Frame", frame.path));
        }
        let bus_name = parts.len() == 2;
        if let Some(bus) = parts
            .first()
            .and_then(|part| part.strip_prefix("FR"))
            .filter(|_| bus_name)
            .and_then(|rest| rest.parse::<u8>().ok())
        {
            return self.send_script_fr_frame(bus, parts[1], frame);
        }
        if let Some(bus) = parts
            .first()
            .and_then(|part| part.strip_prefix("ETH"))
            .filter(|_| bus_name)
            .and_then(|rest| rest.parse::<u8>().ok())
        {
            return self.send_script_eth_frame(bus, parts[1], frame);
        }
        let (channel, name) = if bus_name {
            let channel = (0..self.can_count())
                .find(|ch| self.can_name(*ch).eq_ignore_ascii_case(parts[0]))
                .ok_or_else(|| format!("unknown CAN bus \"{}\"", parts[0]))?
                as u8;
            (channel, parts[1])
        } else {
            (node_channel, parts[0])
        };
        self.send_script_can_frame(channel, node_channel, name, allowed, frame)
    }

    pub(super) fn send_script_can_frame(
        &mut self,
        channel: u8,
        node_channel: u8,
        name: &str,
        allowed: bool,
        frame: &crate::script::FrameInstance,
    ) -> Result<(), String> {
        if channel != node_channel {
            return Err(format!(
                "frame({}) names another CAN bus; a node sends on its own bus",
                frame.path
            ));
        }
        let Some((comm, _, cluster)) = self.can_codec(channel) else {
            return Err(format!(
                "CAN{} has no model: the frame cannot be encoded",
                channel + 1
            ));
        };
        let Some(frame_model) = comm.frame_by_name(cluster, name) else {
            return Err(format!("unknown CAN message \"{name}\""));
        };
        let crate::model::FrameAddress::Can { id, extended, .. } = frame_model.address else {
            return Err(format!("unknown CAN message \"{name}\""));
        };
        let payload = self
            .model_can_signal_payload(channel, id, extended, &frame.values)
            .unwrap_or_else(|| Err(format!("CAN{} has no model", channel + 1)))?;
        let mut data = [0u8; crate::can::frame::MAX_CAN_FD_LEN];
        data[..payload.len()].copy_from_slice(&payload);
        self.emit_script_can(channel, allowed, id, extended, data, payload.len() as u8)
    }

    pub(super) fn emit_script_can(
        &mut self,
        channel: u8,
        allowed: bool,
        id: u32,
        extended: bool,
        data: [u8; crate::can::frame::MAX_CAN_FD_LEN],
        len: u8,
    ) -> Result<(), String> {
        let out = CanFrame {
            t_us: self.sim_t_us,
            channel,
            id,
            extended,
            len,
            data,
            dir: crate::can::frame::Direction::Tx,
            flags: if len > 8 {
                crate::can::frame::FrameFlags::FD
            } else {
                crate::can::frame::FrameFlags::NONE
            },
        };
        if allowed {
            self.hw.write_if_live(channel, &out);
        }
        self.buf.push(out);
        Ok(())
    }

    pub(super) fn send_script_fr_frame(
        &mut self,
        bus: u8,
        name: &str,
        frame: &crate::script::FrameInstance,
    ) -> Result<(), String> {
        let Some(db) = self.fr_dbs.get(&bus).cloned() else {
            return Err(format!("FR{bus} has no cluster description"));
        };
        let ix = db
            .frames
            .iter()
            .position(|frame| frame.name == name)
            .ok_or_else(|| format!("FR{bus} has no frame \"{name}\""))?;
        let slot = db
            .frame_index(ix)
            .expect("index from frames")
            .triggering
            .slot_id as u16;
        let Some(result) = self.model_fr_signal_payload(bus, name, &frame.values) else {
            return Err(format!(
                "FR{bus} frame \"{name}\" is not in the unified model; it cannot be encoded"
            ));
        };
        let data = result?;
        let now_cycle = crate::generator::fr_cycle_at(&db, self.sim_t_us);
        let cycle = if db.frame_ix_at(slot, now_cycle, 2).is_some() {
            now_cycle
        } else {
            db.frame_index(ix).expect("index").triggering.base_cycle as u8
        };
        self.panel_fr_frames.push((bus, slot, data, cycle));
        Ok(())
    }

    pub(super) fn send_script_eth_frame(
        &mut self,
        bus: u8,
        name: &str,
        frame: &crate::script::FrameInstance,
    ) -> Result<(), String> {
        let Some(db) = self.eth_dbs.get(&bus).cloned() else {
            return Err(format!("ETH{bus} has no matrix description"));
        };
        let ix = db
            .frames
            .iter()
            .position(|frame| frame.name == name)
            .ok_or_else(|| format!("ETH{bus} has no frame \"{name}\""))?;
        let def = &db.frames[ix];
        let Some(result) = self.model_eth_signal_payload(bus, name, &frame.values) else {
            return Err(format!(
                "ETH{bus} frame \"{name}\" is not in the unified model; it cannot be encoded"
            ));
        };
        let data = result?;
        let vlan = def.vlan;
        let ethertype = def
            .ethertype_hint
            .unwrap_or(crate::eth::EthFrame::SESSION_ETHERTYPE);
        let injected = crate::eth::EthFrame::session(self.sim_t_us, bus, vlan, ethertype, data);
        self.emit_eth(injected)
    }

    /// Writes one signal through the path its protocol owns. The single
    /// setpoint and a Button's preset share this so both report the same
    /// reasons.
    pub(super) fn apply_panel_write(&mut self, key: SigKey, value: f64) -> Result<(), String> {
        let bus = self
            .key_bus(&key)
            .ok_or_else(|| "the signal's bus is not loaded".to_string());
        match key {
            SigKey::Can { id, ext, name, .. } => {
                let ch = bus?;
                self.panel_write_signal(ch, id, ext, &name, value)?;
                self.injected.push((ch, id));
                Ok(())
            }
            SigKey::Fr { slot, name, .. } => {
                let bus = bus?;
                let (payload, cycle) = self.panel_write_fr_signal(bus, slot, &name, value)?;
                self.panel_fr_frames.push((bus, slot, payload, cycle));
                Ok(())
            }
            SigKey::Eth { frame, name, .. } => {
                let bus = bus?;
                self.panel_write_eth_signal(bus, &frame, &name, value)
            }
            SigKey::Derived { .. } => Err("derived signals are read-only".to_string()),
        }
    }

    /// Panel setpoint write into a FlexRay slot: encode the named signal into
    /// a zeroed payload of the occupant frame's length, clamped to the
    /// declared range. The frame enters this session and, when the FR port is
    /// writable and live, goes onto the wire first.
    pub(super) fn panel_write_fr_signal(
        &mut self,
        bus: u8,
        slot: u16,
        name: &str,
        phys: f64,
    ) -> Result<(Vec<u8>, u8), String> {
        let Some(db) = self.fr_dbs.get(&bus) else {
            return Err(format!("FR{bus} has no cluster description"));
        };
        // The description still decides which frame occupies the slot at this
        // cycle; its place in the frame and the encoding are the model's.
        let now_cycle = crate::generator::fr_cycle_at(db, self.sim_t_us);
        let Some(frame_ix) = db
            .frame_ix_at(slot, now_cycle, 2)
            .or_else(|| db.frame_ix_of_slot(slot))
        else {
            return Err(format!("FR{bus} has no frame in slot {slot}"));
        };
        let Some(frame) = db.frame_index(frame_ix) else {
            return Err(format!("FR{bus} has no frame in slot {slot}"));
        };
        let cycle = now_cycle;
        let model = self.fr_models.get(&bus).cloned().and_then(|runtime| {
            let cluster = self.fr_cluster_ids.get(&bus).copied()?;
            Some((runtime.comm.clone(), runtime.plans.clone(), cluster))
        });
        let payload = if let Some((comm, plans, cluster)) = model {
            let Some(model_frame) = comm.frame_by_name(cluster, &frame.name) else {
                return Err(format!("FR{bus} has no frame in slot {slot}"));
            };
            let Some(plan) = plans.get(model_frame.id) else {
                return Err(format!("cannot encode \"{name}\" in slot {slot}"));
            };
            let Some(signal) = comm
                .signals_in_frame(model_frame)
                .into_iter()
                .find(|signal| signal.name == name)
            else {
                return Err(format!("unknown FlexRay signal \"{name}\" in slot {slot}"));
            };
            let value = match (signal.min, signal.max) {
                (Some(min), Some(max)) if max > min => phys.clamp(min, max),
                _ => phys,
            };
            let mut payload = vec![0u8; model_frame.length as usize];
            let signal_id = signal.id;
            crate::model::encode_payload(&comm, plan, &[(signal_id, value)], &mut payload);
            payload
        } else {
            return Err(format!(
                "FR{bus} has no model: the signal cannot be written"
            ));
        };
        Ok((payload, cycle))
    }

    /// Panel setpoint write into an Ethernet signal: encode the value into
    /// the frame's PDU payload and inject one TX frame into this session.
    /// There is no Ethernet port on this tool, so nothing leaves the process;
    /// the injected flow still matches the matrix by VLAN/length/EtherType,
    /// which is what makes its signals decode and read back.
    pub(super) fn panel_write_eth_signal(
        &mut self,
        bus: u8,
        frame_name: &str,
        name: &str,
        phys: f64,
    ) -> Result<(), String> {
        let Some(db) = self.eth_dbs.get(&bus).cloned() else {
            return Err(format!("ETH{bus} has no matrix description"));
        };
        let Some(ix) = db.frames.iter().position(|frame| frame.name == frame_name) else {
            return Err(format!("unknown Ethernet frame \"{frame_name}\""));
        };
        let def = &db.frames[ix];
        let len = if self.eth_length_includes_header {
            def.length.saturating_sub(14)
        } else {
            def.length
        };
        if len == 0 {
            return Err(format!("frame \"{frame_name}\" declares no payload"));
        }
        let model = self.eth_models.get(&bus).cloned().and_then(|runtime| {
            let cluster = self.eth_cluster_ids.get(&bus).copied()?;
            Some((runtime.comm.clone(), runtime.plans.clone(), cluster))
        });
        let mut payload = vec![0u8; len];
        if let Some((comm, plans, cluster)) = model {
            let Some(frame) = comm.frame_by_name(cluster, frame_name) else {
                return Err(format!("unknown Ethernet frame \"{frame_name}\""));
            };
            let Some(plan) = plans.get(frame.id) else {
                return Err(format!(
                    "cannot encode \"{name}\" in frame \"{frame_name}\""
                ));
            };
            let Some(signal) = comm
                .signals_in_frame(frame)
                .into_iter()
                .find(|signal| signal.name == name)
            else {
                return Err(format!(
                    "cannot encode \"{name}\" in frame \"{frame_name}\""
                ));
            };
            let value = match (signal.min, signal.max) {
                (Some(min), Some(max)) if max > min => phys.clamp(min, max),
                _ => phys,
            };
            let signal_id = signal.id;
            crate::model::encode_payload(&comm, plan, &[(signal_id, value)], &mut payload);
        } else {
            return Err(format!(
                "ETH{bus} has no model: \"{name}\" cannot be written"
            ));
        }
        let ethertype = def
            .ethertype_hint
            .unwrap_or(crate::eth::EthFrame::SESSION_ETHERTYPE);
        let vlan = def.vlan;
        let frame = crate::eth::EthFrame::session(self.sim_t_us, bus, vlan, ethertype, payload);
        self.emit_eth(frame)
    }

    /// Writes a physical value into the base payload and pins that signal
    /// by dropping only its source: grabbing a moving slider means "hold
    /// here". Returns false when the database cannot encode it.
    #[cfg(test)]
    #[cfg(test)]
    pub(crate) fn pin_entry_signal(&mut self, ch: u8, id: u32, name: &str, phys: f64) -> bool {
        let mut data = match self.entry_mut(ch, id) {
            Some(tx) => tx.data,
            None => return false,
        };
        let Some(table) = self.model_dbs.get(ch as usize).and_then(|db| db.as_deref()) else {
            return false;
        };
        if !table.encode_signal(id, name, phys, &mut data) {
            return false;
        }
        let msg_size = table
            .message_of(id)
            .map(|m| m.dlc.min(crate::can::frame::MAX_CAN_FD_LEN as u64) as u8)
            .unwrap_or(0);
        let tx = self.entry_mut(ch, id).expect("entry checked above");
        tx.srcs.retain(|s| s.name != name);
        let len = tx.len.max(msg_size);
        crate::generator::set_tx_base(tx, data, len);
        true
    }

    /// The generator entry `(ch, id)`, if present.
    pub(super) fn entry_mut(&mut self, ch: u8, id: u32) -> Option<&mut TxMsg> {
        self.tx_list
            .iter_mut()
            .find(|t| t.channel == ch && t.id == id)
    }

    /// Replaces the base payload from the generator's hex box. Returns
    /// false if the text is not whole hex bytes or no entry carries the
    /// key.
    pub(super) fn set_entry_hex(&mut self, ch: u8, id: u32, text: &str) -> bool {
        let Some(bytes) = crate::generator::parse_hex_bytes(text) else {
            return false;
        };
        let Some(tx) = self.entry_mut(ch, id) else {
            return false;
        };
        let mut data = [0u8; crate::can::frame::MAX_CAN_FD_LEN];
        data[..bytes.len()].copy_from_slice(&bytes);
        let len = bytes.len() as u8;
        crate::generator::set_tx_base(tx, data, len);
        true
    }

    /// Adds the FlexRay generator entry `(bus, slot)` for `node`, unless it
    /// exists. The caller -- the frontend, which owns the cluster names -- has
    /// already resolved which ECU the description declares as the frame's
    /// sender, because that is the question the Network tree groups by. Name,
    /// payload length and period come from the description when this bus has one
    /// here; without it the entry is the raw filler its row says it is.
    pub(super) fn add_fr_entry(&mut self, bus: u8, slot: u16, node: String) {
        if self
            .fr_tx_list
            .iter()
            .any(|t| t.bus == bus && t.slot == slot)
        {
            return;
        }
        let (name, len, period_us) = self
            .fr_dbs
            .get(&bus)
            .and_then(|db| Some((db, db.frame_ix_of_slot(slot)?)))
            .and_then(|(db, ix)| {
                let f = db.frame_index(ix)?;
                // The schedule's own cadence for this frame: how many cycles
                // between its arrivals, times the cluster cycle.
                let rep = f.triggering.cycle_repetition.max(1) as f64;
                Some((
                    if f.name.is_empty() {
                        format!("slot {slot}")
                    } else {
                        f.name.clone()
                    },
                    (f.length as usize).clamp(1, crate::generator::MAX_FR_PAYLOAD_LEN),
                    (db.params.cycle_time_ms * rep * 1_000.0).round() as u64,
                ))
            })
            .unwrap_or_else(|| (format!("slot {slot}"), 8, 0));
        let data = vec![0u8; len];
        self.fr_tx_list.push(FrTxMsg {
            bus,
            slot,
            name,
            node,
            data_text: crate::generator::hex_of(&data),
            len,
            data,
            cycle_us: match period_us {
                0 => crate::app::DEFAULT_TX_CYCLE_US,
                n => n,
            },
            active: false,
            next_t_us: 0,
            srcs: Vec::new(),
            last_sent: Vec::new(),
        });
    }

    /// The FlexRay generator entry `(bus, slot)`, if present.
    pub(super) fn fr_entry_mut(&mut self, bus: u8, slot: u16) -> Option<&mut FrTxMsg> {
        self.fr_tx_list
            .iter_mut()
            .find(|t| t.bus == bus && t.slot == slot)
    }

    /// Replaces a FlexRay entry's base payload from its hex box.
    /// Writes a FlexRay entry's base payload. The slot's declared length bounds
    /// it -- see [`crate::generator::set_fr_tx_base`] -- and the return value is
    /// the sentence explaining a refusal, `None` when the bytes went in.
    pub(super) fn set_fr_entry_hex(&mut self, bus: u8, slot: u16, text: &str) -> Option<String> {
        // The slot's width comes from the schedule when the description is there
        // to say so; an undescribed slot keeps the length its entry already has.
        let declared = self.fr_dbs.get(&bus).and_then(|db| {
            let ix = db.frame_ix_of_slot(slot)?;
            let f = db.frame_index(ix)?;
            (f.length > 0).then_some(f.length as usize)
        });
        // No such entry: nothing to refuse, nothing to write.
        let current = self
            .fr_tx_list
            .iter()
            .find(|t| t.bus == bus && t.slot == slot)?;
        let len = declared.unwrap_or(current.len);
        if let Some(why) = crate::generator::fr_hex_refusal(text, len) {
            return Some(why);
        }
        let bytes = crate::generator::parse_hex_limited(text, len)?;
        let tx = self
            .fr_entry_mut(bus, slot)
            .expect("entry found a moment ago");
        crate::generator::set_fr_tx_base(tx, bytes, len);
        None
    }

    /// Writes a physical value into a FlexRay entry's base payload and drops only
    /// that signal's source. False -- nothing changed -- when the bus has no
    /// description, the slot holds no frame in it, the frame has no such signal,
    /// or the entry's bytes do not reach it.
    #[cfg(test)]
    #[cfg(test)]
    pub(crate) fn pin_fr_entry_signal(
        &mut self,
        bus: u8,
        slot: u16,
        name: &str,
        phys: f64,
    ) -> bool {
        let (mut data, len) = match self.fr_entry_mut(bus, slot) {
            Some(t) => (t.data.clone(), t.len),
            None => return false,
        };
        let Some(db) = self.fr_dbs.get(&bus) else {
            return false;
        };
        let Some(ix) = db.frame_ix_of_slot(slot) else {
            return false;
        };
        if !db.encode_signal(ix, name, phys, &mut data) {
            return false;
        }
        let tx = self.fr_entry_mut(bus, slot).expect("entry checked above");
        tx.srcs.retain(|s| s.name != name);
        crate::generator::set_fr_tx_base(tx, data, len);
        true
    }
    /// Adds the generator entry `(ch, id)` unless it already exists.
    pub(super) fn add_entry(&mut self, ch: u8, id: u32) {
        if self.tx_list.iter().any(|t| t.channel == ch && t.id == id) {
            return;
        }
        let model_cycle = self.model_can_cycle_us(ch, id);
        let (name, node, len, cycle_us) =
            if let Some((name, node, len, cycle)) = self.model_frame_meta(ch, id) {
                (
                    name,
                    node,
                    len,
                    cycle
                        .or(model_cycle)
                        .unwrap_or(crate::app::DEFAULT_TX_CYCLE_US),
                )
            } else {
                (
                    format!("{id:X}"),
                    String::new(),
                    8,
                    model_cycle.unwrap_or(crate::app::DEFAULT_TX_CYCLE_US),
                )
            };
        let data_text = vec!["00"; len as usize].join(" ");
        self.tx_list.push(TxMsg {
            channel: ch,
            id,
            srcs: Vec::new(),
            extended: id > 0x7FF,
            name,
            node,
            len,
            data: [0; crate::can::frame::MAX_CAN_FD_LEN],
            flags: if len > 8 {
                crate::can::frame::FrameFlags::FD
            } else {
                crate::can::frame::FrameFlags::NONE
            },
            data_text,
            cycle_us,
            active: false,
            next_t_us: 0,
            last_sent: [0; crate::can::frame::MAX_CAN_FD_LEN],
            last_len: 0,
        });
    }
}
