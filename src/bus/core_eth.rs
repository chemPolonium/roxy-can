//! Ethernet/SOME-IP ingestion into the core.

use super::*;

impl BusCore {
    pub(super) fn accept_someip_event(
        &mut self,
        event: crate::eth::someip::SomeIpEvent,
        status: &mut String,
    ) {
        const SOMEIP_CAP: usize = 512;
        let Some(mut event) = self.someip_tp.feed(&event) else {
            return;
        };
        if event.message_type == 0x80
            && let Some(index) = self.someip_pending.iter().position(|request| {
                request.bus == event.bus
                    && request.service == event.service
                    && request.method == event.method
                    && request.client == event.client
                    && request.session == event.session
            })
        {
            let request = self.someip_pending.remove(index);
            self.write_push(
                WriteKind::Info,
                format!(
                    "SOME/IP response 0x{:04X}.0x{:04X} session {}: {} B",
                    request.service,
                    request.method,
                    request.session,
                    event.payload.len()
                ),
            );
        }
        if event.decoded.is_empty()
            && let Some(service) = self.someip_services.service(event.service)
        {
            let fields = service
                .events
                .iter()
                .find(|entry| entry.id == event.method)
                .map(|entry| entry.fields.as_slice())
                .or_else(|| {
                    service
                        .methods
                        .iter()
                        .find(|entry| entry.id == event.method)
                        .map(|entry| {
                            if event.message_type == 0x80 {
                                entry.output.as_slice()
                            } else {
                                entry.input.as_slice()
                            }
                        })
                });
            if let Some(fields) = fields {
                event.decoded = crate::eth::arxml_service::decode_values(fields, &event.payload)
                    .unwrap_or_default();
            }
        }
        self.eval_someip_triggers(
            event.bus,
            event.service,
            event.method,
            event.message_type,
            event.t_us,
            status,
        );
        self.observe_someip_fields(&event);
        if !self.someip_send_guard {
            self.ack_someip_subscribes(&event);
            self.auto_reply_someip_method(&event);
            self.dispatch_someip_nodes(&event);
        }
        if self.someip_events.len() >= SOMEIP_CAP {
            self.someip_events.pop_front();
        }
        self.someip_events.push_back(event);
    }

    /// Publishes decoded SOME/IP fields through the existing signal observer
    /// path. They are addressed as synthetic Ethernet frames so Signals /
    /// Graphics / Monitor / State do not need a fourth key space.
    pub(super) fn dispatch_someip_nodes(&mut self, event: &crate::eth::someip::SomeIpEvent) {
        if self.nodes.is_empty() {
            return;
        }
        let inputs = self.build_node_inputs(self.sim_t_us);
        let mut node_frames = Vec::new();
        let mut someip_sends = Vec::new();
        let mut someip_replies: Vec<crate::script::SomeIpReply> = Vec::new();
        let mut nm_timings: Vec<crate::script::NmTiming> = Vec::new();
        let mut someip_tcp_sends: Vec<crate::script::SomeIpTcpSend> = Vec::new();
        let mut someip_offers: Vec<crate::script::SomeIpOffer> = Vec::new();
        let mut derived = Vec::new();
        let mut sys_writes = Vec::new();
        let mut lines = Vec::new();
        for node in &mut self.nodes {
            let input = inputs.get(&node.channel).cloned().unwrap_or_default();
            if node.dispatch_someip(
                event.service,
                event.method,
                event.client,
                event.session,
                event.return_code,
                &event.payload,
                &input,
            ) == 0
            {
                continue;
            }
            node_frames.extend(
                node.take_frames()
                    .into_iter()
                    .map(|frame| (node.id, node.channel, true, frame)),
            );
            someip_sends.extend(node.take_someip());
            someip_replies.extend(node.take_someip_reply());
            nm_timings.extend(node.take_nm_timing());
            someip_tcp_sends.extend(node.take_someip_tcp());
            someip_offers.extend(node.take_someip_offer());
            for (name, value) in node.take_emitted() {
                derived.push((node.channel, node.id, node.name.clone(), name, value));
            }
            for (key, value) in node.take_sys_sets() {
                sys_writes.push((node.id, key, value));
            }
            let name = node.name.clone();
            for line in node.take_new_lines() {
                lines.push((name.clone(), line));
            }
        }
        self.send_node_frames(node_frames);
        self.apply_script_someip(someip_sends);
        self.apply_someip_replies(someip_replies);
        if let Some(timing) = nm_timings.last().copied() {
            self.nm_timing = Some(timing);
        }
        self.apply_script_someip_tcp(someip_tcp_sends);
        self.apply_script_someip_offers(someip_offers);
        self.apply_sys_writes(sys_writes);
        for (ch, id, name, signal, value) in derived {
            self.ingest_emitted(ch, id, &name, &signal, value);
        }
        for (name, line) in lines {
            self.write_node_line(&name, &line);
        }
    }

    pub(super) fn observe_someip_fields(&mut self, event: &crate::eth::someip::SomeIpEvent) {
        if event.sd.is_some() || event.decoded.is_empty() {
            return;
        }
        let frame = format!("SOMEIP 0x{:04X}/0x{:04X}", event.service, event.method);
        let stride = self.applied_stride_us;
        for value in &event.decoded {
            let Some(numeric) = value.numeric.or_else(|| value.text.parse::<f64>().ok()) else {
                continue;
            };
            let key = self.eth_key(event.bus, frame.clone(), value.name.clone());
            if let Some(entry) = self.subs.get_mut(&key) {
                entry.latest = numeric;
                entry.last_raw = numeric.round() as i64;
                entry.unit.clear();
                entry.type_tag.clear();
                entry.label = None;
                entry.last_update_us = event.t_us;
                entry.push_sample(event.t_us, numeric, stride);
            }
        }
    }

    /// Appends an Ethernet frame to the run's ring and folds it into the
    /// per-flow tally. The next publish hands both to every observer; no
    /// CAN/FlexRay aggregate is touched.
    pub(super) fn ingest_eth_nm(&mut self, frame: &crate::eth::EthFrame) {
        if frame.ethertype != 0x0800 {
            return;
        }
        let ip = frame.payload.as_ref();
        if ip.len() < 28 || ip[0] >> 4 != 4 {
            return;
        }
        let ihl = usize::from(ip[0] & 0x0f) * 4;
        if ihl < 20 || ip.len() < ihl + 8 || ip[9] != 17 {
            return;
        }
        let src = format!("{}.{}.{}.{}", ip[12], ip[13], ip[14], ip[15]);
        let dst = format!("{}.{}.{}.{}", ip[16], ip[17], ip[18], ip[19]);
        let udp = &ip[ihl..];
        let src_port = u16::from_be_bytes([udp[0], udp[1]]);
        let dst_port = u16::from_be_bytes([udp[2], udp[3]]);
        let data = &udp[8..];
        let Some((pdu, signals)) = self
            .eth_dbs
            .get(&frame.bus)
            .and_then(|db| db.decode_nm(&src, &dst, src_port, dst_port, data))
        else {
            return;
        };
        if signals.is_empty() {
            return;
        }
        let summary = signals
            .iter()
            .map(|(name, value)| format!("{name}={value:.3}"))
            .collect::<Vec<_>>()
            .join(" ");
        let key = format!("{}:{pdu}", frame.bus);
        let value_of = |needle: &str| {
            signals
                .iter()
                .find(|(name, _)| name.eq_ignore_ascii_case(needle))
                .map(|(_, value)| *value)
                .unwrap_or(0.0)
        };
        let repeat = value_of("RepeatMessageRequest") != 0.0;
        let sleep_ready = value_of("NMCoordinatorSleepReadyBit") != 0.0
            || value_of("NMCoordinatorSleeptReadyBit") != 0.0
            || value_of("NMCoordinatorSleepReady") != 0.0;
        let next = if repeat {
            NmState::RepeatMessage
        } else if sleep_ready {
            NmState::ReadySleep
        } else {
            NmState::NormalOperation
        };
        let track = self.nm_states.entry(key.clone()).or_insert(NmTrack {
            state: NmState::Unknown,
            since_us: frame.t_us,
            last_seen_us: frame.t_us,
        });
        track.last_seen_us = frame.t_us;
        if track.state != next {
            track.state = next;
            track.since_us = frame.t_us;
            self.write_push(WriteKind::Info, format!("NM {pdu} state {}", next.label()));
        }
        if self.nm_last.get(&key) == Some(&summary) {
            return;
        }
        self.nm_last.insert(key, summary.clone());
        self.write_push(
            WriteKind::Info,
            format!("NM {pdu} {src}:{src_port}→{dst}:{dst_port} {summary}"),
        );
    }

    pub(crate) fn ingest_eth(&mut self, frame: crate::eth::EthFrame) {
        if self.recorder.recording {
            self.recorder.write_event(&BusEvent::Eth(&frame));
        }
        let mut status = String::new();
        if let Some(decode_frame) = self.someip_ip.feed(&frame) {
            if let Some(event) = crate::eth::someip::decode(&decode_frame) {
                self.accept_someip_event(event, &mut status);
            }
            for event in self.someip_tcp.feed(&decode_frame) {
                self.accept_someip_event(event, &mut status);
            }
        }
        self.ingest_eth_nm(&frame);
        self.eval_eth_triggers(&frame, &mut status);
        let key = crate::aggregate::EthFlowKey {
            bus: frame.bus,
            vlan: frame.vlan,
            dst: frame.dst,
            src: frame.src,
            ethertype: frame.ethertype,
            len: frame.len(),
        };
        let agg = self.eth_aggs.entry(key).or_default();
        agg.key = key;
        if agg.count > 0 && frame.t_us > agg.last_t_us {
            let dt = (frame.t_us - agg.last_t_us) as f64;
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
            if agg.min_us == 0.0 || dt < agg.min_us {
                agg.min_us = dt;
            }
            if dt > agg.max_us {
                agg.max_us = dt;
            }
        }
        agg.count += 1;
        agg.last_t_us = frame.t_us;
        agg.payload = std::sync::Arc::clone(&frame.payload);

        // Decode the matched ARXML frame and fold its signals into the same
        // subscription space CAN/FlexRay use. An ambiguous or unknown flow is
        // watched as raw L2 only, never guessed into a frame definition.
        if !self.subs.is_empty()
            && let Some(db) = self.eth_dbs.get(&frame.bus).cloned()
        {
            let flow = crate::eth::db::EthFlow {
                vlan: frame.vlan,
                dst: frame.dst,
                src: frame.src,
                ethertype: frame.ethertype,
                payload_len: frame.len(),
            };
            if let crate::eth::db::EthMatch::Unique(ix) =
                db.match_flow(&flow, self.eth_length_includes_header)
            {
                let frame_name = db.frames[ix].name.clone();
                if let Some(values) = self.model_eth_values(frame.bus, &frame_name, &frame.payload)
                {
                    let stride = self.applied_stride_us;
                    let t_us = frame.t_us;
                    for d in values {
                        let key = self.eth_key(frame.bus, frame_name.clone(), d.name);
                        if let Some(entry) = self.subs.get_mut(&key) {
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
            }
        }

        // E2E continuity is a property of the frame, not of an observer.
        let tracked_frame = self.eth_dbs.get(&frame.bus).and_then(|db| {
            let flow = crate::eth::db::EthFlow {
                vlan: frame.vlan,
                dst: frame.dst,
                src: frame.src,
                ethertype: frame.ethertype,
                payload_len: frame.len(),
            };
            match db.match_flow(&flow, self.eth_length_includes_header) {
                crate::eth::db::EthMatch::Unique(index) => Some(db.frames[index].name.clone()),
                _ => None,
            }
        });
        if let Some(frame_name) = tracked_frame {
            self.track_eth_e2e(frame.bus, &frame_name, &frame.payload);
        }

        self.eth_trace.push(frame);
    }
}
