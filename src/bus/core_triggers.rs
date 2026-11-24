//! Trigger evaluation and action running.

use super::*;

impl BusCore {
    /// Folds one accepted frame into every bus-side consumer: the trace
    /// ring, the run counter, the per-bus load rollups, the per-message
    /// aggregates and the subscribed-signal caches. `stride` is the sampling
    /// period the frontend currently wants for signal history; everything
    /// else this touches is the bus's own state.
    pub(crate) fn ingest(&mut self, f: CanFrame, stride: u64) {
        self.frame_counter += 1;
        // Error frames included: they occupy the bus and the load view
        // counts them; per-message aggregation below skips them.
        let (arb, data) = self.can_bitrates(f.channel as usize);
        if let Some(load) = self.bus_loads.get_mut(f.channel as usize) {
            let wire = crate::load::wire_time_us(&f, arb, data);
            load.note(&f, wire);
        }
        if f.is_error() {
            // Error frames carry no identifier and no payload; they are
            // intentionally kept out of per-message aggregation.
            self.trace.push(f);
            self.trace.enforce_limit(self.trace_limit);
            return;
        }
        self.track_e2e(&f);
        let agg = self
            .aggs
            .entry((
                self.can_cluster_ids
                    .get(f.channel as usize)
                    .copied()
                    .unwrap_or(crate::model::ClusterId(f.channel as u32 + 1)),
                f.id,
                f.extended,
            ))
            .or_insert(MessageAgg {
                id: f.id,
                extended: f.extended,
                channel: f.channel,
                dir: f.dir,
                count: 0,
                rx: 0,
                tx: 0,
                last_t_us: 0,
                cycle_us: 0.0,
                min_us: f64::MAX,
                max_us: 0.0,
                jitter_us: 0.0,
                len: f.len,
                data: f.data,
                flags: f.flags,
            });
        // Only a strictly later timestamp marks a real cycle. A backwards
        // or repeated one is a discontinuity -- a seek, or an out-of-order
        // log row -- and folding it in would pin `min_us` at zero for the
        // rest of the run. The running average keeps its pre-seek value and
        // resumes blending at the next real interval.
        if agg.count > 0 && f.t_us > agg.last_t_us {
            let dt = (f.t_us - agg.last_t_us) as f64;
            let prev_cycle = agg.cycle_us;
            agg.cycle_us = if agg.count == 1 {
                dt
            } else {
                agg.cycle_us * 0.9 + dt * 0.1
            };
            // Jitter: EMA of the intervals' absolute deviation from the
            // running mean cycle, smoothed like the cycle itself.
            let dev = (dt - prev_cycle).abs();
            agg.jitter_us = if agg.count == 1 {
                dev
            } else {
                agg.jitter_us * 0.9 + dev * 0.1
            };
            if dt < agg.min_us {
                agg.min_us = dt;
            }
            if dt > agg.max_us {
                agg.max_us = dt;
            }
        }
        match f.dir {
            Direction::Rx => agg.rx += 1,
            Direction::Tx => agg.tx += 1,
        }
        agg.count += 1;
        agg.last_t_us = f.t_us;
        agg.channel = f.channel;
        agg.dir = f.dir;
        agg.len = f.len;
        agg.data = f.data;
        agg.flags = f.flags;
        for (key, d) in self.subscribed_values(&f) {
            let Some(entry) = self.subs.get_mut(&key) else {
                continue;
            };
            entry.latest = d.phys;
            entry.last_raw = d.raw;
            entry.unit = d.unit;
            entry.type_tag = d.type_tag;
            entry.label = d.label;
            entry.last_update_us = f.t_us;
            entry.push_sample(f.t_us, d.phys, stride);
        }
        self.trace.push(f);
        self.trace.enforce_limit(self.trace_limit);
    }

    /// The frames a frame carries for signals this run subscribes to, from
    /// the model's plan for the channel.
    pub(crate) fn subscribed_values(&self, f: &CanFrame) -> Vec<(SigKey, DecodedSignal)> {
        self.model_can_values(f)
            .unwrap_or_default()
            .into_iter()
            .map(|d| {
                let key = self.can_key(f.channel, f.id, f.extended, d.name.clone());
                (key, d)
            })
            .collect()
    }

    /// The FlexRay side of [`BusCore::eval_triggers`]: a row folds into the
    /// cluster-keeping conditions before its ingest, so a rule that starts a
    /// recording still captures the frame that fired it.
    pub(crate) fn eval_fr_triggers(&mut self, row: &crate::trace::FrRow, status: &mut String) {
        if self.triggers.is_empty() {
            return;
        }
        let mut fired: Vec<FiredTrigger> = Vec::new();
        for i in 0..self.triggers.len() {
            let now = {
                let t = &self.triggers[i];
                if !t.enabled {
                    continue;
                }
                match &t.cond {
                    TriggerCond::FrFramePresent { bus, slot } => {
                        if row.bus != *bus || row.slot != *slot {
                            continue;
                        }
                        true // latch: once seen, it stays seen
                    }
                    TriggerCond::FrSignalCross {
                        bus,
                        slot,
                        signal,
                        threshold,
                        rising,
                    } => {
                        if row.bus != *bus || row.slot != *slot {
                            continue;
                        }
                        let Some(db) = self.fr_dbs.get(bus) else {
                            continue; // no description for this cluster
                        };
                        match db
                            .frame_ix_at(row.slot, row.cycle, row.ab)
                            .and_then(|frame_ix| {
                                self.model_fr_decoded(row, frame_ix, false)
                                    .unwrap_or_default()
                                    .into_iter()
                                    .find(|d| d.name == *signal)
                            }) {
                            Some(d) => {
                                if *rising {
                                    d.phys >= *threshold
                                } else {
                                    d.phys <= *threshold
                                }
                            }
                            // A slot the description has no frame for, or a
                            // signal it does not declare: no opinion, the
                            // level holds -- same courtesy as the CAN arm.
                            None => continue,
                        }
                    }
                    // Everything else watches CAN frames or is swept by step.
                    _ => continue,
                }
            };
            let t = &mut self.triggers[i];
            let was = t.level;
            t.level = now;
            if now && !was && t.can_fire(row.t_us) {
                t.note_fire(row.t_us);
                fired.push(FiredTrigger {
                    index: i,
                    action: t.action.clone(),
                    at_us: row.t_us,
                });
            }
        }
        // A FlexRay edge has no CAN frame to mirror into a reaction send, and
        // the actions a send could aim at are CAN generator entries: the
        // payload map stays empty, so a `Send` rule is a no-op here.
        self.run_actions(fired, &HashMap::new(), status);
    }
    /// SOME/IP messages are events: every matching message can fire, gated
    /// by the rule's own cooldown/max.
    pub(crate) fn eval_someip_triggers(
        &mut self,
        bus: u8,
        service: u16,
        method: u16,
        message_type: u8,
        at_us: u64,
        status: &mut String,
    ) {
        if self.someip_send_guard || self.triggers.is_empty() {
            return;
        }
        let mut fired: Vec<FiredTrigger> = Vec::new();
        for i in 0..self.triggers.len() {
            let t = &self.triggers[i];
            if !t.enabled {
                continue;
            }
            if !t.cond.someip_matches(bus, service, method, message_type) {
                continue;
            }
            if t.can_fire(at_us) {
                let action = t.action.clone();
                let t = &mut self.triggers[i];
                t.note_fire(at_us);
                fired.push(FiredTrigger {
                    index: i,
                    action,
                    at_us,
                });
            }
        }
        if !fired.is_empty() {
            self.run_actions(fired, &HashMap::new(), status);
        }
    }

    /// E2E anomalies are events, not levels: each gap/jump fires immediately.
    pub(crate) fn eval_e2e_triggers(
        &mut self,
        ch: u8,
        id: u32,
        ext: bool,
        at_us: u64,
        status: &mut String,
    ) {
        if self.triggers.is_empty() {
            return;
        }
        let mut fired: Vec<FiredTrigger> = Vec::new();
        for i in 0..self.triggers.len() {
            let t = &self.triggers[i];
            if !t.enabled {
                continue;
            }
            match &t.cond {
                TriggerCond::E2eIssue {
                    ch: want_ch,
                    id: want_id,
                    ext: want_ext,
                } if *want_ch == ch && *want_id == id && *want_ext == ext => {}
                _ => continue,
            }
            if t.can_fire(at_us) {
                let action = t.action.clone();
                let t = &mut self.triggers[i];
                t.note_fire(at_us);
                fired.push(FiredTrigger {
                    index: i,
                    action,
                    at_us,
                });
            }
        }
        if !fired.is_empty() {
            self.run_actions(fired, &HashMap::new(), status);
        }
    }

    /// The Ethernet side of [`BusCore::eval_triggers`]: a captured flow is
    /// presence-style, so the first matching frame latches the rule for this
    /// run. It runs before the flow folds into the aggregates, so a rule that
    /// starts a recording still captures the frame that fired it.
    pub(crate) fn eval_eth_triggers(&mut self, frame: &crate::eth::EthFrame, status: &mut String) {
        if self.triggers.is_empty() {
            return;
        }
        let mut fired: Vec<FiredTrigger> = Vec::new();
        for i in 0..self.triggers.len() {
            let now = {
                let t = &self.triggers[i];
                if !t.enabled {
                    continue;
                }
                match &t.cond {
                    TriggerCond::EthFlow {
                        bus,
                        ethertype,
                        vlan,
                    } => {
                        frame.bus == *bus
                            && frame.ethertype == *ethertype
                            && vlan.is_none_or(|wanted| frame.vlan == Some(wanted))
                    }
                    _ => continue,
                }
            };
            let t = &mut self.triggers[i];
            let was = t.level;
            t.level = now;
            if now && !was && t.can_fire(frame.t_us) {
                t.note_fire(frame.t_us);
                fired.push(FiredTrigger {
                    index: i,
                    action: t.action.clone(),
                    at_us: frame.t_us,
                });
            }
        }
        if !fired.is_empty() {
            self.run_actions(fired, &HashMap::new(), status);
        }
    }

    /// Folds one frame into every enabled trigger and acts on edges.
    /// Runs as the first thing that happens to a received frame, so a
    /// trigger that starts a recording still captures the frame that
    /// fired it. Status messages an action wants to show go into
    /// `status` (last write wins, as the status bar only has one line).
    pub(crate) fn eval_triggers(&mut self, f: &CanFrame, status: &mut String) {
        if self.triggers.is_empty() {
            return;
        }
        let mut fired: Vec<FiredTrigger> = Vec::new();
        for i in 0..self.triggers.len() {
            // Observe under shared borrows first: acting on an edge
            // needs `self` exclusively, and the two cannot overlap.
            let now = {
                let t = &self.triggers[i];
                if !t.enabled {
                    continue;
                }
                match &t.cond {
                    TriggerCond::SignalCross {
                        ch,
                        id,
                        ext,
                        signal,
                        threshold,
                        rising,
                    } => {
                        if f.channel != *ch || f.id != *id || f.extended != *ext || f.is_error() {
                            // Not this message's frame: the level holds.
                            continue;
                        }
                        // No model on the bus means no opinion, exactly as a
                        // missing database did: the level holds.
                        let Some(decoded) = self.model_can_decoded(f, false) else {
                            continue;
                        };
                        match decoded.into_iter().find(|d| d.name == *signal) {
                            Some(d) => {
                                if *rising {
                                    d.phys >= *threshold
                                } else {
                                    d.phys <= *threshold
                                }
                            }
                            // The condition names a signal the database
                            // lacks -- same courtesy as a missing db.
                            None => continue,
                        }
                    }
                    TriggerCond::IdPresent { ch, id } => {
                        if f.channel != *ch || f.id != *id || f.is_error() {
                            continue;
                        }
                        true // latch: once seen, it stays seen
                    }
                    TriggerCond::ErrorFrame { ch } => {
                        if f.channel != *ch || !f.is_error() {
                            continue;
                        }
                        true
                    }
                    TriggerCond::PayloadMatch {
                        ch,
                        id,
                        ext,
                        offset,
                        mask,
                        value,
                    } => {
                        if f.channel != *ch || f.id != *id || f.extended != *ext || f.is_error() {
                            continue;
                        }
                        let at = usize::from(*offset);
                        at < usize::from(f.len) && (f.data[at] & *mask) == (*value & *mask)
                    }
                    // Not frame conditions: swept once per step in
                    // `eval_timeout_triggers` (cycle silence, system vars),
                    // or per FlexRay row in `eval_fr_triggers`.
                    TriggerCond::CycleTimeout { .. }
                    | TriggerCond::SysVar { .. }
                    | TriggerCond::DerivedSignal { .. }
                    | TriggerCond::FrFramePresent { .. }
                    | TriggerCond::FrSignalCross { .. }
                    | TriggerCond::FrTimeout { .. }
                    | TriggerCond::EthFlow { .. }
                    | TriggerCond::E2eIssue { .. }
                    | TriggerCond::SomeIp { .. } => continue,
                }
            };
            let t = &mut self.triggers[i];
            let was = t.level;
            t.level = now;
            if now && !was && t.can_fire(f.t_us) {
                t.note_fire(f.t_us);
                fired.push(FiredTrigger {
                    index: i,
                    action: t.action.clone(),
                    at_us: f.t_us,
                });
            }
        }
        // A reaction send mirrors the triggering frame: decode it once, and
        // every same-named signal of the target entry's message is encoded
        // over the sent payload.
        let mut mirror: HashMap<String, f64> = HashMap::new();
        if fired
            .iter()
            .any(|f| matches!(f.action, TriggerAction::Send { .. }))
        {
            for d in self.model_can_decoded(f, false).unwrap_or_default() {
                mirror.insert(d.name, d.phys);
            }
        }
        self.run_actions(fired, &mirror, status);
    }

    /// Sweep conditions: evaluated once per measurement step against the
    /// aggregates, not per frame. A message only convicts after it has
    /// been seen once (the Missing verdict takes the same stance), and
    /// the level clears when traffic resumes, so every new dropout is a
    /// fresh edge. `grace` is the spec's missed-cycles tolerance, a
    /// frontend setting the bus does not own.
    pub(crate) fn eval_timeout_triggers(&mut self, now_us: u64, grace: u64, status: &mut String) {
        if self.triggers.is_empty() {
            return;
        }
        let mut fired: Vec<FiredTrigger> = Vec::new();
        for i in 0..self.triggers.len() {
            // Level conditions swept once per step: cycle silence against
            // the aggregates, system variables against the live registry.
            let level = match &self.triggers[i].cond {
                TriggerCond::CycleTimeout { ch, id } => {
                    if !self.triggers[i].enabled {
                        continue;
                    }
                    Some(self.timeout_silent(*ch, *id, now_us, grace))
                }
                TriggerCond::FrTimeout { bus, slot } => {
                    if !self.triggers[i].enabled {
                        continue;
                    }
                    Some(self.fr_timeout_silent(*bus, *slot, now_us, grace))
                }
                TriggerCond::DerivedSignal {
                    id,
                    threshold,
                    rising,
                } => {
                    if !self.triggers[i].enabled {
                        continue;
                    }
                    // A lane with no samples yet has no opinion: the level
                    // holds rather than reading a fabricated zero.
                    let Some(runtime) = self.derived.iter().find(|runtime| &runtime.def.id == id)
                    else {
                        continue;
                    };
                    match self.subs.get(&runtime.key) {
                        Some(sub) if !sub.history.is_empty() => Some(if *rising {
                            sub.latest >= *threshold
                        } else {
                            sub.latest <= *threshold
                        }),
                        _ => None,
                    }
                }
                TriggerCond::SysVar {
                    key,
                    threshold,
                    rising,
                } => {
                    if !self.triggers[i].enabled {
                        continue;
                    }
                    let v = self
                        .sysvars
                        .iter()
                        .find(|(d, _)| d.key() == *key)
                        .map(|(_, v)| *v);
                    Some(match v {
                        Some(v) => {
                            if *rising {
                                v >= *threshold
                            } else {
                                v <= *threshold
                            }
                        }
                        // An undefined key reads 0.0 at runtime (see
                        // sys_get): the sweep stays consistent with that.
                        None => {
                            if *rising {
                                0.0 >= *threshold
                            } else {
                                0.0 <= *threshold
                            }
                        }
                    })
                }
                _ => continue,
            };
            let Some(level) = level else {
                continue;
            };
            let t = &mut self.triggers[i];
            let was = t.level;
            t.level = level;
            if level && !was && t.can_fire(now_us) {
                t.note_fire(now_us);
                fired.push(FiredTrigger {
                    index: i,
                    action: t.action.clone(),
                    at_us: now_us,
                });
            }
        }
        // A timeout has no triggering frame, so a reaction send it fires
        // carries nothing to mirror.
        self.run_actions(fired, &HashMap::new(), status);
    }

    /// The spec's own grace comparison decides silence, so a trigger and
    /// the Dropped verdict can never disagree about the same message.
    pub(super) fn timeout_silent(&self, ch: u8, id: u32, now_us: u64, grace: u64) -> bool {
        // Trigger conditions carry no frame class yet: either class's
        // aggregate counts.
        let Some(agg) = self.aggs.values().find(|a| a.channel == ch && a.id == id) else {
            return false; // never seen: no opinion, not a dropout
        };
        let Some(declared) = self.dbc_cycle_us(ch, id) else {
            return false; // no database, message, or declared period
        };
        crate::spec::missing_offender(now_us, agg.last_t_us, declared, grace)
    }

    /// FlexRay silence is judged against the slot's fastest declared
    /// repetition: a slot scheduled every cycle convicts sooner than one
    /// scheduled every fourth.
    pub(super) fn fr_timeout_silent(&self, bus: u8, slot: u16, now_us: u64, grace: u64) -> bool {
        let Some(last) = self
            .fr_aggs
            .values()
            .filter(|agg| agg.bus == bus && agg.slot == slot)
            .map(|agg| agg.last_t_us)
            .max()
        else {
            return false; // never seen: no opinion
        };
        let Some(period) = self.fr_slot_period_us(bus, slot) else {
            return false; // no description or no schedule for the slot
        };
        crate::spec::missing_offender(now_us, last, period, grace)
    }

    pub(super) fn fr_slot_period_us(&self, bus: u8, slot: u16) -> Option<u64> {
        let db = self.fr_dbs.get(&bus)?;
        let cycle_us = (db.params.cycle_time_ms * 1_000.0).round() as u64;
        if cycle_us == 0 {
            return None;
        }
        db.frames
            .iter()
            .filter(|frame| {
                frame.triggering.slot_id == u32::from(slot) && frame.triggering.cycle_repetition > 0
            })
            .map(|frame| cycle_us * u64::from(frame.triggering.cycle_repetition))
            .min()
    }

    pub(super) fn run_actions(
        &mut self,
        fired: Vec<FiredTrigger>,
        mirror: &HashMap<String, f64>,
        status: &mut String,
    ) {
        for FiredTrigger {
            index,
            action,
            at_us,
        } in fired
        {
            let text = self
                .triggers
                .get(index)
                .map(|t| t.cond.short())
                .unwrap_or_default();
            let (pre_override, post_override) = self
                .triggers
                .get(index)
                .map(|t| (t.pre_frames, t.post_frames))
                .unwrap_or((None, None));
            let label = action.label();
            let (ok, result) = match action {
                TriggerAction::StartRecording => {
                    if self.recorder.recording {
                        (true, "already recording".to_string())
                    } else {
                        self.recorder.recording = true;
                        if self.measuring {
                            let opened_path = self.recorder.open();
                            self.recorder.recording = opened_path.is_ok();
                            match &opened_path {
                                Ok(path) => {
                                    *status = if self.recorder.last_eth_record.is_empty()
                                        || self.recorder.last_eth_record == *path
                                    {
                                        format!("trigger started recording to {path}")
                                    } else {
                                        format!(
                                            "trigger started recording to {path} (Ethernet: {})",
                                            self.recorder.last_eth_record
                                        )
                                    };
                                    // A fresh recording cancels any pending roll.
                                    self.post_roll = None;
                                    // Oldest first: the file opens with the
                                    // frames that came before the trigger.
                                    let total = self.pre_buffer.len();
                                    let take = pre_override.unwrap_or(total).min(total);
                                    for pre in self.pre_buffer.drain(..).skip(total - take) {
                                        self.recorder.write_event(&BusEvent::Can(&pre));
                                    }
                                    (true, format!("recording to {path}"))
                                }
                                Err(e) => {
                                    *status = format!("trigger record failed: {e}");
                                    (false, format!("record failed: {e}"))
                                }
                            }
                        } else {
                            *status = "trigger enabled recording for the next start".to_string();
                            (true, "armed for next start".to_string())
                        }
                    }
                }
                TriggerAction::StopRecording => {
                    if !self.recorder.recording {
                        (true, "not recording".to_string())
                    } else if self.measuring {
                        // Roll on: the file stays open for the post-roll, then
                        // the step loop closes it once the countdown is spent.
                        self.post_roll = Some(post_override.unwrap_or(self.post_frames));
                        *status = "trigger stopping after post-roll".to_string();
                        (true, "stopping after post-roll".to_string())
                    } else {
                        self.recorder.recording = false;
                        *status = "trigger disabled recording".to_string();
                        (true, "recording off".to_string())
                    }
                }
                TriggerAction::Send { ch, id } => {
                    if self.send_one_shot(ch, id, at_us, mirror) {
                        (true, format!("sent 0x{id:X}"))
                    } else {
                        (false, format!("no generator entry for 0x{id:X}"))
                    }
                }
                TriggerAction::SendFr { bus, slot } => {
                    if self.send_fr_one_shot(bus, slot, at_us) {
                        (true, format!("queued FR{bus} slot {slot}"))
                    } else {
                        (false, format!("no FR entry for bus {bus} slot {slot}"))
                    }
                }
                TriggerAction::SendEth { bus, frame, data } => {
                    match self.send_eth_frame(bus, &frame, &data) {
                        Ok(()) => (true, format!("sent ETH{bus} {frame}")),
                        Err(error) => (false, error),
                    }
                }
                TriggerAction::SendSomeIp {
                    bus,
                    service,
                    method,
                    client,
                    session,
                    message_type,
                    data,
                    values,
                    transport,
                    host,
                    port,
                } => {
                    if !self.measuring {
                        (
                            false,
                            "SOME/IP send needs a running measurement".to_string(),
                        )
                    } else {
                        let payload = if values.is_empty() {
                            Ok(data.clone())
                        } else {
                            self.encode_someip_payload(service, method, message_type, &values)
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
                                if transport.eq_ignore_ascii_case("tcp") {
                                    match send_someip_tcp(
                                        &host,
                                        port,
                                        service,
                                        method,
                                        client,
                                        session,
                                        message_type,
                                        &payload,
                                    ) {
                                        Ok(()) => (
                                            true,
                                            format!(
                                                "sent SOME/IP TCP 0x{service:04X}.0x{method:04X}"
                                            ),
                                        ),
                                        Err(error) => (false, error),
                                    }
                                } else {
                                    let frame = crate::eth::someip::build_udp_frame(
                                        at_us,
                                        bus,
                                        service,
                                        method,
                                        client,
                                        session,
                                        message_type,
                                        &payload,
                                    );
                                    self.someip_send_guard = true;
                                    let sent = self.emit_eth(frame);
                                    self.someip_send_guard = false;
                                    match sent {
                                        Ok(()) => (
                                            true,
                                            format!("sent SOME/IP 0x{service:04X}.0x{method:04X}"),
                                        ),
                                        Err(error) => (false, error),
                                    }
                                }
                            }
                            Err(error) => (false, error),
                        }
                    }
                }
                TriggerAction::InsertMarker => {
                    self.markers.push(at_us);
                    if self.markers.len() > self.marker_cap {
                        self.markers.remove(0);
                    }
                    (true, "marker inserted".to_string())
                }
                TriggerAction::ClearTrace => {
                    self.trace.clear();
                    self.fr_trace.clear();
                    self.eth_trace.clear();
                    self.publish_trace();
                    // Clearing the view is also the re-arm: appearance
                    // watches fire again on their next occurrence.
                    for t in &mut self.triggers {
                        t.rearm();
                    }
                    *status = "trigger cleared the trace".to_string();
                    (true, "trace cleared".to_string())
                }
                TriggerAction::SetSysVar { key, value } => {
                    if self.sysvar_write(&key, value) {
                        (true, format!("set {key} = {value}"))
                    } else {
                        (false, format!("unknown variable {key}"))
                    }
                }
                TriggerAction::RunScript { name, value } => {
                    if !self.measuring {
                        (false, format!("\"{name}\" needs a running measurement"))
                    } else if self.pending_panel_actions.len() >= 32 {
                        (false, "too many queued script actions".to_string())
                    } else {
                        self.pending_panel_actions.push((name.clone(), value));
                        (true, format!("queued \"{name}\""))
                    }
                }
            };
            self.trigger_fires.push_back(crate::trigger::TriggerFire {
                index,
                at_us,
                text: text.clone(),
                action: label.clone(),
                result: result.clone(),
                ok,
            });
            if self.trigger_fires.len() > TRIGGER_FIRE_CAP {
                self.trigger_fires.pop_front();
            }
            self.write_push(
                if ok {
                    WriteKind::Info
                } else {
                    WriteKind::Warning
                },
                format!("trigger {}: {text} -> {label}: {result}", index + 1),
            );
        }
    }

    /// Transmits one frame from the generator entry `(ch, id)`, stamped
    /// `at_us`. The frame goes onto `buf`, so the running tick processes
    /// it exactly like received traffic -- trace, aggregates, load,
    /// recording -- and the trigger evaluator sees it too. That is safe:
    /// every frame-driven condition latches on the frame it matched, so
    /// a rule reacting to its own output cannot loop.
    ///
    /// `mirror` carries the triggering frame's decoded signal values: every
    /// entry whose name matches a signal of the target message is encoded
    /// over the payload, turning a reaction send into a gateway hop. A
    /// periodic send of the same entry passes an empty map and keeps its
    /// own base bytes and waveforms.
    /// Queues one whole FlexRay entry, encoded at the firing instant. It
    /// lands with the next step's generator block, exactly like a script's
    /// `fr_send`, so a reaction cannot outrun the frame that caused it.
    pub(super) fn send_fr_one_shot(&mut self, bus: u8, slot: u16, at_us: u64) -> bool {
        let Some(i) = self
            .fr_tx_list
            .iter()
            .position(|t| t.bus == bus && t.slot == slot)
        else {
            return false;
        };
        let tx = &self.fr_tx_list[i];
        let payload = self
            .fr_models
            .get(&bus)
            .and_then(|runtime| {
                let cluster = *self.fr_cluster_ids.get(&bus)?;
                crate::generator::fr_tx_payload_with_model(
                    cluster,
                    &runtime.comm,
                    &runtime.plans,
                    tx,
                    at_us,
                )
            })
            .unwrap_or_else(|| tx.data.clone());
        self.fr_from_scripts.push((0, bus, slot, payload));
        true
    }
}
