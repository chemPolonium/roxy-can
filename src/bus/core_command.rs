//! Command entry point: construction and the command dispatcher.

use super::*;

impl BusCore {
    /// Empty bus state; `App::new` layers the sample channel configuration
    /// and the measurement start on top.
    pub(crate) fn new(bus_loads: Vec<crate::load::BusLoad>) -> Self {
        let published_loads = Arc::new(bus_loads.clone());
        BusCore {
            mode: Mode::Virtual,
            run_mode: Mode::Virtual,
            source: Box::new(crate::source::virtual_source::VirtualSource::new()),
            replay_ids: std::collections::HashSet::new(),
            replay_fr_ids: std::collections::HashSet::new(),
            log_ids_file: std::collections::HashSet::new(),
            log_fr_ids_file: std::collections::HashSet::new(),
            replay_map: crate::replay_map::ReplayMap::default(),
            log_census: crate::replay_map::LogCensus::default(),
            drop_news_done: std::collections::HashSet::new(),
            can_sums: Default::default(),
            can_node_roles: Default::default(),
            model_comms: Vec::new(),
            model_plans: Vec::new(),
            model_dbs: Vec::new(),
            can_xml_models: Vec::new(),
            cluster_ids: Default::default(),
            can_cluster_ids: Vec::new(),
            fr_cluster_ids: Default::default(),
            eth_cluster_ids: Default::default(),
            fr_models: Default::default(),
            eth_models: Default::default(),
            bus_counter: 2,
            tx_list: Vec::new(),
            fr_tx_list: Vec::new(),
            sim_t_us: 0,
            sim_prev_us: 0,
            trace: crate::trace::TraceRing::default(),
            published_trace: Arc::new(crate::trace::TraceView::default()),
            fr_trace: crate::trace::FrRing::default(),
            published_fr: Arc::new(crate::trace::FrTraceView::default()),
            eth_trace: crate::eth::EthRing::default(),
            someip_events: std::collections::VecDeque::new(),
            someip_tcp: crate::eth::someip::SomeIpTcp::default(),
            someip_tp: crate::eth::someip::SomeIpTp::default(),
            someip_ip: crate::eth::someip::SomeIpIp::default(),
            someip_services: Default::default(),
            someip_sessions: Default::default(),
            someip_pending: Vec::new(),
            someip_offers: Default::default(),
            someip_subscriptions: Vec::new(),
            someip_replies: Default::default(),
            nm_timing: None,
            nm_last: Default::default(),
            nm_states: Default::default(),
            someip_send_guard: false,
            published_eth: Arc::new(crate::eth::EthTraceView::default()),
            fr_aggs: HashMap::new(),
            eth_aggs: HashMap::new(),
            fr_dbs: Default::default(),
            fr_sources: Default::default(),
            eth_dbs: Default::default(),
            eth_sources: Default::default(),
            model_issues_seen: Default::default(),
            fr_tx_errors: Default::default(),
            e2e_tracks: Default::default(),
            fr_e2e_tracks: Default::default(),
            eth_e2e_tracks: Default::default(),
            eth_length_includes_header: false,
            trace_paused: false,
            paused_at_us: None,
            replay_stop_at: None,
            last_stride_us: SAMPLE_INTERVAL_US,
            last_tol_pct: 0,
            last_grace: 0,
            frame_counter: 0,
            aggs: HashMap::new(),
            expanded_messages: Default::default(),
            bus_loads,
            fr_loads: Default::default(),
            injected: Vec::new(),
            fr_from_scripts: Vec::new(),
            panel_fr_frames: Vec::new(),
            panel_frames: Vec::new(),
            published_loads,
            published_fr_loads: Arc::new(Default::default()),
            loads_dirty: false,
            subs: HashMap::new(),
            derived: Vec::new(),
            markers: Vec::new(),
            pre_buffer: std::collections::VecDeque::new(),
            post_roll: None,
            applied_stride_us: SAMPLE_INTERVAL_US,
            measuring: false,
            buf: Vec::new(),
            recorder: crate::recorder::Recorder::new(),
            triggers: Vec::new(),
            trigger_fires: std::collections::VecDeque::new(),
            spec: crate::spec::Spec::default(),
            laps: 0,
            nodes: Vec::new(),
            pending_panel_actions: Vec::new(),
            node_counter: 0,
            published_nodes: Arc::new(Vec::new()),
            published_blocks: Arc::new(Vec::new()),
            nodes_dirty: false,
            replay_blocks: Vec::new(),
            block_counter: 0,
            emitted_streams: Vec::new(),
            sysvars: Vec::new(),
            write_log: VecDeque::new(),
            published_write: Arc::new(Vec::new()),
            write_dirty: false,
            hw: crate::hw::Hardware::new(),
            trace_limit: TRACE_LIMIT,
            pre_frames: PRE_BUFFER_FRAMES,
            post_frames: POST_ROLL_FRAMES,
            marker_cap: MARKER_CAP,
        }
    }

    /// Executes one command from the frontend. Status text an action
    /// produces goes into `status` -- the bus has no display of its own.
    /// Every command bumps the lap counter so a caller can see progress
    /// through the snapshot.
    pub(crate) fn handle(&mut self, cmd: BusCommand, status: &mut String) {
        self.laps += 1;
        match cmd {
            BusCommand::StartVirtual => {
                self.pending_panel_actions.clear();
                self.start_virtual(status)
            }
            BusCommand::Stop => {
                self.pending_panel_actions.clear();
                self.stop_bus(status)
            }
            BusCommand::SetTracePaused(on) => {
                self.trace_paused = on;
                // A manual pause/resume abandons a pending play-to target.
                self.replay_stop_at = None;
                self.source.set_poll_limit(None);
            }
            BusCommand::ReplayStep { delta_us } => self.replay_step(delta_us, status),
            BusCommand::ReplayPlayTo { target_us } => self.replay_play_to(target_us, status),
            BusCommand::ReplayPlayFor { duration_us } => self.replay_play_for(duration_us, status),
            BusCommand::ToggleRecord => self.toggle_record(status),
            BusCommand::RearmTriggers => self.rearm_latched_triggers(status),
            BusCommand::ClearTrace => self.clear_trace_view(status),
            BusCommand::PanelAction { name, value } => {
                if !self.measuring {
                    *status = format!("[panel] \"{name}\" needs a running measurement");
                } else if self.pending_panel_actions.len() < 32 {
                    self.pending_panel_actions.push((name, value));
                } else {
                    *status = "[panel] too many queued actions".to_string();
                }
            }
            BusCommand::PanelWriteSignal { key, value } => {
                if !self.measuring {
                    *status = "[panel] writing a signal needs a running measurement".to_string();
                } else if let Err(error) = self.apply_panel_write(key, value) {
                    *status = format!("[panel] {error}");
                }
            }
            BusCommand::PanelSendFrame {
                channel,
                id,
                extended,
                data,
                physical,
                signals,
            } => {
                if !self.measuring {
                    *status = "[panel] sending a frame needs a running measurement".to_string();
                } else if channel as usize >= self.can_count() {
                    *status = format!("[panel] CAN channel {channel} does not exist");
                } else if extended && id > 0x1fff_ffff {
                    *status = "[panel] extended IDs stop at 0x1FFFFFFF".to_string();
                } else if !extended && id > 0x7ff {
                    *status = "[panel] standard IDs stop at 0x7FF".to_string();
                } else if physical {
                    match self.panel_can_signal_payload(channel, id, extended, &signals) {
                        Ok(payload) => self.panel_frames.push((channel, id, extended, payload)),
                        Err(error) => *status = format!("[panel] {error}"),
                    }
                } else if !panel_frame_length_ok(data.len()) {
                    *status = "[panel] CAN FD data must be 0-8, 12, 16, 20, 24, 32, 48 or 64 bytes"
                        .to_string();
                } else {
                    self.panel_frames.push((channel, id, extended, data));
                }
            }
            BusCommand::PanelSendFrFrame {
                bus,
                slot,
                data,
                physical,
                signals,
            } => {
                if !self.measuring {
                    *status = "[panel] sending a frame needs a running measurement".to_string();
                } else if !(1..=2047).contains(&slot) {
                    *status = "[panel] FlexRay slot must be 1..2047".to_string();
                } else if physical {
                    match self.panel_fr_signal_payload(bus, slot, &signals) {
                        Ok((payload, cycle)) => {
                            self.panel_fr_frames.push((bus, slot, payload, cycle))
                        }
                        Err(error) => *status = format!("[panel] {error}"),
                    }
                } else if data.len() > crate::generator::MAX_FR_PAYLOAD_LEN {
                    *status = format!(
                        "[panel] FlexRay data must be at most {} bytes",
                        crate::generator::MAX_FR_PAYLOAD_LEN
                    );
                } else {
                    let Some(db) = self.fr_dbs.get(&bus).cloned() else {
                        *status = format!("[panel] FR{bus} has no cluster description");
                        return;
                    };
                    let frame_ix = db.frame_ix_of_slot(slot);
                    let declared = frame_ix
                        .and_then(|ix| db.frame_index(ix))
                        .map(|frame| frame.length as usize);
                    let payload = match declared {
                        Some(width) if data.len() > width => {
                            *status = format!("[panel] FR{bus} slot {slot} carries {width} bytes");
                            return;
                        }
                        Some(width) => {
                            let mut payload = data;
                            payload.resize(width, 0);
                            payload
                        }
                        None => data,
                    };
                    let now_cycle = crate::generator::fr_cycle_at(&db, self.sim_t_us);
                    let cycle = if db.frame_ix_at(slot, now_cycle, 2).is_some() {
                        now_cycle
                    } else {
                        frame_ix
                            .and_then(|ix| db.frame_index(ix))
                            .map(|frame| frame.triggering.base_cycle as u8)
                            .unwrap_or(now_cycle)
                    };
                    self.panel_fr_frames.push((bus, slot, payload, cycle));
                }
            }
            BusCommand::PanelSendEthFrame {
                bus,
                frame,
                data,
                physical,
                signals,
            } => {
                if !self.measuring {
                    *status = "[panel] sending a frame needs a running measurement".to_string();
                } else if physical {
                    let Some(db) = self.eth_dbs.get(&bus).cloned() else {
                        *status = format!("[panel] ETH{bus} has no matrix description");
                        return;
                    };
                    let Some(def) = db.frames.iter().find(|entry| entry.name == frame) else {
                        *status = format!("[panel] unknown Ethernet frame \"{frame}\"");
                        return;
                    };
                    let payload = match self.panel_eth_signal_payload(bus, &db, &frame, &signals) {
                        Ok(payload) => payload,
                        Err(error) => {
                            *status = format!("[panel] {error}");
                            return;
                        }
                    };
                    let injected = crate::eth::EthFrame::session(
                        self.sim_t_us,
                        bus,
                        def.vlan,
                        def.ethertype_hint
                            .unwrap_or(crate::eth::EthFrame::SESSION_ETHERTYPE),
                        payload,
                    );
                    if let Err(error) = self.emit_eth(injected) {
                        *status = format!("[panel] {error}");
                    }
                } else if let Err(error) = self.send_eth_frame(bus, &frame, &data) {
                    *status = format!("[panel] {error}");
                }
            }
            BusCommand::PanelSendSomeIp {
                bus,
                service,
                method,
                client,
                session,
                message_type,
                data,
                values,
                texts,
                transport,
                host,
                port,
            } => {
                if !self.measuring {
                    *status =
                        "[panel] sending a SOME/IP message needs a running measurement".to_string();
                } else {
                    let payload = if values.is_empty() && texts.is_empty() {
                        Ok(data)
                    } else {
                        self.encode_someip_payload_with_text(
                            service,
                            method,
                            message_type,
                            &values,
                            &texts,
                        )
                    };
                    match payload {
                        Ok(payload) => {
                            let session =
                                self.next_someip_session(bus, service, method, client, session);
                            self.note_someip_request(
                                bus,
                                service,
                                method,
                                client,
                                session,
                                message_type,
                            );
                            let frame = crate::eth::someip::build_udp_frame(
                                self.sim_t_us,
                                bus,
                                service,
                                method,
                                client,
                                session,
                                message_type,
                                &payload,
                            );
                            if transport.eq_ignore_ascii_case("tcp") {
                                if let Err(error) = send_someip_tcp(
                                    &host,
                                    port,
                                    service,
                                    method,
                                    client,
                                    session,
                                    message_type,
                                    &payload,
                                ) {
                                    *status = format!("[panel] {error}");
                                }
                            } else {
                                self.someip_send_guard = true;
                                if let Err(error) = self.emit_eth(frame) {
                                    *status = format!("[panel] {error}");
                                }
                                self.someip_send_guard = false;
                            }
                        }
                        Err(error) => *status = format!("[panel] {error}"),
                    }
                }
            }
            BusCommand::SetSomeIpServices(catalog) => self.adopt_someip_services(catalog),
            BusCommand::ClearAggregates => self.clear_aggregates(status),
            BusCommand::SetRunMode(mode) => self.run_mode = mode,
            BusCommand::AddChannel => self.add_channel(status),
            BusCommand::AddEmptyChannel => self.add_empty_channel(status),
            BusCommand::RemoveChannel { ch } => self.remove_channel(ch, status),
            BusCommand::SetNodeRole { ch, node, role } => {
                self.set_node_role(ch, &node, role, status)
            }
            BusCommand::AddNode {
                name,
                channel,
                attached,
            } => {
                self.node_counter += 1;
                let id = self.node_counter;
                self.nodes
                    .push(crate::node::ScriptNode::new(id, name, channel));
                let node_channel = attached
                    .as_ref()
                    .map(|(bound, _)| *bound)
                    .unwrap_or(channel);
                let last = self.nodes.last_mut().expect("just added");
                last.attached = attached.clone();
                // A bound script lives on its node's bus: the binding, not
                // the passed-in channel, decides the wire (and the gate).
                last.channel = node_channel;
                if self.measuring {
                    let codecs = self.can_codec_map();
                    let dbc = self
                        .model_dbs
                        .get(channel as usize)
                        .and_then(|db| db.clone());
                    let keys = self.sysvar_keys();
                    let last = self.nodes.last_mut().expect("just added");
                    last.sysvar_keys = keys;
                    last.start_with(
                        dbc,
                        Some(&self.fr_dbs),
                        Some(&self.eth_dbs),
                        codecs.get(&node_channel).cloned(),
                    );
                }
                self.nodes_dirty = true;
            }
            BusCommand::RemoveNode { id } => {
                self.nodes.retain(|n| n.id != id);
                // The removed node's derived-signal streams have no future
                // emitter; drop them from the selection tree. Existing
                // window selections keep working like after a DBC swap.
                self.emitted_streams
                    .retain(|(k, _)| emitted_node_id(k) != Some(id));
                self.nodes_dirty = true;
            }
            BusCommand::SetNodeName { id, name } => {
                if let Some(n) = self.nodes.iter_mut().find(|n| n.id == id) {
                    n.name = name.clone();
                    // The selection tree lists streams under the owner's
                    // name: a rename rewrites the owner, not the key.
                    for (k, owner) in &mut self.emitted_streams {
                        if emitted_node_id(k) == Some(id) {
                            *owner = name.clone();
                        }
                    }
                    self.nodes_dirty = true;
                }
            }
            BusCommand::SetNodeSource { id, source } => {
                let keys = self.sysvar_keys();
                let codec = self
                    .nodes
                    .iter()
                    .find(|n| n.id == id)
                    .and_then(|n| self.can_codec(n.channel));
                if let Some(n) = self.nodes.iter_mut().find(|n| n.id == id) {
                    let dbc = self
                        .model_dbs
                        .get(n.channel as usize)
                        .and_then(|db| db.clone());
                    n.sysvar_keys = keys;
                    n.set_source(
                        source,
                        dbc,
                        Some(&self.fr_dbs),
                        Some(&self.eth_dbs),
                        codec,
                        self.measuring,
                    );
                    self.nodes_dirty = true;
                }
            }
            BusCommand::SetNodeFile {
                id,
                path,
                file_source,
            } => {
                if let Some(node) = self.nodes.iter_mut().find(|node| node.id == id) {
                    node.set_file(path, file_source);
                    self.nodes_dirty = true;
                }
            }
            BusCommand::CheckNodeSource { id, source } => {
                if let Some(node) = self.nodes.iter_mut().find(|node| node.id == id) {
                    let dbc = self
                        .model_dbs
                        .get(node.channel as usize)
                        .and_then(|db| db.clone());
                    node.check_source(source, dbc, Some(&self.fr_dbs), Some(&self.eth_dbs));
                    self.nodes_dirty = true;
                }
            }
            BusCommand::SetNodeEnabled { id, on } => {
                let keys = self.sysvar_keys();
                let codec = self
                    .nodes
                    .iter()
                    .find(|n| n.id == id)
                    .and_then(|n| self.can_codec(n.channel));
                if let Some(n) = self.nodes.iter_mut().find(|n| n.id == id) {
                    let dbc = self
                        .model_dbs
                        .get(n.channel as usize)
                        .and_then(|db| db.clone());
                    n.sysvar_keys = keys;
                    n.set_enabled(
                        on,
                        dbc,
                        Some(&self.fr_dbs),
                        Some(&self.eth_dbs),
                        codec,
                        self.measuring,
                    );
                    self.nodes_dirty = true;
                }
            }
            BusCommand::SetNodes { nodes } => {
                let keys = self.sysvar_keys();
                let codecs = self.can_codec_map();
                self.nodes = nodes
                    .into_iter()
                    .map(|cfg| {
                        self.node_counter += 1;
                        let mut n =
                            crate::node::ScriptNode::new(self.node_counter, cfg.name, cfg.channel);
                        n.source = cfg.source.clone();
                        n.file_path = Some(cfg.path.clone());
                        n.file_source = cfg.source;
                        n.enabled = cfg.enabled;
                        if let Some(message) = cfg.load_error {
                            n.mark_load_error(message);
                        }
                        // The saved binding is the whole point: without it
                        // every restored script would look like an orphan
                        // and get adopted by an arbitrary node.
                        n.attached = cfg.attached.clone();
                        if let Some((ach, _)) = &n.attached {
                            n.channel = *ach;
                        }
                        if self.measuring && n.enabled {
                            let dbc = self
                                .model_dbs
                                .get(n.channel as usize)
                                .and_then(|db| db.clone());
                            n.sysvar_keys = keys.clone();
                            n.start_with(
                                dbc,
                                Some(&self.fr_dbs),
                                Some(&self.eth_dbs),
                                codecs.get(&n.channel).cloned(),
                            );
                        }
                        n
                    })
                    .collect();
                // Wholesale replacement mints new node ids: every existing
                // derived-signal stream loses its owner.
                self.emitted_streams.clear();
                // Sysvar streams are not node-owned; recreate them right
                // away so observer selections survive the replacement.
                let live: Vec<(String, String, f64)> = self
                    .sysvars
                    .iter()
                    .map(|(d, v)| (d.namespace.clone(), d.name.clone(), *v))
                    .collect();
                for (ns, name, v) in live {
                    self.ingest_sysvar(&ns, &name, v);
                }
                self.nodes_dirty = true;
            }
            BusCommand::DefineSysVar(def) => {
                let key = def.key();
                match self.sysvars.iter().position(|(d, _)| d.key() == key) {
                    Some(i) => {
                        let v = def.init;
                        self.sysvars[i] = (def, v);
                    }
                    None => {
                        let v = def.init;
                        self.sysvars.push((def, v));
                    }
                }
                self.refresh_node_sysvar_keys();
                if let Some((d, v)) = self.sysvars.iter().find(|(d, _)| d.key() == key) {
                    let (ns, name, val) = (d.namespace.clone(), d.name.clone(), *v);
                    self.ingest_sysvar(&ns, &name, val);
                }
                self.nodes_dirty = true;
            }
            BusCommand::DeleteSysVar { namespace, name } => {
                let before = self.sysvars.len();
                self.sysvars
                    .retain(|(d, _)| d.namespace != namespace || d.name != name);
                if self.sysvars.len() != before {
                    self.refresh_node_sysvar_keys();
                    // The dead variable's stream has no future writer.
                    let dead = crate::app::EMITTED_ID_BASE | (SYSVAR_STREAM_ID as u32);
                    self.emitted_streams.retain(|(k, owner)| {
                        // Sysvar streams are CAN keys built by
                        // `ingest_emitted`; a FlexRay key is never one.
                        let SigKey::Can { id, name: sig, .. } = k else {
                            return true;
                        };
                        !(id == &dead && sig == &name && owner == &namespace)
                    });
                    self.nodes_dirty = true;
                }
            }
            BusCommand::SetSysVar {
                namespace,
                name,
                value,
            } => {
                let key = format!("{namespace}::{name}");
                if !self.sysvar_write(&key, value) {
                    *status = format!("[sysvar] undefined: \"{key}\"");
                }
            }
            BusCommand::SetSysVarBandSet {
                namespace,
                name,
                band_set_id,
            } => {
                if let Some((def, _)) = self
                    .sysvars
                    .iter_mut()
                    .find(|(def, _)| def.namespace == namespace && def.name == name)
                {
                    def.band_set_id = band_set_id;
                }
            }
            #[cfg(test)]
            BusCommand::ClearWrite => {
                self.write_log.clear();
                self.write_dirty = true;
            }
            BusCommand::SetBusMode { real } => {
                self.hw.live = real;
                *status = if real {
                    "Bus mode: Real bus -- attached hardware enabled (RX reaches the bus, directed TX is sent)".into()
                } else {
                    "Bus mode: Simulated -- hardware attachments kept but inactive (simulation only)".into()
                };
            }
            BusCommand::AddReplayBlock {
                name,
                channel,
                path,
                node_filter,
                attached,
                ids,
            } => self.add_replay_block(name, channel, path, node_filter, attached, ids, status),
            BusCommand::SetReplayBlock {
                id,
                name,
                channel,
                path,
                node_filter,
                attached,
                ids,
            } => self.set_replay_block(id, name, channel, path, node_filter, attached, ids, status),
            BusCommand::RemoveReplayBlock { id } => {
                self.replay_blocks.retain(|b| b.id != id);
                self.nodes_dirty = true;
            }
            BusCommand::DuplicateReplayBlock { id } => {
                let Some(block) = self.replay_blocks.iter().find(|b| b.id == id) else {
                    return;
                };
                let name = format!("{} copy", block.name);
                let channel = block.channel;
                let path = block.path.clone();
                let node_filter = block.node_filter.clone();
                let attached = block.attached.clone();
                let ids = block.ids.clone();
                // Reuse the add path so the copy mints its own id and
                // reloads its queue only when someone enables it.
                self.add_replay_block(name, channel, path, node_filter, attached, ids, status);
            }
            BusCommand::SetReplayBlockEnabled { id, on } => {
                self.set_replay_block_enabled(id, on, status)
            }
            BusCommand::SetReplayBlocks { blocks } => {
                self.replay_blocks = blocks
                    .into_iter()
                    .map(|cfg| {
                        self.block_counter += 1;
                        let mut b = crate::block::ReplayBlock::new(
                            self.block_counter,
                            cfg.name,
                            cfg.channel,
                            cfg.path,
                            cfg.node_filter,
                            cfg.attached,
                            cfg.ids,
                            cfg.enabled,
                        );
                        if b.enabled {
                            let dbc = self
                                .model_dbs
                                .get(b.channel as usize)
                                .and_then(|db| db.clone());
                            let node_ids = b
                                .node_filter
                                .as_deref()
                                .and_then(|node| self.model_node_tx_ids(b.channel, node));
                            b.load_queue_with(dbc.as_deref(), node_ids, self.sim_t_us);
                        }
                        b
                    })
                    .collect();
                self.nodes_dirty = true;
            }
            BusCommand::SetReplaySpeed(speed) => self.source.set_speed(speed),
            BusCommand::SetFrDbs { dbs, sources } => {
                self.fr_dbs = dbs;
                self.fr_sources = sources;
                self.rebuild_fr_models();
                // Which frame an arrival was cannot be worked out later from
                // the slot alone, so a description that appears or disappears
                // mid-run re-tallies the rows that are on screen.
                self.rebuild_fr_aggs();
                // Dotted reads resolve their signal name to a slot from the
                // previous description; re-resolve so a replaced cluster
                // moves them with it instead of going silently stale.
                if self.measuring {
                    for node in &mut self.nodes {
                        node.refresh_fr_named(&self.fr_dbs);
                    }
                }
            }
            BusCommand::SetEthDbs { dbs, sources } => {
                if self.nm_timing.is_none()
                    && let Some(timing) = dbs.values().find_map(|db| db.nm_timing)
                {
                    self.nm_timing = Some(crate::script::NmTiming {
                        repeat_ms: timing.repeat_ms,
                        ready_sleep_ms: timing.ready_sleep_ms,
                        prepare_sleep_ms: timing.prepare_sleep_ms,
                        timeout_ms: timing.timeout_ms,
                    });
                }
                self.eth_dbs = dbs;
                self.eth_sources = sources;
                self.rebuild_eth_models();
                // Named-cluster reads store the bus they resolved to; a
                // replaced matrix can move or lose that name.
                if self.measuring {
                    for node in &mut self.nodes {
                        node.refresh_eth_named(&self.eth_dbs);
                    }
                }
            }
            BusCommand::SetEthLengthIncludesHeader(on) => self.eth_length_includes_header = on,
            BusCommand::SetChannelModel { ch, comm, sources } => {
                self.set_channel_model(ch, comm, sources)
            }
            BusCommand::SetClusterIds { next, can, fr, eth } => {
                self.cluster_ids = crate::model::ClusterIds::new(next);
                let old_can = std::mem::take(&mut self.can_cluster_ids);
                self.can_cluster_ids = can.into_iter().map(crate::model::ClusterId).collect();
                self.fr_cluster_ids = fr
                    .into_iter()
                    .map(|(bus, id)| (bus, crate::model::ClusterId(id)))
                    .collect();
                self.eth_cluster_ids = eth
                    .into_iter()
                    .map(|(bus, id)| (bus, crate::model::ClusterId(id)))
                    .collect();
                // Roles and checksums follow their channel to the restored id;
                // the model's own references are rebound below.
                let roles = std::mem::take(&mut self.can_node_roles);
                let sums = std::mem::take(&mut self.can_sums);
                for (index, old) in old_can.iter().enumerate() {
                    let Some(id) = self.can_cluster_ids.get(index).copied() else {
                        break;
                    };
                    if let Some(v) = roles.get(old) {
                        self.can_node_roles.insert(id, v.clone());
                    }
                    if let Some(v) = sums.get(old) {
                        self.can_sums.insert(id, v.clone());
                    }
                }
                for index in 0..self.can_count() {
                    while self.model_comms.len() <= index {
                        self.model_comms.push(None);
                        self.model_plans.push(None);
                        self.model_dbs.push(None);
                        self.can_xml_models.push(None);
                    }
                    let id = self.can_cluster_ids[index];
                    if let Some(comm) = self.model_comms[index].as_mut() {
                        Arc::make_mut(comm).rebind_cluster(id);
                    }
                    self.rebuild_can_model(index);
                }
                self.rebuild_fr_models();
                self.rebuild_eth_models();
            }
            BusCommand::SetExpandedMessages(keys) => {
                self.expanded_messages = keys.into_iter().collect();
            }
            BusCommand::SetEntryActive { ch, id, on } => {
                // Activating anchors the schedule at the current clock:
                // `next_t_us` still sits at the last slot before the entry
                // was switched off, and letting the catch-up loop run from
                // there would re-emit frames dated across the whole off
                // period. A re-enabled entry starts sending from now.
                // Deactivating touches only the flag, so payload, waveforms
                // and schedule survive the pause.
                let sim = self.sim_t_us;
                if let Some(tx) = self.entry_mut(ch, id) {
                    tx.active = on;
                    if on {
                        tx.next_t_us = sim;
                    }
                }
            }
            BusCommand::SetEntryFd { ch, id, fd } => {
                if let Some(tx) = self.entry_mut(ch, id) {
                    tx.flags = if fd {
                        tx.flags.union(crate::can::frame::FrameFlags::FD)
                    } else {
                        tx.flags
                            .remove(crate::can::frame::FrameFlags::FD)
                            .remove(crate::can::frame::FrameFlags::BRS)
                    };
                }
            }
            BusCommand::SetEntryBrs { ch, id, brs } => {
                if let Some(tx) = self.entry_mut(ch, id) {
                    let fd = tx.flags.contains(crate::can::frame::FrameFlags::FD);
                    tx.flags = if brs && fd {
                        tx.flags.union(crate::can::frame::FrameFlags::BRS)
                    } else {
                        tx.flags.remove(crate::can::frame::FrameFlags::BRS)
                    };
                }
            }
            BusCommand::SetEntryCycle { ch, id, cycle_us } => {
                if let Some(tx) = self.entry_mut(ch, id) {
                    tx.cycle_us = cycle_us;
                    // Same convention as every other way of switching a
                    // message on: the new schedule starts now rather than
                    // at the end of the old one.
                    tx.next_t_us = 0;
                }
            }
            BusCommand::SetEntryHex { ch, id, text } => {
                self.set_entry_hex(ch, id, &text);
            }
            BusCommand::RemoveEntry { ch, id } => {
                self.tx_list.retain(|t| !(t.channel == ch && t.id == id));
            }
            BusCommand::AddEntry { ch, id } => self.add_entry(ch, id),
            BusCommand::SetEntrySource { ch, id, src } => {
                if let Some(tx) = self.entry_mut(ch, id) {
                    match tx.srcs.iter_mut().find(|s| s.name == src.name) {
                        Some(held) => *held = src,
                        None => tx.srcs.push(src),
                    }
                }
            }
            BusCommand::ClearEntrySource { ch, id, name } => {
                if let Some(tx) = self.entry_mut(ch, id) {
                    tx.srcs.retain(|s| s.name != name);
                }
            }
            BusCommand::AddFrEntry { bus, slot, node } => self.add_fr_entry(bus, slot, node),
            BusCommand::RemoveFrEntry { bus, slot } => {
                self.fr_tx_list
                    .retain(|t| !(t.bus == bus && t.slot == slot));
            }
            BusCommand::SetFrEntryActive { bus, slot, on } => {
                // Same anchoring as the CAN entry: re-enabling starts from now,
                // so the catch-up loop never re-emits the off period.
                let sim = self.sim_t_us;
                if let Some(tx) = self.fr_entry_mut(bus, slot) {
                    tx.active = on;
                    if on {
                        tx.next_t_us = sim;
                    }
                }
            }
            BusCommand::SetFrEntryCycle {
                bus,
                slot,
                cycle_us,
            } => {
                if let Some(tx) = self.fr_entry_mut(bus, slot) {
                    tx.cycle_us = cycle_us;
                    tx.next_t_us = 0;
                }
            }
            BusCommand::SetFrEntryHex { bus, slot, text } => {
                // A refusal goes on the bar (and into the Write ring) rather than
                // leaving the box to snap back to what it held.
                if let Some(why) = self.set_fr_entry_hex(bus, slot, &text) {
                    *status = why;
                }
            }
            BusCommand::SetFrEntrySource { bus, slot, src } => {
                if let Some(tx) = self.fr_entry_mut(bus, slot) {
                    match tx.srcs.iter_mut().find(|s| s.name == src.name) {
                        Some(held) => *held = src,
                        None => tx.srcs.push(src),
                    }
                }
            }
            BusCommand::Subscribe { key } => {
                self.subscribe_signal(key);
                // A source lane appearing after its derived channel was
                // defined should start the curve immediately, even stopped.
                self.evaluate_derived();
            }
            BusCommand::SetDerived(defs) => {
                self.set_derived(defs);
                self.evaluate_derived();
            }
            BusCommand::LogStatus { kind, text } => self.write_push(kind, text),
            BusCommand::Unsubscribe { key } => {
                self.subs.remove(&key);
            }
            BusCommand::StartReplay { path, speed } => self.start_replay(&path, speed, status),
            BusCommand::SetReplayMap(map) => {
                self.replay_map = map;
                self.remap_log_ids();
            }
            BusCommand::SetChannelConfig {
                ch,
                name,
                dbc_path,
                bitrate_kbps,
                fd_data_kbps,
                node_roles,
            } => {
                self.set_channel_config(ch, name, dbc_path, bitrate_kbps, fd_data_kbps, node_roles)
            }
            BusCommand::LoadDbc { ch, paths } => self.load_dbc(ch, paths, status),
            BusCommand::SetBusCounter(n) => self.bus_counter = n,
            BusCommand::SetRecordPath(path) => self.recorder.record_path = path,
            BusCommand::SetRecordFilter { filter } => self.recorder.filter = filter,
            BusCommand::SetTraceLimit { frames } => {
                self.trace_limit = frames.clamp(1_000, 5_000_000)
            }
            BusCommand::SetHardwareChannel {
                bus,
                driver,
                adapter,
                kbps,
                fd_data_kbps,
            } => {
                // Prefer init access (the wire-egress switches can then
                // direct traffic onto the wire). When another program
                // holds the channel, fall back to receive-only. Both
                // drivers apply the bus's FD data-phase rate when the
                // channel is opened with init access; a rejected config
                // comes back fd=false, and hw_fd_note says classic.
                let opened = match driver {
                    crate::hw::HwDriver::Kvaser => {
                        crate::hw::kvaser::KvaserChannel::open(adapter, kbps, fd_data_kbps, true)
                            .map(|p| {
                                let fd = p.fd;
                                (crate::hw::HwPort::Kvaser(p), fd)
                            })
                            .map_err(|e| {
                                (e, {
                                    crate::hw::kvaser::KvaserChannel::open(
                                        adapter,
                                        kbps,
                                        fd_data_kbps,
                                        false,
                                    )
                                    .map(|p| {
                                        let fd = p.fd;
                                        (crate::hw::HwPort::Kvaser(p), fd)
                                    })
                                })
                            })
                    }
                    crate::hw::HwDriver::Vector => {
                        // Same init-first attach: xlCanFdSetConfiguration
                        // needs init access, so an FD bus on a channel
                        // owned elsewhere degrades to monitoring only.
                        crate::hw::vector::VectorChannel::open(adapter, kbps, fd_data_kbps, true)
                            .map(|p| {
                                let fd = p.fd;
                                (crate::hw::HwPort::Vector(p), fd)
                            })
                            .map_err(|e| {
                                (e, {
                                    crate::hw::vector::VectorChannel::open(
                                        adapter,
                                        kbps,
                                        fd_data_kbps,
                                        false,
                                    )
                                    .map(|p| {
                                        let fd = p.fd;
                                        (crate::hw::HwPort::Vector(p), fd)
                                    })
                                })
                            })
                    }
                };
                match opened {
                    Ok((port, fd)) => {
                        let driver_name = match driver {
                            crate::hw::HwDriver::Kvaser => "Kvaser",
                            crate::hw::HwDriver::Vector => "Vector",
                        };
                        let fd_error = match &port {
                            crate::hw::HwPort::Vector(v) => v.fd_error.clone(),
                            _ => None,
                        };
                        self.hw.attach(bus, driver, adapter, kbps, true, port);
                        *status = format!(
                            "hardware attached to {bus}: {driver_name} ch{adapter} @ {kbps} kbit/s (TX/RX){}",
                            hw_fd_note(fd_data_kbps, fd, fd_error.as_deref())
                        );
                    }
                    Err((init_err, retry)) => match retry {
                        Ok((port, fd)) => {
                            let fd_error = match &port {
                                crate::hw::HwPort::Vector(v) => v.fd_error.clone(),
                                _ => None,
                            };
                            self.hw.attach(bus, driver, adapter, kbps, false, port);
                            *status = format!(
                                "hardware attached to {bus}: ch{adapter} @ {kbps} kbit/s (receive-only -- channel taken by another program){}",
                                hw_fd_note(fd_data_kbps, fd, fd_error.as_deref())
                            );
                        }
                        Err(e) => *status = format!("hardware attach failed: {init_err} / {e}"),
                    },
                }
            }
            BusCommand::DetachHardware { bus } => {
                self.hw.detach(bus);
                *status = format!("hardware detached from bus {}", bus + 1);
            }
            BusCommand::AttachEthPort { bus, device } => match self.hw.attach_eth(bus, &device) {
                Ok(()) => *status = format!("ETH{bus} live port: {device}"),
                Err(error) => *status = format!("ETH{bus} attach failed: {error}"),
            },
            BusCommand::DetachEthPort { bus } => {
                self.hw.detach_eth(bus);
                *status = format!("ETH{bus} live port detached");
            }
            BusCommand::RefreshHardware => {
                self.hw.detach_vector_ports();
                let result = crate::hw::vector::reset_driver();
                self.hw.refresh_seq += 1;
                // Put the presence scan in the status line as well as the
                // probe: when an offline channel still appears on screen,
                // this says whether PnP sees the adapter or the list is
                // stale.
                let pnp = match crate::hw::vector::present_usb_pids() {
                    Some(pids) => format!("Vector USB PIDs {pids:04X?}"),
                    None => "Vector PnP query unavailable".to_string(),
                };
                *status = match result {
                    Ok(()) => format!("Vector driver reopened; {pnp}"),
                    Err(e) => format!(
                        "Vector connections closed, but the driver could not reopen: {e}; {pnp}"
                    ),
                };
            }
            BusCommand::SetFrWatch {
                bus,
                channel_index,
                fibex_path,
            } => match channel_index {
                Some(idx) => {
                    // The description is configured from the database the
                    // frontend parsed for *this* bus and pushed with
                    // `SetFrDbs`; re-reading the file here parsed it a second
                    // time.
                    match self.fr_dbs.get(&bus) {
                        Some(db) => match self.hw.attach_fr(bus, idx, &fibex_path, db) {
                            Ok(()) => {
                                *status = format!(
                                    "FlexRay watch attached: FR{bus} / Vector ch{idx} (TX/RX)"
                                );
                            }
                            Err(e) => {
                                *status = format!("FlexRay watch attach failed: FR{bus} {e}");
                            }
                        },
                        None => {
                            *status = format!(
                                "FlexRay watch attach failed: FR{bus} description not loaded"
                            )
                        }
                    }
                }
                None => {
                    self.hw.detach_fr(bus);
                    *status = format!("FlexRay watch detached: FR{bus}");
                }
            },
            BusCommand::RemoveFrBus { bus, label } => self.remove_fr_bus(bus, &label, status),
            BusCommand::SetRunLimits {
                pre_frames,
                post_frames,
                marker_cap,
            } => {
                if let Some(n) = pre_frames {
                    self.pre_frames = n.clamp(0, 32_768);
                }
                if let Some(n) = post_frames {
                    self.post_frames = n.clamp(0, 32_768);
                }
                if let Some(n) = marker_cap {
                    self.marker_cap = n.clamp(8, 65_536);
                }
            }
            BusCommand::SetEntryConfig {
                ch,
                id,
                active,
                cycle_us,
                fd,
                flags,
                data_text,
                srcs,
            } => self.set_entry_config(ch, id, active, cycle_us, fd, flags, data_text, srcs),
            #[cfg(test)]
            BusCommand::ClearDatabases => {
                for ch in 0..self.can_count() {
                    self.set_can_paths(ch, Vec::new());
                    self.set_can_table(ch, None);
                }
                self.can_sums.clear();
                self.tx_list.clear();
                self.subs.clear();
                for runtime in &mut self.derived {
                    runtime.last_t_us = None;
                }
            }
            BusCommand::ClearSpec => self.spec.clear(),
            #[cfg(test)]
            BusCommand::SendNow { ch, id } => {
                // Only a running bus delivers: a stopped one would hold the
                // request until some future run, which is never what the
                // button means.
                if self.measuring {
                    self.injected.push((ch, id));
                }
            }
            BusCommand::SetTriggers(triggers) => self.triggers = triggers,
            BusCommand::AddTrigger {
                cond,
                action,
                enabled,
            } => {
                let mut trigger = crate::trigger::Trigger::new(cond, action);
                trigger.enabled = enabled;
                self.triggers.push(trigger);
            }
            BusCommand::RemoveTrigger { index } => {
                if index < self.triggers.len() {
                    self.triggers.remove(index);
                }
            }
            BusCommand::DuplicateTrigger { index } => {
                if let Some(t) = self.triggers.get(index).cloned() {
                    let mut copy = t;
                    copy.enabled = false;
                    copy.level = false;
                    copy.fired = 0;
                    copy.last_fire_t_us = 0;
                    self.triggers.push(copy);
                    *status = format!(
                        "rule {} duplicated as rule {}",
                        index + 1,
                        self.triggers.len()
                    );
                }
            }
            BusCommand::SetTriggerEnabled { index, on } => {
                if let Some(t) = self.triggers.get_mut(index) {
                    t.enabled = on;
                }
            }
            BusCommand::EditTrigger {
                index,
                cond,
                action,
                cooldown_us,
                max_fires,
                pre_frames,
                post_frames,
            } => {
                if let Some(t) = self.triggers.get_mut(index) {
                    t.cond = cond;
                    t.action = action;
                    t.cooldown_us = cooldown_us;
                    t.max_fires = max_fires;
                    t.pre_frames = pre_frames;
                    t.post_frames = post_frames;
                    t.level = false;
                }
            }
            BusCommand::RearmTrigger { index } => {
                if let Some(t) = self.triggers.get_mut(index) {
                    t.rearm_rule();
                    *status = format!("re-armed trigger {}", index + 1);
                }
            }
        }
    }
}
