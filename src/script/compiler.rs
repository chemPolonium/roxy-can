//! AST to bytecode. Two passes: signatures first (so scripts may call a
//! function defined later in the file), then bodies. Top-level `let`s
//! become globals; function bodies use stack locals with block scopes.

use super::parser::{BinOp, Expr, FnDecl, Item, OnDecl, OnKind, Program, SpannedStmt, Stmt, UnOp};
use super::{
    FrameInstance, Function, HOST_FNS, Handler, HandlerKind, Op, Script, ScriptError, Value,
    WILDCARD_LABEL,
};
use std::collections::{HashMap, HashSet};

const MAX_LOCALS: usize = u8::MAX as usize;
const MAX_ARGS: usize = u8::MAX as usize;

pub fn compile(program: Program) -> Result<Script, ScriptError> {
    let mut c = Comp {
        constants: Vec::new(),
        globals: Vec::new(),
        functions: vec![Function {
            arity: 0,
            name: "main".to_string(),
            code: Vec::new(),
            lines: Vec::new(),
            local_marks: Vec::new(),
        }],
        fn_index: HashMap::new(),
        handlers: Vec::new(),
        code: Vec::new(),
        line_marks: Vec::new(),
        local_marks: Vec::new(),
        cur_line: 0,
        locals: Vec::new(),
        depth: 0,
        in_fn: false,
        line: 1,
        col: 1,
        break_jumps: Vec::new(),
        continue_jumps: Vec::new(),
        signal_refs: Vec::new(),
        set_sig_values: Vec::new(),
        named_signal_refs: Vec::new(),
        fr_named_refs: Vec::new(),
        eth_named_refs: Vec::new(),
        fr_bus_refs: Vec::new(),
        eth_bus_refs: Vec::new(),
        sysvar_refs: Vec::new(),
        recv_wildcard: false,
        cur_handler: None,
        send_refs: Vec::new(),
        recv_refs: Vec::new(),
        opaque_sends: Vec::new(),
        frame_globals: HashMap::new(),
        frame_locals: Vec::new(),
        frame_sends: Vec::new(),
        frame_fields: Vec::new(),
        frame_consts: Vec::new(),
        timer_arms: Vec::new(),
    };

    // Pass 1: function signatures, so later items may call earlier names.
    for item in &program.items {
        if let Item::Fn(f) = item {
            let idx = c.functions.len() as u16;
            if c.functions.len() >= u16::MAX as usize {
                return c.err(&f.name, "too many functions");
            }
            if c.fn_index.insert(f.name.clone(), idx).is_some() {
                return c.err(&f.name, "duplicate function name");
            }
            c.functions.push(Function {
                arity: f.params.len(),
                name: f.name.clone(),
                code: Vec::new(),
                lines: Vec::new(),
                local_marks: Vec::new(),
            });
        }
    }

    // Handler sanity: one on start, one handler per message id, one
    // handler per one-shot timer name, one wildcard.
    let mut seen_start = false;
    let mut seen_stop = false;
    let mut seen_panel_actions: HashSet<String> = HashSet::new();
    let mut seen_ids: HashSet<u32> = HashSet::new();
    let mut seen_named_messages: HashSet<String> = HashSet::new();
    let mut seen_named: HashSet<String> = HashSet::new();
    let mut seen_wildcard = false;
    let mut seen_error = false;
    let mut seen_ext_ids: HashSet<u32> = HashSet::new();
    let mut seen_fr: HashSet<(Option<u8>, u16)> = HashSet::new();
    for item in &program.items {
        if let Item::On(on) = item {
            match &on.kind {
                OnKind::Start => {
                    if seen_start {
                        return c.err_at(on.line, on.col, "duplicate 'on start' handler");
                    }
                    seen_start = true;
                }
                OnKind::Stop => {
                    if seen_stop {
                        return c.err_at(on.line, on.col, "duplicate 'on stop' handler");
                    }
                    seen_stop = true;
                }
                OnKind::PanelAction { name, .. } => {
                    if !seen_panel_actions.insert(name.clone()) {
                        return c.err_at(
                            on.line,
                            on.col,
                            &format!("duplicate handler for panel action \"{name}\""),
                        );
                    }
                }
                OnKind::Message { id } => {
                    if !seen_ids.insert(*id) {
                        return c.err_at(
                            on.line,
                            on.col,
                            &format!("duplicate handler for id {id:#x}"),
                        );
                    }
                }
                OnKind::NamedMessage { name } => {
                    if !seen_named_messages.insert(name.clone()) {
                        return c.err_at(
                            on.line,
                            on.col,
                            &format!("duplicate handler for message \"{name}\""),
                        );
                    }
                }
                OnKind::ExtendedMessage { id } => {
                    if !seen_ext_ids.insert(*id) {
                        return c.err_at(
                            on.line,
                            on.col,
                            &format!("duplicate handler for extended id {id:#x}"),
                        );
                    }
                }
                OnKind::AnyMessage => {
                    if seen_wildcard {
                        return c.err_at(on.line, on.col, "duplicate 'on message *' handler");
                    }
                    seen_wildcard = true;
                    c.recv_wildcard = true;
                }
                OnKind::ErrorFrame => {
                    if seen_error {
                        return c.err_at(on.line, on.col, "duplicate 'on errorFrame' handler");
                    }
                    seen_error = true;
                }
                OnKind::Timer { .. } => {}
                OnKind::Oneshot { name } => {
                    if !seen_named.insert(name.clone()) {
                        return c.err_at(
                            on.line,
                            on.col,
                            &format!("duplicate handler for timer \"{name}\""),
                        );
                    }
                }
                // One handler per addressed arrival. `on fr slot 13` and
                // `on fr 0 slot 13` are different addresses -- the first is
                // the any-cluster form and fires alongside the second on FR0,
                // exactly as a wildcard `on message *` fires alongside
                // `on message 0x100`.
                OnKind::FrSlot { bus, slot } => {
                    if !seen_fr.insert((*bus, *slot)) {
                        let want = bus.map_or(String::new(), |b| format!("{b} "));
                        return c.err_at(
                            on.line,
                            on.col,
                            &format!("duplicate handler for fr {want}slot {slot}"),
                        );
                    }
                }
            }
        }
    }

    // Pass 2: bodies and the main flow.
    for item in program.items {
        match item {
            Item::Fn(f) => c.compile_fn(f)?,
            Item::On(on) => c.compile_handler(on)?,
            Item::Stmt(s) => c.stmt(&s)?,
        }
    }
    // The main chunk ends like any function: `return nil;`.
    let nil = c.constant(Value::Nil);
    c.emit(Op::Const(nil));
    c.emit(Op::Return);
    c.functions[0].code = std::mem::take(&mut c.code);
    c.functions[0].lines = std::mem::take(&mut c.line_marks);
    c.functions[0].local_marks = std::mem::take(&mut c.local_marks);
    Ok(Script {
        constants: c.constants,
        globals: c.globals,
        functions: c.functions,
        handlers: c.handlers,
        host_fns: HOST_FNS.iter().map(|(n, _, _)| n.to_string()).collect(),
        signal_refs: c.signal_refs,
        set_sig_values: c.set_sig_values,
        named_signal_refs: c.named_signal_refs,
        fr_named_refs: c.fr_named_refs,
        eth_named_refs: c.eth_named_refs,
        fr_bus_refs: c.fr_bus_refs,
        eth_bus_refs: c.eth_bus_refs,
        send_refs: c.send_refs,
        recv_refs: c.recv_refs,
        recv_wildcard: c.recv_wildcard,
        sysvar_refs: c.sysvar_refs,
        opaque_sends: c.opaque_sends,
        frame_sends: c.frame_sends,
        frame_fields: c.frame_fields,
        frame_consts: c.frame_consts,
        timer_arms: c.timer_arms,
    })
}

struct Comp {
    constants: Vec<Value>,
    globals: Vec<String>,
    functions: Vec<Function>,
    handlers: Vec<Handler>,
    fn_index: HashMap<String, u16>,
    /// Code of the chunk currently being compiled.
    code: Vec<Op>,
    /// Sparse op-index -> source-line table of the current chunk: the
    /// first op emitted on each new line opens an entry.
    line_marks: Vec<(u16, u32)>,
    /// Sparse op-index -> active local names, for the debugger.
    local_marks: Vec<(u16, Vec<String>)>,
    cur_line: u32,
    /// Locals of the function being compiled: name and scope depth.
    locals: Vec<(String, u32)>,
    depth: u32,
    in_fn: bool,
    line: u32,
    /// Column of the statement being compiled, for error messages.
    col: u32,
    /// Jump instruction indices for `break` inside the innermost loop,
    /// patched at loop exit. One entry per loop nesting level.
    break_jumps: Vec<Vec<usize>>,
    /// Continue jump code indices for the innermost loop, patched to the
    /// step/cond position.
    continue_jumps: Vec<Vec<usize>>,
    /// Signal references made by literal-argument `sig` / `set_sig`
    /// calls, deduped: metadata the host can check against the database
    /// at node-assembly time.
    signal_refs: Vec<(u32, String)>,
    /// Literal `set_sig` values: `(id, signal, value)` where the value
    /// constant-folded, for the host's range check at node start.
    set_sig_values: Vec<(u32, String, f64)>,
    /// Named reads (`Message.Signal`, `Message["Signal Name"]`) as
    /// `(message, signal)` pairs, deduped, for the host's start check.
    named_signal_refs: Vec<(String, String)>,
    /// Dotted FlexRay reads `FR{n}.Signal`, deduped.
    fr_named_refs: Vec<(u8, String)>,
    /// Dotted Ethernet reads `ETH{n}.Frame.Signal`, deduped.
    eth_named_refs: Vec<(u8, String, String)>,
    /// Dotted reads that pick the bus by description name, deduped.
    fr_bus_refs: Vec<(String, String)>,
    eth_bus_refs: Vec<(String, String, String)>,
    /// System variable keys (`"ns::name"`) named by literal-argument
    /// `sys_get` / `sys_set` calls, deduped, for the host's start check.
    sysvar_refs: Vec<String>,
    /// The script declares `on message *`.
    recv_wildcard: bool,
    /// R2 spike: the handler being compiled, for attributing derived
    /// sends. `None` inside plain user functions.
    cur_handler: Option<(HandlerKind, String)>,
    /// R2 spike: statically derived sends, attributed to their handler.
    send_refs: Vec<(String, u32, bool)>,
    /// R2 spike: sends whose id the compiler could not derive.
    opaque_sends: Vec<String>,
    /// Statically known frame instance origins, by global variable name.
    frame_globals: HashMap<String, String>,
    /// Frame origins of locals/parameters; `None` means the value is not a
    /// statically known frame instance.
    frame_locals: Vec<(String, Option<String>, u32)>,
    /// Frame paths a known `send_frame` may send.
    frame_sends: Vec<String>,
    /// `(frame path, signal)` written on a known instance.
    frame_fields: Vec<(String, String)>,
    /// Constant writes `(frame path, signal, physical value)`, for the host's
    /// start-time range warning.
    frame_consts: Vec<(String, String, f64)>,
    /// R2 response mapping: `(arming handler label, timer name)` per
    /// literal `set_timer` call.
    timer_arms: Vec<(String, String)>,
    /// R2 静态事实表 -- 接收集：`on message <id>` / `on extended message
    /// <id>` 声明监听的 `(id, extended)`。
    recv_refs: Vec<(u32, bool)>,
}

/// The static label one handler carries -- in the `--check-script` report, in
/// the send/timer attribution, and in the assembly warnings. Two handlers of
/// the same kind differ only by this string, so the wording lives in one place:
/// a FlexRay handler repeats the cluster only when the script named one, so
/// `<on fr slot 13>` and `<on fr 0 slot 13>` stay distinguishable.
fn handler_label(kind: &HandlerKind) -> String {
    match kind {
        HandlerKind::Start => "<on start>".to_string(),
        HandlerKind::Stop => "<on stop>".to_string(),
        HandlerKind::PanelAction { name, .. } => format!("<on panel_action \"{name}\">"),
        HandlerKind::Message { id } => format!("<on message {id:#x}>"),
        HandlerKind::NamedMessage { name } => format!("<on message {name}>"),
        HandlerKind::ExtendedMessage { id } => format!("<on extended message {id:#x}>"),
        HandlerKind::AnyMessage => WILDCARD_LABEL.to_string(),
        HandlerKind::ErrorFrame => "<on errorFrame>".to_string(),
        HandlerKind::Timer { period_ms } => format!("<on timer {period_ms}>"),
        HandlerKind::Oneshot { name } => format!("<on timer \"{name}\">"),
        HandlerKind::FrSlot { bus: None, slot } => format!("<on fr slot {slot}>"),
        HandlerKind::FrSlot {
            bus: Some(bus),
            slot,
        } => format!("<on fr {bus} slot {slot}>"),
    }
}

/// The value a constant write states, when the expression is a literal or a
/// negated literal. Anything else depends on runtime state and cannot be
/// range-checked at start.
fn const_number(e: &Expr) -> Option<f64> {
    match e {
        Expr::Int(i) => Some(*i as f64),
        Expr::Float(f) => Some(*f),
        Expr::Unary(UnOp::Neg, inner) => const_number(inner).map(|v| -v),
        _ => None,
    }
}

impl Comp {
    fn err<T>(&self, where_: &str, msg: &str) -> Result<T, ScriptError> {
        Err(ScriptError {
            line: self.line,
            col: Some(self.col),
            msg: format!("{where_}: {msg}"),
        })
    }

    fn emit(&mut self, op: Op) {
        self.code.push(op);
    }

    fn constant(&mut self, v: Value) -> u16 {
        if let Some(i) = self.constants.iter().position(|c| *c == v) {
            return i as u16;
        }
        self.constants.push(v);
        (self.constants.len() - 1) as u16
    }

    fn global_slot(&mut self, name: &str) -> u16 {
        if let Some(i) = self.globals.iter().position(|g| g == name) {
            return i as u16;
        }
        self.globals.push(name.to_string());
        (self.globals.len() - 1) as u16
    }

    fn err_at<T>(&self, line: u32, col: u32, msg: &str) -> Result<T, ScriptError> {
        Err(ScriptError {
            line,
            col: Some(col),
            msg: msg.to_string(),
        })
    }

    fn resolve_local(&self, name: &str) -> Option<u8> {
        self.locals
            .iter()
            .rposition(|(n, _)| n == name)
            .and_then(|i| u8::try_from(i).ok())
    }

    /// The catalog path a variable is known to hold, if any.
    fn frame_origin(&self, name: &str) -> Option<String> {
        if let Some((_, origin, _)) = self.frame_locals.iter().rev().find(|(n, _, _)| n == name) {
            return origin.clone();
        }
        if self.resolve_local(name).is_some() {
            return None;
        }
        self.frame_globals.get(name).cloned()
    }

    fn expr_frame_origin(&self, expr: &Expr) -> Option<String> {
        match expr {
            Expr::FrameNew(path) => Some(path.clone()),
            Expr::Ident(name) => self.frame_origin(name),
            _ => None,
        }
    }

    fn note_frame_field(&mut self, path: &str, field: &str) {
        let entry = (path.to_string(), field.to_string());
        if !self.frame_fields.contains(&entry) {
            self.frame_fields.push(entry);
        }
    }

    fn note_frame_const(&mut self, path: &str, field: &str, value: f64) {
        let entry = (path.to_string(), field.to_string(), value);
        if !self.frame_consts.contains(&entry) {
            self.frame_consts.push(entry);
        }
    }

    fn note_frame_send(&mut self, path: &str) {
        if !self.frame_sends.iter().any(|existing| existing == path) {
            self.frame_sends.push(path.to_string());
        }
    }

    fn stmt(&mut self, s: &SpannedStmt) -> Result<(), ScriptError> {
        // Every op emitted for this statement reports the statement's
        // line at runtime; errors name the statement's column too.
        self.line = s.line;
        self.col = s.col;
        self.line_mark();
        match &s.stmt {
            Stmt::Let(name, expr) => {
                self.expr(expr)?;
                let origin = self.expr_frame_origin(expr);
                if self.in_fn {
                    if self.resolve_local(name).is_some() {
                        return self.err(name, "duplicate variable in this scope");
                    }
                    if self.locals.len() >= MAX_LOCALS {
                        return self.err(name, "too many locals in one function");
                    }
                    // The value just pushed *is* the local slot: it stays
                    // on the stack for the variable's whole lifetime, so
                    // there is nothing to store.
                    self.locals.push((name.clone(), self.depth));
                    self.frame_locals.push((name.clone(), origin, self.depth));
                } else {
                    let slot = self.global_slot(name);
                    self.emit(Op::SetGlobal(slot));
                    match origin {
                        Some(path) => {
                            self.frame_globals.insert(name.clone(), path);
                        }
                        None => {
                            self.frame_globals.remove(name);
                        }
                    }
                }
            }
            Stmt::Assign(name, expr) => {
                self.expr(expr)?;
                let origin = self.expr_frame_origin(expr);
                if let Some(idx) = self.resolve_local(name) {
                    self.emit(Op::SetLocal(idx));
                    if let Some(entry) = self
                        .frame_locals
                        .iter_mut()
                        .rev()
                        .find(|(n, _, _)| n == name)
                    {
                        entry.1 = origin;
                    }
                } else if self.globals.iter().any(|g| g == name) {
                    let slot = self.global_slot(name);
                    self.emit(Op::SetGlobal(slot));
                    match origin {
                        Some(path) => {
                            self.frame_globals.insert(name.clone(), path);
                        }
                        None => {
                            self.frame_globals.remove(name);
                        }
                    }
                } else {
                    return self.err(name, "assignment to an undeclared variable");
                }
            }
            Stmt::AssignMember(name, field, expr) => {
                let origin = self.frame_origin(name);
                self.expr(expr)?;
                if let Some(path) = &origin {
                    self.note_frame_field(path, field);
                    if let Some(value) = const_number(expr) {
                        self.note_frame_const(path, field, value);
                    }
                }
                let f = self.constant(Value::Str(field.clone()));
                if let Some(idx) = self.resolve_local(name) {
                    self.emit(Op::SetFrameFieldLocal(idx, f));
                } else if self.globals.iter().any(|g| g == name) {
                    let slot = self.global_slot(name);
                    self.emit(Op::SetFrameFieldGlobal(slot, f));
                } else {
                    return self.err(
                        name,
                        "signal reads are read-only: create a frame instance \
                         with frame(...) and write its fields, or use set_sig/set_fr_sig",
                    );
                }
            }
            Stmt::AssignIndex(name, index, value) => {
                // Stack order (top down): value, index, container --
                // exactly what `SetIndex` consumes. The container must be
                // a plain variable holding the shared buffer.
                if let Some(idx) = self.resolve_local(name) {
                    self.emit(Op::GetLocal(idx));
                } else if self.globals.iter().any(|g| g == name) {
                    let slot = self.global_slot(name);
                    self.emit(Op::GetGlobal(slot));
                } else {
                    return self.err(name, "assignment to an undeclared variable");
                }
                self.expr(index)?;
                self.expr(value)?;
                self.emit(Op::SetIndex);
            }
            Stmt::If { cond, then, els } => {
                self.expr(cond)?;
                let j_else = self.emit_jump(Op::JumpIfFalse);
                self.block(then)?;
                if let Some(els) = els {
                    let j_end = self.emit_jump(Op::Jump);
                    self.patch(j_else);
                    self.block(els)?;
                    self.patch(j_end);
                } else {
                    self.patch(j_else);
                }
            }
            Stmt::While { cond, body } => {
                let start = self.code.len() as u16;
                self.expr(cond)?;
                let j_end = self.emit_jump(Op::JumpIfFalse);
                // Break inside this loop jumps to the same exit; continue
                // jumps back to the condition.
                self.break_jumps.push(Vec::new());
                self.continue_jumps.push(Vec::new());
                self.block(body)?;
                let breaks = self.break_jumps.pop().unwrap_or_default();
                let continues = self.continue_jumps.pop().unwrap_or_default();
                self.emit(Op::Jump(start));
                self.patch(j_end);
                for j in breaks {
                    self.patch(j);
                }
                for j in continues {
                    self.patch_to(j, start);
                }
            }
            Stmt::For {
                init,
                cond,
                step,
                body,
            } => {
                let loop_depth = self.depth;
                if let Some(init) = init {
                    self.depth += 1;
                    self.stmt(init.as_ref())?;
                    self.depth -= 1;
                }
                self.break_jumps.push(Vec::new());
                self.continue_jumps.push(Vec::new());
                let start = self.code.len() as u16;
                let j_end = match cond {
                    Some(cond) => {
                        self.expr(cond)?;
                        Some(self.emit_jump(Op::JumpIfFalse))
                    }
                    None => None,
                };
                self.depth += 1;
                self.block(body)?;
                self.depth -= 1;
                // `continue` lands on the step, not the condition.
                let step_pos = self.code.len() as u16;
                if let Some(step) = step {
                    self.stmt(step.as_ref())?;
                }
                self.emit(Op::Jump(start));
                if let Some(j_end) = j_end {
                    self.patch(j_end);
                }
                // Patch break jumps to the loop exit point.
                let breaks = self.break_jumps.pop().unwrap_or_default();
                let continues = self.continue_jumps.pop().unwrap_or_default();
                for j in breaks {
                    self.patch(j);
                }
                for j in continues {
                    self.patch_to(j, step_pos);
                }
                self.drop_locals(loop_depth);
            }
            Stmt::Return(expr) => {
                if !self.in_fn {
                    return self.err("'return'", "outside a function");
                }
                match expr {
                    Some(e) => self.expr(e)?,
                    None => {
                        let nil = self.constant(Value::Nil);
                        self.emit(Op::Const(nil));
                    }
                }
                self.emit(Op::Return);
            }
            Stmt::Break => {
                self.emit(Op::Jump(u16::MAX));
                match self.break_jumps.last_mut() {
                    Some(breaks) => breaks.push(self.code.len() - 1),
                    None => return self.err("'break'", "outside a loop"),
                }
            }
            Stmt::Continue => {
                self.emit(Op::Jump(u16::MAX));
                match self.continue_jumps.last_mut() {
                    Some(jumps) => jumps.push(self.code.len() - 1),
                    None => return self.err("'continue'", "outside a loop"),
                }
            }
            Stmt::Block(stmts) => {
                self.block(stmts)?;
            }
            Stmt::Expr(e) => {
                self.expr(e)?;
                self.emit(Op::Pop);
            }
        }
        Ok(())
    }

    fn line_mark(&mut self) {
        if self.cur_line != self.line {
            let at = self.code.len() as u16;
            self.line_marks.push((at, self.line));
            self.local_marks.push((
                at,
                self.locals.iter().map(|(name, _)| name.clone()).collect(),
            ));
            self.cur_line = self.line;
        }
    }

    fn block(&mut self, stmts: &[SpannedStmt]) -> Result<(), ScriptError> {
        self.depth += 1;
        for s in stmts {
            self.stmt(s)?;
        }
        self.depth -= 1;
        // Leaving a scope discards its locals at runtime too, so the
        // stack top always equals `base + locals.len()` and a new `let`
        // lands exactly on the slot the index bookkeeping expects.
        self.drop_locals(self.depth);
        Ok(())
    }

    fn drop_locals(&mut self, depth: u32) {
        while self.locals.last().is_some_and(|(_, d)| *d > depth) {
            self.locals.pop();
            self.emit(Op::Pop);
        }
        self.frame_locals.retain(|(_, _, d)| *d <= depth);
    }

    /// Emits a jump with a placeholder target and returns the
    /// instruction index to [`Self::patch`] once the target is known.
    fn emit_jump(&mut self, mk: impl Fn(u16) -> Op) -> usize {
        self.emit(mk(u16::MAX));
        self.code.len() - 1
    }

    fn patch(&mut self, at: usize) {
        let target = self.code.len() as u16;
        match &mut self.code[at] {
            Op::Jump(t) | Op::JumpIfFalse(t) => *t = target,
            _ => unreachable!("patched a non-jump"),
        }
    }

    /// Patches a jump to a known position: the `continue` target, which
    /// sits before the code emitted after the loop body.
    fn patch_to(&mut self, at: usize, target: u16) {
        match &mut self.code[at] {
            Op::Jump(t) => *t = target,
            _ => unreachable!("patched a non-jump"),
        }
    }

    fn expr(&mut self, e: &Expr) -> Result<(), ScriptError> {
        match e {
            Expr::Int(n) => {
                let c = self.constant(Value::Int(*n));
                self.emit(Op::Const(c));
            }
            Expr::Float(x) => {
                let c = self.constant(Value::Float(*x));
                self.emit(Op::Const(c));
            }
            Expr::Bool(b) => {
                let c = self.constant(Value::Bool(*b));
                self.emit(Op::Const(c));
            }
            Expr::Str(s) => {
                let c = self.constant(Value::Str(s.clone()));
                self.emit(Op::Const(c));
            }
            Expr::FrameNew(path) => {
                let c = self.constant(Value::Frame(FrameInstance::new(path.clone())));
                self.emit(Op::Const(c));
            }
            Expr::Member(base, field) => {
                let Expr::Ident(name) = &**base else {
                    return self.err(field, "only a variable holds a frame field");
                };
                if let Some(idx) = self.resolve_local(name) {
                    self.emit(Op::GetLocal(idx));
                    let f = self.constant(Value::Str(field.clone()));
                    self.emit(Op::GetFrameField(f));
                } else if self.globals.iter().any(|g| g == name) {
                    let slot = self.global_slot(name);
                    self.emit(Op::GetGlobal(slot));
                    let f = self.constant(Value::Str(field.clone()));
                    self.emit(Op::GetFrameField(f));
                } else {
                    // Existing database read: `Message.Signal`.
                    self.expr(&Expr::Call(
                        "sig_named".to_string(),
                        vec![Expr::Str(name.clone()), Expr::Str(field.clone())],
                    ))?;
                }
            }
            Expr::Ident(name) => {
                if let Some(idx) = self.resolve_local(name) {
                    self.emit(Op::GetLocal(idx));
                } else if self.globals.iter().any(|g| g == name) {
                    let slot = self.global_slot(name);
                    self.emit(Op::GetGlobal(slot));
                } else {
                    return self.err(name, "unknown variable");
                }
            }
            Expr::Index(container, index) => {
                // The container must be a plain variable: buffers have
                // reference semantics, so the read shares the stored one.
                self.expr(container)?;
                self.expr(index)?;
                self.emit(Op::GetIndex);
            }
            Expr::Unary(op, e) => {
                self.expr(e)?;
                self.emit(match op {
                    UnOp::Neg => Op::Neg,
                    UnOp::Not => Op::Not,
                });
            }
            Expr::Binary(op, lhs, rhs) => match op {
                // Short-circuit: the left value decides whether the right
                // side is even evaluated; the stack always ends with one
                // boolean either way.
                BinOp::And => {
                    self.expr(lhs)?;
                    let j = self.emit_jump(Op::JumpIfFalse);
                    self.expr(rhs)?;
                    let j_end = self.emit_jump(Op::Jump);
                    self.patch(j);
                    let f = self.constant(Value::Bool(false));
                    self.emit(Op::Const(f));
                    self.patch(j_end);
                }
                BinOp::Or => {
                    self.expr(lhs)?;
                    let j_false = self.emit_jump(Op::JumpIfFalse);
                    let t = self.constant(Value::Bool(true));
                    self.emit(Op::Const(t));
                    let j_end = self.emit_jump(Op::Jump);
                    self.patch(j_false);
                    self.expr(rhs)?;
                    self.patch(j_end);
                }
                plain => {
                    self.expr(lhs)?;
                    self.expr(rhs)?;
                    self.emit(match plain {
                        BinOp::Add => Op::Add,
                        BinOp::Sub => Op::Sub,
                        BinOp::Mul => Op::Mul,
                        BinOp::Div => Op::Div,
                        BinOp::Mod => Op::Mod,
                        BinOp::Eq => Op::Eq,
                        BinOp::Ne => Op::Ne,
                        BinOp::Lt => Op::Lt,
                        BinOp::Le => Op::Le,
                        BinOp::Gt => Op::Gt,
                        BinOp::Ge => Op::Ge,
                        BinOp::BitAnd => Op::BitAnd,
                        BinOp::BitOr => Op::BitOr,
                        BinOp::BitXor => Op::BitXor,
                        BinOp::Shl => Op::Shl,
                        BinOp::Shr => Op::Shr,
                        BinOp::And | BinOp::Or => unreachable!("handled above"),
                    });
                }
            },
            Expr::Call(name, args) => {
                if args.len() > MAX_ARGS {
                    return self.err(name, "too many arguments");
                }
                for a in args {
                    self.expr(a)?;
                }
                if let Some(&idx) = self.fn_index.get(name) {
                    if self.functions[idx as usize].arity != args.len() {
                        return self.err(
                            name,
                            &format!(
                                "expects {} argument(s), got {}",
                                self.functions[idx as usize].arity,
                                args.len()
                            ),
                        );
                    }
                    self.emit(Op::Call(idx, args.len() as u8));
                } else if let Some((id, min, max)) = HOST_FNS
                    .iter()
                    .enumerate()
                    .find_map(|(i, (n, min, max))| (*n == name).then_some((i as u16, *min, *max)))
                {
                    if args.len() < min || args.len() > max {
                        return self.err(
                            name,
                            &format!("expects {}..{} argument(s), got {}", min, max, args.len()),
                        );
                    }
                    if *name == "send_frame" {
                        if args.len() == 1
                            && let Some(path) = self.expr_frame_origin(&args[0])
                        {
                            self.note_frame_send(&path);
                        } else {
                            let why = "send_frame(...) gets a frame instance the compiler \
                                       cannot resolve"
                                .to_string();
                            if !self.opaque_sends.contains(&why) {
                                self.opaque_sends.push(why);
                            }
                        }
                    }
                    // Literal signal references to `sig` / `set_sig` are
                    // recorded so the host can check them against the
                    // database at node-assembly time.
                    if *name == "sig"
                        && args.len() == 2
                        && let (Expr::Int(id), Expr::Str(sig)) = (&args[0], &args[1])
                        && *id >= 0
                    {
                        let r = (*id as u32, sig.clone());
                        if !self.signal_refs.contains(&r) {
                            self.signal_refs.push(r);
                        }
                    }
                    // System variable accesses are recorded the same way:
                    // the host checks the keys against the defined
                    // registry when the node starts.
                    if (*name == "sys_get" || *name == "sys_set")
                        && !args.is_empty()
                        && let Expr::Str(key) = &args[0]
                        && !self.sysvar_refs.contains(key)
                    {
                        self.sysvar_refs.push(key.clone());
                    }
                    // A named read desugars into sig_named with two string
                    // literals: record the pair so the host can resolve both
                    // names against the database at start.
                    if *name == "sig_named"
                        && let (Expr::Str(msg), Expr::Str(sig)) = (&args[0], &args[1])
                    {
                        let r = (msg.clone(), sig.clone());
                        if !self.named_signal_refs.contains(&r) {
                            self.named_signal_refs.push(r);
                        }
                    }
                    // Dotted FR/ETH reads carry their bus and names as
                    // literals; the host resolves the slot/frame at start.
                    if *name == "fr_sig_named"
                        && let (Expr::Int(bus), Expr::Str(sig)) = (&args[0], &args[1])
                        && (0..=255).contains(bus)
                    {
                        let r = (*bus as u8, sig.clone());
                        if !self.fr_named_refs.contains(&r) {
                            self.fr_named_refs.push(r);
                        }
                    }
                    if *name == "eth_sig"
                        && let (Expr::Int(bus), Expr::Str(frame), Expr::Str(sig)) =
                            (&args[0], &args[1], &args[2])
                        && (0..=255).contains(bus)
                    {
                        let r = (*bus as u8, frame.clone(), sig.clone());
                        if !self.eth_named_refs.contains(&r) {
                            self.eth_named_refs.push(r);
                        }
                    }
                    if *name == "fr_sig_bus"
                        && let (Expr::Str(bus), Expr::Str(sig)) = (&args[0], &args[1])
                    {
                        let r = (bus.clone(), sig.clone());
                        if !self.fr_bus_refs.contains(&r) {
                            self.fr_bus_refs.push(r);
                        }
                    }
                    if *name == "eth_sig_bus"
                        && let (Expr::Str(bus), Expr::Str(frame), Expr::Str(sig)) =
                            (&args[0], &args[1], &args[2])
                    {
                        let r = (bus.clone(), frame.clone(), sig.clone());
                        if !self.eth_bus_refs.contains(&r) {
                            self.eth_bus_refs.push(r);
                        }
                    }
                    // R2 fail-closed rule: every `send` / `send_ext` id
                    // must be statically derivable -- a literal, constant
                    // arithmetic, or `frame_id()`. The derived set is
                    // recorded for the host's assembly checks; anything
                    // non-derivable is a compile error, not a runtime
                    // surprise. A wildcard forward is the one declared
                    // exception: reported, not rejected.
                    if (*name == "send" || *name == "send_ext")
                        && !args.is_empty()
                        && let Some((kind, from)) = self.cur_handler.clone()
                    {
                        let ext_call = *name == "send_ext";
                        // `frame_id()` echoes the frame that triggered the
                        // handler: bounded by the event binding itself.
                        let is_frame_id =
                            matches!(&args[0], Expr::Call(n, a) if n == "frame_id" && a.is_empty());
                        if is_frame_id {
                            match &kind {
                                HandlerKind::Message { id } => {
                                    self.send_refs.push((from, *id, false));
                                }
                                HandlerKind::ExtendedMessage { id } => {
                                    self.send_refs.push((from, *id, true));
                                }
                                HandlerKind::AnyMessage => self.opaque_sends.push(format!(
                                    "{from}: wildcard forward (frame_id), set = received set"
                                )),
                                HandlerKind::NamedMessage { .. } => self.opaque_sends.push(
                                    format!(
                                        "{from}: named-message forward (frame_id), id resolved at start"
                                    ),
                                ),
                                // `frame_id()` here is a slot number. Deriving a
                                // CAN send from it would put a frame on the
                                // node's CAN channel whose id happens to be the
                                // slot -- exactly the mistake the static send
                                // set exists to catch, so it names itself.
                                HandlerKind::FrSlot { .. } => {
                                    return self.err(
                                        &from,
                                        "frame_id() is a FlexRay slot here; send() would put it on the CAN wire as an id",
                                    );
                                }
                                _ => {
                                    return self.err(&from, "frame_id outside a frame event");
                                }
                            }
                        } else if let Some(v) = self.const_int(&args[0]) {
                            match u32::try_from(v) {
                                // 29 bits: the largest id any CAN frame carries.
                                Ok(id) if id <= 0x1FFF_FFFF => {
                                    self.send_refs.push((from, id, ext_call));
                                }
                                _ => {
                                    return self.err(
                                        &from,
                                        &format!("send id {v} outside the frame-id range"),
                                    );
                                }
                            }
                        } else {
                            return self.err(
                                &from,
                                "send id must be statically derivable: a literal, \
                                 constant arithmetic, or frame_id()",
                            );
                        }
                    }
                    // R2 response mapping: literal `set_timer("name", ms)`
                    // calls are recorded with the handler that arms them,
                    // so the report can tie events to their responses.
                    if *name == "set_timer"
                        && args.len() == 2
                        && let Some((kind, _)) = self.cur_handler.clone()
                        && let Expr::Str(tname) = &args[0]
                    {
                        let label = handler_label(&kind);
                        self.timer_arms.push((label, tname.clone()));
                    }
                    self.emit(Op::CallHost(id, args.len() as u8));
                } else {
                    // Neither a script function nor a builtin: a
                    // host-extension call (node runtime builtins, external
                    // components). The name travels in the constant pool
                    // and resolves at runtime -- the host may register
                    // functions the compiler has never seen.
                    let name_const = self.constant(Value::Str(name.clone()));
                    self.emit(Op::CallExtern(name_const, args.len() as u8));
                    // `set_sig` rides this path (it is a node extern, not a
                    // builtin), so its signal reference -- and a literal
                    // write value, for the host's range check at node
                    // start -- is recorded here.
                    if *name == "set_sig"
                        && args.len() == 4
                        && let (Expr::Int(id), Expr::Str(sig)) = (&args[1], &args[2])
                        && *id >= 0
                    {
                        let r = (*id as u32, sig.clone());
                        if !self.signal_refs.contains(&r) {
                            self.signal_refs.push(r);
                        }
                        if let Some(v) = self.const_f64(&args[3]) {
                            let entry = (*id as u32, sig.clone(), v);
                            if !self.set_sig_values.contains(&entry) {
                                self.set_sig_values.push(entry);
                            }
                        }
                    }
                }
            }
        }
        Ok(())
    }

    fn compile_fn(&mut self, f: FnDecl) -> Result<(), ScriptError> {
        let idx = self.fn_index[&f.name] as usize;
        let saved_code = std::mem::take(&mut self.code);
        let saved_marks = std::mem::take(&mut self.line_marks);
        let saved_locals_marks = std::mem::take(&mut self.local_marks);
        let saved_cur = self.cur_line;
        let saved_locals = std::mem::take(&mut self.locals);
        let saved_frame_locals = std::mem::take(&mut self.frame_locals);
        let saved_depth = self.depth;
        let saved_in_fn = self.in_fn;
        // A plain function has no handler context: sends inside one are
        // attributed by whoever calls it -- beyond the spike's dataflow.
        let saved_handler = self.cur_handler.take();
        self.code = Vec::new();
        self.cur_line = 0;
        self.in_fn = true;
        self.depth = 1;
        if f.params.len() > MAX_LOCALS {
            return self.err(&f.name, "too many parameters");
        }
        self.locals = f.params.iter().map(|p| (p.clone(), 1)).collect();
        self.frame_locals = f.params.iter().map(|p| (p.clone(), None, 1)).collect();
        self.block(&f.body)?;
        // Implicit `return nil;` for bodies that fall off the end.
        let nil = self.constant(Value::Nil);
        self.emit(Op::Const(nil));
        self.emit(Op::Return);
        self.functions[idx].code = std::mem::take(&mut self.code);
        self.functions[idx].lines = std::mem::take(&mut self.line_marks);
        self.functions[idx].local_marks = std::mem::take(&mut self.local_marks);
        self.code = saved_code;
        self.line_marks = saved_marks;
        self.local_marks = saved_locals_marks;
        self.cur_line = saved_cur;
        self.locals = saved_locals;
        self.frame_locals = saved_frame_locals;
        self.depth = saved_depth;
        self.in_fn = saved_in_fn;
        self.cur_handler = saved_handler;
        Ok(())
    }

    /// Constant-folds an expression to an integer, for the R2 spike's
    /// static id derivation: literals, `neg`, and arithmetic over
    /// constants. Anything else (variables, host calls, floats) is
    /// `None` -- the fail-closed direction.
    fn const_int(&self, e: &Expr) -> Option<i64> {
        match e {
            Expr::Int(v) => Some(*v),
            Expr::Unary(UnOp::Neg, x) => self.const_int(x)?.checked_neg(),
            Expr::Binary(op, l, r) => {
                let (l, r) = (self.const_int(l)?, self.const_int(r)?);
                match op {
                    BinOp::Add => l.checked_add(r),
                    BinOp::Sub => l.checked_sub(r),
                    BinOp::Mul => l.checked_mul(r),
                    BinOp::Div if r != 0 => l.checked_div(r),
                    BinOp::Mod if r != 0 => l.checked_rem(r),
                    // Bit arithmetic folds too, so `send(0x100 | 0x8)`
                    // stays inside the statically derivable id set.
                    BinOp::BitAnd => Some(l & r),
                    BinOp::BitOr => Some(l | r),
                    BinOp::BitXor => Some(l ^ r),
                    BinOp::Shl => {
                        if (0..64).contains(&r) {
                            l.checked_shl(r as u32)
                        } else {
                            None
                        }
                    }
                    BinOp::Shr => {
                        if (0..64).contains(&r) {
                            l.checked_shr(r as u32)
                        } else {
                            None
                        }
                    }
                    _ => None,
                }
            }
            _ => None,
        }
    }

    /// Constant-folds an expression to a float: number literals and
    /// negation. Values written as arithmetic go unchecked by design --
    /// a computed setpoint is runtime business.
    fn const_f64(&self, e: &Expr) -> Option<f64> {
        match e {
            Expr::Int(v) => Some(*v as f64),
            Expr::Float(v) => Some(*v),
            Expr::Unary(UnOp::Neg, x) => Some(-self.const_f64(x)?),
            _ => None,
        }
    }

    /// Compiles one event handler body into its own chunk and registers
    /// it in the handler table. Bodies behave like zero-argument
    /// functions: locals, globals access, early return.
    fn compile_handler(&mut self, on: OnDecl) -> Result<(), ScriptError> {
        let kind = match &on.kind {
            OnKind::Start => HandlerKind::Start,
            OnKind::Stop => HandlerKind::Stop,
            OnKind::PanelAction { name, arg } => HandlerKind::PanelAction {
                name: name.clone(),
                arg: arg.clone(),
            },
            OnKind::Message { id } => HandlerKind::Message { id: *id },
            OnKind::NamedMessage { name } => HandlerKind::NamedMessage { name: name.clone() },
            OnKind::ExtendedMessage { id } => HandlerKind::ExtendedMessage { id: *id },
            OnKind::AnyMessage => HandlerKind::AnyMessage,
            OnKind::ErrorFrame => HandlerKind::ErrorFrame,
            OnKind::Timer { period_ms } => HandlerKind::Timer {
                period_ms: *period_ms,
            },
            OnKind::Oneshot { name } => HandlerKind::Oneshot { name: name.clone() },
            OnKind::FrSlot { bus, slot } => HandlerKind::FrSlot {
                bus: *bus,
                slot: *slot,
            },
        };
        // 接收集（R2 静态事实表）：帧事件 handler 声明监听的 id。
        match &on.kind {
            OnKind::Message { id } => {
                if !self.recv_refs.contains(&(*id, false)) {
                    self.recv_refs.push((*id, false));
                }
            }
            OnKind::ExtendedMessage { id } if !self.recv_refs.contains(&(*id, true)) => {
                self.recv_refs.push((*id, true));
            }
            _ => {}
        }
        let saved_code = std::mem::take(&mut self.code);
        let saved_marks = std::mem::take(&mut self.line_marks);
        let saved_locals_marks = std::mem::take(&mut self.local_marks);
        let saved_cur = self.cur_line;
        let saved_locals = std::mem::take(&mut self.locals);
        let saved_frame_locals = std::mem::take(&mut self.frame_locals);
        let saved_depth = self.depth;
        let saved_in_fn = self.in_fn;
        let saved_handler = self.cur_handler.take();
        let label = handler_label(&kind);
        self.code = Vec::new();
        self.cur_line = 0;
        self.in_fn = true;
        self.depth = 1;
        // A panel-action parameter is local slot 0: the node pushes the
        // control's value before running this chunk.
        self.locals = match &kind {
            HandlerKind::PanelAction {
                arg: Some(name), ..
            } => vec![(name.clone(), 1)],
            _ => Vec::new(),
        };
        self.frame_locals = self
            .locals
            .iter()
            .map(|(name, _)| (name.clone(), None, 1))
            .collect();
        let arity = self.locals.len();
        // The handler context must be live while the body compiles: the
        // R2 spike attributes every derived send to this handler.
        self.cur_handler = Some((kind.clone(), label.clone()));
        self.block(&on.body)?;
        let nil = self.constant(Value::Nil);
        self.emit(Op::Const(nil));
        self.emit(Op::Return);
        let chunk = self.functions.len() as u16;
        let lines = std::mem::take(&mut self.line_marks);
        self.functions.push(Function {
            arity,
            name: label,
            code: std::mem::take(&mut self.code),
            lines,
            local_marks: std::mem::take(&mut self.local_marks),
        });
        self.handlers.push(Handler { kind, chunk });
        self.cur_handler = saved_handler;
        self.code = saved_code;
        self.line_marks = saved_marks;
        self.local_marks = saved_locals_marks;
        self.cur_line = saved_cur;
        self.locals = saved_locals;
        self.frame_locals = saved_frame_locals;
        self.depth = saved_depth;
        self.in_fn = saved_in_fn;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::super::lexer::lex;
    use super::super::parser::parse;
    use super::*;

    #[test]
    fn frame_instances_report_paths_and_fields() {
        let script = compile_ok(
            r#"
            on timer 10 {
                let f = frame(EngineStatus);
                f.EngineSpeed = 1000;
                send_frame(f);
            }
            "#,
        );
        assert_eq!(script.frame_sends, ["EngineStatus"]);
        assert_eq!(
            script.frame_fields,
            [("EngineStatus".to_string(), "EngineSpeed".to_string())]
        );
    }

    #[test]
    fn constant_frame_fields_are_recorded_for_range_checks() {
        let script = compile_ok(
            r#"
            on timer 10 {
                let f = frame(EngineStatus);
                f.EngineTemp = -40;
                f.EngineSpeed = 1000 + 1;
                f.EngineSpeed = 2000;
                send_frame(f);
            }
            "#,
        );
        assert_eq!(
            script.frame_consts,
            [
                ("EngineStatus".to_string(), "EngineTemp".to_string(), -40.0),
                (
                    "EngineStatus".to_string(),
                    "EngineSpeed".to_string(),
                    2000.0
                )
            ],
            "literals and negated literals only"
        );
    }

    fn compile_ok(src: &str) -> Script {
        let program = parse(lex(src).unwrap()).unwrap();
        compile(program).unwrap()
    }

    fn compile_err(src: &str) -> String {
        // Through the full pipeline: some rejections happen at parse time.
        super::super::compile(src).unwrap_err().to_string()
    }

    #[test]
    fn forward_calls_resolve() {
        let script = compile_ok("print(later(1)); fn later(n) { return n + 1; }");
        assert_eq!(script.functions.len(), 2); // The call lives in main (chunk 0); `later`'s own body has none.
        assert!(
            script.functions[0]
                .code
                .iter()
                .any(|op| matches!(op, Op::Call(1, 1)))
        );
    }

    #[test]
    fn debug_metadata_names_functions_and_locals() {
        let script = compile_ok("fn add(a) {\n    let b = a + 1;\n    print(b);\n}\nadd(4);\n");
        assert_eq!(script.functions[0].name, "main");
        let add = script
            .functions
            .iter()
            .find(|function| function.name == "add")
            .expect("add chunk");
        assert!(add.lines.iter().any(|(_, line)| *line == 2));
        assert!(add.lines.iter().any(|(_, line)| *line == 3));
        let names: Vec<Vec<String>> = add
            .local_marks
            .iter()
            .map(|(_, names)| names.clone())
            .collect();
        assert!(names.contains(&vec!["a".to_string()]));
        assert!(names.contains(&vec!["a".to_string(), "b".to_string()]));
    }

    #[test]
    fn rejections() {
        assert!(compile_err("print(nope);").contains("unknown"));
        assert!(compile_err("x = 1;").contains("undeclared"));
        assert!(compile_err("on stop { } on stop { }").contains("duplicate 'on stop'"));
        assert!(
            compile_err("on panel_action(\"Go\") { } on panel_action(\"Go\") { }")
                .contains("duplicate handler for panel action")
        );
        assert!(compile_err("on panel_action(\"\") { }").contains("cannot be empty"));
        assert!(compile_err("on start { } on start { }").contains("duplicate 'on start'"));
        assert!(compile_err("fn f(a) { } print(f());").contains("argument"));
        assert!(compile_err("fn f() { } fn f() { }").contains("duplicate"));
        assert!(compile_err("fn f() { fn g() { } }").contains("top level"));
    }

    #[test]
    fn while_loop_jumps_close_backwards() {
        let script = compile_ok("while (true) { }");
        let has_back_jump = script.functions[0]
            .code
            .iter()
            .any(|op| matches!(op, Op::Jump(t) if *t < 4));
        assert!(has_back_jump);
    }

    // ---- R2 spike: static send-set derivation ----------------------------

    #[test]
    fn literal_and_arithmetic_sends_are_derived() {
        let script = compile_ok("on timer 100 { send(0x100, 1); send_ext(0x100 + 0x20, 1); }");
        assert!(
            script.opaque_sends.is_empty(),
            "both sends are derivable: {:?}",
            script.opaque_sends
        );
        assert_eq!(script.send_refs.len(), 2);
        assert_eq!(
            script.send_refs[0],
            ("<on timer 100>".to_string(), 0x100, false)
        );
        assert_eq!(
            script.send_refs[1],
            ("<on timer 100>".to_string(), 0x120, true)
        );
    }

    #[test]
    fn frame_id_sends_are_bounded_by_the_event() {
        let script = compile_ok("on message 0x6A0 { send(frame_id(), 1); }");
        assert_eq!(script.send_refs.len(), 1);
        assert_eq!(
            script.send_refs[0],
            ("<on message 0x6a0>".to_string(), 0x6A0, false)
        );
    }

    #[test]
    fn variables_make_the_send_opaque() {
        // Fail-closed: a send id the compiler cannot derive is a compile
        // error, not a runtime surprise. (`let n = 0x100;` is not tracked
        // -- write the literal in the call.)
        let err = compile_err("on timer 100 { let n = 0x100; send(n, 1); }");
        assert!(err.contains("must be statically derivable"), "{err}");
    }

    #[test]
    fn a_wildcard_forward_is_named_as_such() {
        let script = compile_ok("on message * { send(frame_id(), 1); }");
        assert!(script.send_refs.is_empty());
        assert_eq!(script.opaque_sends.len(), 1);
        assert!(script.opaque_sends[0].contains("wildcard forward"));
    }

    /// The static response mapping: the request event that arms the
    /// "resp" timer shows up as the arming handler, and the reply frame
    /// shows up in that row's sends. The periodic handler is absent.
    #[test]
    fn the_response_map_ties_events_to_their_replies() {
        let script = compile_ok(
            r#"
            on message 0x6A0 {
                set_timer("resp", 200);
            }
            on timer "resp" {
                send(0x6A1, 1);
            }
            on timer 100 {
                send(0x200, 1);
            }
        "#,
        );
        let map = script.response_map();
        assert_eq!(map.len(), 1, "one one-shot timer, one response row");
        let row = &map[0];
        assert_eq!(row.armed_by, vec!["<on message 0x6a0>".to_string()]);
        assert_eq!(row.sends, vec![(0x6A1, false)]);
    }

    #[test]
    fn frame_id_outside_a_frame_event_is_rejected() {
        let err = compile_err("on start { send(frame_id(), 1); }");
        assert!(err.contains("frame_id outside a frame event"), "{err}");
    }

    #[test]
    fn an_out_of_range_send_id_is_rejected() {
        let err = compile_err("on timer 10 { send(0x20000000, 1); }");
        assert!(err.contains("outside the frame-id range"), "{err}");
    }

    // ---- R2 表达力增量：位运算 / 复合赋值 / switch ------------------------

    /// Bit arithmetic on constants folds, so id arithmetic with `|` / `&`
    /// stays inside the statically derivable send set.
    #[test]
    fn bit_arithmetic_keeps_send_ids_derivable() {
        let script = compile_ok("on timer 100 { send(0x100 | 0x8, 1); send(1 << 4, 2); }");
        assert!(
            script.opaque_sends.is_empty(),
            "folded through the bitwise ops: {:?}",
            script.opaque_sends
        );
        assert_eq!(script.send_refs[0].1, 0x108);
        assert_eq!(script.send_refs[1].1, 0x10);
    }

    /// The bitwise ops compute at runtime; floats refuse to promote and
    /// out-of-range shifts are errors, not silent masks.
    #[test]
    fn bitwise_ops_run_and_reject_nonsense() {
        let script = compile_ok(
            r#"
            let packed = 0xF0 | 0x0F;
            let masked = packed & 0x3C;
            let flipped = 0xFF ^ 0x0F;
            let high = 1 << 6;
            let low = 0x100 >> 4;
            print(packed, masked, flipped, high, low);
        "#,
        );
        let mut vm = crate::script::Vm::new(script);
        vm.run().unwrap();
        // One print call joins its arguments on one line.
        assert_eq!(vm.output[0], "255 60 240 64 16");

        // Floats do not promote into bit land.
        let script = compile_ok("print(1.5 & 1);");
        let mut vm = crate::script::Vm::new(script);
        let err = vm.run().unwrap_err().to_string();
        assert!(err.contains("ints"), "{err}");

        // Shifts outside 0..64 are errors.
        let script = compile_ok("print(1 << 64);");
        let mut vm = crate::script::Vm::new(script);
        assert!(vm.run().is_err());
    }

    /// Compound stores compile as the expanded form, on globals, locals
    /// and buffer elements alike.
    #[test]
    fn compound_assignment_stores() {
        let script = compile_ok(
            r#"
            let total = 10;
            fn bump(by) {
                let local = 1;
                local += by;
                return local;
            }
            total += 5;
            total *= 2;
            let buf = bytes(2);
            buf[0] = 1;
            buf[0] <<= 3;
            print(total, bump(4), buf[0]);
        "#,
        );
        let mut vm = crate::script::Vm::new(script);
        vm.run().unwrap();
        assert_eq!(vm.output[0], "30 5 8");
    }

    /// Each case is exclusive (no fallthrough), `default` catches the
    /// rest, and a call subject evaluates exactly once.
    #[test]
    fn switch_selects_one_case() {
        let script = compile_ok(
            r#"
            let state = 2;
            let calls = 0;
            fn subject() {
                calls += 1;
                return 2;
            }
            switch (state) {
                case 1: { print("one"); }
                case 2: { print("two"); }
                default: { print("other"); }
            }
            switch (subject()) {
                case 1: { print("again-one"); }
                case 2: { print("again-two"); }
            }
            print(calls);
        "#,
        );
        let mut vm = crate::script::Vm::new(script);
        vm.run().unwrap();
        assert_eq!(
            vm.output,
            vec!["two", "again-two", "1"],
            "one case each, subject called once"
        );
    }

    /// R2 spike deliverable: run the static send-set derivation over the
    /// shipped examples and print the coverage report. The examples are
    /// the "expected user scripts" sample the roadmap asked for -- the
    /// spike's question is whether the derivable set covers their sends.
    #[test]
    fn r2_spike_reports_static_send_coverage_on_examples() {
        let Ok(rd) = std::fs::read_dir("examples") else {
            panic!(
                "examples/ missing -- it is tracked in the repository, so a broken checkout must fail, not skip"
            );
        };
        let mut total_derived = 0usize;
        let mut total_opaque = 0usize;
        let mut files = 0usize;
        for file in rd.flatten() {
            let path = file.path();
            if path.extension().is_none_or(|e| e != "rxcan") {
                continue;
            }
            let src = std::fs::read_to_string(&path).unwrap();
            let script = match crate::script::compile(&src) {
                Ok(s) => s,
                Err(e) => {
                    println!("{}: compile error {e}", path.display());
                    total_opaque += 1;
                    continue;
                }
            };
            files += 1;
            total_derived += script.send_refs.len();
            total_opaque += script.opaque_sends.len();
            println!(
                "{}: {} derived send(s), {} opaque",
                path.display(),
                script.send_refs.len(),
                script.opaque_sends.len()
            );
            for (from, id, ext) in &script.send_refs {
                println!("    + {}  {id:#X}{}", from, if *ext { "x" } else { "" });
            }
            for why in &script.opaque_sends {
                println!("    ? {why}");
            }
        }
        println!(
            "coverage: {total_derived} derived, {total_opaque} opaque over {files} example(s)"
        );
        assert!(files >= 4, "the shipped examples were swept");
    }
}
