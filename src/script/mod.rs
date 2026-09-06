//! The node scripting language: a small C-flavoured source text compiled
//! to bytecode and run by a stack VM. Nothing here touches the bus --
//! scripts are pure code plus host calls; the node runtime (S2) wires
//! them into the core loop.
//!
//! # Language reference (v1)
//!
//! ```text
//! // Globals, functions, event handlers -- any order.
//! let base = 800;                     // int / float / bool / string
//!
//! fn limit(v) {                       // user functions, recursion OK
//!     if (v > 5000) { return 5000; }
//!     return v;
//! }
//!
//! on start { print("node up"); }      // once per measurement start
//!
//! on message 0x100 {                  // frame with this id arrives
//!     let rpm = sig(0x100, "RPM");    // latest published value (error if unseen)
//!     send(0x200, rpm * 2);           // raw payload: int bytes 0..255
//! }
//!
//! on timer 100 {                      // every 100 ms
//!     let buf = bytes(8);             // zero-filled byte buffer
//!     buf[0] = 0xAB;                  // element store (0..255)
//!     set_sig(buf, 0x200, "RPM", 1000 + random(0, 50));  // DBC encode
//!     send(0x200, buf);               // buffer as payload
//!     print("t", now(), limit(buf[0]));                  // text log
//! }
//! ```
//!
//! Types: int, float, bool, string, bytes (reference semantics). Math is
//! int-exact / float-promoting; `+` with a string concatenates. Control
//! flow: `if (..) .. else ..`, `while (..) ..`, `for (init; cond; step) ..`,
//! `break`/`continue` inside loops, early `return` inside functions.
//! Bytecode: constants + ops, stack VM
//! with per-callback instruction budget and frame-depth cap.
//!
//! Architecture seam for external libraries: host functions live in one
//! table ([`HOST_FNS`]) that the compiler resolves to `Op::CallHost(id)`
//! and the VM dispatches by the same index; anything else resolves at
//! runtime, first through the [`register_extern`] registry (in-process
//! external simulation components) and then through the [`HostExternFn`]
//! hook the embedder sets per node. Neither seam changes the bytecode
//! format.

// S2 (the node runtime) is what calls into this module from the product
// path; until it lands the module is reachable only from tests, which is
// exactly why the dead-code sweep must stay quiet here.
#![allow(dead_code)]

use std::collections::HashMap;

mod compiler;
mod lexer;
mod parser;
mod vm;

pub use vm::{TimerOp, Vm};

/// A runtime value of the script language.
#[derive(Clone, Debug)]
pub enum Value {
    Nil,
    Bool(bool),
    Int(i64),
    Float(f64),
    Str(String),
    /// A byte buffer with reference semantics: assignments and calls
    /// share the one buffer (like CANoe's message arrays), so `buf[0] = x`
    /// through any alias is visible everywhere. Arc + Mutex because the
    /// VM runs on the core thread and the value must stay Send.
    Bytes(std::sync::Arc<std::sync::Mutex<Vec<u8>>>),
}

impl PartialEq for Value {
    /// Bytes buffers compare by contents; everything else structurally.
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Value::Nil, Value::Nil) => true,
            (Value::Bool(a), Value::Bool(b)) => a == b,
            (Value::Int(a), Value::Int(b)) => a == b,
            (Value::Float(a), Value::Float(b)) => a == b,
            (Value::Str(a), Value::Str(b)) => a == b,
            (Value::Bytes(a), Value::Bytes(b)) => {
                *a.lock().expect("buffer poisoned") == *b.lock().expect("buffer poisoned")
            }
            _ => false,
        }
    }
}

impl std::fmt::Display for Value {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Value::Nil => write!(f, "nil"),
            Value::Bool(b) => write!(f, "{b}"),
            Value::Int(n) => write!(f, "{n}"),
            // Debug keeps a ".0" on whole floats so 3.0 never prints as 3.
            Value::Float(x) => write!(f, "{x:?}"),
            Value::Str(s) => write!(f, "{s}"),
            // Hex, the CAN-native reading of payload bytes.
            Value::Bytes(b) => {
                let b = b.lock().expect("buffer poisoned");
                let hex = b
                    .iter()
                    .map(|x| format!("{x:02X}"))
                    .collect::<Vec<_>>()
                    .join(" ");
                write!(f, "[{hex}]")
            }
        }
    }
}

/// One instruction. Jumps store absolute instruction indices, patched by
/// the compiler; `Call` targets user function chunks, `CallHost` targets
/// the host function table.
#[derive(Clone, Copy, Debug)]
pub enum Op {
    Const(u16),
    GetGlobal(u16),
    SetGlobal(u16),
    GetLocal(u8),
    SetLocal(u8),
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Neg,
    Not,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    Jump(u16),
    JumpIfFalse(u16),
    Call(u16, u8),
    CallHost(u16, u8),
    /// Runtime-resolved host extension call (external libraries, node
    /// builtins backed by host state): arg 0 is a constant-pool index of
    /// the function's name, arg 1 the argument count. The VM hands name
    /// and arguments to the host extension hook; an unclaimed name is a
    /// runtime error.
    CallExtern(u16, u8),
    GetIndex,
    SetIndex,
    Len,
    Pop,
    Return,
}

/// A compiled body of code: chunk 0 is the program main, the rest are
/// user functions and event handler bodies.
#[derive(Debug)]
pub struct Function {
    pub name: String,
    pub arity: usize,
    pub code: Vec<Op>,
    /// Sparse op-index -> source-line table (first op on each new line).
    /// Runtime errors resolve their line through this.
    pub lines: Vec<(u16, u32)>,
}

/// One compiled event handler: what triggers it and which chunk runs.
#[derive(Debug, Clone)]
pub struct Handler {
    pub kind: HandlerKind,
    pub chunk: u16,
}

/// The event kinds a node can react to. The node runtime (S2) feeds
/// these; the language only compiles the bodies.
#[derive(Clone, Debug, PartialEq)]
pub enum HandlerKind {
    Start,
    Message {
        id: u32,
    },
    Timer {
        period_ms: u64,
    },
    /// Fires once per `set_timer(name, ms)` arming.
    Oneshot {
        name: String,
    },
}

/// A compiled script, ready for the VM. Immutable after compilation --
/// VM state (globals, stacks) lives in [`Vm`].
#[derive(Debug)]
pub struct Script {
    pub constants: Vec<Value>,
    /// Global variable names, ordered by slot.
    pub globals: Vec<String>,
    pub functions: Vec<Function>,
    /// Event handlers in declaration order; the node runtime dispatches
    /// events against this table.
    pub handlers: Vec<Handler>,
    /// Host function names, ordered by id (mirrors [`HOST_FNS`] plus any
    /// future external registrations).
    pub host_fns: Vec<String>,
}

/// The host functions every script can call. The compiler resolves names
/// to ids here; the VM implements the behaviour with the same indices.
/// External simulation components register additional entries here
/// later -- the language and bytecode format do not change.
pub const HOST_FNS: &[(&str, usize, usize)] = &[
    // (name, min_args, max_args)
    ("print", 1, 16),
    ("send", 1, 9),
    ("now", 0, 0),
    ("sig", 2, 2),
    ("bytes", 1, 1),
    ("len", 1, 1),
    ("set_period", 1, 1),
    ("stop_timer", 0, 0),
    ("set_timer", 2, 2),
    ("cancel_timer", 1, 1),
    ("frame_byte", 1, 1),
    ("frame_dlc", 0, 0),
    ("random", 2, 2),
    ("srand", 1, 1),
    ("ramp", 3, 3),
    ("sine_wave", 3, 3),
    ("triangle", 3, 3),
    ("square", 3, 3),
    ("counter", 3, 3),
    ("bit_and", 2, 2),
    ("bit_or", 2, 2),
    ("bit_xor", 2, 2),
    ("bit_not", 1, 1),
    ("bit_shl", 2, 2),
    ("bit_shr", 2, 2),
    // Stimulus math: pure functions over floats, radians for trig.
    ("abs", 1, 1),
    ("floor", 1, 1),
    ("ceil", 1, 1),
    ("round", 1, 1),
    ("sin", 1, 1),
    ("cos", 1, 1),
    ("min", 2, 2),
    ("max", 2, 2),
    ("clamp", 3, 3),
];

/// One external simulation function, callable from any script once
/// registered under its script-visible name. Arguments arrive as
/// [`Value`]s; the function validates their count and types itself (the
/// pattern `set_sig`/`get_sig` follow). Returning `Ok(None)` pushes nil:
/// procedures that act rather than answer need no wrapper.
pub type ExternFn = fn(&[Value]) -> Result<Option<Value>, String>;

/// The in-process registry behind [`register_extern`] -- S4's seam: an
/// external simulation component (a battery model, a diagnostic stack,
/// a plant simulator) registers its script-callable entry points at
/// startup and every script in the process can call them. Dynamic
/// libraries will register through the same table.
fn extern_registry() -> &'static std::sync::Mutex<HashMap<String, ExternFn>> {
    static REG: std::sync::OnceLock<std::sync::Mutex<HashMap<String, ExternFn>>> =
        std::sync::OnceLock::new();
    REG.get_or_init(|| std::sync::Mutex::new(HashMap::new()))
}

/// Registers `name` as callable from scripts. Errors when the name
/// collides with a builtin; re-registering an existing extern name
/// replaces it, so a component can reload its models.
pub fn register_extern(name: &str, f: ExternFn) -> Result<(), String> {
    if HOST_FNS.iter().any(|(n, _, _)| *n == name) {
        return Err(format!("'{name}' is a builtin and cannot be replaced"));
    }
    extern_registry()
        .lock()
        .expect("extern registry poisoned")
        .insert(name.to_string(), f);
    Ok(())
}

/// The registered extern function for `name`, if any. Called by the VM
/// before the per-node [`HostExternFn`] hook.
pub(crate) fn extern_lookup(name: &str) -> Option<ExternFn> {
    extern_registry()
        .lock()
        .expect("extern registry poisoned")
        .get(name)
        .copied()
}

/// What the host publishes for a script to read between events: the bus
/// clock in seconds and the latest physical value of every decoded
/// signal on the node's channel, keyed by `(message id, signal name)`./// The node runtime refreshes this before each handler run.
#[derive(Clone, Debug, Default)]
pub struct HostInput {
    pub now_s: f64,
    pub signals: HashMap<(u32, String), f64>,
}

/// A compile-time error, positioned at the offending source line and,
/// when the offending token is known, its character column.
#[derive(Clone, Debug)]
pub struct ScriptError {
    pub line: u32,
    pub col: Option<u32>,
    pub msg: String,
}

impl std::fmt::Display for ScriptError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.col {
            Some(col) => write!(f, "line {}:{col}: {}", self.line, self.msg),
            None => write!(f, "line {}: {}", self.line, self.msg),
        }
    }
}

/// Compiles source text into a [`Script`].
pub fn compile(src: &str) -> Result<Script, ScriptError> {
    let tokens = lexer::lex(src)?;
    let program = parser::parse(tokens)?;
    compiler::compile(program)
}

/// Compiles and runs source text, returning everything `print`ed.
#[cfg(test)]
pub(crate) fn run_for_output(src: &str) -> Result<Vec<String>, String> {
    let script = compile(src).map_err(|e| e.to_string())?;
    let mut vm = vm::Vm::new(script);
    vm.run().map_err(|e| e.to_string())?;
    Ok(vm.output)
}

/// A stand-in external component: doubles its numeric argument.
fn regtest_double(args: &[Value]) -> Result<Option<Value>, String> {
    match args.first() {
        Some(Value::Int(n)) => Ok(Some(Value::Int(n * 2))),
        Some(Value::Float(f)) => Ok(Some(Value::Float(f * 2.0))),
        _ => Err("regtest_double(n) needs a number".into()),
    }
}

/// A stand-in external procedure: acts, answers nothing.
fn regtest_noop(_args: &[Value]) -> Result<Option<Value>, String> {
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use vm::Vm;

    fn out(src: &str) -> Vec<String> {
        run_for_output(src).expect("script should run")
    }

    fn err(src: &str) -> String {
        run_for_output(src).expect_err("script should fail")
    }

    #[test]
    fn arithmetic_precedence_and_mixed_numeric_promotion() {
        assert_eq!(out("print(1 + 2 * 3);"), ["7"]);
        assert_eq!(out("print((1 + 2) * 3);"), ["9"]);
        assert_eq!(out("print(7 % 3);"), ["1"]);
        // Int/Int stays exact; anything mixed promotes to float.
        assert_eq!(out("print(1 / 2);"), ["0"]);
        assert_eq!(out("print(1.0 / 2);"), ["0.5"]);
        assert_eq!(out("print(1 + 0.5);"), ["1.5"]);
        assert_eq!(out("print(-2 * 3.5);"), ["-7.0"]);
    }

    #[test]
    fn variables_and_strings() {
        let src = r#"
            let base = 800;
            let name = "node";
            print(name, "starts at", base);
            base = base + 100;
            print(base);
            print("tab\tquote\"done");
        "#;
        assert_eq!(out(src), ["node starts at 800", "900", "tab\tquote\"done"]);
    }

    #[test]
    fn control_flow_if_while_for() {
        assert_eq!(
            out("if (1 < 2) { print(\"lt\"); } else { print(\"ge\"); }"),
            ["lt"]
        );
        let src = r#"
            let sum = 0;
            let i = 1;
            while (i <= 4) {
                sum = sum + i;
                i = i + 1;
            }
            print(sum);
            for (let j = 0; j < 3; j = j + 1) {
                print("j", j);
            }
        "#;
        assert_eq!(out(src), ["10", "j 0", "j 1", "j 2"]);
    }

    #[test]
    fn functions_globals_and_recursion() {
        let src = r#"
            let calls = 0;
            fn add(a, b) {
                calls = calls + 1;
                return a + b;
            }
            fn fib(n) {
                if (n < 2) { return n; }
                return fib(n - 1) + fib(n - 2);
            }
            print(add(2, 3));
            print(fib(10));
            print(calls);
        "#;
        assert_eq!(out(src), ["5", "55", "1"]);
    }

    #[test]
    fn short_circuit_logic_skips_the_right_side() {
        let src = r#"
            let hits = 0;
            fn bump() {
                hits = hits + 1;
                return true;
            }
            if (false && bump()) { print("no"); }
            if (true || bump()) { print("yes"); }
            print(hits);
            print(!false, 1 == 1.0, 2 != 3, 1 <= 1, 2 > 1);
        "#;
        assert_eq!(out(src), ["yes", "0", "true true true true true"]);
    }

    #[test]
    fn compile_errors_name_the_line() {
        let e = err("let x = ;");
        assert!(e.contains("line 1"), "{e}");
        let e = err("print(nope);");
        assert!(e.contains("unknown"), "{e}");
        let e = err("fn f() { return 1; } return 2;");
        assert!(e.contains("outside"), "{e}");
    }

    #[test]
    fn compile_errors_name_the_column() {
        let e = compile("let ok = 1;\n    zz = ok;")
            .unwrap_err()
            .to_string();
        assert!(e.contains("line 2:5"), "{e}");
        let e = compile("on message 0x800 { }").unwrap_err().to_string();
        assert!(e.contains("line 1:12"), "{e}");
    }

    /// S4: an external component registers at startup, every script
    /// calls it -- no per-node hook needed, no bytecode change.
    #[test]
    fn registered_externs_are_callable_from_scripts() {
        register_extern("regtest_double", regtest_double).expect("fresh name");
        assert_eq!(out("print(regtest_double(21));"), ["42"]);
        assert_eq!(out("print(regtest_double(1.5));"), ["3.0"]);

        // A procedure that returns nothing lands as nil...
        register_extern("regtest_noop", regtest_noop).expect("fresh name");
        let e = err("let r = regtest_noop(); print(r + 1);");
        assert!(e.contains("nil"), "{e}");
        // ...and an unclaimed name is a runtime error naming it.
        let e = err("print(never_registered());");
        assert!(e.contains("never_registered"), "{e}");
    }

    #[test]
    fn builtin_names_are_reserved_for_the_language() {
        assert!(
            register_extern("print", regtest_noop)
                .err()
                .is_some_and(|e| e.contains("builtin"))
        );
    }

    #[test]
    fn runtime_errors_stop_the_vm() {
        assert!(err("print(1 / 0);").contains("zero"));
        assert!(err("print(1 % 0);").contains("zero"));
        assert!(err("if (1) { print(1); }").contains("bool"));
    }

    #[test]
    fn language_reference_example_compiles_and_runs() {
        // The doc-comment example at the top of this module, verbatim:
        // if this drifts, the reference is lying.
        let src = r#"
            let base = 800;

            fn limit(v) {
                if (v > 5000) { return 5000; }
                return v;
            }

            on start { print("node up"); }
        "#;
        let script = compile(src).unwrap();
        assert_eq!(script.handlers.len(), 1, "one on start handler");
        let start_chunk = script.handlers[0].chunk;
        let mut vm = Vm::new(script);
        vm.run().unwrap();
        vm.run_handler(start_chunk).unwrap();
        assert_eq!(vm.output, ["node up"]);
    }

    #[test]
    fn break_exits_while_and_for_loops() {
        assert_eq!(
            out(r#"
                    let n = 0;
                    while (true) {
                        n = n + 1;
                        if (n >= 3) { break; }
                    }
                    print(n);
                "#),
            ["3"]
        );
        assert_eq!(
            out(r#"
                    let s = 0;
                    for (let i = 0; i < 100; i = i + 1) {
                        s = s + i;
                        if (s > 10) { break; }
                    }
                    print(s);
                "#),
            ["15"]
        );
    }

    #[test]
    fn continue_skips_to_the_next_iteration() {
        // While: continue re-tests the condition.
        assert_eq!(
            out(r#"
                    let n = 0;
                    let evens = 0;
                    while (n < 10) {
                        n = n + 1;
                        if (n % 2 == 1) { continue; }
                        evens = evens + 1;
                    }
                    print(evens);
                "#),
            ["5"]
        );
        // For: continue still runs the step, or the loop would hang.
        assert_eq!(
            out(r#"
                    let s = 0;
                    for (let i = 0; i < 10; i = i + 1) {
                        if (i % 2 == 0) { continue; }
                        s = s + i;
                    }
                    print(s);
                "#),
            ["25"]
        );
        // Nested loops: continue binds to the innermost one.
        assert_eq!(
            out(r#"
                    let hits = 0;
                    for (let i = 0; i < 3; i = i + 1) {
                        for (let j = 0; j < 3; j = j + 1) {
                            if (j == 1) { continue; }
                            hits = hits + 1;
                        }
                    }
                    print(hits);
                "#),
            ["6"]
        );
    }

    #[test]
    fn break_and_continue_outside_a_loop_fail_to_compile() {
        assert!(
            compile("break;")
                .unwrap_err()
                .to_string()
                .contains("'break'")
        );
        assert!(
            compile("continue;")
                .unwrap_err()
                .to_string()
                .contains("'continue'")
        );
    }

    #[test]
    fn runtime_errors_name_the_source_line() {
        let src = "let a = 1;\nlet b = 2;\nprint(a / (b - b));";
        let e = err(src);
        assert!(e.contains("line 3"), "{e}");
    }

    #[test]
    fn the_budget_stops_runaway_loops() {
        let script = compile("while (true) { }").unwrap();
        let mut vm = Vm::new(script).with_budget(1_000);
        let e = vm.run().expect_err("budget must stop the loop");
        assert!(e.to_string().contains("budget"), "{e}");
    }

    #[test]
    fn deep_recursion_hits_the_frame_cap_not_the_host_stack() {
        let script = compile("fn f(n) { return f(n + 1); } print(f(0));").unwrap();
        let mut vm = Vm::new(script);
        let e = vm.run().expect_err("recursion must be capped");
        assert!(e.to_string().contains("recursion"), "{e}");
    }

    #[test]
    fn event_handlers_compile_into_a_table() {
        let script = compile(
            r#"
                on start { print("start"); }
                on message 0x100 { print("eng"); }
                on timer 100 { print("tick"); }
            "#,
        )
        .unwrap();
        assert_eq!(script.handlers.len(), 3);
        assert_eq!(script.handlers[0].kind, HandlerKind::Start);
        assert_eq!(script.handlers[1].kind, HandlerKind::Message { id: 0x100 });
        assert_eq!(
            script.handlers[2].kind,
            HandlerKind::Timer { period_ms: 100 }
        );
        // Each body is its own chunk (main is chunk 0, handlers follow in
        // declaration order), invokable against the live state.
        let mut vm = Vm::new(script);
        for chunk in 1..=3u16 {
            vm.run_handler(chunk).unwrap();
        }
        assert_eq!(vm.output, ["start", "eng", "tick"]);
    }

    #[test]
    fn handler_bodies_run_against_shared_globals() {
        let script = compile(
            r#"
                let seen = 0;
                on message 0x200 { seen = seen + 1; print("seen", seen); }
            "#,
        )
        .unwrap();
        let chunk = script_chunk(&script, HandlerKind::Message { id: 0x200 });
        let mut vm = Vm::new(script);
        // The node runtime order: main once (globals initialize), then
        // handlers per event.
        vm.run().unwrap();
        vm.run_handler(chunk).unwrap();
        vm.run_handler(chunk).unwrap();
        assert_eq!(vm.output, ["seen 1", "seen 2"]);
    }

    /// Finds the compiled chunk of the first handler matching `kind`.
    fn script_chunk(script: &Script, kind: HandlerKind) -> u16 {
        script
            .handlers
            .iter()
            .find(|h| h.kind == kind)
            .map(|h| h.chunk)
            .expect("handler present")
    }

    #[test]
    fn handler_sanity_is_enforced() {
        assert!(
            compile("on start { } on start { }")
                .unwrap_err()
                .to_string()
                .contains("duplicate 'on start'")
        );
        assert!(
            compile("on message 0x100 { } on message 0x100 { }")
                .unwrap_err()
                .to_string()
                .contains("duplicate")
        );
        assert!(
            compile("on message 0x800 { }")
                .unwrap_err()
                .to_string()
                .contains("11-bit")
        );
        assert!(
            compile("on timer 0 { }")
                .unwrap_err()
                .to_string()
                .contains("positive")
        );
        assert!(
            compile("on timer \"a\" { } on timer \"a\" { }")
                .unwrap_err()
                .to_string()
                .contains("duplicate")
        );
        assert!(
            compile("fn f() { on start { } }")
                .unwrap_err()
                .to_string()
                .contains("top level")
        );
    }

    #[test]
    fn hex_ids_lex_into_handler_kinds() {
        let script = compile("on message 0x7FF { }").unwrap();
        assert_eq!(
            script.handlers[0].kind,
            HandlerKind::Message { id: 0x7FF },
            "the top of the standard id range"
        );
    }
}
