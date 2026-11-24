//! ScriptNode methods: lifecycle.

use super::*;

impl ScriptNode {
    pub fn new(id: u64, name: String, channel: u8) -> Self {
        Self {
            id,
            name,
            channel,
            source: String::new(),
            panel_actions: Vec::new(),
            enabled: true,
            file_path: None,
            file_source: String::new(),
            checked_source: None,
            check_issue: None,
            check_summary: None,
            attached: None,
            sysvar_keys: Vec::new(),
            runtime: None,
            log: VecDeque::new(),
            log_dirty: false,
            errored: false,
            issue: None,
            tests: Vec::new(),
            pending_lines: Vec::new(),
        }
    }

    pub fn running(&self) -> bool {
        self.runtime.is_some() && !self.errored
    }

    /// Panel action names declared by the last successful compile.
    pub fn panel_actions(&self) -> &[String] {
        &self.panel_actions
    }

    pub fn errored(&self) -> bool {
        self.errored
    }

    pub fn issue(&self) -> Option<&ScriptIssue> {
        self.issue.as_ref()
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
    pub(super) fn push_log(&mut self, line: String) {
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
    pub(super) fn push_log_into(
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

    /// Arms the node for a measurement. Test convenience wrapper around
    /// [`Self::start_with`].
    #[cfg(test)]
    pub fn start(&mut self, dbc: Option<std::sync::Arc<crate::dbc::SymbolTable>>) {
        self.start_with(dbc, None, None, None);
    }

    /// [`Self::start`] plus the FlexRay/Ethernet descriptions the dotted
    /// reads (`FR0.CarSpeed`, `ETH0.Frame.Signal`) resolve against, and the
    /// CAN model the buffer builtins encode/decode through.
    pub fn start_with(
        &mut self,
        dbc: Option<std::sync::Arc<crate::dbc::SymbolTable>>,
        fr_dbs: Option<&std::collections::BTreeMap<u8, std::sync::Arc<crate::fr_db::FrDb>>>,
        eth_dbs: Option<&std::collections::BTreeMap<u8, std::sync::Arc<crate::eth::db::EthDb>>>,
        can_codec: Option<CanCodec>,
    ) {
        self.errored = false;
        self.issue = None;
        self.tests.clear();
        let mut script = match compile(&self.source) {
            Ok(s) => s,
            Err(e) => {
                self.panel_actions.clear();
                self.push_log(format!("[compile] {e}"));
                self.errored = true;
                self.issue = Some(ScriptIssue {
                    line: e.line,
                    col: e.col,
                    message: e.msg,
                });
                return;
            }
        };
        let handlers = script.handlers.clone();
        self.panel_actions = panel_action_names(&handlers);
        // 装配层校验（DBC 部分）：信号引用、发送集对照、反向漏实现、
        // set_sig 字面量范围。GUI 与 `--check-script --dbc` 共用同一套。
        if let Some(db) = &dbc {
            resolve_frame_send_refs(&mut script, db);
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
        for line in frame_checks(&script, dbc.as_deref(), fr_dbs, eth_dbs) {
            Self::push_log_into(
                &mut self.log,
                &mut self.log_dirty,
                &mut self.pending_lines,
                line,
            );
        }
        // 装配层校验：脚本引用的系统变量须已在系统变量管理器中定义，
        // 否则 sys_get 读到 0、sys_set 的写入被丢弃——提前报出来。
        for key in &script.sysvar_refs {
            if !self.sysvar_keys.iter().any(|k| k == key) {
                Self::push_log_into(
                    &mut self.log,
                    &mut self.log_dirty,
                    &mut self.pending_lines,
                    format!("[check] system variable undefined: \"{key}\""),
                );
            }
        }
        // 命名信号读取（Message.Signal / Message["Signal Name"]）解析：数据库把报文名变成 id，只在这里做
        // 一次。VM 拿到的是纯 string → id 映射，语言内核保持零总线
        // 假设；解析不了的引用由 dbc_checks 在上面报 [check]。
        let mut named_messages = std::collections::HashMap::new();
        if let Some(db) = &dbc {
            for (msg, _sig) in &script.named_signal_refs {
                if let Some(id) = db.message_id_by_name(msg) {
                    named_messages.insert(msg.clone(), id);
                }
            }
        }
        let mut named_handler_ids: HashMap<String, (u32, bool)> = HashMap::new();
        let named_handlers: Vec<&str> = handlers
            .iter()
            .filter_map(|handler| match &handler.kind {
                HandlerKind::NamedMessage { name } => Some(name.as_str()),
                _ => None,
            })
            .collect();
        if !named_handlers.is_empty() {
            match &dbc {
                Some(db) => {
                    // Numeric handlers own their ids first; a named handler
                    // resolving onto one of them is a duplicate, not a second
                    // consumer.
                    let mut taken: std::collections::HashSet<(u32, bool)> = handlers
                        .iter()
                        .filter_map(|handler| match handler.kind {
                            HandlerKind::Message { id, .. } => Some((id, false)),
                            HandlerKind::ExtendedMessage { id } => Some((id, true)),
                            _ => None,
                        })
                        .collect();
                    for name in named_handlers {
                        let Some(key) = db.message_key_by_name(name) else {
                            continue; // dbc_checks reports the unknown name
                        };
                        if taken.insert(key) {
                            named_handler_ids.insert(name.to_string(), key);
                        } else {
                            self.push_log(format!(
                                "[check] duplicate handler for message \"{name}\" (0x{:X}{})",
                                key.0,
                                if key.1 { "x" } else { "" }
                            ));
                        }
                    }
                }
                None => self.push_log(
                    "[check] named message handlers need a database on this node's channel"
                        .to_string(),
                ),
            }
        }
        // Dotted FlexRay/Ethernet reads resolve their names once, here.
        // The VM only ever sees a concrete slot; the Ethernet reads stay
        // name-keyed because their host input is keyed the same way.
        let mut fr_named_slots = HashMap::new();
        for (bus, sig) in &script.fr_named_refs {
            let Some(db) = fr_dbs.and_then(|dbs| dbs.get(bus)) else {
                self.push_log(format!(
                    "[check] FR{bus} has no cluster description for \"{sig}\""
                ));
                continue;
            };
            match fr_slot_of(db, *bus, sig) {
                Ok(slot) => {
                    fr_named_slots.insert((*bus, sig.clone()), slot);
                }
                Err(msg) => self.push_log(format!("[check] {msg}")),
            }
        }
        for (bus, frame, sig) in &script.eth_named_refs {
            let Some(db) = eth_dbs.and_then(|dbs| dbs.get(bus)) else {
                self.push_log(format!(
                    "[check] ETH{bus} has no description for \"{frame}.{sig}\""
                ));
                continue;
            };
            if let Err(msg) = eth_signal_check(db, *bus, frame, sig) {
                self.push_log(format!("[check] {msg}"));
            }
        }
        // Same reads with the bus named after its cluster description.
        let mut fr_bus_named_slots = HashMap::new();
        for (bus_name, sig) in &script.fr_bus_refs {
            let Some(dbs) = fr_dbs else {
                self.push_log(format!(
                    "[check] FlexRay cluster {bus_name:?} is not loaded for \"{sig}\""
                ));
                continue;
            };
            let bus = match fr_bus_by_name(dbs, bus_name) {
                Ok(bus) => bus,
                Err(msg) => {
                    self.push_log(format!("[check] {msg}"));
                    continue;
                }
            };
            let Some(db) = dbs.get(&bus) else {
                continue;
            };
            match fr_slot_of(db, bus, sig) {
                Ok(slot) => {
                    fr_bus_named_slots.insert((bus_name.clone(), sig.clone()), (bus, slot));
                }
                Err(msg) => self.push_log(format!("[check] {msg}")),
            }
        }
        let mut eth_bus_named_ids = HashMap::new();
        for (bus_name, frame, sig) in &script.eth_bus_refs {
            let Some(dbs) = eth_dbs else {
                self.push_log(format!(
                    "[check] Ethernet cluster {bus_name:?} is not loaded for \"{frame}.{sig}\""
                ));
                continue;
            };
            let bus = match eth_bus_by_name(dbs, bus_name) {
                Ok(bus) => bus,
                Err(msg) => {
                    self.push_log(format!("[check] {msg}"));
                    continue;
                }
            };
            let Some(db) = dbs.get(&bus) else {
                continue;
            };
            if let Err(msg) = eth_signal_check(db, bus, frame, sig) {
                self.push_log(format!("[check] {msg}"));
                continue;
            }
            eth_bus_named_ids.insert((bus_name.clone(), frame.clone(), sig.clone()), bus);
        }
        let mut vm = Vm::new(script);
        vm.node_id = self.id;
        vm.reset_budget(NODE_HANDLER_BUDGET);
        vm.named_messages = named_messages;
        vm.fr_named_slots = fr_named_slots;
        vm.fr_bus_named_slots = fr_bus_named_slots;
        vm.eth_bus_named_ids = eth_bus_named_ids;
        vm.node_channel = self.channel;
        vm.host_extern = Some(Box::new(move |name, args| {
            node_extern(can_codec.as_ref(), name, args)
        }));
        if let Err(e) = vm.run() {
            let message = e.to_string();
            self.push_log(format!("[start] {message}"));
            self.errored = true;
            self.issue = runtime_issue(&message);
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
            named_handler_ids,
            emitted: Vec::new(),
            pending_sys: Vec::new(),
            fr_out: Vec::new(),
            fr_sig: Vec::new(),
            frames: Vec::new(),
            someip: Vec::new(),
            someip_tcp: Vec::new(),
            someip_offer: Vec::new(),
            someip_reply: Vec::new(),
            nm_timing: Vec::new(),
            last_values: HashMap::new(),
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
        if !rt.vm.outbox.is_empty() || !rt.vm.fr_outbox.is_empty() || !rt.frames.is_empty() {
            rt.vm.outbox.clear();
            rt.vm.fr_outbox.clear();
            rt.frames.clear();
            Self::push_log_into(
                &mut self.log,
                &mut self.log_dirty,
                &mut self.pending_lines,
                "[start] sends are ignored in on start; use a timer or frame handler".to_string(),
            );
        }
        self.runtime = Some(rt);
    }

    /// Ends a measurement: runs `on stop` with the final host input, keeps
    /// its `test(...)` results and then drops the runtime. A no-op when the
    /// node was not running, so Stop can be repeated safely.
    pub fn stop_with(&mut self, input: &HostInput) {
        let Some(mut rt) = self.runtime.take() else {
            return;
        };
        if self.enabled && !self.errored {
            rt.vm.host_input = input.clone();
            // The stop event has no frame context; a stale arrival must
            // not leak into frame_*() or this.signal.
            rt.vm.frame_present = false;
            rt.vm.frame_is_fr = false;
            rt.vm.fr_cycle = 0;
            for handler in rt.handlers.clone() {
                if !matches!(handler.kind, HandlerKind::Stop) {
                    continue;
                }
                rt.vm.reset_budget(NODE_HANDLER_BUDGET);
                rt.vm.timer_ops.clear();
                if let Err(e) = rt.vm.run_handler(handler.chunk) {
                    Self::push_log_into(
                        &mut self.log,
                        &mut self.log_dirty,
                        &mut self.pending_lines,
                        format!("[stop] {e}"),
                    );
                    break;
                }
                Self::drain_vm(
                    &mut rt,
                    &mut self.log,
                    &mut self.log_dirty,
                    &mut self.pending_lines,
                );
            }
        }
        self.tests = std::mem::take(&mut rt.vm.tests);
        self.runtime = None;
        self.errored = false;
        self.issue = None;
        if !self.tests.is_empty() {
            let failed = self.tests.iter().filter(|test| test.failures > 0).count();
            let total = self.tests.len();
            let summary = if failed == 0 {
                format!("[tests] {total}/{total} passed")
            } else {
                let names: Vec<&str> = self
                    .tests
                    .iter()
                    .filter(|test| test.failures > 0)
                    .map(|test| test.name.as_str())
                    .collect();
                format!(
                    "[tests] {}/{total} passed; {failed} failed: {}",
                    total - failed,
                    names.join(", ")
                )
            };
            self.push_log(summary);
        }
    }

    /// Applies a source edit. While measuring a running node recompiles
    /// and restarts in place (globals reset -- a fresh start of that
    /// node); while stopped the edit simply waits for the next start.
    /// Marks a node whose project-referenced file could not be read. It stays
    /// linked and disabled, with the reason shown beside the editor.
    pub fn mark_load_error(&mut self, message: String) {
        self.enabled = false;
        self.errored = true;
        self.runtime = None;
        self.issue = Some(ScriptIssue {
            line: 1,
            col: None,
            message,
        });
    }

    /// Updates the on-disk file association without compiling or applying.
    pub fn set_file(&mut self, path: String, file_source: String) {
        self.file_path = Some(path);
        self.file_source = file_source;
    }

    /// Compiles and runs the same static checks as node start, but leaves the
    /// applied source and runtime untouched.
    pub fn check_source(
        &mut self,
        source: String,
        dbc: Option<std::sync::Arc<crate::dbc::SymbolTable>>,
        fr_dbs: Option<&std::collections::BTreeMap<u8, std::sync::Arc<crate::fr_db::FrDb>>>,
        eth_dbs: Option<&std::collections::BTreeMap<u8, std::sync::Arc<crate::eth::db::EthDb>>>,
    ) {
        self.checked_source = None;
        self.check_issue = None;
        self.check_summary = None;
        let mut script = match compile(&source) {
            Ok(script) => script,
            Err(error) => {
                self.check_issue = Some(ScriptIssue {
                    line: error.line,
                    col: error.col,
                    message: error.msg,
                });
                self.check_summary = Some(ScriptCheckSummary::default());
                return;
            }
        };
        if let Some(db) = &dbc {
            resolve_frame_send_refs(&mut script, db);
        }
        let mut lines = Vec::new();
        if let Some(db) = &dbc {
            let attached = self.attached.as_ref().map(|(_, name)| name.as_str());
            lines.extend(dbc_checks(&script, db, attached));
        }
        lines.extend(frame_checks(&script, dbc.as_deref(), fr_dbs, eth_dbs));
        self.check_summary = Some(ScriptCheckSummary {
            handlers: script.handlers.len(),
            sends: script
                .send_refs
                .iter()
                .map(|(_, id, ext)| (*id, *ext))
                .collect(),
            receives: script.recv_refs.clone(),
            lines,
        });
        self.checked_source = Some(source);
    }

    pub fn set_source(
        &mut self,
        source: String,
        dbc: Option<std::sync::Arc<crate::dbc::SymbolTable>>,
        fr_dbs: Option<&std::collections::BTreeMap<u8, std::sync::Arc<crate::fr_db::FrDb>>>,
        eth_dbs: Option<&std::collections::BTreeMap<u8, std::sync::Arc<crate::eth::db::EthDb>>>,
        can_codec: Option<CanCodec>,
        measuring: bool,
    ) {
        self.source = source;
        self.panel_actions.clear();
        self.issue = None;
        self.tests.clear();
        if self.checked_source.as_deref() != Some(self.source.as_str()) {
            self.checked_source = None;
            self.check_issue = None;
            self.check_summary = None;
        }
        if measuring && self.enabled {
            self.start_with(dbc, fr_dbs, eth_dbs, can_codec);
        } else if let Ok(script) = compile(&self.source) {
            // Stopped nodes still publish their declared panel actions so
            // the Panel editor can offer them before the first start.
            self.panel_actions = panel_action_names(&script.handlers);
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
        fr_dbs: Option<&std::collections::BTreeMap<u8, std::sync::Arc<crate::fr_db::FrDb>>>,
        eth_dbs: Option<&std::collections::BTreeMap<u8, std::sync::Arc<crate::eth::db::EthDb>>>,
        can_codec: Option<CanCodec>,
        measuring: bool,
    ) {
        self.enabled = on;
        if measuring {
            if on {
                self.start_with(dbc, fr_dbs, eth_dbs, can_codec);
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
}
