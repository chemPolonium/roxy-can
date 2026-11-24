//! Publishes runtime state to the frontend: nodes, loads, writes, panel actions.

use super::*;

impl BusCore {
    /// Republishes changed signal caches for the next snapshot; pointer
    /// clones otherwise. One call per step (and after a run reset),
    /// never per frame.
    pub(super) fn refresh_sub_histories(&mut self) {
        for sub in self.subs.values_mut() {
            sub.refresh_published_history();
        }
    }

    /// Republishes the shared ring view. Called on steps that ingested
    /// frames (and on resets); the copy is one sealed tail plus chunk
    /// refcounts -- the ring itself is shared, never deep-copied.
    pub(super) fn expire_nm_states(&mut self, now_us: u64) {
        let timing = self.nm_timing.unwrap_or(crate::script::NmTiming {
            repeat_ms: 3_000,
            ready_sleep_ms: 3_000,
            prepare_sleep_ms: 2_000,
            timeout_ms: 5_000,
        });
        let mut transitions = Vec::new();
        for (key, track) in &mut self.nm_states {
            let next = if now_us.saturating_sub(track.last_seen_us)
                > timing.timeout_ms.saturating_mul(1_000)
            {
                NmState::BusSleep
            } else if track.state == NmState::ReadySleep
                && now_us.saturating_sub(track.since_us)
                    > timing.ready_sleep_ms.saturating_mul(1_000)
            {
                NmState::PrepareBusSleep
            } else if track.state == NmState::PrepareBusSleep
                && now_us.saturating_sub(track.since_us)
                    > timing.prepare_sleep_ms.saturating_mul(1_000)
            {
                NmState::BusSleep
            } else {
                continue;
            };
            if next != track.state {
                track.state = next;
                track.since_us = now_us;
                transitions.push((key.clone(), next));
            }
        }
        for (key, state) in transitions {
            self.write_push(WriteKind::Info, format!("NM {key} state {}", state.label()));
        }
    }

    pub(crate) fn publish_trace(&mut self) {
        let now = self.sim_t_us;
        self.expire_someip_requests(now);
        self.expire_nm_states(now);
        self.published_trace = self.trace.publish();
        self.published_fr = self.fr_trace.publish();
        self.published_eth = self.eth_trace.publish();
    }

    /// One FlexRay frame lands: stamped against the sim clock when the
    /// source had no time of its own, then into the FR ring, the per-frame
    /// aggregates and the cluster's occupancy window. Kept apart from
    /// [`BusCore::ingest`] -- FR rows roll up their own load, not the CAN
    /// per-identifier classes -- but when a FlexRay description database is
    /// loaded their decoded signals fold into subscriptions so the observers
    /// can plot them.
    /// One FlexRay arrival through the whole receive path: triggers first (a
    /// FlexRay edge that starts a recording still captures the frame that fired
    /// it), then the ring and aggregates, then the script handlers waiting on
    /// that slot. Every source of a row -- a replayed log, a watched port, a
    /// generator entry -- goes through here, so none of them can be seen by half
    /// the session.
    pub(super) fn accept_fr_row(
        &mut self,
        row: crate::trace::FrRow,
        now_us: u64,
        listeners: bool,
        queued: &mut Vec<CanFrame>,
        status: &mut String,
    ) {
        self.eval_fr_triggers(&row, status);
        let for_nodes = listeners.then(|| row.clone());
        self.ingest_fr_row(row);
        if let Some(row) = for_nodes {
            self.dispatch_fr_nodes(&row, now_us, queued);
        }
    }

    pub(crate) fn ingest_fr_row(&mut self, mut row: crate::trace::FrRow) {
        if row.t_us == 0 {
            row.t_us = self.sim_t_us;
        }
        // A FlexRay arrival is traffic like any CAN frame: it counts, and an
        // open recording captures it, so a FlexRay session survives as a file
        // that replays and re-exports.
        self.frame_counter += 1;
        if self.recorder.recording {
            self.recorder.write_event(&BusEvent::Fr(&row));
        }
        let ix = self.tally_fr_row(&row);
        if let Some(frame_ix) = ix {
            self.track_fr_e2e(&row, frame_ix);
        }
        // Its share of the medium is one static slot, however long the payload:
        // `gstaticSlot` macroticks of the cluster's own `gmacrotick`. No
        // description, or one without slot timing, and the frame still counts --
        // only the load stays unknown, which the Statistics row prints as "-"
        // rather than as a 0 % it never measured.
        let occupied = self
            .fr_dbs
            .get(&row.bus)
            .and_then(|db| crate::load::fr_slot_wire_us(&db.params));
        self.fr_loads
            .entry(row.bus)
            .or_default()
            .note(row.t_us, occupied);
        self.loads_dirty = true;
        // Fold the frame's FlexRay signals into their subscriptions so the
        // Graphics / Data / State / Monitor observers can plot live values,
        // mirroring [`BusCore::ingest`]'s CAN fold. The row's own cluster
        // supplies the description, because a slot number means nothing on
        // another one; rows for a slot its database has no frame for decode to
        // nothing, which is the same "watch-only" outcome as before.
        if !self.subs.is_empty() {
            let db = self.fr_dbs.get(&row.bus);
            // The frame this row resolved to, by index: the decoders take that
            // index instead of re-finding the declaration.
            let decoded = db.zip(ix).and_then(|(db, i)| {
                db.frame_index(i)
                    .map(|f| (db, i, f.triggering.slot_id as u16))
            });
            if let Some((_, frame_ix, slot)) = decoded {
                let stride = self.applied_stride_us;
                let t_us = row.t_us;
                let values = self.model_fr_values(&row, frame_ix).unwrap_or_default();
                for d in values {
                    let key = self.fr_key(row.bus, slot, d.name);
                    let Some(entry) = self.subs.get_mut(&key) else {
                        continue;
                    };
                    entry.latest = d.phys;
                    entry.last_raw = d.raw;
                    entry.unit = d.unit;
                    entry.type_tag = d.type_tag;
                    entry.label = d.label;
                    entry.last_update_us = t_us;
                    entry.push_sample(t_us, d.phys, stride);
                }
            }
        }
        self.fr_trace.push(row, self.trace_limit);
    }

    /// Folds one arrival into the tally of the **frame** it carried, and
    /// returns the description index that resolved it. A static slot is
    /// scheduled per cycle phase and can belong to several frames in turn, so
    /// `(bus, slot)` alone would mix them into a row whose name changes with
    /// the last arrival and whose period belongs to no frame at all.
    pub(super) fn tally_fr_row(&mut self, row: &crate::trace::FrRow) -> Option<usize> {
        let db = self.fr_dbs.get(&row.bus);
        let ix = db.and_then(|db| db.frame_ix_at(row.slot, row.cycle, row.ab));
        let occupant = match (ix, row.name.as_deref()) {
            (Some(i), _) => crate::aggregate::FrOccupant::Frame(i),
            (None, Some(logical)) => {
                crate::aggregate::FrOccupant::Logged(logical.to_owned().into_boxed_str())
            }
            (None, None) => crate::aggregate::FrOccupant::Unknown,
        };
        let agg = self
            .fr_aggs
            .entry((row.bus, row.slot, occupant.clone()))
            .or_default();
        agg.bus = row.bus;
        agg.slot = row.slot;
        agg.occupant = occupant;
        // Only a strictly later timestamp marks a real cycle (seek /
        // out-of-order rows), and the smoothing mirrors the CAN path.
        if agg.count > 0 && row.t_us > agg.last_t_us {
            let dt = (row.t_us - agg.last_t_us) as f64;
            let prev = agg.cycle_us;
            agg.cycle_us = if agg.count == 1 {
                dt
            } else {
                agg.cycle_us * 0.9 + dt * 0.1
            };
            let dev = (dt - prev).abs();
            agg.jitter_us = if agg.count == 1 {
                dev
            } else {
                agg.jitter_us * 0.9 + dev * 0.1
            };
            // Zero means "no interval yet", so the floor is set by the first
            // real one rather than by a sentinel the views would have to know.
            if agg.min_us == 0.0 || dt < agg.min_us {
                agg.min_us = dt;
            }
            if dt > agg.max_us {
                agg.max_us = dt;
            }
        }
        agg.count += 1;
        agg.last_t_us = row.t_us;
        agg.ab = row.ab;
        // The name follows from the occupant, so the first arrival of this row
        // fixes it for good -- nothing about later frames can make it flip.
        if agg.name.is_none() {
            agg.name = db
                .zip(ix)
                .and_then(|(db, i)| db.frame_index(i))
                .map(|f| f.name.clone())
                .or_else(|| row.name.clone());
        }
        agg.payload = row.payload.clone();
        ix
    }

    /// Re-runs the FlexRay tally over the rows still in the ring. Which frame
    /// an arrival carried is decided by the description of its own cluster at
    /// the moment it is counted, so a description loaded or dropped mid-run has
    /// to redo the tallies it did not see -- otherwise the traffic from before
    /// the file stayed in one unnamed row while the same frame started a second
    /// one below it.
    ///
    /// The price: rows already evicted from the ring cannot be re-sorted, so
    /// loading a description after replaying more than a ring's worth of frames
    /// recounts only what is still on screen. That is the smaller wrong -- the
    /// alternative is a count that mixes two frames' traffic forever.
    pub(super) fn rebuild_fr_aggs(&mut self) {
        // The published view is the ring without the tail copy a re-collect
        // would need; holding it keeps the tally loop from borrowing `self`.
        let rows = self.fr_trace.publish();
        self.fr_aggs.clear();
        for row in rows.iter() {
            self.tally_fr_row(row);
        }
    }

    /// Republishes the load rollups after a step (or a channel add/remove)
    /// touched them. The rollup state itself is a deep copy -- bounded by
    /// one window of frames per bus -- so it only re-clones when changed.
    pub(crate) fn publish_loads(&mut self) {
        if self.loads_dirty {
            self.published_loads = Arc::new(self.bus_loads.clone());
            self.published_fr_loads = Arc::new(self.fr_loads.clone());
            self.loads_dirty = false;
        }
    }

    /// Republishes the node views after a command or a script print
    /// changed something. Alongside the script nodes, one role card per
    /// DBC node per bus (not only transmitters) lands in `group_cards` —
    /// every declared node is visible, which is what lets the user assign
    /// a role to a node that sends nothing yet.
    pub(crate) fn publish_nodes(&mut self) {
        if !self.nodes_dirty {
            return;
        }
        let views: Vec<NodeView> = self
            .nodes
            .iter()
            .map(|n| NodeView {
                id: n.id,
                name: n.name.clone(),
                channel: n.channel,
                source: n.source.clone(),
                enabled: n.enabled,
                file_path: n.file_path.clone(),
                file_source: n.file_source.clone(),
                checked_source: n.checked_source.clone(),
                check_issue: n.check_issue.clone(),
                check_summary: n.check_summary.clone(),
                attached: n.attached.clone(),
                running: n.running(),
                errored: n.errored(),
                issue: n.issue().cloned(),
                tests: n.tests.clone(),
                panel_actions: n.panel_actions().to_vec(),
                log: n.log_snapshot(),
            })
            .collect();
        self.published_nodes = Arc::new(views);
        self.published_blocks = Arc::new(
            self.replay_blocks
                .iter()
                .map(|b| ReplayBlockView {
                    id: b.id,
                    name: b.name.clone(),
                    channel: b.channel,
                    path: b.path.clone(),
                    node_filter: b.node_filter.clone(),
                    attached: b.attached.clone(),
                    ids: b.ids.clone(),
                    enabled: b.enabled,
                    frames: b.queue_len(),
                    last_error: b.last_error.clone(),
                })
                .collect(),
        );
        self.nodes_dirty = false;
    }

    /// Appends one line to the Write ring, stamped with the bus clock and
    /// the local wall clock (the Write window displays the wall time).
    pub(crate) fn write_push(&mut self, kind: WriteKind, text: String) {
        let wall_us = wall_us_now();
        self.write_log.push_back(WriteLine {
            wall_us,
            kind,
            text,
        });
        if self.write_log.len() > WRITE_LOG_CAP {
            self.write_log.pop_front();
        }
        self.write_dirty = true;
    }

    /// Classifies a node log line and mirrors it into the Write ring
    /// under the node's name.
    pub(super) fn write_node_line(&mut self, node_name: &str, line: &str) {
        let kind = if line.starts_with("[error]") {
            WriteKind::Error
        } else if line.starts_with("[check]")
            || line.starts_with("[timer]")
            || line.starts_with("[sysvar]")
            || line.starts_with("[compile]")
            || line.starts_with("[start]")
            || line.starts_with("[tests]")
        {
            WriteKind::Warning
        } else {
            WriteKind::Script
        };
        self.write_push(kind, format!("[{node_name}] {line}"));
    }

    /// Rebuilds the published Write ring after a change.
    pub(crate) fn publish_write(&mut self) {
        if !self.write_dirty {
            return;
        }
        self.published_write = Arc::new(self.write_log.iter().cloned().collect());
        self.write_dirty = false;
    }

    /// Arms every enabled node for a measurement: recompile from source,
    /// run the main chunk, fire `on start`.
    pub(super) fn nodes_start(&mut self) {
        let keys = self.sysvar_keys();
        let codecs = self.can_codec_map();
        for node in &mut self.nodes {
            if node.enabled {
                let dbc = self
                    .model_dbs
                    .get(node.channel as usize)
                    .and_then(|db| db.clone());
                node.sysvar_keys = keys.clone();
                node.start_with(
                    dbc,
                    Some(&self.fr_dbs),
                    Some(&self.eth_dbs),
                    codecs.get(&node.channel).cloned(),
                );
            }
        }
        self.nodes_dirty = true;
    }

    /// The defined system variable keys ("ns::name").
    pub(super) fn sysvar_keys(&self) -> Vec<String> {
        self.sysvars.iter().map(|(d, _)| d.key()).collect()
    }

    /// Pushes the current key set to every node: the start check reports
    /// a script's references outside this set.
    pub(super) fn refresh_node_sysvar_keys(&mut self) {
        let keys = self.sysvar_keys();
        for n in &mut self.nodes {
            n.sysvar_keys = keys.clone();
        }
    }

    /// Node timer handlers that are due at wall clock `now_us`; their
    /// queued frames join this step's buffer. `inputs` carries what
    /// `now()`/`sig()` read, per channel.
    pub(super) fn run_node_timers(&mut self, now_us: u64, inputs: &HashMap<u8, HostInput>) {
        let mut derived: Vec<(u8, u64, String, String, f64)> = Vec::new();
        let mut sys_writes: Vec<(u64, String, f64)> = Vec::new();
        let mut fr_sends: Vec<(u64, u8, u16, Vec<u8>)> = Vec::new();
        let mut fr_sig_writes: Vec<(u64, u8, u16, String, f64)> = Vec::new();
        let mut node_lines: Vec<(String, String)> = Vec::new();
        let mut node_frames: Vec<(u64, u8, bool, crate::script::FrameInstance)> = Vec::new();
        let mut someip_sends: Vec<crate::script::SomeIpSend> = Vec::new();
        let mut someip_replies: Vec<crate::script::SomeIpReply> = Vec::new();
        let mut nm_timings: Vec<crate::script::NmTiming> = Vec::new();
        let mut someip_tcp_sends: Vec<crate::script::SomeIpTcpSend> = Vec::new();
        let mut someip_offers: Vec<crate::script::SomeIpOffer> = Vec::new();
        for node in &mut self.nodes {
            let input = inputs.get(&node.channel).cloned().unwrap_or_default();
            // 绑定脚本的发帧受所属 DBC 节点的角色开关：节点离线时脚本同样
            // 不发送（模拟 = 允许发送）。
            let script_allowed = node.attached.as_ref().is_none_or(|(ach, anode)| {
                Self::role_in(
                    &self.can_cluster_ids,
                    &self.can_node_roles,
                    *ach as usize,
                    anode,
                ) == NodeRole::Simulated
            });
            // Value-change handlers see this step's published inputs; their
            // sends join the same buffer as timer-driven ones.
            let mut sends = node.dispatch_changes(&input);
            sends.extend(node.run_timers(now_us, &input));
            for (id, ext, data, flags) in sends {
                let frame = Self::node_frame(node.channel, id, ext, &data, flags, self.sim_t_us);
                // Real bus 模式下脚本帧与生成器帧一样经已连接的通道发出。
                if script_allowed {
                    self.hw.write_if_live(node.channel, &frame);
                    self.buf.push(frame);
                }
            }
            Self::drain_node_fr(node, &mut fr_sends, &mut fr_sig_writes);
            node_frames.extend(
                node.take_frames()
                    .into_iter()
                    .map(|frame| (node.id, node.channel, script_allowed, frame)),
            );
            someip_sends.extend(node.take_someip());
            someip_replies.extend(node.take_someip_reply());
            nm_timings.extend(node.take_nm_timing());
            someip_tcp_sends.extend(node.take_someip_tcp());
            someip_offers.extend(node.take_someip_offer());
            for (name, v) in node.take_emitted() {
                derived.push((node.channel, node.id, node.name.clone(), name, v));
            }
            for (key, v) in node.take_sys_sets() {
                sys_writes.push((node.id, key, v));
            }
            let node_name = node.name.clone();
            for line in node.take_new_lines() {
                node_lines.push((node_name.clone(), line));
            }
            if node.take_log_if_dirty().is_some() {
                self.nodes_dirty = true;
            }
        }
        self.apply_script_someip(someip_sends);
        self.apply_someip_replies(someip_replies);
        if let Some(timing) = nm_timings.last().copied() {
            self.nm_timing = Some(timing);
        }
        self.apply_script_someip_tcp(someip_tcp_sends);
        self.apply_script_someip_offers(someip_offers);
        for (ch, node_id, node_name, name, v) in derived {
            self.ingest_emitted(ch, node_id, &node_name, &name, v);
        }
        self.apply_sys_writes(sys_writes);
        self.fr_from_scripts.append(&mut fr_sends);
        self.apply_fr_sig_writes(fr_sig_writes);
        self.send_node_frames(node_frames);
        for (node_name, line) in node_lines {
            self.write_node_line(&node_name, &line);
        }
    }

    /// Panel button actions queued since the last step. Each handler runs in
    /// its node's VM with the normal budget; its sends, emits and writes are
    /// collected exactly like a timer handler so they join this same step.
    pub(super) fn run_panel_actions(
        &mut self,
        inputs: &HashMap<u8, HostInput>,
        status: &mut String,
    ) {
        let actions = std::mem::take(&mut self.pending_panel_actions);
        if actions.is_empty() {
            return;
        }
        let mut derived: Vec<(u8, u64, String, String, f64)> = Vec::new();
        let mut sys_writes: Vec<(u64, String, f64)> = Vec::new();
        let mut fr_sends: Vec<(u64, u8, u16, Vec<u8>)> = Vec::new();
        let mut fr_sig_writes: Vec<(u64, u8, u16, String, f64)> = Vec::new();
        let mut node_lines: Vec<(String, String)> = Vec::new();
        let mut missing: Vec<String> = Vec::new();
        let mut node_frames: Vec<(u64, u8, bool, crate::script::FrameInstance)> = Vec::new();
        for (action, value) in actions {
            let mut ran = 0usize;
            for node in &mut self.nodes {
                let input = inputs.get(&node.channel).cloned().unwrap_or_default();
                let (count, node_out) = node.dispatch_panel_action(&action, value, &input);
                if count == 0 {
                    continue;
                }
                ran += count;
                let script_allowed = node.attached.as_ref().is_none_or(|(ach, anode)| {
                    Self::role_in(
                        &self.can_cluster_ids,
                        &self.can_node_roles,
                        *ach as usize,
                        anode,
                    ) == NodeRole::Simulated
                });
                if script_allowed {
                    for (id, ext, data, flags) in &node_out {
                        let frame =
                            Self::node_frame(node.channel, *id, *ext, data, *flags, self.sim_t_us);
                        self.hw.write_if_live(node.channel, &frame);
                        self.buf.push(frame);
                    }
                }
                Self::drain_node_fr(node, &mut fr_sends, &mut fr_sig_writes);
                node_frames.extend(
                    node.take_frames()
                        .into_iter()
                        .map(|frame| (node.id, node.channel, script_allowed, frame)),
                );
                for (name, v) in node.take_emitted() {
                    derived.push((node.channel, node.id, node.name.clone(), name, v));
                }
                for (key, v) in node.take_sys_sets() {
                    sys_writes.push((node.id, key, v));
                }
                let node_name = node.name.clone();
                for line in node.take_new_lines() {
                    node_lines.push((node_name.clone(), line));
                }
                if node.take_log_if_dirty().is_some() {
                    self.nodes_dirty = true;
                }
            }
            if ran == 0 {
                missing.push(action);
            }
        }
        for (ch, node_id, node_name, name, v) in derived {
            self.ingest_emitted(ch, node_id, &node_name, &name, v);
        }
        self.apply_sys_writes(sys_writes);
        self.fr_from_scripts.append(&mut fr_sends);
        self.apply_fr_sig_writes(fr_sig_writes);
        self.send_node_frames(node_frames);
        for (node_name, line) in node_lines {
            self.write_node_line(&node_name, &line);
        }
        if !missing.is_empty() {
            let list = missing
                .iter()
                .take(3)
                .map(|name| format!("\"{name}\""))
                .collect::<Vec<_>>()
                .join(", ");
            *status = format!(
                "[panel] no running script handles {list}{}",
                if missing.len() > 3 { ", …" } else { "" }
            );
        }
    }

    /// Delivers one frame to the matching node handlers; the frames they
    /// queue are returned for the step loop to append to the buffer.
    /// Decodes the current frame's DBC signals and merges them into the
    /// host input so `sig()` in node handlers reads fresh values.
    pub(super) fn dispatch_node_frame(
        &mut self,
        f: &CanFrame,
        input: &HostInput,
    ) -> Vec<crate::script::ScriptSend> {
        let mut out = Vec::new();
        let mut derived: Vec<(u8, u64, String, String, f64)> = Vec::new();
        let mut sys_writes: Vec<(u64, String, f64)> = Vec::new();
        let mut node_lines: Vec<(String, String)> = Vec::new();
        let mut fr_sends: Vec<(u64, u8, u16, Vec<u8>)> = Vec::new();
        let mut fr_sig_writes: Vec<(u64, u8, u16, String, f64)> = Vec::new();
        let mut node_frames: Vec<(u64, u8, bool, crate::script::FrameInstance)> = Vec::new();
        let mut someip_sends: Vec<crate::script::SomeIpSend> = Vec::new();
        let mut someip_replies: Vec<crate::script::SomeIpReply> = Vec::new();
        let mut nm_timings: Vec<crate::script::NmTiming> = Vec::new();
        let mut someip_tcp_sends: Vec<crate::script::SomeIpTcpSend> = Vec::new();
        let mut someip_offers: Vec<crate::script::SomeIpOffer> = Vec::new();
        let data = &f.data[..f.len as usize];
        for node in &mut self.nodes {
            // 绑定脚本的发帧受所属 DBC 节点的角色开关（见 run_node_timers）。
            let script_allowed = node.attached.as_ref().is_none_or(|(ach, anode)| {
                Self::role_in(
                    &self.can_cluster_ids,
                    &self.can_node_roles,
                    *ach as usize,
                    anode,
                ) == NodeRole::Simulated
            });
            let node_out =
                node.dispatch_frame(f.channel, f.id, f.extended, f.is_error(), data, input);
            // The node's own wire egress: Real bus 模式下反应帧与生成器帧
            // 一样经已连接的通道发出。
            if script_allowed {
                for (id, ext, data, flags) in &node_out {
                    let frame = Self::node_frame(node.channel, *id, *ext, data, *flags, f.t_us);
                    self.hw.write_if_live(node.channel, &frame);
                }
            }
            out.extend(node_out);
            Self::drain_node_fr(node, &mut fr_sends, &mut fr_sig_writes);
            node_frames.extend(
                node.take_frames()
                    .into_iter()
                    .map(|frame| (node.id, node.channel, script_allowed, frame)),
            );
            someip_sends.extend(node.take_someip());
            someip_replies.extend(node.take_someip_reply());
            nm_timings.extend(node.take_nm_timing());
            someip_tcp_sends.extend(node.take_someip_tcp());
            someip_offers.extend(node.take_someip_offer());
            for (name, v) in node.take_emitted() {
                derived.push((node.channel, node.id, node.name.clone(), name, v));
            }
            for (key, v) in node.take_sys_sets() {
                sys_writes.push((node.id, key, v));
            }
            let node_name = node.name.clone();
            for line in node.take_new_lines() {
                node_lines.push((node_name.clone(), line));
            }
            if node.take_log_if_dirty().is_some() {
                self.nodes_dirty = true;
            }
        }
        self.apply_script_someip(someip_sends);
        self.apply_someip_replies(someip_replies);
        if let Some(timing) = nm_timings.last().copied() {
            self.nm_timing = Some(timing);
        }
        self.apply_script_someip_tcp(someip_tcp_sends);
        self.apply_script_someip_offers(someip_offers);
        for (ch, node_id, node_name, name, v) in derived {
            self.ingest_emitted(ch, node_id, &node_name, &name, v);
        }
        self.apply_sys_writes(sys_writes);
        self.fr_from_scripts.append(&mut fr_sends);
        self.apply_fr_sig_writes(fr_sig_writes);
        self.send_node_frames(node_frames);
        for (node_name, line) in node_lines {
            self.write_node_line(&node_name, &line);
        }
        out
    }

    /// Delivers one FlexRay arrival to every node's `on fr slot` handlers --
    /// the receiving half of what [`Self::dispatch_node_frame`] is for a CAN
    /// frame. Called after the row's ingest, so `fr_sig()` inside a handler
    /// reads the values this very frame carried, the same ordering the CAN walk
    /// uses for `sig()`.
    ///
    /// A node's own CAN channel is not a filter here: an arrival has none, and
    /// the cluster the handler names is the only address that means anything.
    /// What a handler sends still leaves on that channel, which is the gateway
    /// shape -- FlexRay in, CAN out -- and the only direction our hardware
    /// path supports today. Queued frames come back for the caller to put on
    /// the internal bus, stamped with the arrival's own time.
    pub(super) fn dispatch_fr_nodes(
        &mut self,
        row: &crate::trace::FrRow,
        now_us: u64,
        queued: &mut Vec<CanFrame>,
    ) {
        let inputs = self.build_node_inputs(now_us);
        let mut derived: Vec<(u8, u64, String, String, f64)> = Vec::new();
        let mut sys_writes: Vec<(u64, String, f64)> = Vec::new();
        let mut node_lines: Vec<(String, String)> = Vec::new();
        let mut fr_sends: Vec<(u64, u8, u16, Vec<u8>)> = Vec::new();
        let mut fr_sig_writes: Vec<(u64, u8, u16, String, f64)> = Vec::new();
        let mut node_frames: Vec<(u64, u8, bool, crate::script::FrameInstance)> = Vec::new();
        for node in &mut self.nodes {
            if !node.waits_on_flexray() {
                continue;
            }
            // The same role gate as the CAN path: a script bound to a DBC node
            // whose ECU is not simulated does not get to answer either bus.
            let script_allowed = node.attached.as_ref().is_none_or(|(ach, anode)| {
                Self::role_in(
                    &self.can_cluster_ids,
                    &self.can_node_roles,
                    *ach as usize,
                    anode,
                ) == NodeRole::Simulated
            });
            let input = inputs.get(&node.channel).cloned().unwrap_or_default();
            let node_out = node.dispatch_fr_row(row.bus, row.slot, row.cycle, &row.payload, &input);
            for (id, ext, data, flags) in &node_out {
                let frame = Self::node_frame(node.channel, *id, *ext, data, *flags, row.t_us);
                if script_allowed {
                    self.hw.write_if_live(node.channel, &frame);
                }
                queued.push(frame);
            }
            Self::drain_node_fr(node, &mut fr_sends, &mut fr_sig_writes);
            node_frames.extend(
                node.take_frames()
                    .into_iter()
                    .map(|frame| (node.id, node.channel, script_allowed, frame)),
            );
            for (name, v) in node.take_emitted() {
                derived.push((node.channel, node.id, node.name.clone(), name, v));
            }
            for (key, v) in node.take_sys_sets() {
                sys_writes.push((node.id, key, v));
            }
            let node_name = node.name.clone();
            for line in node.take_new_lines() {
                node_lines.push((node_name.clone(), line));
            }
            if node.take_log_if_dirty().is_some() {
                self.nodes_dirty = true;
            }
        }
        for (ch, node_id, node_name, name, v) in derived {
            self.ingest_emitted(ch, node_id, &node_name, &name, v);
        }
        self.apply_sys_writes(sys_writes);
        self.fr_from_scripts.append(&mut fr_sends);
        self.apply_fr_sig_writes(fr_sig_writes);
        self.send_node_frames(node_frames);
        for (node_name, line) in node_lines {
            self.write_node_line(&node_name, &line);
        }
    }

    /// Applies the system variable writes a handler run queued: clamp
    /// against the definition, publish to the observers, and report an
    /// undefined key to the writing node's log.
    pub(super) fn apply_sys_writes(&mut self, writes: Vec<(u64, String, f64)>) {
        for (node_id, key, v) in writes {
            if !self.sysvar_write(&key, v)
                && let Some(n) = self.nodes.iter_mut().find(|n| n.id == node_id)
            {
                n.note(format!("[sysvar] undefined, write dropped: \"{key}\""));
            }
        }
    }

    /// Drains one node's FlexRay queues into the step's two collectors. The
    /// three dispatch paths (node timers, a CAN arrival, a FlexRay arrival) do
    /// the same thing, and this cannot be a `&mut self` method because the
    /// caller is already iterating `&mut self.nodes`.
    pub(super) fn drain_node_fr(
        node: &mut crate::node::ScriptNode,
        sends: &mut Vec<(u64, u8, u16, Vec<u8>)>,
        sig_writes: &mut Vec<(u64, u8, u16, String, f64)>,
    ) {
        let id = node.id;
        for (bus, slot, data) in node.take_fr_out() {
            sends.push((id, bus, slot, data));
        }
        for (bus, slot, name, v) in node.take_fr_sig_sets() {
            sig_writes.push((id, bus, slot, name, v));
        }
    }

    /// Applies the `set_fr_sig` writes a handler run queued: encode one signal
    /// into the generator entry that fills this slot, through the description
    /// this 路 carries. A refusal is advice, not a fault -- the node keeps
    /// running and the reason goes to its log, the same shape an undefined
    /// system variable write takes.
    pub(super) fn apply_fr_sig_writes(&mut self, writes: Vec<(u64, u8, u16, String, f64)>) {
        for (node_id, bus, slot, name, v) in writes {
            let ix = self
                .fr_tx_list
                .iter()
                .position(|t| t.bus == bus && t.slot == slot);
            let reason = match (self.fr_dbs.get(&bus).cloned(), ix) {
                (None, _) => Some(format!(
                    "FR{bus} has no cluster description: the description gives each signal its place in the payload"
                )),
                (Some(_), None) => Some(format!(
                    "FR{bus} slot {slot} has no transmit entry: set_fr_sig edits the entry this bus sends"
                )),
                (Some(_), Some(ix)) => {
                    // The model owns the signal's place in the frame; a bus
                    // without one cannot encode a named signal.
                    let model = self.fr_models.get(&bus).cloned().and_then(|runtime| {
                        let cluster = self.fr_cluster_ids.get(&bus).copied()?;
                        Some((cluster, runtime))
                    });
                    match model {
                        Some((cluster, runtime)) => crate::generator::fr_tx_set_signal_with_model(
                            &mut self.fr_tx_list[ix],
                            cluster,
                            &runtime.comm,
                            &runtime.plans,
                            &name,
                            v,
                        )
                        .err(),
                        None => Some(format!(
                            "FR{bus} has no model: the signal cannot be placed in the payload"
                        )),
                    }
                }
            };
            if let Some(reason) = reason
                && let Some(n) = self.nodes.iter_mut().find(|n| n.id == node_id)
            {
                n.note(format!("[set_fr_sig] write dropped: {reason}"));
            }
        }
    }

    /// Stores one system variable write, clamped to its bounds, and
    /// publishes the sample to the observers under the sysvar stream id.
    /// Returns false when no such variable is defined.
    pub(super) fn sysvar_write(&mut self, key: &str, value: f64) -> bool {
        let Some(i) = self.sysvars.iter().position(|(d, _)| d.key() == key) else {
            return false;
        };
        let (def, v) = &mut self.sysvars[i];
        *v = def.clamp(value);
        let (ns, name, val) = (def.namespace.clone(), def.name.clone(), *v);
        self.ingest_sysvar(&ns, &name, val);
        true
    }

    /// Publishes one system variable value to the observers. Each
    /// variable is a synthetic stream grouped under its namespace, the
    /// same tree Data and Graphics browse.
    pub(super) fn ingest_sysvar(&mut self, namespace: &str, name: &str, v: f64) {
        self.ingest_emitted(0, SYSVAR_STREAM_ID, namespace, name, v);
    }

    /// Run start: every system variable returns to its declared init
    /// value and republishes, so observers never plot a stale tail.
    pub(super) fn init_sysvars(&mut self) {
        let defs: Vec<(String, String, f64)> = self
            .sysvars
            .iter()
            .map(|(d, _)| (d.namespace.clone(), d.name.clone(), d.init))
            .collect();
        for (d, v) in &mut self.sysvars {
            *v = d.init;
        }
        for (ns, name, v) in defs {
            self.ingest_sysvar(&ns, &name, v);
        }
    }

    /// Folds one `emit_value` sample into its synthetic subscription,
    /// creating the stream on first emission. The key carries
    /// [`crate::app::EMITTED_ID_BASE`] in its id, so a derived signal can
    /// never collide with a real frame's signals; the selection tree lists
    /// these streams under the owning node's name.
    pub(super) fn ingest_emitted(
        &mut self,
        ch: u8,
        node_id: u64,
        node_name: &str,
        name: &str,
        v: f64,
    ) {
        let key = self.can_key(
            ch,
            crate::app::EMITTED_ID_BASE | node_id as u32,
            false,
            name.to_string(),
        );
        if !self.emitted_streams.iter().any(|(k, _)| k == &key) {
            // A script computing names in a loop would otherwise balloon
            // the registry, the selection tree, and the subscription map
            // with one-shot streams. The cap is generous for real derived
            // signals and keeps a runaway script cheap.
            if self.emitted_streams.len() >= MAX_EMITTED_STREAMS {
                return;
            }
            self.emitted_streams
                .push((key.clone(), node_name.to_string()));
            self.nodes_dirty = true;
        }
        self.subscribe_signal(key.clone());
        let stride = self.applied_stride_us;
        let t_us = self.sim_t_us;
        let Some(entry) = self.subs.get_mut(&key) else {
            return;
        };
        entry.latest = v;
        entry.last_raw = v as i64;
        entry.last_update_us = t_us;
        entry.push_sample(t_us, v, stride);
    }

    /// Builds one [`HostInput`] per channel that has nodes: the bus clock
    /// in seconds plus every decoded signal value seen on that channel
    /// (from the aggregates, so it covers replay and live alike).
    pub(super) fn build_node_inputs(&self, now_us: u64) -> HashMap<u8, HostInput> {
        let mut inputs: HashMap<u8, HostInput> = HashMap::new();
        for node in &self.nodes {
            inputs.entry(node.channel).or_insert_with(|| HostInput {
                now_s: now_us as f64 / 1e6,
                signals: HashMap::new(),
                fr_signals: HashMap::new(),
                eth_signals: HashMap::new(),
                sysvars: HashMap::new(),
                derived_signals: HashMap::new(),
            });
        }
        // System variables are global: every channel's nodes read the
        // same live set.
        let mut sysmap: HashMap<String, f64> = HashMap::with_capacity(self.sysvars.len());
        for (d, v) in &self.sysvars {
            sysmap.insert(d.key(), *v);
        }
        for input in inputs.values_mut() {
            input.sysvars = sysmap.clone();
        }
        for agg in self.aggs.values() {
            let Some(input) = inputs.get_mut(&agg.channel) else {
                continue;
            };
            let f = CanFrame {
                t_us: agg.last_t_us,
                channel: agg.channel,
                id: agg.id,
                extended: agg.extended,
                len: agg.len,
                data: agg.data,
                dir: agg.dir,
                flags: agg.flags,
            };
            let decoded = self.model_can_decoded(&f, false).unwrap_or_default();
            for d in decoded {
                input.signals.insert((agg.id, d.name), d.phys);
            }
        }
        // The FlexRay half of the same read. `fr_sig` addresses a signal by
        // (cluster, slot, name), and the value is the one the frame that last
        // held that slot decoded to -- a slot is shared by several frames in
        // turn, so the arriving row's own frame is the only layout that means
        // anything. Decoded from the per-frame tallies rather than folded in on
        // ingest, so a run with no script nodes pays nothing for a capability
        // nobody calls; with nodes, this is the same per-build walk the CAN
        // branch above already does over its own aggregates.
        if !inputs.is_empty() {
            for ((bus, slot, occupant), agg) in &self.fr_aggs {
                let Some(frame_ix) = occupant.frame_ix() else {
                    continue;
                };
                let row = crate::trace::FrRow {
                    bus: *bus,
                    t_us: agg.last_t_us,
                    ab: 0,
                    slot: *slot,
                    cycle: 0,
                    payload: agg.payload.to_vec(),
                    header_crc: 0,
                    flags: 0,
                    name: None,
                };
                let decoded = self
                    .model_fr_decoded(&row, frame_ix, false)
                    .unwrap_or_default();
                for d in decoded {
                    let key = (*bus, *slot, d.name);
                    for input in inputs.values_mut() {
                        input.fr_signals.insert(key.clone(), d.phys);
                    }
                }
            }
        }
        // Ethernet signals follow the same rule as the FlexRay block: one
        // read per (bus, frame, signal), decoded from the flow tally against
        // that bus's ARXML. `eth_sig`/`ETHn.Frame.Signal` address exactly
        // this key, so an unmatched flow stays invisible rather than
        // guessed into a frame definition.
        if !inputs.is_empty() {
            for (key, agg) in &self.eth_aggs {
                let Some(db) = self.eth_dbs.get(&key.bus) else {
                    continue;
                };
                let flow = key.flow();
                let frame = match db.match_flow(&flow, self.eth_length_includes_header) {
                    crate::eth::db::EthMatch::Unique(index) => db.frames[index].name.clone(),
                    _ => continue,
                };
                let Some(decoded) = self.model_eth_decoded(key.bus, &frame, &agg.payload, false)
                else {
                    continue;
                };
                for d in decoded {
                    let key = (key.bus, frame.clone(), d.name);
                    for input in inputs.values_mut() {
                        input.eth_signals.insert(key.clone(), d.phys);
                    }
                }
            }
        }
        // Derived lanes are global, so every channel's nodes see the same
        // table. The value is the newest sample the core took at the end of
        // the previous lap -- `derived("Name")` in a step therefore reads the
        // step's inputs, not a value computed after the step began.
        if !inputs.is_empty() && !self.derived.is_empty() {
            let derived: HashMap<String, f64> = self
                .derived
                .iter()
                .filter_map(|runtime| {
                    let sub = self.subs.get(&runtime.key)?;
                    (!sub.history.is_empty()).then(|| (runtime.def.name.clone(), sub.latest))
                })
                .collect();
            for input in inputs.values_mut() {
                input.derived_signals = derived.clone();
            }
        }
        inputs
    }

    /// Frames the script queued: `dir` Tx, stamped on the bus's own
    /// timeline; an id above 0x7FF travels extended unless the script
    /// forced the class the other way (`send_ext`). A payload past 8
    /// bytes becomes a CAN FD frame, snapped to a valid FD length exactly
    /// like the generator's own emit path.
    pub(super) fn node_frame(
        channel: u8,
        id: u32,
        force_extended: bool,
        data: &[u8],
        send_flags: u8,
        t_us: u64,
    ) -> CanFrame {
        use crate::can::frame::{FrameFlags, dlc2len, len2dlc};
        let data_len = data.len().min(crate::can::frame::MAX_CAN_FD_LEN);
        let force_fd = send_flags & crate::script::CAN_SEND_FD != 0;
        let brs = send_flags & crate::script::CAN_SEND_BRS != 0;
        let (len, mut flags) = if force_fd || brs || data_len > 8 {
            (dlc2len(len2dlc(data_len as u8)), FrameFlags::FD)
        } else {
            (data_len as u8, FrameFlags::NONE)
        };
        if flags.contains(FrameFlags::FD) && brs {
            flags = flags.union(FrameFlags::BRS);
        }
        let mut buf = [0u8; crate::can::frame::MAX_CAN_FD_LEN];
        buf[..data_len].copy_from_slice(&data[..data_len]);
        CanFrame {
            t_us,
            channel,
            id,
            extended: force_extended || id > 0x7FF,
            len,
            data: buf,
            dir: Direction::Tx,
            flags,
        }
    }

    /// The frame read plus the text the just-drained commands produced.
    /// This is the form the publisher hands to the mailbox.
    pub(crate) fn snapshot_with_status(&self, status: Option<String>) -> Snapshot {
        let mut snap = self.snapshot();
        snap.status = status;
        snap
    }
}
