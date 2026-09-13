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
    /// Optional file path for standalone `.rxcan` source files. Set when
    /// the user saves or loads a node script from disk. Persisted with
    /// the project so the user can find their files again.
    pub file_path: Option<String>,
    /// 绑定的 DBC 节点（总线, 节点名）。绑定脚本的发帧受该节点角色
    /// 闸控制——节点离线/监听时脚本同样不发车。None = 独立脚本。
    pub attached: Option<(u8, String)>,
    /// Defined system variable keys ("ns::name"), refreshed by the bus
    /// before every (re)start: the start check reports script references
    /// outside this set.
    pub sysvar_keys: Vec<String>,
    /// Present only while measuring: recompiled from `source` at every
    /// start, so edits apply without a separate compile action.
    runtime: Option<NodeRuntime>,
    /// `print` output and runtime errors, oldest first, capped.
    log: VecDeque<String>,
    log_dirty: bool,
    /// Set when a handler failed: the node stops executing until the next
    /// start or source edit, so one bad loop cannot spam the log.
    errored: bool,
    /// Lines pushed to `log` and not yet mirrored into the bus's Write
    /// ring. Drained by the bus on the same step.
    pending_lines: Vec<String>,
}

struct NodeRuntime {
    vm: Vm,
    handlers: Vec<Handler>,
    /// Derived-signal samples queued by `emit_value` and not yet taken by
    /// the bus. Drained from the VM after every handler run.
    emitted: Vec<(String, f64)>,
    /// System variable writes queued by `sys_set` and not yet taken by
    /// the bus. Same lifecycle as `emitted`.
    pending_sys: Vec<(String, f64)>,
    /// One slot per Timer handler, in handler order; named one-shot slots
    /// join on `set_timer`. `next_due_us == 0` means "not armed yet": the
    /// first step after start arms periodic slots one period out.
    timers: Vec<TimerSlot>,
}

struct TimerSlot {
    handler_index: usize,
    period_ms: u64,
    next_due_us: u64,
    /// Some for a named one-shot: spent after firing, re-armed by
    /// `set_timer` from any handler.
    name: Option<String>,
    /// Set by `stop_timer()` inside a periodic handler or by a spent /
    /// cancelled one-shot: the slot never fires again until re-armed.
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
            file_path: None,
            attached: None,
            sysvar_keys: Vec::new(),
            runtime: None,
            log: VecDeque::new(),
            log_dirty: false,
            errored: false,
            pending_lines: Vec::new(),
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
        Self::push_log_into(
            &mut self.log,
            &mut self.log_dirty,
            &mut self.pending_lines,
            line,
        );
    }

    /// The ring logic over borrowed pieces: handlers run while `runtime`
    /// is mutably borrowed, and the log fields are disjoint node fields.
    /// Every line also lands in `pending` for the bus's Write ring.
    fn push_log_into(
        log: &mut VecDeque<String>,
        dirty: &mut bool,
        pending: &mut Vec<String>,
        line: String,
    ) {
        if log.len() == LOG_CAP {
            log.pop_front();
        }
        log.push_back(line.clone());
        *dirty = true;
        pending.push(line);
    }

    /// Drains the lines the bus has not yet mirrored into the Write ring.
    pub fn take_new_lines(&mut self) -> Vec<String> {
        std::mem::take(&mut self.pending_lines)
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
        // 装配层校验（DBC 部分）：信号引用、发送集对照、反向漏实现、
        // set_sig 字面量范围。GUI 与 `--check-script --dbc` 共用同一套。
        if let Some(db) = &dbc {
            let attached = self.attached.as_ref().map(|(_, n)| n.as_str());
            for line in dbc_checks(&script, db, attached) {
                Self::push_log_into(
                    &mut self.log,
                    &mut self.log_dirty,
                    &mut self.pending_lines,
                    line,
                );
            }
        }
        // 装配层校验：脚本引用的系统变量须已在系统变量管理器中定义，
        // 否则 sys_get 读到 0、sys_set 的写入被丢弃——提前报出来。
        for key in &script.sysvar_refs {
            if !self.sysvar_keys.iter().any(|k| k == key) {
                Self::push_log_into(
                    &mut self.log,
                    &mut self.log_dirty,
                    &mut self.pending_lines,
                    format!("[check] 系统变量未定义: \"{key}\""),
                );
            }
        }
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
                    name: None,
                    stopped: false,
                }),
                // One-shots start disarmed: only `set_timer` arms them.
                HandlerKind::Oneshot { name } => Some(TimerSlot {
                    handler_index: i,
                    period_ms: 0,
                    next_due_us: 0,
                    name: Some(name.clone()),
                    stopped: true,
                }),
                _ => None,
            })
            .collect();
        let mut rt = NodeRuntime {
            vm,
            handlers,
            emitted: Vec::new(),
            pending_sys: Vec::new(),
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
        Self::drain_vm(
            &mut rt,
            &mut self.log,
            &mut self.log_dirty,
            &mut self.pending_lines,
        );
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
                .filter(|t| t.next_due_us != 0 && !t.stopped)
                .map(|t| t.next_due_us)
                .collect(),
            _ => Vec::new(),
        }
    }

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
    ) -> Vec<(u32, bool, Vec<u8>)> {
        let mut out = Vec::new();
        let Some(rt) = self.runtime.as_mut() else {
            return out;
        };
        if !self.enabled || self.errored || channel != self.channel {
            return out;
        }
        rt.vm.frame_bytes = data.to_vec();
        rt.vm.frame_id = id;
        // Leftover ops from `on start` arm here if no tick ran first.
        let now_us = (input.now_s.max(0.0) * 1e6) as u64;
        let pending: Vec<crate::script::TimerOp> = rt.vm.timer_ops.drain(..).collect();
        for op in pending {
            if let Some(warn) = Self::apply_timer_op(rt, op, now_us, None) {
                Self::push_log_into(&mut self.log, &mut self.log_dirty, &mut self.pending_lines,warn);
            }
        }
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
                HandlerKind::Message { id: mid } => {
                    !is_error && *mid == id && extended == (*mid > 0x7FF)
                }
                HandlerKind::ExtendedMessage { id: mid } => !is_error && extended && *mid == id,
                _ => false,
            })
            .map(|h| h.chunk)
            .collect();
        for chunk in matches {
            rt.vm.reset_budget(NODE_HANDLER_BUDGET);
            rt.vm.host_input = input.clone();
            if let Err(e) = rt.vm.run_handler(chunk) {
                Self::push_log_into(&mut self.log, &mut self.log_dirty, &mut self.pending_lines,format!("[error] {e}"));
                self.errored = true;
                return out;
            }
            Self::drain_vm(
                rt,
                &mut self.log,
                &mut self.log_dirty,
                &mut self.pending_lines,
            );
            // Named one-shot ops stay meaningful from any handler. The
            // running-timer ops (`set_period`/`stop_timer`) name no slot
            // here and are dropped.
            let ops: Vec<crate::script::TimerOp> = rt.vm.timer_ops.drain(..).collect();
            for op in ops {
                if let Some(warn) = Self::apply_timer_op(rt, op, now_us, None) {
                    Self::push_log_into(&mut self.log, &mut self.log_dirty, &mut self.pending_lines,warn);
                }
            }
            out.append(&mut rt.vm.outbox);
        }
        out
    }

    /// Fires due timer handlers. A timer armed lazily at `start` gets its
    /// first due one full period out; missed periods (a stalled host)
    /// collapse into one fire plus a resync. A named one-shot is spent
    /// the moment it fires; `set_timer` inside its own handler re-arms it.
    pub fn run_timers(&mut self, now_us: u64, input: &HostInput) -> Vec<(u32, bool, Vec<u8>)> {
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
        let pending: Vec<crate::script::TimerOp> = rt.vm.timer_ops.drain(..).collect();
        for op in pending {
            if let Some(warn) = Self::apply_timer_op(rt, op, now_us, None) {
                Self::push_log_into(&mut self.log, &mut self.log_dirty, &mut self.pending_lines,warn);
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
        for (slot_index, chunk) in due {
            rt.vm.reset_budget(NODE_HANDLER_BUDGET);
            rt.vm.timer_ops.clear();
            if let Err(e) = rt.vm.run_handler(chunk) {
                Self::push_log_into(&mut self.log, &mut self.log_dirty, &mut self.pending_lines,format!("[error] {e}"));
                self.errored = true;
                return out;
            }
            // Apply timer control the handler queued: a new period takes
            // effect from now, and a stopped timer never fires again.
            let ops: Vec<crate::script::TimerOp> = rt.vm.timer_ops.drain(..).collect();
            for op in ops {
                if let Some(warn) = Self::apply_timer_op(rt, op, now_us, Some(slot_index)) {
                    Self::push_log_into(&mut self.log, &mut self.log_dirty, &mut self.pending_lines,warn);
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
    fn apply_timer_op(
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
    fn drain_vm(
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
    }

    /// Hands the bus everything `emit_value` queued since the last call.
    pub fn take_emitted(&mut self) -> Vec<(String, f64)> {
        self.runtime
            .as_mut()
            .map(|rt| std::mem::take(&mut rt.emitted))
            .unwrap_or_default()
    }

    /// Hands the bus everything `sys_set` queued since the last call.
    pub fn take_sys_sets(&mut self) -> Vec<(String, f64)> {
        self.runtime
            .as_mut()
            .map(|rt| std::mem::take(&mut rt.pending_sys))
            .unwrap_or_default()
    }

    fn fail(&mut self, msg: &str) {
        self.errored = true;
        self.push_log(format!("[error] {msg}"));
    }

    /// Bus-side advisory into the node's log ring (an undefined system
    /// variable write, today): visible without failing the node.
    pub fn note(&mut self, line: String) {
        self.push_log(line);
    }
}

/// The DBC-backed assembly checks a compiled script must pass at start:
/// literal signal references exist, the send set matches the DBC's
/// transmitter declarations (both directions), and literal `set_sig`
/// values sit inside their declared ranges. `attached` is the bound DBC
/// node's name when the script has one; the send-set and reverse checks
/// only apply to bound scripts. Returns formatted log lines -- shared
/// verbatim by the GUI's node start and the headless `--check-script
/// --dbc` gate.
pub fn dbc_checks(
    script: &crate::script::Script,
    db: &crate::dbc::SymbolTable,
    attached: Option<&str>,
) -> Vec<String> {
    let mut lines = Vec::new();
    // Literal signal references in `sig` / `set_sig` must exist, so a
    // typo'd name is reported at assembly instead of silently missing.
    for (id, sig) in &script.signal_refs {
        let known = db
            .message_of(*id)
            .is_some_and(|m| m.signals.iter().any(|s| s.name == *sig));
        if !known {
            lines.push(format!("[check] 信号未在 DBC 中找到: 0x{id:X} \"{sig}\""));
        }
    }
    // 发送集对照 DBC 发送者声明：发送了不属于绑定节点的报文 →
    // warning；不在 DBC 中的自定义报文 → info。
    if let Some(attached_node) = attached {
        for (_, id, ext) in &script.send_refs {
            let key = (*id, *ext);
            match db.messages.get(&key) {
                Some(m) if m.transmitter == attached_node => {} // own message
                Some(m) => lines.push(format!(
                    "[check] 0x{id:X} 是 {} 的报文，不是 {attached_node} 的",
                    m.transmitter
                )),
                None => lines.push(format!("[check] 0x{id:X} 不在 DBC 中（自定义报文）")),
            }
        }
    }
    // 反向漏实现：DBC 声明本节点发送、脚本却从不发送的报文。只是
    // info——生成器代发、脚本只做响应/监听都是常态；通配脚本（转发者/
    // 记录者）不检查。列出条目封顶，全量看编辑器收发 tab。
    if let Some(attached_node) = attached
        && !script.recv_wildcard
    {
        let mut missing: Vec<(u32, bool, &str)> = db
            .messages
            .iter()
            .filter(|(k, m)| {
                m.transmitter == attached_node
                    && !script.send_refs.iter().any(|(_, id, ext)| (*id, *ext) == **k)
            })
            .map(|(k, m)| (k.0, k.1, m.name.as_str()))
            .collect();
        missing.sort();
        if !missing.is_empty() {
            const SHOW: usize = 8;
            let list = missing
                .iter()
                .take(SHOW)
                .map(|(id, ext, name)| format!("0x{id:X}{} {name}", if *ext { "x" } else { "" }))
                .collect::<Vec<_>>()
                .join(", ");
            let more = if missing.len() > SHOW {
                format!(" 等 {} 条", missing.len())
            } else {
                String::new()
            };
            lines.push(format!(
                "[info] DBC 声明本节点发送、脚本未发送（可能由生成器代发）: {list}{more}"
            ));
        }
    }
    // 通配转发归类（R2）：`on message *` 把脚本声明成「转发者/记录者」
    // ——发送集 = 接收集 ∩ 库声明，逐 id 的收发核对都不适用（上面反向
    // 检查已跳过）。转发器把发出的帧再收回是自激成环的唯一来源，检测
    // 到通配 handler 内有发送就如实提示（防护是否在，静态不可判定，
    // 所以是 info 不是 check——sniffer 示例的 frame_id 自排除即答案）。
    if let Some(attached_node) = attached
        && script.recv_wildcard
    {
        lines.push(format!(
            "[info] on message *：本脚本按「转发者/记录者」归类，发送集 = 接收集 ∩ 库声明（{attached_node}）"
        ));
        if script.wildcard_sends() {
            lines.push(
                "[info] 通配 handler 内有发送：发出的帧会再次进入本 handler（自激成环风险）——转发前用 frame_id() 排除自己的报文（参考 examples/sniffer.rxcan）"
                    .to_string(),
            );
        }
    }
    // 静态写入集：set_sig 的字面量值对照 DBC 信号物理界限。字面量才能
    // 静态判定；表达式的越界交给运行时（编码函数自然截断）。min==max
    // 的区间在 DBC 里表示"未声明界限"，不检查。
    for (id, sig, v) in &script.set_sig_values {
        let Some(m) = db.message_of(*id) else {
            continue; // 未知名已由信号引用检查报告
        };
        let Some(s) = m.signals.iter().find(|s| s.name == *sig) else {
            continue;
        };
        if s.max > s.min && (*v < s.min || *v > s.max) {
            lines.push(format!(
                "[check] set_sig 0x{id:X} \"{sig}\" 值 {v} 超出 DBC 声明范围 {}..{}",
                s.min, s.max
            ));
        }
    }
    lines
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
    use std::collections::HashMap;

    fn node(source: &str) -> ScriptNode {
        let mut n = ScriptNode::new(1, "n".into(), 0);
        n.source = source.to_string();
        n
    }

    /// Two-node mini database: EngineECU transmits 0x100 + 0x101, GearBox
    /// transmits 0x200. RPM is range-limited to 0..8000.
    fn engine_dbc() -> std::sync::Arc<crate::dbc::SymbolTable> {
        std::sync::Arc::new(crate::dbc::load_dbc_str(
            r#"
VERSION ""

BS_:

BU_: EngineECU GearBox

BO_ 256 EngineStatus: 8 EngineECU
 SG_ RPM : 0|16@1+ (1,0) [0|8000] "rpm" GearBox

BO_ 257 EngineCmd: 8 EngineECU
 SG_ Target : 0|8@1+ (1,0) [0|100] "%" GearBox

BO_ 512 GearInfo: 8 GearBox
 SG_ Ratio : 0|8@1+ (1,0) [0|255] "" EngineECU
"#,
        )
        .expect("test dbc parses"))
    }

    /// R2 reverse check: DBC traffic the bound node declares but the
    /// script never sends surfaces as an info line; what the script does
    /// send is not flagged.
    #[test]
    fn reverse_check_reports_dbc_traffic_the_script_never_sends() {
        let mut n = node("on start { send(0x100); }");
        n.attached = Some((0, "EngineECU".to_string()));
        n.start(Some(engine_dbc()));
        let logs = n.log_snapshot();
        assert!(
            logs.iter().any(|l| l.contains("0x101") && l.contains("脚本未发送")),
            "EngineCmd (0x101) is named: {logs:?}"
        );
        assert!(
            !logs.iter().any(|l| l.contains("未发送") && l.contains("0x100")),
            "the implemented EngineStatus is not flagged"
        );
        // Foreign DBC traffic (GearBox's 0x200) is nobody's business here.
        assert!(
            !logs.iter().any(|l| l.contains("未发送") && l.contains("0x200")),
            "another node's messages are not this script's job"
        );
    }

    /// A wildcard script is a forwarder or recorder by declaration: the
    /// reverse check does not apply to it.
    #[test]
    fn reverse_check_skips_wildcard_scripts() {
        let mut n = node("on message * { }");
        n.attached = Some((0, "EngineECU".to_string()));
        n.start(Some(engine_dbc()));
        assert!(
            !n.log_snapshot().iter().any(|l| l.contains("脚本未发送")),
            "a wildcard forwarder is exempt"
        );
    }

    /// R2 wildcard classification: a `on message *` script is named a
    /// forwarder with the send-set rule, and a wildcard handler that
    /// transmits gets the self-excitation hint -- guarded (the static
    /// analysis cannot see the guard) at info level, not check.
    #[test]
    fn wildcard_scripts_are_classified_and_warned_about_self_excitation() {
        let mut n = node(
            r#"
                on message * {
                    if (frame_id() != 0x700 && frame_dlc() > 0) {
                        send(0x700);
                    }
                }
            "#,
        );
        n.attached = Some((0, "EngineECU".to_string()));
        n.start(Some(engine_dbc()));
        let logs = n.log_snapshot();
        assert!(
            logs.iter().any(|l| l.contains("转发者") && l.contains("接收集")),
            "the forwarder classification line is there: {logs:?}"
        );
        assert!(
            logs.iter()
                .any(|l| l.contains("自激成环") && l.starts_with("[info]")),
            "the self-excitation hint is info-level: {logs:?}"
        );

        // A wildcard recorder that never sends skips the hint.
        let mut n = node("on message * { }");
        n.attached = Some((0, "EngineECU".to_string()));
        n.start(Some(engine_dbc()));
        assert!(
            !n.log_snapshot()
                .iter()
                .any(|l| l.contains("自激成环")),
            "no send, no loop hint"
        );
    }

    /// The frame_id pure forward is caught too -- the send is opaque but
    /// still attributed to the wildcard handler.
    #[test]
    fn wildcard_frame_id_forward_triggers_the_hint() {
        let mut n = node("on message * { send(frame_id()); }");
        n.attached = Some((0, "EngineECU".to_string()));
        n.start(Some(engine_dbc()));
        assert!(
            n.log_snapshot()
                .iter()
                .any(|l| l.contains("自激成环")),
            "an unguarded pure forward is the textbook loop: {logs:?}",
            logs = n.log_snapshot()
        );
    }

    /// Literal `set_sig` values are range-checked against the database;
    /// in-range writes stay silent.
    #[test]
    fn set_sig_literal_outside_dbc_range_is_reported() {
        let mut n = node(
            r#"
                on start {
                    let b = bytes(8);
                    set_sig(b, 0x100, "RPM", 9999);
                    set_sig(b, 0x100, "RPM", 3000);
                }
            "#,
        );
        n.attached = Some((0, "EngineECU".to_string()));
        n.start(Some(engine_dbc()));
        let logs = n.log_snapshot();
        assert!(
            logs.iter().any(|l| l.contains("9999") && l.contains("超出")),
            "the out-of-range literal is named: {logs:?}"
        );
        assert!(
            !logs.iter().any(|l| l.contains("3000")),
            "the in-range literal stays silent"
        );
    }

    /// The start check reports literal `sys_get` / `sys_set` keys that
    /// are not in the defined set the bus pushed in.
    #[test]
    fn undefined_sysvar_references_are_reported_at_start() {
        let mut n = node(
            r#"
                on start {
                    let v = sys_get("Demo::Speed");
                    sys_set("Demo::Setpoint", v + 1);
                }
            "#,
        );
        n.sysvar_keys = vec!["Demo::Speed".to_string()];
        n.start(None);
        let logs = n.log_snapshot();
        assert!(
            logs.iter().any(|l| l.contains("Demo::Setpoint")),
            "the undefined write is reported: {logs:?}"
        );
        assert!(
            !logs.iter().any(|l| l.contains("Demo::Speed")),
            "the defined read is silent"
        );
    }

    /// A `sys_set` in a handler reaches `take_sys_sets`; `sys_get` reads
    /// the values the bus published into the host input.
    #[test]
    fn sys_set_from_a_handler_reaches_the_drain() {
        let mut n = node(
            r#"
                on message 0x100 {
                    sys_set("Demo::Setpoint", sys_get("Demo::Speed") * 2);
                }
            "#,
        );
        n.sysvar_keys = vec!["Demo::Speed".to_string(), "Demo::Setpoint".to_string()];
        n.start(None);
        let mut hit = HostInput::default();
        hit.sysvars.insert("Demo::Speed".to_string(), 21.0);
        n.dispatch_frame(0, 0x100, false, false, &[], &hit);
        assert_eq!(
            n.take_sys_sets(),
            vec![("Demo::Setpoint".to_string(), 42.0)]
        );
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
        let out = n.dispatch_frame(0, 0x100, false, false, &[], &HostInput::default());
        assert_eq!(out, vec![(0x200, false, vec![1, 2])]);

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
        bad.dispatch_frame(0, 0x100, false, false, &[], &HostInput::default());
        assert!(bad.errored());
        assert!(
            bad.dispatch_frame(0, 0x100, false, false, &[], &HostInput::default())
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

    /// Literal `sig` / `set_sig` references are checked against the
    /// channel database at Apply: a typo'd name is a logged warning, not
    /// a runtime surprise. With no database there is nothing to check.
    #[test]
    fn unknown_signal_references_are_reported_at_apply() {
        let dbc = Some(std::sync::Arc::new(
            crate::dbc::load_dbc_str(
                r#"VERSION "check"

NS_ :

BS_:

BU_: ECU

BO_ 256 Real: 2 ECU
 SG_ RealSig : 0|16@1+ (0.1,0) [0|0] ""  ECU
"#,
            )
            .unwrap(),
        ));
        let mut n = node(
            r#"
                on start {
                    let v = sig(0x100, "RealSig");
                    let w = sig(0x100, "TypoSig");
                }
            "#,
        );
        n.start(dbc.clone());
        let log = n.log_snapshot();
        let check_lines: Vec<&String> = log.iter().filter(|l| l.starts_with("[check]")).collect();
        assert_eq!(
            check_lines.len(),
            1,
            "exactly the typo is flagged: {check_lines:?}"
        );
        assert!(check_lines[0].contains("TypoSig"));

        // Without a database there is no vocabulary to check against —
        // and no false warnings either.
        let mut n2 = node("on start { let v = sig(0x100, \"Whatever\"); }");
        n2.start(None);
        let log2 = n2.log_snapshot();
        assert!(
            log2.iter().all(|l| !l.starts_with("[check]")),
            "no db: no static check warnings: {log2:?}"
        );
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

    /// A periodic slot re-anchors each beat on the schedule, not on the
    /// (late) fire: wake jitter must not drift every beat after it.
    #[test]
    fn periodic_beats_stay_on_schedule_after_a_late_wake() {
        let mut n = node(
            r#"
                let ticks = 0;
                on timer 100 { ticks = ticks + 1; print("t", ticks); }
            "#,
        );
        n.start(None);
        n.run_timers(50_000, &HostInput::default()); // arms: due 150_000
        n.run_timers(160_000, &HostInput::default()); // late wake: tick 1 at 160
        n.run_timers(249_999, &HostInput::default()); // not yet
        n.run_timers(250_000, &HostInput::default()); // beat 2 at 250, not 260
        let ticks = n
            .log_snapshot()
            .iter()
            .filter(|l| l.starts_with("t "))
            .count();
        assert_eq!(ticks, 2, "beats 150+100=250, unaffected by the late wake");
    }

    #[test]
    fn channel_mismatch_is_ignored() {
        let mut n = node("on message 0x100 { send(0x200); }");
        n.start(None);
        assert!(
            n.dispatch_frame(1, 0x100, false, false, &[], &HostInput::default())
                .is_empty(),
            "other channel"
        );
        assert_eq!(
            n.dispatch_frame(0, 0x100, false, false, &[], &HostInput::default())
                .len(),
            1
        );
    }

    /// Frame classes are told apart: a bare id never matches across the
    /// standard/extended boundary, in either direction.
    #[test]
    fn extended_frames_need_extended_handlers() {
        let mut n = node(
            r#"
                on message 0x100 { print("std"); }
                on message 0x1C3D1E5 { print("ext"); }
            "#,
        );
        n.start(None);
        // Standard 0x100 fires the standard handler only.
        assert_eq!(
            n.dispatch_frame(0, 0x100, false, false, &[], &HostInput::default())
                .len(),
            0
        );
        assert_eq!(std_count(&n), 1);
        // Extended 0x100 fires nothing: no handler names an extended
        // frame with that id.
        n.dispatch_frame(0, 0x100, true, false, &[], &HostInput::default());
        assert_eq!(std_count(&n), 1);
        // Extended 0x1C3D1E5 fires the extended handler only.
        n.dispatch_frame(0, 0x1C3D1E5, true, false, &[], &HostInput::default());
        assert_eq!(std_count(&n), 1);
        assert_eq!(ext_count(&n), 1);
        // The same id arriving as standard fires nothing.
        n.dispatch_frame(0, 0x1C3D1E5, false, false, &[], &HostInput::default());
        assert_eq!(ext_count(&n), 1);

        fn std_count(n: &ScriptNode) -> usize {
            n.log_snapshot().iter().filter(|l| *l == "std").count()
        }
        fn ext_count(n: &ScriptNode) -> usize {
            n.log_snapshot().iter().filter(|l| *l == "ext").count()
        }
    }

    /// Extended frames whose numeric id sits inside the standard range are
    /// reachable through the explicit `on extended message` form and
    /// `send_ext`, where plain `on message` / `send` cannot go.
    #[test]
    fn extended_syntax_addresses_small_numeric_extended_ids() {
        let mut n = node(
            r#"
            let count = 0;
            on message 0x50 { print("std"); }
            on extended message 0x50 { count = count + 1; print("ext", count); }
            on message 0x60 { send_ext(0x60, 7); }
        "#,
        );
        n.start(None);
        // Standard 0x50: the plain handler; extended 0x50: the explicit one.
        n.dispatch_frame(0, 0x50, false, false, &[], &HostInput::default());
        let log = n.log_snapshot();
        assert_eq!(log[0], "std");
        n.dispatch_frame(0, 0x50, true, false, &[], &HostInput::default());
        let log = n.log_snapshot();
        assert_eq!(
            log[1], "ext 1",
            "the extended form catches what plain cannot"
        );

        // send_ext always travels extended, even below 0x700.
        let out = n.dispatch_frame(0, 0x60, false, false, &[], &HostInput::default());
        assert_eq!(out, vec![(0x60, true, vec![7])]);
    }

    /// A buffer payload past 8 bytes travels as CAN FD with a valid FD
    /// length; the VM refuses anything past the 64-byte ceiling.
    #[test]
    fn long_buffers_travel_as_fd_frames() {
        let mut n = node(
            r#"
                on message 0x100 {
                    let fd = bytes(12);
                    fd[11] = 0xEE;
                    send(0x300, fd);
                }
            "#,
        );
        n.start(None);
        let out = n.dispatch_frame(0, 0x100, false, false, &[], &HostInput::default());
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].0, 0x300);
        // A 12-byte payload needs CAN FD but not an extended id: those
        // are two independent dimensions. The core snaps the length to
        // a valid FD DLC and sets the FD flag.
        assert!(!out[0].1);
        assert_eq!(out[0].2.len(), 12);

        let mut too_long = node("let big = bytes(65); send(0x300, big);");
        too_long.start(None);
        too_long.dispatch_frame(0, 0x100, false, false, &[], &HostInput::default());
        assert!(
            too_long.errored(),
            "past 64 bytes the send must fail the handler"
        );
    }

    /// `on message *` sees every frame on the channel; specific handlers
    /// still fire for their own ids, in declaration order.
    #[test]
    fn a_wildcard_handler_sees_every_frame() {
        let mut n = node(
            r#"
                let hits = 0;
                on message * {
                    hits = hits + 1;
                    print("any", frame_byte(0), frame_id());
                }
                on message 0x55 { print("specific"); }
            "#,
        );
        n.start(None);
        let hit = HostInput {
            now_s: 0.01,
            ..HostInput::default()
        };
        n.dispatch_frame(0, 0x100, false, false, &[7], &hit);
        n.dispatch_frame(0, 0x55, false, false, &[9], &hit);
        n.dispatch_frame(0, 0x1ABCDEF, true, false, &[11], &hit);
        let log = n.log_snapshot();
        assert_eq!(log[0], "any 7 256", "the wildcard saw 0x100");
        // For 0x55 both handlers match and run in declaration order:
        // the wildcard was declared first.
        assert_eq!(log[1], "any 9 85");
        assert_eq!(log[2], "specific");
        assert_eq!(
            log[3], "any 11 28036591",
            "the wildcard saw the extended frame"
        );
        assert_eq!(hits_of(&n), 3);

        fn hits_of(n: &ScriptNode) -> usize {
            n.log_snapshot()
                .iter()
                .filter(|l| l.starts_with("any "))
                .count()
        }
    }

    /// Error frames belong to `on errorFrame` alone: wildcard and
    /// id handlers never see them.
    #[test]
    fn an_error_frame_reaches_only_the_error_handler() {
        let mut n = node(
            r#"
                let errors = 0;
                on message * {
                    print("any", frame_dlc());
                }
                on errorFrame {
                    errors = errors + 1;
                    print("err", errors);
                }
            "#,
        );
        n.start(None);
        // An error frame carries no payload and no regular handler runs.
        n.dispatch_frame(0, 0x1, false, true, &[], &HostInput::default());
        let log = n.log_snapshot();
        assert_eq!(log, ["err 1"], "only the error handler ran: {log:?}");
        // Regular traffic keeps flowing through the wildcard.
        n.dispatch_frame(0, 0x100, false, false, &[1], &HostInput::default());
        let log = n.log_snapshot();
        assert_eq!(log.last().map(|l| l.as_str()), Some("any 1"));
        assert_eq!(log.len(), 2, "no error handler fire on a data frame");
    }

    /// The delayed-response shape: a frame arrives, the node answers one
    /// fixed delay later, exactly once per arming.
    #[test]
    fn a_set_timer_one_shot_fires_once_from_an_event() {
        let mut n = node(
            r#"
                on message 0x100 { set_timer("resp", 100); }
                on timer "resp" { send(0x200, 1); }
            "#,
        );
        n.start(None);
        let hit = HostInput {
            now_s: 0.05,
            ..HostInput::default()
        };
        assert!(
            n.dispatch_frame(0, 0x100, false, false, &[], &hit)
                .is_empty(),
            "arming"
        );
        assert!(n.run_timers(100_000, &hit).is_empty(), "not yet due");
        let out = n.run_timers(150_000, &hit);
        assert_eq!(out.len(), 1, "one shot, one frame");
        assert_eq!(out[0].0, 0x200);
        assert!(
            n.run_timers(400_000, &hit).is_empty(),
            "a one-shot never repeats"
        );
    }

    #[test]
    fn a_one_shot_can_re_arm_itself() {
        let mut n = node(
            r#"
                let fired = 0;
                on start { set_timer("beat", 100); }
                on timer "beat" {
                    fired = fired + 1;
                    print("beat", fired);
                    if (fired < 3) { set_timer("beat", 100); }
                }
            "#,
        );
        n.start(None);
        let t0 = HostInput::default();
        // The `on start` arming lands on the first tick's clock.
        n.run_timers(0, &t0);
        n.run_timers(100_000, &t0); // beat 1, re-arms
        n.run_timers(200_000, &t0); // beat 2, re-arms
        n.run_timers(300_000, &t0); // beat 3, no re-arm
        n.run_timers(500_000, &t0);
        let beats = n
            .log_snapshot()
            .iter()
            .filter(|l| l.starts_with("beat "))
            .count();
        assert_eq!(beats, 3, "log: {:?}", n.log_snapshot());
    }

    #[test]
    fn cancel_timer_disarms_a_one_shot() {
        let mut n = node(
            r#"
                on start { set_timer("beat", 100); }
                on message 0x1 { cancel_timer("beat"); }
                on timer "beat" { print("beat"); }
            "#,
        );
        n.start(None);
        let t0 = HostInput::default();
        n.run_timers(0, &t0);
        let hit = HostInput {
            now_s: 0.01,
            ..HostInput::default()
        };
        n.dispatch_frame(0, 0x1, false, false, &[], &hit);
        assert!(
            n.run_timers(500_000, &t0).is_empty(),
            "a cancelled one-shot never fires"
        );
        assert!(n.log_snapshot().iter().all(|l| !l.contains("beat")));
    }

    #[test]
    fn set_timer_without_a_handler_logs_a_warning() {
        let mut n = node(r#"on start { set_timer("nope", 50); }"#);
        n.start(None);
        n.run_timers(0, &HostInput::default());
        assert!(
            n.log_snapshot()
                .iter()
                .any(|l| l.contains("nope") && l.contains("no such handler")),
            "log: {:?}",
            n.log_snapshot()
        );
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
            sysvars: HashMap::new(),
        };
        n.dispatch_frame(0, 0x100, false, false, &[], &input);
        assert_eq!(n.log_snapshot(), ["1.5", "2400.0"]);

        // An unseen signal is a runtime error, not a silent zero.
        let mut n2 = node(r#"on message 0x100 { print(sig(0x100, "Nope")); }"#);
        n2.start(None);
        n2.dispatch_frame(0, 0x100, false, false, &[], &HostInput::default());
        assert!(n2.errored(), "a missing signal must not read as zero");
    }

    #[test]
    fn extended_ids_send_flagged_extended() {
        // The outbox carries the raw id plus the "force extended" bit
        // (`send_ext` sets it); the core ORs it with the id-size rule
        // when building the frame.
        let mut n = node("on message 0x100 { send(0x18FF10, 1); }");
        n.start(None);
        let out = n.dispatch_frame(0, 0x100, false, false, &[], &HostInput::default());
        assert_eq!(out, vec![(0x18FF10, false, vec![1])]);
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
        let out = n.dispatch_frame(0, 0x1, false, false, &[], &HostInput::default());
        assert_eq!(
            out,
            vec![(0x200, false, vec![0xA0, 0x0F, 0, 0, 0, 0, 0, 0])]
        );
    }
}
