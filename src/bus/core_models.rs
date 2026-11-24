//! Description-model rebuilds, decode plans, E2E tracking and model metadata.

use super::*;

impl BusCore {
    /// The model plan for one CAN frame, when the channel's model is still
    /// the one built from its current table. Used by paths that only need
    /// the plan's structure (E2E framing) rather than its decoded signals.
    pub(super) fn model_can_plan(&self, f: &CanFrame) -> Option<Arc<crate::model::DecodePlan>> {
        let ch = f.channel as usize;
        let comm = self.model_comms.get(ch)?.as_ref()?;
        let plans = self.model_plans.get(ch)?.as_ref()?;
        let cluster = *self.can_cluster_ids.get(ch)?;
        let frame = comm.frames_in_cluster(cluster).find(|frame| {
            matches!(
                frame.address,
                crate::model::FrameAddress::Can {
                    id,
                    extended,
                    ..
                } if id == f.id && extended == f.extended
            )
        })?;
        plans.get(frame.id).cloned()
    }

    pub(super) fn track_e2e(&mut self, f: &CanFrame) {
        let Some(plan) = self.model_can_plan(f) else {
            return;
        };
        if plan.e2e.is_empty() {
            return;
        }
        let payload = &f.data[..f.len.min(f.data.len() as u8) as usize];
        let Some(reading) = crate::model::decode_e2e(&plan, payload).into_iter().next() else {
            return;
        };
        let change = note_e2e(
            self.e2e_tracks
                .entry((f.channel, f.id, f.extended))
                .or_default(),
            reading,
        );
        if matches!(change, E2eChange::Gap | E2eChange::Jump) {
            let mut status = String::new();
            self.eval_e2e_triggers(f.channel, f.id, f.extended, f.t_us, &mut status);
        }
    }

    pub(super) fn track_fr_e2e(&mut self, row: &crate::trace::FrRow, frame_ix: usize) {
        let (Some(runtime), Some(db)) = (self.fr_models.get(&row.bus), self.fr_dbs.get(&row.bus))
        else {
            return;
        };
        if !Arc::ptr_eq(&runtime.db, db) {
            return;
        }
        let (comm, plans) = (&runtime.comm, &runtime.plans);
        let Some(name) = db.frame_index(frame_ix).map(|frame| frame.name.clone()) else {
            return;
        };
        let Some(&cluster) = self.fr_cluster_ids.get(&row.bus) else {
            return;
        };
        let Some(frame) = comm.frame_by_name(cluster, &name) else {
            return;
        };
        let Some(plan) = plans.get(frame.id) else {
            return;
        };
        let Some(reading) = crate::model::decode_e2e(plan, &row.payload)
            .into_iter()
            .next()
        else {
            return;
        };
        note_e2e(
            self.fr_e2e_tracks.entry((row.bus, row.slot)).or_default(),
            reading,
        );
    }

    pub(super) fn track_eth_e2e(&mut self, bus: u8, frame_name: &str, payload: &[u8]) {
        let (Some(runtime), Some(db)) = (self.eth_models.get(&bus), self.eth_dbs.get(&bus)) else {
            return;
        };
        if !Arc::ptr_eq(&runtime.db, db) {
            return;
        }
        let (comm, plans) = (&runtime.comm, &runtime.plans);
        let Some(&cluster) = self.eth_cluster_ids.get(&bus) else {
            return;
        };
        let Some(frame) = comm.frame_by_name(cluster, frame_name) else {
            return;
        };
        let Some(plan) = plans.get(frame.id) else {
            return;
        };
        let Some(reading) = crate::model::decode_e2e(plan, payload).into_iter().next() else {
            return;
        };
        note_e2e(
            self.eth_e2e_tracks
                .entry(frame_name.to_string())
                .or_default(),
            reading,
        );
    }

    /// Model-side signal values for a CAN frame, in the shape the tables
    /// consume. `None` means the channel has no model or the model does not
    /// know the frame.
    pub(crate) fn model_can_values(&self, f: &CanFrame) -> Option<Vec<crate::dbc::DecodedSignal>> {
        self.model_can_decoded(f, true)
    }

    /// Every signal the plan decodes, regardless of subscriptions: used by
    /// status listings and reference logs, which are not observers.
    pub(crate) fn model_can_all_values(
        &self,
        f: &CanFrame,
    ) -> Option<Vec<crate::dbc::DecodedSignal>> {
        self.model_can_decoded(f, false)
    }

    pub(crate) fn model_can_decoded(
        &self,
        f: &CanFrame,
        only_subscribed: bool,
    ) -> Option<Vec<crate::dbc::DecodedSignal>> {
        let comm = self
            .model_comms
            .get(f.channel as usize)
            .and_then(|entry| entry.as_ref())?;
        let plans = self
            .model_plans
            .get(f.channel as usize)
            .and_then(|entry| entry.as_ref())?;
        let cluster = *self.can_cluster_ids.get(f.channel as usize)?;
        // A standard and an extended message can share one id; the class is
        // part of the frame's identity, not a check after the fact.
        let frame = comm.frames_in_cluster(cluster).find(|frame| {
            matches!(
                frame.address,
                crate::model::FrameAddress::Can {
                    id,
                    extended,
                    ..
                } if id == f.id && extended == f.extended
            )
        })?;
        let plan = plans.get(frame.id)?;
        let payload = &f.data[..f.len as usize];
        let decoded = crate::model::decode_payload(comm, plan, payload);
        let mut out = Vec::with_capacity(decoded.len());
        for value in decoded {
            let Some(signal) = comm.signals.get(&value.signal) else {
                continue;
            };
            let key = self.can_key(f.channel, f.id, f.extended, signal.name.clone());
            if only_subscribed && !self.subs.contains_key(&key) {
                continue;
            }
            // A signal past the end of a short frame has no physical value;
            // it must not hide the signals that do fit.
            if let Some(decoded) = Self::model_signal_decoded(comm, value) {
                out.push(decoded);
            }
        }
        Some(out)
    }

    pub(crate) fn model_signal_decoded(
        comm: &crate::model::Comm,
        value: crate::model::DecodedSignal,
    ) -> Option<crate::dbc::DecodedSignal> {
        let signal = comm.signals.get(&value.signal)?;
        let physical = value.physical?;
        let type_tag = if signal.float {
            if signal.length == 32 { "f32" } else { "f64" }.to_string()
        } else {
            format!("{}{}", if signal.signed { "i" } else { "u" }, signal.length)
        };
        let label = signal
            .values
            .iter()
            .find(|(code, _)| *code == value.raw as i64)
            .map(|(_, text)| text.clone());
        Some(crate::dbc::DecodedSignal {
            name: signal.name.clone(),
            phys: physical,
            raw: value.raw as i64,
            unit: signal.unit.clone(),
            type_tag,
            label,
        })
    }

    pub(crate) fn model_fr_values(
        &self,
        row: &crate::trace::FrRow,
        frame_ix: usize,
    ) -> Option<Vec<crate::dbc::DecodedSignal>> {
        self.model_fr_decoded(row, frame_ix, true)
    }

    pub(crate) fn model_fr_all_values(
        &self,
        row: &crate::trace::FrRow,
        frame_ix: usize,
    ) -> Option<Vec<crate::dbc::DecodedSignal>> {
        self.model_fr_decoded(row, frame_ix, false)
    }

    pub(super) fn model_fr_decoded(
        &self,
        row: &crate::trace::FrRow,
        frame_ix: usize,
        only_subscribed: bool,
    ) -> Option<Vec<crate::dbc::DecodedSignal>> {
        let runtime = self.fr_models.get(&row.bus)?;
        let db = self.fr_dbs.get(&row.bus)?;
        if !Arc::ptr_eq(&runtime.db, db) {
            return None;
        }
        let (comm, plans) = (&runtime.comm, &runtime.plans);
        let frame_name = &db.frame_index(frame_ix)?.name;
        let cluster = self.fr_cluster_ids.get(&row.bus).copied()?;
        let frame = comm.frame_by_name(cluster, frame_name)?;
        let plan = plans.get(frame.id)?;
        let decoded = crate::model::decode_payload(comm, plan, &row.payload);
        let mut out = Vec::with_capacity(decoded.len());
        for value in decoded {
            let Some(signal) = comm.signals.get(&value.signal) else {
                continue;
            };
            let key = self.fr_key(row.bus, row.slot, signal.name.clone());
            if only_subscribed && !self.subs.contains_key(&key) {
                continue;
            }
            if let Some(decoded) = Self::model_signal_decoded(comm, value) {
                out.push(decoded);
            }
        }
        Some(out)
    }

    pub(crate) fn model_eth_values(
        &self,
        bus: u8,
        frame_name: &str,
        payload: &[u8],
    ) -> Option<Vec<crate::dbc::DecodedSignal>> {
        self.model_eth_decoded(bus, frame_name, payload, true)
    }

    pub(crate) fn model_eth_all_values(
        &self,
        bus: u8,
        frame_name: &str,
        payload: &[u8],
    ) -> Option<Vec<crate::dbc::DecodedSignal>> {
        self.model_eth_decoded(bus, frame_name, payload, false)
    }

    pub(super) fn model_eth_decoded(
        &self,
        bus: u8,
        frame_name: &str,
        payload: &[u8],
        only_subscribed: bool,
    ) -> Option<Vec<crate::dbc::DecodedSignal>> {
        let runtime = self.eth_models.get(&bus)?;
        let db = self.eth_dbs.get(&bus)?;
        if !Arc::ptr_eq(&runtime.db, db) {
            return None;
        }
        let (comm, plans) = (&runtime.comm, &runtime.plans);
        let cluster = self.eth_cluster_ids.get(&bus).copied()?;
        let frame = comm.frame_by_name(cluster, frame_name)?;
        let plan = plans.get(frame.id)?;
        let decoded = crate::model::decode_payload(comm, plan, payload);
        let mut out = Vec::with_capacity(decoded.len());
        for value in decoded {
            let Some(signal) = comm.signals.get(&value.signal) else {
                continue;
            };
            let key = self.eth_key(bus, frame_name.to_string(), signal.name.clone());
            if only_subscribed && !self.subs.contains_key(&key) {
                continue;
            }
            if let Some(decoded) = Self::model_signal_decoded(comm, value) {
                out.push(decoded);
            }
        }
        Some(out)
    }

    /// Gives every `Simulated` node on the bus the generator entries it is
    /// missing, so a database that gained messages makes them show up.
    /// New entries arrive **inactive** -- neither a DBC edit nor a project
    /// restore may start traffic by itself; switching the role or the TX
    /// switch is the explicit act that does.
    pub(super) fn seed_entries_for_simulated(&mut self, ch: usize) {
        let simulated: Vec<String> = self
            .can_node_roles(ch)
            .into_iter()
            .filter(|(_, r)| *r == NodeRole::Simulated)
            .map(|(n, _)| n)
            .collect();
        for node in simulated {
            let ids = self.model_tx_ids(ch as u8, &node).unwrap_or_default();
            for id in ids {
                self.add_entry(ch as u8, id);
            }
        }
    }

    /// Re-reads every attached DBC file and reloads the merged table when
    /// any content checksum changed. Returns the reload status text when
    /// something was reloaded. Cheap: one small read per file per sweep,
    /// rate-limited by the caller. Keeps external DBC edits flowing into
    /// decode without a manual reload.
    pub(crate) fn maybe_reload_changed_dbcs(&mut self) -> Option<String> {
        // First pass: find buses whose checksum changed (read-only scan).
        let mut to_reload: Vec<usize> = Vec::new();
        for ch in 0..self.can_count() {
            let paths = self.can_paths(ch);
            let Some(sums) = self
                .can_cluster_ids
                .get(ch)
                .and_then(|c| self.can_sums.get(c))
            else {
                continue;
            };
            if paths.len() != sums.len() {
                continue; // lists out of sync until the next explicit load
            }
            for (i, path) in paths.iter().enumerate() {
                let path = path.trim();
                if path.is_empty() {
                    continue;
                }
                let Ok(content) = crate::dbc::read_file(std::path::Path::new(path)) else {
                    continue;
                };
                if content_sum(content.as_bytes()) != sums[i] {
                    to_reload.push(ch);
                    break; // one reload per bus per sweep
                }
            }
        }
        // Second pass: reload and collect status text.
        let mut reloaded: Option<String> = None;
        for ch in to_reload {
            let mut status = String::new();
            if self.load_channel(ch, &mut status) {
                reloaded = Some(status);
            }
        }
        reloaded
    }

    /// Stores the bus's database path list (primary first, extras after)
    /// and parses everything in the same stroke -- the path is not really
    /// set until its file has been accepted.
    pub(super) fn load_dbc(&mut self, ch: u8, paths: Vec<String>, status: &mut String) {
        if ch as usize >= self.can_count() {
            return;
        }
        self.set_can_paths(ch as usize, paths);
        self.load_channel(ch as usize, status);
    }

    /// Static per-bus declarations: rename, bitrate figures, the DBC
    /// path (without loading it -- that is [`Self::load_dbc`]'s job), or
    /// the node-role map.
    pub(super) fn set_channel_config(
        &mut self,
        ch: u8,
        name: Option<String>,
        dbc_path: Option<String>,
        bitrate_kbps: Option<u32>,
        fd_data_kbps: Option<u32>,
        node_roles: Option<std::collections::BTreeMap<String, crate::app::NodeRole>>,
    ) {
        if ch as usize >= self.can_count() {
            return;
        }
        let ch = ch as usize;
        if let Some(p) = dbc_path {
            // Replaces the primary database; any extra attached databases
            // stay on the bus.
            let mut paths = self.can_paths(ch);
            if paths.is_empty() {
                paths.push(p);
            } else {
                paths[0] = p;
            }
            self.set_can_paths(ch, paths);
        }
        if name.is_some() || bitrate_kbps.is_some() || fd_data_kbps.is_some() {
            self.update_can_comm(ch, |comm| {
                for cluster in comm.clusters.values_mut() {
                    if let Some(name) = &name {
                        cluster.name = name.clone();
                    }
                }
                for network in comm.networks.values_mut() {
                    if let Some(name) = &name {
                        network.name = name.clone();
                    }
                    if let crate::model::NetworkParams::Can(params) = &mut network.params {
                        if let Some(k) = bitrate_kbps {
                            params.bitrate_kbps = k.max(1);
                        }
                        if let Some(k) = fd_data_kbps {
                            params.fd_data_kbps = k.max(1);
                        }
                    }
                }
            });
        }
        if let Some(roles) = node_roles {
            // Project restore pins the declarations wholesale. Whether
            // anything transmits is decided by the restored generator
            // rows, so this only records intent -- no entry activation
            // here.
            let cluster = self.can_cluster_ids[ch];
            if roles.is_empty() {
                self.can_node_roles.remove(&cluster);
            } else {
                self.can_node_roles.insert(cluster, roles);
            }
            self.nodes_dirty = true;
        }
    }

    /// Restores one generator row wholesale from a saved project, with
    /// every field's own convention applied: FD flag, the anti-typo
    /// period floor is the sender's to apply first, `None` payload keeps
    /// the row's current base bytes, and the schedule restarts now.
    // The signature mirrors the `SetEntryConfig` command one-to-one; a
    // payload struct would just relocate the field list.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn set_entry_config(
        &mut self,
        ch: u8,
        id: u32,
        active: bool,
        cycle_us: u64,
        fd: bool,
        flags: Option<crate::can::frame::FrameFlags>,
        data_text: Option<String>,
        srcs: Vec<crate::sim::ValueSrc>,
    ) {
        let Some(tx) = self.entry_mut(ch, id) else {
            return;
        };
        tx.active = active;
        tx.cycle_us = cycle_us;
        tx.next_t_us = 0;
        tx.flags = flags.unwrap_or(if fd {
            crate::can::frame::FrameFlags::FD
        } else {
            crate::can::frame::FrameFlags::NONE
        });
        if let Some(text) = data_text
            && let Some(bytes) = crate::generator::parse_hex_bytes(&text)
        {
            let mut data = [0u8; crate::can::frame::MAX_CAN_FD_LEN];
            data[..bytes.len()].copy_from_slice(&bytes);
            crate::generator::set_tx_base(tx, data, bytes.len() as u8);
        }
        tx.srcs = srcs;
    }

    /// Loads every bus's declared database in one go. This is bootstrap
    /// work, run before the first publish (and before the core thread
    /// ever starts), not a command: there is no frontend yet to receive
    /// the status lines.
    pub(crate) fn bootstrap_dbcs(&mut self) {
        for ch in 0..self.can_count() {
            let mut status = String::new();
            self.load_channel(ch, &mut status);
        }
    }

    /// Pre-populates the generator with one entry per message the loaded
    /// databases declare. Bootstrap work, like [`Self::bootstrap_dbcs`]:
    /// it runs before the first publish, so a threaded frontend's very
    /// first snapshot already shows the full send table.
    pub(crate) fn populate_generator(&mut self) {
        let ids: Vec<(u8, u32)> = (0..self.can_count())
            .flat_map(|ch| {
                self.model_dbs
                    .get(ch)
                    .and_then(|db| db.as_ref())
                    .map(|db| db.order.iter().map(move |&(id, _)| (ch as u8, id)))
                    .into_iter()
                    .flatten()
            })
            .collect();
        for (ch, id) in ids {
            self.add_entry(ch, id);
        }
    }

    /// Removes bus `ch` and remaps every bus-side, channel-indexed
    /// reference one step down. Window state is the frontend's to remap.
    pub(super) fn remove_channel(&mut self, ch: usize, status: &mut String) {
        if self.can_count() <= 1 {
            *status = "at least one bus is required".to_string();
            return;
        }
        if ch >= self.can_count() {
            return;
        }
        let name = self.can_name(ch);
        let removed_cluster = self.can_cluster_ids.get(ch).copied();
        if ch < self.model_comms.len() {
            self.model_comms.remove(ch);
            self.model_plans.remove(ch);
            self.model_dbs.remove(ch);
            self.can_xml_models.remove(ch);
        }
        if ch < self.can_cluster_ids.len() {
            self.can_cluster_ids.remove(ch);
        }
        if let Some(cluster) = removed_cluster {
            self.can_sums.remove(&cluster);
            self.can_node_roles.remove(&cluster);
        }
        self.bus_loads.remove(ch);
        self.loads_dirty = true;
        self.aggs.retain(|_, agg| agg.channel as usize != ch);
        for agg in self.aggs.values_mut() {
            if agg.channel as usize > ch {
                agg.channel -= 1;
            }
        }
        // Keys name their cluster, so a removed CAN channel takes exactly its
        // own subscriptions; the channels that stay keep theirs unchanged.
        let dead: Vec<SigKey> = self
            .subs
            .keys()
            .filter(|key| self.key_bus(key).is_none())
            .cloned()
            .collect();
        for key in dead {
            self.subs.remove(&key);
        }
        self.spec.drop_channel(ch as u8);
        self.tx_list.retain(|t| t.channel as usize != ch);
        for t in &mut self.tx_list {
            if t.channel as usize > ch {
                t.channel -= 1;
            }
        }
        // 映射表记的是 cluster：指到被删 cluster 的表项变成「不回放」，
        // 其余表项原样保留，后面的路不需要往前挪。
        if let Some(cluster) = removed_cluster {
            self.replay_map
                .drop_cluster(crate::replay_map::LogBusKind::Can, cluster);
        }
        self.remap_log_ids();
        // The hardware attachment of the removed bus closes with it; the
        // other buses' attachments shift down with their bus.
        self.hw.remove_bus(ch);
        // Script nodes and replay blocks bind to a bus by index: the ones
        // on the removed bus lose their meaning and go with it, the rest
        // shift down -- the same policy the tx entries follow. Derived
        // streams follow their owning node's bus.
        self.nodes.retain(|n| n.channel as usize != ch);
        for n in &mut self.nodes {
            if n.channel as usize > ch {
                n.channel -= 1;
            }
        }
        self.replay_blocks.retain(|b| b.channel as usize != ch);
        for b in &mut self.replay_blocks {
            if b.channel as usize > ch {
                b.channel -= 1;
            }
        }
        // Triggers bind to a bus by index as well: a rule watching the
        // removed bus is meaningless, and a Send reaction aimed at it has
        // no target left. Everything else shifts down. A FlexRay rule's index
        // is not in this space at all -- the CAN renumbering leaves it alone.
        let removed = ch as u8;
        self.triggers.retain(|t| {
            let cond_gone = t.cond.can_bus() == Some(removed);
            let action_gone = matches!(&t.action, TriggerAction::Send { ch, .. } if *ch == removed);
            !cond_gone && !action_gone
        });
        for t in &mut self.triggers {
            let cond_ch = match &mut t.cond {
                TriggerCond::SignalCross { ch, .. }
                | TriggerCond::IdPresent { ch, .. }
                | TriggerCond::ErrorFrame { ch }
                | TriggerCond::CycleTimeout { ch, .. }
                | TriggerCond::PayloadMatch { ch, .. }
                | TriggerCond::E2eIssue { ch, .. } => ch,
                // Global, or aiming at a FlexRay/Ethernet bus: no CAN index.
                TriggerCond::SysVar { .. }
                | TriggerCond::DerivedSignal { .. }
                | TriggerCond::FrFramePresent { .. }
                | TriggerCond::FrSignalCross { .. }
                | TriggerCond::FrTimeout { .. }
                | TriggerCond::EthFlow { .. }
                | TriggerCond::SomeIp { .. } => continue,
            };
            if *cond_ch > removed {
                *cond_ch -= 1;
            }
            if let TriggerAction::Send { ch, .. } = &mut t.action
                && *ch > removed
            {
                *ch -= 1;
            }
        }
        // A stream belongs to the cluster of its owning node: the removed
        // channel takes its streams, and the rest keep their keys.
        let live = self.can_cluster_ids.clone();
        self.emitted_streams.retain(|(k, _)| match k.cluster() {
            Some(cluster) => live.contains(&cluster),
            None => true,
        });
        self.nodes_dirty = true;
        self.trace.rewrite(|f| {
            if f.channel as usize == ch {
                return false;
            }
            if f.channel as usize > ch {
                f.channel -= 1;
            }
            true
        });
        self.publish_trace();
        *status = format!("{name} removed");
    }

    /// Deletes one FlexRay 路 and everything keyed by its cluster index.
    ///
    /// The survivors keep their numbers, which is the whole difference from
    /// [`Self::remove_channel`]: a FlexRay bus index is not a position in a
    /// list but the **cluster number** everything else speaks -- a BLF stamps
    /// each frame with its own `clusterNo`, a script's `on fr 1 slot 5` and
    /// the record filter's `FR1:5` are typed with it, and a trigger rule
    /// stores it. Shifting the survivors down would silently re-aim every one
    /// of those at a different network. The gap is what `next_flexray_bus`
    /// fills when the user adds the next 路, so indexes stay small without
    /// anyone having to move one.
    pub(super) fn remove_fr_bus(&mut self, bus: u8, label: &str, status: &mut String) {
        let removed_cluster = self.fr_cluster_ids.get(&bus).copied();
        // The port closes with the bus it watched.
        self.hw.detach_fr(bus);
        self.fr_dbs.remove(&bus);
        self.rebuild_fr_models();
        self.fr_loads.remove(&bus);
        self.loads_dirty = true;
        // What this 路 was asked to send, and what the loaded log was already
        // sending for it, both go with it.
        self.fr_tx_list.retain(|t| t.bus != bus);
        // 这路的 cluster 不再存在，映射表里指向它的表项变成「不回放」。
        if let Some(cluster) = removed_cluster {
            self.replay_map
                .drop_cluster(crate::replay_map::LogBusKind::Fr, cluster);
        }
        self.remap_log_ids();
        // The model rebuild above already dropped this bus's cluster id, so
        // the FlexRay subscriptions left naming a cluster the map no longer
        // holds are exactly the ones on the removed bus.
        let live: Vec<crate::model::ClusterId> = self.fr_cluster_ids.values().copied().collect();
        self.subs.retain(|k, _| match k {
            SigKey::Fr { cluster, .. } => live.contains(cluster),
            _ => true,
        });
        self.fr_trace.drop_bus(bus);
        self.publish_trace();
        // The tallies are the surviving rows' tallies, and a frame name
        // resolved from the description that just went cannot be resolved
        // again -- re-fold the ring rather than hand-patch keys.
        self.rebuild_fr_aggs();
        self.spec.drop_fr_bus(bus);
        // A rule whose condition names a cluster that no longer exists can
        // never fire again, so it goes; CAN-indexed rules are not this bus's
        // business.
        self.triggers.retain(|t| t.cond.fr_bus() != Some(bus));
        *status = format!("FlexRay bus {label} removed");
    }

    /// Declares a DBC node's role.
    ///
    /// `Simulated` adds whatever generator entry the node is missing and
    /// switches them on. The period of an entry that already exists is
    /// never rewritten, so a value tuned by hand outlives the click.
    /// Leaving `Simulated` only stops sending: entries keep their payload
    /// and waveforms, so simulating the node again restores it exactly as
    /// it was. `Monitor` and `Absent` are behaviourally the same here --
    /// neither transmits -- and differ in what they declare: a monitor is
    /// present and listening, an absent node is not on the simulated bus.
    pub(super) fn set_node_role(
        &mut self,
        channel: u8,
        node: &str,
        role: NodeRole,
        status: &mut String,
    ) {
        if channel as usize >= self.can_count() {
            return;
        }
        // The role is recorded first and unconditionally: a node that sends
        // nothing still has to remember what we declared it to be.
        let cluster = self.can_cluster_ids[channel as usize];
        match role {
            NodeRole::Absent => {
                if let Some(roles) = self.can_node_roles.get_mut(&cluster) {
                    roles.remove(node);
                    if roles.is_empty() {
                        self.can_node_roles.remove(&cluster);
                    }
                }
            }
            role => {
                self.can_node_roles
                    .entry(cluster)
                    .or_default()
                    .insert(node.to_string(), role);
            }
        }

        // Membership comes from the live database, not from each entry's
        // stamped `node`: loading another DBC does not rebuild the generator,
        // so a stamp can name a message this node no longer owns.
        let ids = self.model_tx_ids(channel, node).unwrap_or_default();
        if role == NodeRole::Simulated {
            for id in &ids {
                self.add_entry(channel, *id);
            }
            // 开关刚打开：把该节点已启用条目的排程锚定到当前时钟。开关关闭
            // 期间发送循环不跑、next_t_us 冻结，不锚定会在开关刚打开时把
            // 陈旧排程倾泻成突发。只动排程，绝不改写条目的开/关——
            // 那是用户的逐条自定义，角色切换无权触碰。
            let sim = self.sim_t_us;
            for t in &mut self.tx_list {
                if t.channel == channel && (ids.contains(&t.id) || t.node == node) && t.active {
                    t.next_t_us = sim;
                }
            }
        }
        // 切出「模拟」（监听/离线）：条目开/关原样保留，开关关闭即停发。
        let bus = self.can_name(channel as usize);
        *status = match role {
            NodeRole::Simulated => format!(
                "simulating {node} on {bus} ({} message(s), entries follow their own switches)",
                ids.len()
            ),
            NodeRole::Absent => format!("{node} offline from {bus}"),
        };
        // The role cards read the role map: republish.
        self.nodes_dirty = true;
    }
    /// What the model declares for this message: `Some(0)` for an
    /// event-triggered one, `None` when it says nothing at all.
    pub(crate) fn dbc_cycle_us(&self, ch: u8, id: u32) -> Option<u64> {
        self.model_can_cycle_us(ch, id)
    }

    /// The CAN ids a model-declared sender ECU puts on `ch`, from the frame
    /// ports the document carries.
    pub(super) fn model_tx_ids(&self, ch: u8, node: &str) -> Option<Vec<u32>> {
        let (comm, _, cluster) = self.can_codec(ch)?;
        let mut ids: Vec<u32> = comm
            .frames_sent_by(cluster, node)
            .into_iter()
            .filter_map(|frame| frame.address.can_id())
            .collect();
        ids.sort_unstable();
        ids.dedup();
        Some(ids)
    }

    /// Like [`Self::model_tx_ids`], but only when the model actually knows the
    /// node; an unknown node falls back to the description table's error.
    pub(super) fn model_node_tx_ids(&self, ch: u8, node: &str) -> Option<Vec<u32>> {
        let (comm, _, _) = self.can_codec(ch)?;
        if !comm.ecus.values().any(|ecu| ecu.name == node) {
            return None;
        }
        self.model_tx_ids(ch, node)
    }

    /// The model's name for one CAN frame, if the model knows it.
    pub(crate) fn model_can_name(&self, ch: u8, id: u32, ext: bool) -> Option<String> {
        let (comm, _, cluster) = self.can_codec(ch)?;
        comm.frame_by_can_id(cluster, id)
            .into_iter()
            .find(|frame| {
                matches!(
                    frame.address,
                    crate::model::FrameAddress::Can { extended, .. } if extended == ext
                )
            })
            .map(|frame| frame.name.clone())
    }

    /// Frame name, sender, length and cycle as the model declares them.
    pub(super) fn model_frame_meta(
        &self,
        ch: u8,
        id: u32,
    ) -> Option<(String, String, u8, Option<u64>)> {
        let (comm, _, cluster) = self.can_codec(ch)?;
        let frame = comm.frame_by_can_id(cluster, id).into_iter().next()?;
        Some((
            frame.name.clone(),
            comm.frame_sender(frame).unwrap_or_default().to_string(),
            frame.length.min(crate::can::frame::MAX_CAN_FD_LEN as u32) as u8,
            frame.timing.cycle_us,
        ))
    }

    pub(super) fn model_can_cycle_us(&self, ch: u8, id: u32) -> Option<u64> {
        let comm = self.model_comms.get(ch as usize)?.as_ref()?;
        let cluster = *self.can_cluster_ids.get(ch as usize)?;
        comm.frames_in_cluster(cluster)
            .find(|frame| frame.address.can_id() == Some(id))
            .and_then(|frame| frame.timing.cycle_us)
    }
}
