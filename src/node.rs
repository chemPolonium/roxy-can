//! Simulation nodes: a compiled script bound to one bus channel,
//! CANoe-style. A node reacts to events -- measurement start, frames
//! carrying ids it watches, its own periodic timers -- and produces
//! behaviour by queueing frames onto the bus and printing into its log.
//! All node interaction is text; there are no panels.
//!
//! The runtime lives in the bus core: handlers run inside `step` with an
//! instruction budget per callback, and a `send` from the script lands in
//! an outbox that the core drains onto the bus as real frames.

use crate::script::{Handler, HandlerKind, HostInput, Value, Vm, compile};
use std::collections::VecDeque;

/// Log lines kept per node, oldest first.
const LOG_CAP: usize = 200;
/// Instruction allowance per handler run: plenty for stimulus logic,
/// finite enough that a runaway loop cannot stall the core thread.
const NODE_HANDLER_BUDGET: u64 = 100_000;

pub struct ScriptNode {
    /// Stable identity for commands and persistence -- never an index.
    pub id: u64,
    pub name: String,
    pub channel: u8,
    pub source: String,
    pub enabled: bool,
    /// Present only while measuring: recompiled from `source` at every
    /// start, so edits apply without a separate compile action.
    runtime: Option<NodeRuntime>,
    /// `print` output and runtime errors, oldest first, capped.
    log: VecDeque<String>,
    log_dirty: bool,
    /// Set when a handler failed: the node stops executing until the next
    /// start or source edit, so one bad loop cannot spam the log.
    errored: bool,
}

struct NodeRuntime {
    vm: Vm,
    handlers: Vec<Handler>,
    /// One slot per Timer handler, in handler order. `next_due_us == 0`
    /// means "not armed yet": the first step after start arms it one
    /// period out.
    timers: Vec<TimerSlot>,
}

struct TimerSlot {
    handler_index: usize,
    period_ms: u64,
    next_due_us: u64,
    /// Set by `stop_timer()` inside the handler: the slot never fires
    /// again this run.
    stopped: bool,
}

impl ScriptNode {
    pub fn new(id: u64, name: String, channel: u8) -> Self {
        Self {
            id,
            name,
            channel,
            source: String::new(),
            enabled: true,
            runtime: None,
            log: VecDeque::new(),
            log_dirty: false,
            errored: false,
        }
    }

    pub fn running(&self) -> bool {
        self.runtime.is_some() && !self.errored
    }

    pub fn errored(&self) -> bool {
        self.errored
    }

    /// Oldest-first copy of the log, for the snapshot.
    pub fn log_snapshot(&self) -> Vec<String> {
        self.log.iter().cloned().collect()
    }

    pub fn take_log_if_dirty(&mut self) -> Option<Vec<String>> {
        if self.log_dirty {
            self.log_dirty = false;
            Some(self.log_snapshot())
        } else {
            None
        }
    }

    /// Appends one log line, capping the ring.
    fn push_log(&mut self, line: String) {
        Self::push_log_into(&mut self.log, &mut self.log_dirty, line);
    }

    /// The ring logic over borrowed pieces: handlers run while `runtime`
    /// is mutably borrowed, and the log is a disjoint field.
    fn push_log_into(log: &mut VecDeque<String>, dirty: &mut bool, line: String) {
        if log.len() == LOG_CAP {
            log.pop_front();
        }
        log.push_back(line);
        *dirty = true;
    }

    /// Arms the node for a measurement: recompile, run the main chunk
    /// (globals initialize), fire `on start`, arm timers lazily. Compile
    /// or main errors land in the log and leave the node idle. `dbc` is
    /// the channel's database, backing the signal read/write builtins.
    pub fn start(&mut self, dbc: Option<std::sync::Arc<crate::dbc::SymbolTable>>) {
        self.errored = false;
        let script = match compile(&self.source) {
            Ok(s) => s,
            Err(e) => {
                self.push_log(format!("[compile] {e}"));
                self.errored = true;
                return;
            }
        };
        let handlers = script.handlers.clone();
        let mut vm = Vm::new(script);
        vm.reset_budget(NODE_HANDLER_BUDGET);
        vm.host_extern = Some(Box::new(move |name, args| {
            node_extern(dbc.as_deref(), name, args)
        }));
        if let Err(e) = vm.run() {
            self.push_log(format!("[start] {e}"));
            self.errored = true;
            return;
        }
        let timers = handlers
            .iter()
            .enumerate()
            .filter_map(|(i, h)| match &h.kind {
                HandlerKind::Timer { period_ms } => Some(TimerSlot {
                    handler_index: i,
                    period_ms: *period_ms,
                    next_due_us: 0,
                    stopped: false,
                }),
                _ => None,
            })
            .collect();
        let mut rt = NodeRuntime {
            vm,
            handlers,
            timers,
        };
        // `on start` handlers, in declaration order.
        for h in rt.handlers.clone() {
            if matches!(h.kind, HandlerKind::Start)
                && let Err(e) = rt.vm.run_handler(h.chunk)
            {
                self.fail(&e.to_string());
                return;
            }
        }
        Self::drain_vm(&mut rt, &mut self.log, &mut self.log_dirty);
        self.runtime = Some(rt);
    }

    /// Tears the runtime down; the source and log stay.
    pub fn stop(&mut self) {
        self.runtime = None;
        self.errored = false;
    }

    /// Applies a source edit. While measuring a running node recompiles
    /// and restarts in place (globals reset -- a fresh start of that
    /// node); while stopped the edit simply waits for the next start.
    pub fn set_source(
        &mut self,
        source: String,
        dbc: Option<std::sync::Arc<crate::dbc::SymbolTable>>,
        measuring: bool,
    ) {
        self.source = source;
        if measuring && self.enabled {
            self.start(dbc);
        } else if self.runtime.is_some() {
            self.runtime = None;
            self.errored = false;
        }
    }

    /// Enabled nodes run with the measurement; toggling on while
    /// measuring starts the node immediately.
    pub fn set_enabled(
        &mut self,
        on: bool,
        dbc: Option<std::sync::Arc<crate::dbc::SymbolTable>>,
        measuring: bool,
    ) {
        self.enabled = on;
        if measuring {
            if on {
                self.start(dbc);
            } else {
                self.runtime = None;
                self.errored = false;
            }
        }
    }

    /// Wall-clock dues of every armed timer, for the core loop's sleep
    /// deadline.
    pub fn timer_dues(&self) -> Vec<u64> {
        match &self.runtime {
            Some(rt) if self.enabled && !self.errored => rt
                .timers
                .iter()
                .filter(|t| t.next_due_us != 0)
                .map(|t| t.next_due_us)
                .collect(),
            _ => Vec::new(),
        }
    }

    /// Delivers one bus frame: handlers for this id on this channel run,
    /// in declaration order. `input` is what `now()`/`sig()` read.
    /// Returns the frames the script queued.
    pub fn dispatch_frame(
        &mut self,
        channel: u8,
        id: u32,
        input: &HostInput,
    ) -> Vec<(u32, Vec<u8>)> {
        let mut out = Vec::new();
        let Some(rt) = self.runtime.as_mut() else {
            return out;
        };
        if !self.enabled || self.errored || channel != self.channel {
            return out;
        }
        let matches: Vec<u16> = rt
            .handlers
            .iter()
            .filter(|h| matches!(h.kind, HandlerKind::Message { id: mid } if mid == id))
            .map(|h| h.chunk)
            .collect();
        for chunk in matches {
            rt.vm.reset_budget(NODE_HANDLER_BUDGET);
            rt.vm.host_input = input.clone();
            if let Err(e) = rt.vm.run_handler(chunk) {
                Self::push_log_into(&mut self.log, &mut self.log_dirty, format!("[error] {e}"));
                self.errored = true;
                return out;
            }
            Self::drain_vm(rt, &mut self.log, &mut self.log_dirty);
            out.append(&mut rt.vm.outbox);
        }
        out
    }

    /// Fires due timer handlers. A timer armed lazily at `start` gets its
    /// first due one full period out; missed periods (a stalled host)
    /// collapse into one fire plus a resync.
    pub fn run_timers(&mut self, now_us: u64, input: &HostInput) -> Vec<(u32, Vec<u8>)> {
        let mut out = Vec::new();
        let Some(rt) = self.runtime.as_mut() else {
            return out;
        };
        if !self.enabled || self.errored {
            return out;
        }
        rt.vm.host_input = input.clone();
        rt.vm.timer_ops.clear();
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
                    slot.next_due_us = now_us.saturating_add(slot.period_ms * 1_000);
                    (slot_index, chunk)
                })
            })
            .collect();
        for (slot_index, chunk) in due {
            rt.vm.reset_budget(NODE_HANDLER_BUDGET);
            rt.vm.timer_ops.clear();
            if let Err(e) = rt.vm.run_handler(chunk) {
                Self::push_log_into(&mut self.log, &mut self.log_dirty, format!("[error] {e}"));
                self.errored = true;
                return out;
            }
            // Apply timer control the handler queued: a new period takes
            // effect from now, and a stopped timer never fires again.
            for op in rt.vm.timer_ops.drain(..) {
                match op {
                    crate::script::TimerOp::SetPeriod(ms) => {
                        rt.timers[slot_index].period_ms = ms;
                        rt.timers[slot_index].next_due_us = now_us.saturating_add(ms * 1_000);
                    }
                    crate::script::TimerOp::Stop => rt.timers[slot_index].stopped = true,
                }
            }
            Self::drain_vm(rt, &mut self.log, &mut self.log_dirty);
            out.append(&mut rt.vm.outbox);
        }
        out
    }

    /// Moves freshly printed lines from the VM into the node's log ring.
    fn drain_vm(rt: &mut NodeRuntime, log: &mut VecDeque<String>, dirty: &mut bool) {
        for line in rt.vm.output.drain(..) {
            Self::push_log_into(log, dirty, line);
        }
    }

    fn fail(&mut self, msg: &str) {
        self.errored = true;
        self.push_log(format!("[error] {msg}"));
    }
}

/// The node's host-registered builtins -- the S4 seam made real: anything
/// beyond the language's builtin table lands here, and external
/// simulation components will register the same way.
///
/// - `set_sig(buf, id, "Name", value)`: encodes one physical value into
///   the buffer through the channel database (buffer pads to 8 bytes).
/// - `get_sig(buf, id, "Name")`: decodes one physical value out of the
///   buffer, erroring when the database lacks the signal or no value is
///   representable.
fn node_extern(
    dbc: Option<&crate::dbc::SymbolTable>,
    name: &str,
    args: &[Value],
) -> Result<Option<Value>, String> {
    let db =
        dbc.ok_or_else(|| "no database on this channel: signal builtins unavailable".to_string())?;
    match name {
        "set_sig" => {
            if args.len() != 4 {
                return Err("set_sig(buf, id, \"Name\", value) takes 4 arguments".to_string());
            }
            let Value::Bytes(buf) = &args[0] else {
                return Err("set_sig: first argument must be a byte buffer".into());
            };
            let Value::Int(id) = args[1] else {
                return Err("set_sig: message id must be an int".into());
            };
            let Value::Str(sig) = &args[2] else {
                return Err("set_sig: signal name must be a string".into());
            };
            let value = match &args[3] {
                Value::Int(n) => *n as f64,
                Value::Float(f) => *f,
                _ => return Err("set_sig: value must be a number".into()),
            };
            let mut data = buf.lock().expect("buffer poisoned");
            if data.len() < 8 {
                data.resize(8, 0);
            }
            if db.encode_signal(id as u32, sig, value, &mut data) {
                Ok(Some(Value::Nil))
            } else {
                Err(format!("set_sig: unknown message/signal {id:#x} {sig}"))
            }
        }
        "get_sig" => {
            if args.len() != 3 {
                return Err("get_sig(buf, id, \"Name\") takes 3 arguments".into());
            }
            let Value::Bytes(buf) = &args[0] else {
                return Err("get_sig: first argument must be a byte buffer".into());
            };
            let Value::Int(id) = args[1] else {
                return Err("get_sig: message id must be an int".into());
            };
            let Value::Str(sig) = &args[2] else {
                return Err("get_sig: signal name must be a string".into());
            };
            let data = buf.lock().expect("buffer poisoned");
            let frame = crate::can::frame::CanFrame {
                t_us: 0,
                channel: 0,
                id: id as u32,
                extended: false,
                len: data.len() as u8,
                data: {
                    let mut d = [0u8; crate::can::frame::MAX_CAN_FD_LEN];
                    let n = data.len().min(d.len());
                    d[..n].copy_from_slice(&data[..n]);
                    d
                },
                dir: crate::can::frame::Direction::Rx,
                flags: crate::can::frame::FrameFlags::NONE,
            };
            let decoded = db
                .decode_signals(&frame)
                .into_iter()
                .find(|s| s.name == *sig)
                .ok_or_else(|| format!("get_sig: unknown message/signal {id:#x} {sig}"))?;
            Ok(Some(Value::Float(decoded.phys)))
        }
        _ => Ok(None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(source: &str) -> ScriptNode {
        let mut n = ScriptNode::new(1, "n".into(), 0);
        n.source = source.to_string();
        n
    }

    #[test]
    fn start_message_and_timers_drive_the_log_and_outbox() {
        let mut n = node(
            r#"
                let ticks = 0;
                on start { print("hello"); }
                on message 0x100 { send(0x200, 1, 2); }
                on timer 100 {
                    ticks = ticks + 1;
                    print("tick", ticks);
                    send(0x300);
                }
            "#,
        );
        n.start(None);
        assert!(n.running());
        // on start already printed.
        assert_eq!(n.log_snapshot(), ["hello"]);

        // A watched frame queues the reaction payload.
        let out = n.dispatch_frame(0, 0x100, &HostInput::default());
        assert_eq!(out, vec![(0x200, vec![1, 2])]);

        // Timers arm lazily one period out (armed at 50 ms -> first due
        // at 150 ms), then fire and resync.
        assert!(n.run_timers(50_000, &HostInput::default()).is_empty());
        assert!(n.run_timers(120_000, &HostInput::default()).is_empty());
        let out = n.run_timers(160_000, &HostInput::default());
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].0, 0x300);
        assert!(n.log_snapshot().last().unwrap().starts_with("tick 1"));

        // An error in a handler stops the node until restart.
        let mut bad = node("on message 0x100 { print(1 / 0); }");
        bad.start(None);
        bad.dispatch_frame(0, 0x100, &HostInput::default());
        assert!(bad.errored());
        assert!(
            bad.dispatch_frame(0, 0x100, &HostInput::default())
                .is_empty(),
            "errored nodes stay quiet"
        );
        bad.start(None);
        assert!(bad.running(), "a restart clears the error");
    }

    #[test]
    fn compile_errors_land_in_the_log() {
        let mut n = node("on start { print(; }");
        n.start(None);
        assert!(!n.running());
        assert!(n.log_snapshot()[0].contains("[compile]"));
    }

    #[test]
    fn stop_timer_halts_a_periodic_handler_after_its_runs() {
        let mut n = node(
            r#"
                let n = 0;
                on timer 100 {
                    n = n + 1;
                    print("tick", n);
                    if (n == 2) { stop_timer(); }
                }
            "#,
        );
        n.start(None);
        n.run_timers(50_000, &HostInput::default()); // arms: due 150_000
        n.run_timers(160_000, &HostInput::default()); // tick 1, arms 310_000
        n.run_timers(320_000, &HostInput::default()); // tick 2, stops
        n.run_timers(1_000_000, &HostInput::default()); // stopped: silence
        eprintln!("DEBUG LOG: {:?}", n.log_snapshot());
        let ticks = n
            .log_snapshot()
            .iter()
            .filter(|l| l.starts_with("tick "))
            .count();
        assert_eq!(ticks, 2, "stop_timer must halt the periodic handler");
    }

    #[test]
    fn set_period_reschedules_the_running_timer() {
        let mut n = node(
            r#"
                let ticks = 0;
                on timer 100 {
                    ticks = ticks + 1;
                    if (ticks == 1) { set_period(50); }
                    print("t", ticks);
                }
            "#,
        );
        n.start(None);
        n.run_timers(50_000, &HostInput::default()); // arms: due 150_000
        n.run_timers(150_000, &HostInput::default()); // tick 1, period -> 50ms
        n.run_timers(199_999, &HostInput::default()); // not yet
        n.run_timers(200_000, &HostInput::default()); // tick 2
        let ticks = n
            .log_snapshot()
            .iter()
            .filter(|l| l.starts_with("t "))
            .count();
        assert_eq!(ticks, 2);
    }

    #[test]
    fn channel_mismatch_is_ignored() {
        let mut n = node("on message 0x100 { send(0x200); }");
        n.start(None);
        assert!(
            n.dispatch_frame(1, 0x100, &HostInput::default()).is_empty(),
            "other channel"
        );
        assert_eq!(n.dispatch_frame(0, 0x100, &HostInput::default()).len(), 1);
    }

    #[test]
    fn now_and_sig_read_the_published_host_input() {
        let mut n = node(
            r#"
                on message 0x100 {
                    print(now());
                    print(sig(0x100, "RPM"));
                }
            "#,
        );
        n.start(None);
        let input = HostInput {
            now_s: 1.5,
            signals: [((0x100, "RPM".to_string()), 2400.0)].into_iter().collect(),
        };
        n.dispatch_frame(0, 0x100, &input);
        assert_eq!(n.log_snapshot(), ["1.5", "2400.0"]);

        // An unseen signal is a runtime error, not a silent zero.
        let mut n2 = node(r#"on message 0x100 { print(sig(0x100, "Nope")); }"#);
        n2.start(None);
        n2.dispatch_frame(0, 0x100, &HostInput::default());
        assert!(n2.errored(), "a missing signal must not read as zero");
    }

    #[test]
    fn extended_ids_send_flagged_extended() {
        // The outbox carries the raw id; the core derives the extended
        // flag from its size when building the frame.
        let mut n = node("on message 0x100 { send(0x18FF10, 1); }");
        n.start(None);
        let out = n.dispatch_frame(0, 0x100, &HostInput::default());
        assert_eq!(out, vec![(0x18FF10, vec![1])]);
    }

    const SIG_DBC: &str = r#"VERSION "roxy-can node sig test"

NS_ :

BU_: ECU

BO_ 512 Status: 8 ECU
 SG_ RPM : 0|16@1+ (0.25,0) [0|0] "" ECU
"#;

    #[test]
    fn set_sig_and_get_sig_encode_through_the_channel_database() {
        let dbc = std::sync::Arc::new(crate::dbc::load_dbc_str(SIG_DBC).unwrap());
        let mut n = node(
            r#"
                let buf = bytes(8);
                on start {
                    set_sig(buf, 0x200, "RPM", 1000);
                    print(get_sig(buf, 0x200, "RPM"));
                }
                on message 0x1 { send(0x200, buf); }
            "#,
        );
        n.start(Some(dbc));
        assert!(n.running(), "log: {:?}", n.log_snapshot());
        assert_eq!(
            n.log_snapshot(),
            ["1000.0"],
            "get_sig reads back what set_sig encoded"
        );

        // A later frame flushes the on-start send: the composed buffer
        // leaves as the payload, little-endian raw 4000 (1000 / 0.25) in
        // bytes 0-1.
        let out = n.dispatch_frame(0, 0x1, &HostInput::default());
        assert_eq!(out, vec![(0x200, vec![0xA0, 0x0F, 0, 0, 0, 0, 0, 0])]);
    }
}
