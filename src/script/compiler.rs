//! AST to bytecode. Two passes: signatures first (so scripts may call a
//! function defined later in the file), then bodies. Top-level `let`s
//! become globals; function bodies use stack locals with block scopes.

use super::parser::{BinOp, Expr, FnDecl, Item, OnDecl, OnKind, Program, Stmt, UnOp};
use super::{Function, HOST_FNS, Handler, HandlerKind, Op, Script, ScriptError, Value};
use std::collections::{HashMap, HashSet};

const MAX_LOCALS: usize = u8::MAX as usize;
const MAX_ARGS: usize = u8::MAX as usize;

pub fn compile(program: Program) -> Result<Script, ScriptError> {
    let mut c = Comp {
        constants: Vec::new(),
        globals: Vec::new(),
        functions: vec![Function {
            name: "<main>".to_string(),
            arity: 0,
            code: Vec::new(),
        }],
        fn_index: HashMap::new(),
        handlers: Vec::new(),
        code: Vec::new(),
        locals: Vec::new(),
        depth: 0,
        in_fn: false,
        line: 1,
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
                name: f.name.clone(),
                arity: f.params.len(),
                code: Vec::new(),
            });
        }
    }

    // Handler sanity: one on start, one handler per message id.
    let mut seen_start = false;
    let mut seen_ids: HashSet<u32> = HashSet::new();
    for item in &program.items {
        if let Item::On(on) = item {
            match &on.kind {
                OnKind::Start => {
                    if seen_start {
                        return c.err_at(on.line, "duplicate 'on start' handler");
                    }
                    seen_start = true;
                }
                OnKind::Message { id } => {
                    if !seen_ids.insert(*id) {
                        return c.err_at(on.line, &format!("duplicate handler for id {id:#x}"));
                    }
                }
                OnKind::Timer { .. } => {}
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
    Ok(Script {
        constants: c.constants,
        globals: c.globals,
        functions: c.functions,
        handlers: c.handlers,
        host_fns: HOST_FNS.iter().map(|(n, _, _)| n.to_string()).collect(),
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
    /// Locals of the function being compiled: name and scope depth.
    locals: Vec<(String, u32)>,
    depth: u32,
    in_fn: bool,
    line: u32,
}

impl Comp {
    fn err<T>(&self, where_: &str, msg: &str) -> Result<T, ScriptError> {
        Err(ScriptError {
            line: self.line,
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

    fn err_at<T>(&self, line: u32, msg: &str) -> Result<T, ScriptError> {
        Err(ScriptError {
            line,
            msg: msg.to_string(),
        })
    }

    fn resolve_local(&self, name: &str) -> Option<u8> {
        self.locals
            .iter()
            .rposition(|(n, _)| n == name)
            .and_then(|i| u8::try_from(i).ok())
    }

    fn stmt(&mut self, s: &Stmt) -> Result<(), ScriptError> {
        match s {
            Stmt::Let(name, expr) => {
                self.expr(expr)?;
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
                } else {
                    let slot = self.global_slot(name);
                    self.emit(Op::SetGlobal(slot));
                }
            }
            Stmt::Assign(name, expr) => {
                self.expr(expr)?;
                if let Some(idx) = self.resolve_local(name) {
                    self.emit(Op::SetLocal(idx));
                } else if self.globals.iter().any(|g| g == name) {
                    let slot = self.global_slot(name);
                    self.emit(Op::SetGlobal(slot));
                } else {
                    return self.err(name, "assignment to an undeclared variable");
                }
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
                self.block(body)?;
                self.emit(Op::Jump(start));
                self.patch(j_end);
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
                    self.stmt(init)?;
                    self.depth -= 1;
                }
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
                if let Some(step) = step {
                    self.stmt(step)?;
                }
                self.depth -= 1;
                self.emit(Op::Jump(start));
                if let Some(j_end) = j_end {
                    self.patch(j_end);
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

    fn block(&mut self, stmts: &[Stmt]) -> Result<(), ScriptError> {
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
                    self.emit(Op::CallHost(id, args.len() as u8));
                } else {
                    return self.err(name, "unknown function");
                }
            }
        }
        Ok(())
    }

    fn compile_fn(&mut self, f: FnDecl) -> Result<(), ScriptError> {
        let idx = self.fn_index[&f.name] as usize;
        let saved_code = std::mem::take(&mut self.code);
        let saved_locals = std::mem::take(&mut self.locals);
        let saved_depth = self.depth;
        let saved_in_fn = self.in_fn;
        self.code = Vec::new();
        self.in_fn = true;
        self.depth = 1;
        if f.params.len() > MAX_LOCALS {
            return self.err(&f.name, "too many parameters");
        }
        self.locals = f.params.iter().map(|p| (p.clone(), 1)).collect();
        self.block(&f.body)?;
        // Implicit `return nil;` for bodies that fall off the end.
        let nil = self.constant(Value::Nil);
        self.emit(Op::Const(nil));
        self.emit(Op::Return);
        self.functions[idx].code = std::mem::take(&mut self.code);
        self.code = saved_code;
        self.locals = saved_locals;
        self.depth = saved_depth;
        self.in_fn = saved_in_fn;
        Ok(())
    }

    /// Compiles one event handler body into its own chunk and registers
    /// it in the handler table. Bodies behave like zero-argument
    /// functions: locals, globals access, early return.
    fn compile_handler(&mut self, on: OnDecl) -> Result<(), ScriptError> {
        let kind = match &on.kind {
            OnKind::Start => HandlerKind::Start,
            OnKind::Message { id } => HandlerKind::Message { id: *id },
            OnKind::Timer { period_ms } => HandlerKind::Timer {
                period_ms: *period_ms,
            },
        };
        let saved_code = std::mem::take(&mut self.code);
        let saved_locals = std::mem::take(&mut self.locals);
        let saved_depth = self.depth;
        let saved_in_fn = self.in_fn;
        self.code = Vec::new();
        self.in_fn = true;
        self.depth = 1;
        self.block(&on.body)?;
        let nil = self.constant(Value::Nil);
        self.emit(Op::Const(nil));
        self.emit(Op::Return);
        let chunk = self.functions.len() as u16;
        let label = match &kind {
            HandlerKind::Start => "<on start>".to_string(),
            HandlerKind::Message { id } => format!("<on message {id:#x}>"),
            HandlerKind::Timer { period_ms } => format!("<on timer {period_ms}>"),
        };
        self.functions.push(Function {
            name: label,
            arity: 0,
            code: std::mem::take(&mut self.code),
        });
        self.handlers.push(Handler { kind, chunk });
        self.code = saved_code;
        self.locals = saved_locals;
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
        assert_eq!(script.functions.len(), 2);
        // The call lives in main (chunk 0); `later`'s own body has none.
        assert!(
            script.functions[0]
                .code
                .iter()
                .any(|op| matches!(op, Op::Call(1, 1)))
        );
    }

    #[test]
    fn rejections() {
        assert!(compile_err("print(nope);").contains("unknown"));
        assert!(compile_err("x = 1;").contains("undeclared"));
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
}
