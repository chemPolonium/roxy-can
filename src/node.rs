//! Simulation nodes: a compiled script bound to one bus channel,
//! CANoe-style. A node reacts to events -- measurement start, frames
//! carrying ids it watches, its own periodic timers -- and produces
//! behaviour by queueing frames onto the bus and printing into its log.
//! All node interaction is text; there are no panels.
//!
//! The runtime lives in the bus core: handlers run inside `step` with an
//! instruction budget per callback, and a `send` from the script lands in
//! an outbox that the core drains onto the bus as real frames.

use crate::script::{Handler, HandlerKind, HostInput, Vm, compile};
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
    /// or main errors land in the log and leave the node idle.
    pub fn start(&mut self) {
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
    pub fn set_source(&mut self, source: String, measuring: bool) {
        self.source = source;
        if measuring && self.enabled {
            self.start();
        } else if self.runtime.is_some() {
            self.runtime = None;
            self.errored = false;
        }
    }

    /// Enabled nodes run with the measurement; toggling on while
    /// measuring starts the node immediately.
    pub fn set_enabled(&mut self, on: bool, measuring: bool) {
        self.enabled = on;
        if measuring {
            if on {
                self.start();
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
        let due: Vec<u16> = rt
            .timers
            .iter_mut()
            .filter_map(|slot| {
                if slot.next_due_us == 0 {
                    slot.next_due_us = now_us.saturating_add(slot.period_ms * 1_000);
                    return None;
                }
                (slot.next_due_us <= now_us).then(|| {
                    let chunk = rt.handlers[slot.handler_index].chunk;
                    slot.next_due_us = now_us.saturating_add(slot.period_ms * 1_000);
                    chunk
                })
            })
            .collect();
        for chunk in due {
            rt.vm.reset_budget(NODE_HANDLER_BUDGET);
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
        n.start();
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
        bad.start();
        bad.dispatch_frame(0, 0x100, &HostInput::default());
        assert!(bad.errored());
        assert!(
            bad.dispatch_frame(0, 0x100, &HostInput::default())
                .is_empty(),
            "errored nodes stay quiet"
        );
        bad.start();
        assert!(bad.running(), "a restart clears the error");
    }

    #[test]
    fn compile_errors_land_in_the_log() {
        let mut n = node("on start { print(; }");
        n.start();
        assert!(!n.running());
        assert!(n.log_snapshot()[0].contains("[compile]"));
    }

    #[test]
    fn channel_mismatch_is_ignored() {
        let mut n = node("on message 0x100 { send(0x200); }");
        n.start();
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
        n.start();
        let input = HostInput {
            now_s: 1.5,
            signals: [((0x100, "RPM".to_string()), 2400.0)].into_iter().collect(),
        };
        n.dispatch_frame(0, 0x100, &input);
        assert_eq!(n.log_snapshot(), ["1.5", "2400.0"]);

        // An unseen signal is a runtime error, not a silent zero.
        let mut n2 = node(r#"on message 0x100 { print(sig(0x100, "Nope")); }"#);
        n2.start();
        n2.dispatch_frame(0, 0x100, &HostInput::default());
        assert!(n2.errored(), "a missing signal must not read as zero");
    }

    #[test]
    fn extended_ids_send_flagged_extended() {
        // The outbox carries the raw id; the core derives the extended
        // flag from its size when building the frame.
        let mut n = node("on message 0x100 { send(0x18FF10, 1); }");
        n.start();
        let out = n.dispatch_frame(0, 0x100, &HostInput::default());
        assert_eq!(out, vec![(0x18FF10, vec![1])]);
    }
}
