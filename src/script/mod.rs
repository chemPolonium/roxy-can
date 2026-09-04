//! The node scripting language: a small C-flavoured source text compiled
//! to bytecode and run by a stack VM. Nothing here touches the bus --
//! scripts are pure code plus host calls; the node runtime (S2) wires
//! them into the core loop.
//!
//! Architecture seam for external libraries: host functions live in one
//! table ([`HOST_FNS`]) that the compiler resolves to `Op::CallHost(id)`
//! and the VM dispatches by the same index. External simulation
//! components will register extra entries in that table; nothing else in
//! the language changes.

// S2 (the node runtime) is what calls into this module from the product
// path; until it lands the module is reachable only from tests, which is
// exactly why the dead-code sweep must stay quiet here.
#![allow(dead_code)]

mod compiler;
mod lexer;
mod parser;
mod vm;

/// A runtime value of the script language.
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Nil,
    Bool(bool),
    Int(i64),
    Float(f64),
    Str(String),
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
}

/// One compiled event handler: what triggers it and which chunk runs.
#[derive(Debug)]
pub struct Handler {
    pub kind: HandlerKind,
    pub chunk: u16,
}

/// The event kinds a node can react to. The node runtime (S2) feeds
/// these; the language only compiles the bodies.
#[derive(Clone, Debug, PartialEq)]
pub enum HandlerKind {
    Start,
    Message { id: u32 },
    Timer { period_ms: u64 },
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
];

/// A compile-time error, positioned at the offending source line.
#[derive(Clone, Debug)]
pub struct ScriptError {
    pub line: u32,
    pub msg: String,
}

impl std::fmt::Display for ScriptError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "line {}: {}", self.line, self.msg)
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
    fn runtime_errors_stop_the_vm() {
        assert!(err("print(1 / 0);").contains("zero"));
        assert!(err("print(1 % 0);").contains("zero"));
        assert!(err("if (1) { print(1); }").contains("bool"));
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
