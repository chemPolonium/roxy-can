//! The stack VM: executes a compiled [`Script`]. Frames are cheap and
//! capped; every executed instruction decrements a budget so a runaway
//! loop (S2: a runaway node callback) can never wedge the host.

use super::{HostInput, Op, Script, Value};

/// Where `print` output goes. The node runtime will plug its log ring in
/// here; tests collect into a `Vec<String>`.
pub trait OutSink {
    fn write_line(&mut self, text: String);
}

impl OutSink for Vec<String> {
    fn write_line(&mut self, text: String) {
        self.push(text);
    }
}

#[derive(Debug)]
pub struct VmError(pub String);

impl std::fmt::Display for VmError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

struct Frame {
    chunk: usize,
    ip: usize,
    /// Stack index of this frame's local 0.
    base: usize,
}

const MAX_FRAMES: usize = 256;
const DEFAULT_BUDGET: u64 = 10_000_000;

pub struct Vm {
    script: Script,
    globals: Vec<Value>,
    stack: Vec<Value>,
    frames: Vec<Frame>,
    budget: u64,
    steps: u64,
    /// Lines produced by `print`, in order.
    pub output: Vec<String>,
    /// Messages queued by `send(id, ...)`: identifier plus up-to-8 payload
    /// bytes. The host drains this after each handler run and decides
    /// what "send" means (for a CAN node: a frame onto the bus). An id
    /// above 0x7FF flags the frame extended.
    pub outbox: Vec<(u32, Vec<u8>)>,
    /// Host-published read values: the clock and latest signal values.
    /// The node runtime refreshes this before each handler run; `now()`
    /// and `sig()` read it.
    pub host_input: HostInput,
}

impl Vm {
    pub fn new(script: Script) -> Self {
        let globals = vec![Value::Nil; script.globals.len()];
        Self {
            script,
            globals,
            stack: Vec::new(),
            frames: Vec::new(),
            budget: DEFAULT_BUDGET,
            steps: 0,
            output: Vec::new(),
            outbox: Vec::new(),
            host_input: HostInput::default(),
        }
    }

    pub fn with_budget(mut self, budget: u64) -> Self {
        self.budget = budget;
        self
    }

    /// Re-arms the instruction budget for one callback: every handler run
    /// gets the full allowance, so a chatty node cannot starve its own
    /// later events.
    pub fn reset_budget(&mut self, budget: u64) {
        self.budget = budget;
        self.steps = 0;
    }

    /// Runs the script main to completion. Re-running after an error is
    /// not supported (the state is left as the error hit it).
    pub fn run(&mut self) -> Result<(), VmError> {
        self.run_from(0)
    }

    /// Runs one handler chunk against the current VM state: globals keep
    /// their values between events, the value stack starts fresh. This is
    /// how the node runtime will deliver events in S2.
    pub fn run_handler(&mut self, chunk: u16) -> Result<(), VmError> {
        self.run_from(chunk as usize)
    }

    fn run_from(&mut self, chunk: usize) -> Result<(), VmError> {
        self.frames = vec![Frame {
            chunk,
            ip: 0,
            base: 0,
        }];
        loop {
            self.steps += 1;
            if self.steps > self.budget {
                return Err(VmError("instruction budget exceeded".into()));
            }
            let frame = self.frames.last().expect("run without a frame");
            let Some(op) = self.script.functions[frame.chunk].code.get(frame.ip) else {
                return Err(VmError("code ran off its chunk".into()));
            };
            let op = *op;
            self.frames.last_mut().expect("frame").ip += 1;
            match op {
                Op::Const(c) => {
                    let v = self.script.constants[c as usize].clone();
                    self.stack.push(v);
                }
                Op::GetGlobal(g) => {
                    let v = self
                        .globals
                        .get(g as usize)
                        .cloned()
                        .ok_or_else(|| VmError(format!("bad global slot {g}")))?;
                    self.stack.push(v);
                }
                Op::SetGlobal(g) => {
                    let v = self.pop()?;
                    if g as usize >= self.globals.len() {
                        self.globals.resize(g as usize + 1, Value::Nil);
                    }
                    self.globals[g as usize] = v;
                }
                Op::GetLocal(n) => {
                    let base = self.frames.last().expect("frame").base;
                    let v = self
                        .stack
                        .get(base + n as usize)
                        .cloned()
                        .ok_or_else(|| VmError("read of an uninitialised local".into()))?;
                    self.stack.push(v);
                }
                Op::SetLocal(n) => {
                    let v = self.pop()?;
                    let base = self.frames.last().expect("frame").base;
                    let at = base + n as usize;
                    let len = self.stack.len();
                    if at >= len {
                        return Err(VmError("local slot out of range".into()));
                    }
                    self.stack[at] = v;
                }
                Op::Add => self.binary(|a, b| arith(a, b, Arith::Add))?,
                Op::Sub => self.binary(|a, b| arith(a, b, Arith::Sub))?,
                Op::Mul => self.binary(|a, b| arith(a, b, Arith::Mul))?,
                Op::Div => self.binary(|a, b| arith(a, b, Arith::Div))?,
                Op::Mod => self.binary(|a, b| arith(a, b, Arith::Mod))?,
                Op::Neg => {
                    let v = self.pop()?;
                    match v {
                        Value::Int(n) => {
                            let r = n
                                .checked_neg()
                                .ok_or_else(|| VmError("integer overflow".into()))?;
                            self.stack.push(Value::Int(r));
                        }
                        Value::Float(x) => self.stack.push(Value::Float(-x)),
                        other => return Err(VmError(format!("cannot negate {}", kind(&other)))),
                    }
                }
                Op::Not => {
                    let v = self.pop()?;
                    match v {
                        Value::Bool(b) => self.stack.push(Value::Bool(!b)),
                        other => {
                            return Err(VmError(format!("'!' needs a bool, got {}", kind(&other))));
                        }
                    }
                }
                Op::Eq => {
                    let (b, a) = self.pop2()?;
                    self.stack.push(Value::Bool(values_eq(&a, &b)));
                }
                Op::Ne => {
                    let (b, a) = self.pop2()?;
                    self.stack.push(Value::Bool(!values_eq(&a, &b)));
                }
                Op::Lt => self.compare(|o| o == std::cmp::Ordering::Less)?,
                Op::Le => self.compare(|o| o != std::cmp::Ordering::Greater)?,
                Op::Gt => self.compare(|o| o == std::cmp::Ordering::Greater)?,
                Op::Ge => self.compare(|o| o != std::cmp::Ordering::Less)?,
                Op::Jump(t) => {
                    self.frames.last_mut().expect("frame").ip = t as usize;
                }
                Op::JumpIfFalse(t) => {
                    let v = self.pop()?;
                    match v {
                        Value::Bool(false) => {
                            self.frames.last_mut().expect("frame").ip = t as usize;
                        }
                        Value::Bool(true) => {}
                        other => {
                            return Err(VmError(format!(
                                "condition must be a bool, got {}",
                                kind(&other)
                            )));
                        }
                    }
                }
                Op::Call(idx, argc) => {
                    if self.frames.len() >= MAX_FRAMES {
                        return Err(VmError("recursion too deep".into()));
                    }
                    let base = self.stack.len() - argc as usize;
                    self.frames.push(Frame {
                        chunk: idx as usize,
                        ip: 0,
                        base,
                    });
                }
                Op::CallHost(id, argc) => self.call_host(id as usize, argc as usize)?,
                Op::Pop => {
                    self.pop()?;
                }
                Op::Return => {
                    let rv = self.pop()?;
                    let base = self.frames.last().expect("frame").base;
                    self.stack.truncate(base);
                    self.stack.push(rv);
                    self.frames.pop();
                    if self.frames.is_empty() {
                        self.stack.pop();
                        return Ok(());
                    }
                }
            }
        }
    }

    fn pop(&mut self) -> Result<Value, VmError> {
        self.stack
            .pop()
            .ok_or_else(|| VmError("stack empty".into()))
    }

    fn pop2(&mut self) -> Result<(Value, Value), VmError> {
        let b = self.pop()?;
        let a = self.pop()?;
        Ok((a, b))
    }

    fn binary(&mut self, f: impl Fn(Value, Value) -> Result<Value, String>) -> Result<(), VmError> {
        let (a, b) = self.pop2()?;
        let v = f(a, b).map_err(VmError)?;
        self.stack.push(v);
        Ok(())
    }

    fn compare(&mut self, want: impl Fn(std::cmp::Ordering) -> bool) -> Result<(), VmError> {
        let (a, b) = self.pop2()?;
        let ord = numeric_order(&a, &b)
            .ok_or_else(|| VmError(format!("cannot order {} and {}", kind(&a), kind(&b))))?;
        self.stack.push(Value::Bool(want(ord)));
        Ok(())
    }

    fn call_host(&mut self, id: usize, argc: usize) -> Result<(), VmError> {
        if id >= self.script.host_fns.len() {
            return Err(VmError(format!("unknown host function {id}")));
        }
        let name = self.script.host_fns[id].clone();
        if self.stack.len() < argc {
            return Err(VmError("stack underflow in host call".into()));
        }
        let args: Vec<Value> = self.stack.split_off(self.stack.len() - argc);
        match name.as_str() {
            "print" => {
                let line = args
                    .iter()
                    .map(|v| v.to_string())
                    .collect::<Vec<_>>()
                    .join(" ");
                self.output.write_line(line);
            }
            "now" => {
                // Producing calls must return before the tail push below:
                // the pushed value IS the call's result.
                self.stack.push(Value::Float(self.host_input.now_s));
                return Ok(());
            }
            "sig" => {
                // sig(id, "Name"): the latest physical value the host
                // published for that signal on this node's channel.
                let (Value::Int(id), Value::Str(sig)) = (&args[0], &args[1]) else {
                    return Err(VmError(
                        "sig(id, \"Name\") needs an int and a string".into(),
                    ));
                };
                match self.host_input.signals.get(&(*id as u32, sig.clone())) {
                    Some(v) => self.stack.push(Value::Float(*v)),
                    None => {
                        return Err(VmError(format!(
                            "sig: no value for {id:#x} {sig:?} (not seen yet)"
                        )));
                    }
                }
                return Ok(());
            }
            "send" => {
                // send(id, b0, b1, ...): one classic-frame payload. The
                // host decides what "send" means; here it only lands in
                // the outbox, well-formed or not. An id above 0x7FF
                // travels as an extended frame.
                let id = match &args[0] {
                    Value::Int(n) if (0..=0x1FF_FFFF).contains(n) => *n as u32,
                    other => {
                        return Err(VmError(format!(
                            "send: id {} out of range (0..0x1FFFFFFF)",
                            kind(other)
                        )));
                    }
                };
                if args.len() - 1 > 8 {
                    return Err(VmError("send: at most 8 data bytes".into()));
                }
                let mut data = Vec::with_capacity(args.len() - 1);
                for b in &args[1..] {
                    match b {
                        Value::Int(n) if (0..=255).contains(n) => data.push(*n as u8),
                        other => {
                            return Err(VmError(format!(
                                "send: data byte must be 0..255, got {}",
                                kind(other)
                            )));
                        }
                    }
                }
                self.outbox.push((id, data));
            }
            other => return Err(VmError(format!("host function '{other}' not implemented"))),
        }
        self.stack.push(Value::Nil);
        Ok(())
    }
}

fn kind(v: &Value) -> &'static str {
    match v {
        Value::Nil => "nil",
        Value::Bool(_) => "bool",
        Value::Int(_) => "int",
        Value::Float(_) => "float",
        Value::Str(_) => "string",
    }
}

fn values_eq(a: &Value, b: &Value) -> bool {
    match (a, b) {
        // Ints and floats compare across their types: 1 == 1.0.
        (Value::Int(_) | Value::Float(_), Value::Int(_) | Value::Float(_)) => {
            as_float(a) == as_float(b)
        }
        _ => a == b,
    }
}

fn numeric_order(a: &Value, b: &Value) -> Option<std::cmp::Ordering> {
    use std::cmp::Ordering;
    let (x, y) = match (a, b) {
        (Value::Int(x), Value::Int(y)) => (*x as f64, *y as f64),
        (Value::Int(x), Value::Float(y)) => (*x as f64, *y),
        (Value::Float(x), Value::Int(y)) => (*x, *y as f64),
        (Value::Float(x), Value::Float(y)) => (*x, *y),
        _ => return None,
    };
    x.partial_cmp(&y).or(match (x.is_nan(), y.is_nan()) {
        (true, true) => Some(Ordering::Equal),
        (true, false) => Some(Ordering::Greater),
        (false, true) => Some(Ordering::Less),
        _ => None,
    })
}

enum Arith {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
}

fn as_float(x: &Value) -> f64 {
    match x {
        Value::Int(n) => *n as f64,
        Value::Float(f) => *f,
        _ => f64::NAN,
    }
}

fn is_num(v: &Value) -> bool {
    matches!(v, Value::Int(_) | Value::Float(_))
}

fn arith(a: Value, b: Value, op: Arith) -> Result<Value, String> {
    use Value::{Float, Int};
    let sign = match op {
        Arith::Add => "+",
        Arith::Sub => "-",
        Arith::Mul => "*",
        Arith::Div => "/",
        Arith::Mod => "%",
    };
    // String concatenation on '+' with either side a string: the other
    // value renders as it would in print, so log lines read naturally.
    if matches!(op, Arith::Add) && (matches!(a, Value::Str(_)) || matches!(b, Value::Str(_))) {
        let mut s = match a {
            Value::Str(x) => x,
            other => other.to_string(),
        };
        match b {
            Value::Str(y) => s.push_str(&y),
            other => s.push_str(&other.to_string()),
        }
        return Ok(Value::Str(s));
    }
    if !is_num(&a) || !is_num(&b) {
        return Err(format!(
            "cannot use {} in arithmetic ('{sign}')",
            if is_num(&a) { kind(&b) } else { kind(&a) }
        ));
    }
    if let (Int(x), Int(y)) = (&a, &b) {
        let r = match op {
            Arith::Add => x.checked_add(*y),
            Arith::Sub => x.checked_sub(*y),
            Arith::Mul => x.checked_mul(*y),
            Arith::Div => {
                if *y == 0 {
                    return Err("integer division by zero".into());
                }
                x.checked_div(*y)
            }
            Arith::Mod => {
                if *y == 0 {
                    return Err("integer modulo by zero".into());
                }
                x.checked_rem(*y)
            }
        };
        return r.map(Int).ok_or_else(|| "integer overflow".to_string());
    }
    let (x, y) = (as_float(&a), as_float(&b));
    let r = match op {
        Arith::Add => x + y,
        Arith::Sub => x - y,
        Arith::Mul => x * y,
        Arith::Div => x / y,
        Arith::Mod => x % y,
    };
    Ok(Float(r))
}

#[cfg(test)]
mod tests {
    use super::super::compile;

    fn out(src: &str) -> Vec<String> {
        let script = compile(src).unwrap();
        let mut vm = super::Vm::new(script);
        vm.run().unwrap();
        vm.output
    }

    #[test]
    fn print_joins_arguments_with_spaces() {
        assert_eq!(out("print(\"a\", 1, 2.5, true);"), ["a 1 2.5 true"]);
    }

    #[test]
    fn locals_scopes_and_globals_interact() {
        let src = r#"
            let g = 10;
            fn f() {
                let g = 5;
                g = g + 1;
                return g;
            }
            print(f());
            print(g);
        "#;
        assert_eq!(out(src), ["6", "10"]);
    }

    #[test]
    fn nil_in_arithmetic_is_a_runtime_error() {
        let script = compile("fn f() { return; } print(f() + 1);").unwrap();
        let mut vm = super::Vm::new(script);
        let e = vm.run().unwrap_err();
        assert!(e.to_string().contains("nil"), "{e}");
    }
}
