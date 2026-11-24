//! ScriptNode methods: dispatch.

use super::*;

impl ScriptNode {
    /// Delivers one bus frame: handlers for this id on this channel run,
    /// in declaration order. `input` is what `now()`/`sig()` read.
    /// `data` is the triggering frame's payload, exposed to `on message`
    /// handlers via `frame_byte(n)` / `frame_dlc()`. A handler whose id
    /// sits in the standard range matches standard frames only; a larger
    /// handler id names a 29-bit extended frame, mirroring `send`. Error
    /// frames only reach `on errorFrame` handlers.
    /// Returns the frames the script queued as `(id, extended, payload)`.
    pub fn dispatch_frame(
        &mut self,
        channel: u8,
        id: u32,
        extended: bool,
        is_error: bool,
        data: &[u8],
        input: &HostInput,
    ) -> Vec<crate::script::ScriptSend> {
        let mut out = Vec::new();
        let Some(rt) = self.runtime.as_mut() else {
            return out;
        };
        if !self.enabled || self.errored || channel != self.channel {
            return out;
        }
        rt.vm.frame_bytes = data.to_vec();
        rt.vm.frame_id = id;
        rt.vm.frame_bus = channel;
        rt.vm.frame_present = true;
        rt.vm.frame_events.push((id, input.now_s));
        if rt.vm.frame_events.len() > 1024 {
            rt.vm.frame_events.remove(0);
        }
        rt.vm.frame_is_fr = false;
        // A CAN frame has no cycle; clearing it keeps `fr_cycle()` and
        // `this.cycle` from reporting the previous FlexRay arrival.
        rt.vm.fr_cycle = 0;
        // Leftover ops from `on start` arm here if no tick ran first.
        let now_us = (input.now_s.max(0.0) * 1e6) as u64;
        let pending: Vec<crate::script::TimerOp> = std::mem::take(&mut rt.vm.timer_ops);
        for op in pending {
            if let Some(warn) = Self::apply_timer_op(rt, op, now_us, None) {
                Self::push_log_into(
                    &mut self.log,
                    &mut self.log_dirty,
                    &mut self.pending_lines,
                    warn,
                );
            }
        }
        let named = &rt.named_handler_ids;
        let matches: Vec<u16> = rt
            .handlers
            .iter()
            .filter(|h| match &h.kind {
                // Error frames belong to `on errorFrame` alone; regular
                // handlers never see them. An extended-message handler
                // matches extended frames by numeric id even inside the
                // standard range.
                HandlerKind::ErrorFrame => is_error,
                HandlerKind::AnyMessage => !is_error,
                HandlerKind::Message { id: mid, bus } => {
                    !is_error
                        && *mid == id
                        && extended == (*mid > 0x7FF)
                        && bus.is_none_or(|wanted| wanted == channel)
                }
                HandlerKind::NamedMessage { name } => {
                    !is_error
                        && named
                            .get(name)
                            .is_some_and(|(mid, m_ext)| *mid == id && *m_ext == extended)
                }
                HandlerKind::ExtendedMessage { id: mid } => !is_error && extended && *mid == id,
                _ => false,
            })
            .map(|h| h.chunk)
            .collect();
        for chunk in matches {
            let mut sink = RunSink {
                log: &mut self.log,
                log_dirty: &mut self.log_dirty,
                pending_lines: &mut self.pending_lines,
                out: &mut out,
            };
            let r = Self::run_chunk(rt, &mut sink, chunk, None, input);
            if let Err(error) = r {
                self.errored = true;
                self.issue = runtime_issue(&error);
                return out;
            }
        }
        out
    }

    /// Runs every `on someip <service>.<method>` handler. Called by the core
    /// after a decoded SOME/IP event; the handler sees client/session/return
    /// and payload through the `someip_*` builtins.
    #[allow(clippy::too_many_arguments)]
    pub fn dispatch_someip(
        &mut self,
        service: u16,
        method: u16,
        client: u16,
        session: u16,
        return_code: u8,
        payload: &[u8],
        input: &HostInput,
    ) -> usize {
        let mut ran = 0;
        let Some(rt) = self.runtime.as_mut() else {
            return 0;
        };
        if !self.enabled || self.errored {
            return 0;
        }
        rt.vm.frame_present = false;
        rt.vm.someip_service = service;
        rt.vm.someip_method = method;
        rt.vm.someip_client = client;
        rt.vm.someip_session = session;
        rt.vm.someip_return = return_code;
        rt.vm.someip_payload = payload.to_vec();
        let chunks: Vec<u16> = rt
            .handlers
            .iter()
            .filter(|handler| {
                matches!(
                    handler.kind,
                    HandlerKind::SomeIp {
                        service: want_service,
                        method: want_method,
                    } if want_service == service && want_method == method
                )
            })
            .map(|handler| handler.chunk)
            .collect();
        for chunk in chunks {
            let mut sink = RunSink {
                log: &mut self.log,
                log_dirty: &mut self.log_dirty,
                pending_lines: &mut self.pending_lines,
                out: &mut Vec::new(),
            };
            if let Err(error) = Self::run_chunk(rt, &mut sink, chunk, None, input) {
                self.errored = true;
                self.issue = runtime_issue(&error);
                break;
            }
            ran += 1;
        }
        ran
    }

    /// Runs every `on panel_action("name")` handler in declaration order.
    /// The web button path uses this instead of calling a user function
    /// directly, so the handler still runs inside the node's step budget and
    /// its sends/emits/log lines follow the normal outbox path. `value` is
    /// the control's numeric argument; a handler that declares a parameter
    /// receives it in local slot 0 (`nil` when absent).
    pub fn dispatch_panel_action(
        &mut self,
        name: &str,
        value: Option<f64>,
        input: &HostInput,
    ) -> (usize, Vec<crate::script::ScriptSend>) {
        let mut out = Vec::new();
        let Some(rt) = self.runtime.as_mut() else {
            return (0, out);
        };
        if !self.enabled || self.errored {
            return (0, out);
        }
        rt.vm.host_input = input.clone();
        rt.vm.frame_present = false;
        rt.vm.frame_is_fr = false;
        rt.vm.fr_cycle = 0;
        rt.vm.frame_bytes.clear();
        let matches: Vec<(u16, bool)> = rt
            .handlers
            .iter()
            .filter(|handler| {
                matches!(&handler.kind, HandlerKind::PanelAction { name: action, .. } if action == name)
            })
            .map(|handler| {
                (
                    handler.chunk,
                    matches!(&handler.kind, HandlerKind::PanelAction { arg: Some(_), .. }),
                )
            })
            .collect();
        let mut ran = 0;
        for (chunk, with_arg) in matches {
            ran += 1;
            let arg = with_arg
                .then(|| value.map_or(crate::script::Value::Nil, crate::script::Value::Float));
            let mut sink = RunSink {
                log: &mut self.log,
                log_dirty: &mut self.log_dirty,
                pending_lines: &mut self.pending_lines,
                out: &mut out,
            };
            let result = Self::run_chunk(rt, &mut sink, chunk, arg, input);
            if let Err(error) = result {
                self.errored = true;
                self.issue = runtime_issue(&error);
                break;
            }
        }
        (ran, out)
    }

    /// Does this node listen for any FlexRay arrival? The bus asks once per
    /// step: a FlexRay-heavy log must not pay a dispatch scan (let alone a
    /// `build_node_inputs`) while no script has an `on fr slot` handler.
    pub fn waits_on_flexray(&self) -> bool {
        self.enabled
            && !self.errored
            && self.runtime.as_ref().is_some_and(|rt| {
                rt.handlers
                    .iter()
                    .any(|h| matches!(h.kind, HandlerKind::FrSlot { .. }))
            })
    }

    /// Delivers one FlexRay arrival to this node's `on fr slot` handlers.
    ///
    /// A node binds to a CAN channel, and an arrival has no CAN channel to
    /// match against, so the cluster written in the handler is the only filter
    /// there is: `on fr slot 13` fires on slot 13 of every cluster,
    /// `on fr 0 slot 13` on FR0's alone. The frame context is the arrival
    /// itself -- `frame_id()` reads the slot, `frame_dlc()`/`frame_byte()` the
    /// payload, `fr_cycle()` the cycle it came in. Sends still leave on the
    /// node's CAN channel: FlexRay has no send path yet, so a handler can
    /// react to a FlexRay frame with CAN traffic, which is what a gateway does.
    pub fn dispatch_fr_row(
        &mut self,
        bus: u8,
        slot: u16,
        cycle: u8,
        payload: &[u8],
        input: &HostInput,
    ) -> Vec<crate::script::ScriptSend> {
        let mut out = Vec::new();
        let Some(rt) = self.runtime.as_mut() else {
            return out;
        };
        if !self.enabled || self.errored {
            return out;
        }
        rt.vm.frame_bytes = payload.to_vec();
        rt.vm.frame_id = u32::from(slot);
        rt.vm.frame_bus = bus;
        rt.vm.frame_present = true;
        rt.vm.frame_is_fr = true;
        rt.vm.fr_cycle = cycle;
        let matches: Vec<u16> = rt
            .handlers
            .iter()
            .filter(|h| {
                let HandlerKind::FrSlot {
                    bus: want,
                    slot: want_slot,
                } = &h.kind
                else {
                    return false;
                };
                *want_slot == slot && want.is_none_or(|b| b == bus)
            })
            .map(|h| h.chunk)
            .collect();
        for chunk in matches {
            let mut sink = RunSink {
                log: &mut self.log,
                log_dirty: &mut self.log_dirty,
                pending_lines: &mut self.pending_lines,
                out: &mut out,
            };
            let r = Self::run_chunk(rt, &mut sink, chunk, None, input);
            if let Err(error) = r {
                self.errored = true;
                self.issue = runtime_issue(&error);
                return out;
            }
        }
        out
    }

    /// Runs one handler chunk and takes out what the run left behind: log
    /// lines, emitted values, sysvar writes, timer ops, queued sends. Shared
    /// by the CAN and the FlexRay delivery -- the two differ in which handlers
    /// match and what the frame context holds, never in what a run produces.
    /// `Err` is a failed handler; the caller marks the node errored and stops.
    pub(super) fn run_chunk(
        rt: &mut NodeRuntime,
        sink: &mut RunSink,
        chunk: u16,
        arg: Option<crate::script::Value>,
        input: &HostInput,
    ) -> Result<(), String> {
        // The handler's clock is the input's, not the step's: both deliveries
        // publish `now_s` from the same value they stamp their frames with.
        let now_us = (input.now_s.max(0.0) * 1e6) as u64;
        rt.vm.reset_budget(NODE_HANDLER_BUDGET);
        rt.vm.host_input = input.clone();
        let run = match arg {
            Some(value) => rt.vm.run_handler_with(chunk, value),
            None => rt.vm.run_handler(chunk),
        };
        if let Err(e) = run {
            let message = e.to_string();
            Self::push_log_into(
                &mut *sink.log,
                &mut *sink.log_dirty,
                &mut *sink.pending_lines,
                format!("[error] {message}"),
            );
            return Err(message);
        }
        Self::drain_vm(
            rt,
            &mut *sink.log,
            &mut *sink.log_dirty,
            &mut *sink.pending_lines,
        );
        // Named one-shot ops stay meaningful from any handler. The
        // running-timer ops (`set_period`/`stop_timer`) name no slot
        // here and are dropped.
        let ops: Vec<crate::script::TimerOp> = std::mem::take(&mut rt.vm.timer_ops);
        for op in ops {
            if let Some(warn) = Self::apply_timer_op(rt, op, now_us, None) {
                Self::push_log_into(
                    &mut *sink.log,
                    &mut *sink.log_dirty,
                    &mut *sink.pending_lines,
                    warn,
                );
            }
        }
        sink.out.append(&mut rt.vm.outbox);
        Ok(())
    }

    /// Fires due timer handlers. A timer armed lazily at `start` gets its
    /// first due one full period out; missed periods (a stalled host)
    /// collapse into one fire plus a resync. A named one-shot is spent
    /// the moment it fires; `set_timer` inside its own handler re-arms it.
    pub fn run_timers(&mut self, now_us: u64, input: &HostInput) -> Vec<crate::script::ScriptSend> {
        let mut out = Vec::new();
        let Some(rt) = self.runtime.as_mut() else {
            return out;
        };
        if !self.enabled || self.errored {
            return out;
        }
        rt.vm.host_input = input.clone();
        // Ops queued before this tick (e.g. `set_timer` from `on start`)
        // arm against the first tick's clock.
        let pending: Vec<crate::script::TimerOp> = std::mem::take(&mut rt.vm.timer_ops);
        for op in pending {
            if let Some(warn) = Self::apply_timer_op(rt, op, now_us, None) {
                Self::push_log_into(
                    &mut self.log,
                    &mut self.log_dirty,
                    &mut self.pending_lines,
                    warn,
                );
            }
        }
        let due: Vec<(usize, u16)> = rt
            .timers
            .iter_mut()
            .enumerate()
            .filter_map(|(slot_index, slot)| {
                if slot.stopped {
                    return None;
                }
                if slot.next_due_us == 0 {
                    slot.next_due_us = now_us.saturating_add(slot.period_ms * 1_000);
                    return None;
                }
                (slot.next_due_us <= now_us).then(|| {
                    let chunk = rt.handlers[slot.handler_index].chunk;
                    if slot.name.is_some() {
                        // Spent on firing; a `set_timer` in the handler
                        // re-arms the slot afterwards.
                        slot.stopped = true;
                        slot.next_due_us = 0;
                    } else {
                        // Anchor the next beat on the schedule, not on
                        // the late fire: one late wake must not shift
                        // every beat after it. Beats missed outright (a
                        // long pause) collapse into the next due.
                        let period = slot.period_ms.saturating_mul(1_000);
                        slot.next_due_us = if period == 0 {
                            now_us
                        } else {
                            let mut due = slot.next_due_us.saturating_add(period);
                            while due <= now_us {
                                due = due.saturating_add(period);
                            }
                            due
                        };
                    }
                    (slot_index, chunk)
                })
            })
            .collect();
        // Timers are not frame events: `this.*` has no current frame to
        // describe, and a stale arrival must not leak into the handler.
        rt.vm.frame_present = false;
        rt.vm.frame_is_fr = false;
        rt.vm.fr_cycle = 0;
        for (slot_index, chunk) in due {
            rt.vm.reset_budget(NODE_HANDLER_BUDGET);
            rt.vm.timer_ops.clear();
            if let Err(e) = rt.vm.run_handler(chunk) {
                let message = e.to_string();
                Self::push_log_into(
                    &mut self.log,
                    &mut self.log_dirty,
                    &mut self.pending_lines,
                    format!("[error] {message}"),
                );
                self.errored = true;
                self.issue = runtime_issue(&message);
                return out;
            }
            // Apply timer control the handler queued: a new period takes
            // effect from now, and a stopped timer never fires again.
            let ops: Vec<crate::script::TimerOp> = std::mem::take(&mut rt.vm.timer_ops);
            for op in ops {
                if let Some(warn) = Self::apply_timer_op(rt, op, now_us, Some(slot_index)) {
                    Self::push_log_into(
                        &mut self.log,
                        &mut self.log_dirty,
                        &mut self.pending_lines,
                        warn,
                    );
                }
            }
            Self::drain_vm(
                rt,
                &mut self.log,
                &mut self.log_dirty,
                &mut self.pending_lines,
            );
            out.append(&mut rt.vm.outbox);
        }
        out
    }

    /// Applies one queued timer op. `slot` is the running handler's own
    /// slot when called from the timer loop, None elsewhere -- the
    /// running-timer ops warn when they have no slot to act on.
    /// Returns a warning line for an `set_timer` naming no declared
    /// one-shot, or a stray `set_period`/`stop_timer`.
    pub(super) fn apply_timer_op(
        rt: &mut NodeRuntime,
        op: crate::script::TimerOp,
        now_us: u64,
        slot: Option<usize>,
    ) -> Option<String> {
        match op {
            crate::script::TimerOp::SetPeriod(ms) => {
                let Some(i) = slot else {
                    return Some(
                        "[timer] set_period outside an `on timer` handler has no effect"
                            .to_string(),
                    );
                };
                let s = &mut rt.timers[i];
                s.period_ms = ms;
                s.next_due_us = now_us.saturating_add(ms * 1_000);
                if s.name.is_some() {
                    // A named slot is spent before its handler runs, so a
                    // `set_period` there counts as a re-arm.
                    s.stopped = false;
                }
                None
            }
            crate::script::TimerOp::Stop => {
                if let Some(i) = slot {
                    rt.timers[i].stopped = true;
                    return None;
                }
                Some("[timer] stop_timer outside an `on timer` handler has no effect".to_string())
            }
            crate::script::TimerOp::Arm { name, ms } => {
                let due = now_us.saturating_add(ms.saturating_mul(1_000));
                // Re-arm in place when the name already has a slot.
                if let Some(s) = rt
                    .timers
                    .iter_mut()
                    .find(|s| s.name.as_deref() == Some(name.as_str()))
                {
                    s.period_ms = ms;
                    s.next_due_us = due;
                    s.stopped = false;
                    return None;
                }
                // Otherwise bind a fresh slot to its declared handler.
                let declared = rt.handlers.iter().enumerate().find_map(|(i, h)| {
                    matches!(&h.kind, HandlerKind::Oneshot { name: n } if *n == name).then_some(i)
                });
                if let Some(i) = declared {
                    rt.timers.push(TimerSlot {
                        handler_index: i,
                        period_ms: ms,
                        next_due_us: due,
                        name: Some(name),
                        stopped: false,
                    });
                    return None;
                }
                Some(format!("[timer] set_timer(\"{name}\"): no such handler"))
            }
            crate::script::TimerOp::Cancel { name } => {
                if let Some(s) = rt
                    .timers
                    .iter_mut()
                    .find(|s| s.name.as_deref() == Some(name.as_str()))
                {
                    s.stopped = true;
                    s.next_due_us = 0;
                }
                None
            }
        }
    }

    /// Moves freshly printed lines from the VM into the node's log ring,
    /// and derived-signal samples into the runtime's emission queue.
    pub(super) fn drain_vm(
        rt: &mut NodeRuntime,
        log: &mut VecDeque<String>,
        dirty: &mut bool,
        pending: &mut Vec<String>,
    ) {
        for line in rt.vm.output.drain(..) {
            Self::push_log_into(log, dirty, pending, line);
        }
        rt.emitted.append(&mut rt.vm.emitted);
        rt.pending_sys.append(&mut rt.vm.sys_sets);
        rt.fr_out.append(&mut rt.vm.fr_outbox);
        rt.fr_sig.append(&mut rt.vm.fr_sig_sets);
        rt.frames.append(&mut rt.vm.frame_outbox);
        rt.someip.append(&mut rt.vm.someip_outbox);
        rt.someip_tcp.append(&mut rt.vm.someip_tcp_outbox);
        rt.someip_offer.append(&mut rt.vm.someip_offer_outbox);
        rt.someip_reply.append(&mut rt.vm.someip_reply_outbox);
        rt.nm_timing.append(&mut rt.vm.nm_timing_outbox);
    }

    /// Fires `on signal` / `on sysvar` handlers whose watched value changed
    /// since the previous bus step. The first observation only seeds the
    /// baseline: a freshly started measurement must not fire every handler
    /// just because the signal has an initial value.
    pub fn dispatch_changes(&mut self, input: &HostInput) -> Vec<crate::script::ScriptSend> {
        let mut out = Vec::new();
        let Some(rt) = self.runtime.as_mut() else {
            return out;
        };
        if !self.enabled || self.errored {
            return out;
        }
        let watches: Vec<(u16, String, f64)> = rt
            .handlers
            .iter()
            .filter_map(|handler| match &handler.kind {
                HandlerKind::Signal { name } => Self::named_signal_value(rt, input, name)
                    .map(|value| (handler.chunk, format!("signal:{name}"), value)),
                HandlerKind::SysVar { key } => input
                    .sysvars
                    .get(key)
                    .copied()
                    .map(|value| (handler.chunk, format!("sysvar:{key}"), value)),
                _ => None,
            })
            .collect();
        for (chunk, key, value) in watches {
            let changed = rt.last_values.get(&key).is_some_and(|prev| *prev != value);
            rt.last_values.insert(key, value);
            if !changed {
                continue;
            }
            rt.vm.frame_present = false;
            rt.vm.frame_is_fr = false;
            rt.vm.fr_cycle = 0;
            rt.vm.frame_bytes.clear();
            let mut sink = RunSink {
                log: &mut self.log,
                log_dirty: &mut self.log_dirty,
                pending_lines: &mut self.pending_lines,
                out: &mut out,
            };
            if let Err(error) = Self::run_chunk(rt, &mut sink, chunk, None, input) {
                self.errored = true;
                self.issue = runtime_issue(&error.to_string());
                break;
            }
        }
        out
    }

    /// Resolve a dotted `"Message.Signal"` watch through the DBC names the
    /// host injected, or a bare signal name against any watched signal.
    fn named_signal_value(rt: &NodeRuntime, input: &HostInput, name: &str) -> Option<f64> {
        if let Some((msg, sig)) = name.split_once('.') {
            let id = *rt.vm.named_messages.get(msg)?;
            return input.signals.get(&(id, sig.to_string())).copied();
        }
        input
            .signals
            .iter()
            .find(|((_, sig), _)| sig == name)
            .map(|(_, value)| *value)
    }
}
