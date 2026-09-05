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

/// A host-registered extension builtin: name-driven, argument list in,
/// optional result value out. `None` means "not one of mine" (the VM
/// then reports an unknown function). This is the seam external
/// simulation components plug into (S4).
pub type HostExternFn = Box<dyn FnMut(&str, &[Value]) -> Result<Option<Value>, String> + Send>;

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
    /// Extension point for host-registered builtins (see
    /// [`HostExternFn`]). Called when the builtin table has no entry for
    /// a called name.
    pub host_extern: Option<HostExternFn>,
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
            host_extern: None,
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
                Op::GetIndex => {
                    // (container, index) on the stack: byte buffers only.
                    let idx = self.pop()?;
                    let container = self.pop()?;
                    let Value::Bytes(b) = container else {
                        return Err(VmError("indexing needs a byte buffer".into()));
                    };
                    let Value::Int(i) = idx else {
                        return Err(VmError("index must be an int".into()));
                    };
                    let b = b.lock().expect("buffer poisoned");
                    let i = usize::try_from(i).unwrap_or(usize::MAX);
                    let byte = *b
                        .get(i)
                        .ok_or_else(|| VmError(format!("index {i} outside 0..{}", b.len())))?;
                    self.stack.push(Value::Int(byte as i64));
                }
                Op::SetIndex => {
                    // (container, index, value) on the stack.
                    let value = self.pop()?;
                    let idx = self.pop()?;
                    let container = self.pop()?;
                    let Value::Bytes(b) = container else {
                        return Err(VmError("indexing needs a byte buffer".into()));
                    };
                    let Value::Int(i) = idx else {
                        return Err(VmError("index must be an int".into()));
                    };
                    let Value::Int(byte) = value else {
                        return Err(VmError("buffer elements must be ints".into()));
                    };
                    if !(0..=255).contains(&byte) {
                        return Err(VmError(format!(
                            "buffer elements must be 0..255, got {byte}"
                        )));
                    }
                    let mut b = b.lock().expect("buffer poisoned");
                    let i = usize::try_from(i).unwrap_or(usize::MAX);
                    if i >= b.len() {
                        return Err(VmError(format!("index {i} outside 0..{}", b.len())));
                    }
                    b[i] = byte as u8;
                }
                Op::Len => {
                    let v = self.pop()?;
                    let n = match v {
                        Value::Bytes(b) => b.lock().expect("buffer poisoned").len(),
                        Value::Str(s) => s.chars().count(),
                        other => {
                            return Err(VmError(format!(
                                "len needs a buffer or string, got {}",
                                kind(&other)
                            )));
                        }
                    };
                    self.stack.push(Value::Int(n as i64));
                }
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
                Op::CallExtern(c, argc) => {
                    // Runtime-resolved extension call: name from the
                    // constant pool, dispatched through the host's extern
                    // hook. Unclaimed names are runtime errors -- the
                    // compiler cannot know what the host registers.
                    let Value::Str(name) = self.script.constants[c as usize].clone() else {
                        return Err(VmError("extern name constant must be a string".into()));
                    };
                    if self.stack.len() < argc as usize {
                        return Err(VmError("stack underflow in extern call".into()));
                    }
                    let args: Vec<Value> = self.stack.split_off(self.stack.len() - argc as usize);
                    let result = match self.host_extern.as_mut() {
                        Some(f) => f(&name, &args),
                        None => Ok(None),
                    };
                    match result {
                        Ok(Some(v)) => self.stack.push(v),
                        Ok(None) => return Err(VmError(format!("unknown function '{name}'"))),
                        Err(e) => return Err(VmError(e)),
                    }
                }
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
                // send(id, b0, b1, ...) or send(id, buf): the payload is
                // either literal bytes or one byte buffer. The host
                // decides what "send" means; here it only lands in the
                // outbox. An id above 0x7FF travels extended.
                let id = match &args[0] {
                    Value::Int(n) if (0..=0x1FF_FFFF).contains(n) => *n as u32,
                    other => {
                        return Err(VmError(format!(
                            "send: id {} out of range (0..0x1FFFFFFF)",
                            kind(other)
                        )));
                    }
                };
                let data = if args.len() == 2 && matches!(args[1], Value::Bytes(_)) {
                    // Single-buffer form: the buffer IS the payload.
                    match &args[1] {
                        Value::Bytes(b) => b.lock().expect("buffer poisoned").clone(),
                        _ => unreachable!(),
                    }
                } else {
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
                    data
                };
                self.outbox.push((id, data));
            }
            // Stimulus math: floats in and out; `now()` reads the same
            // clock, so e.g. sin(now()) animates with the bus.
            "abs" | "floor" | "ceil" | "round" | "sin" | "cos" => {
                let x = as_float(&args[0]);
                if !is_num(&args[0]) {
                    return Err(VmError(format!(
                        "{name} needs a number, got {}",
                        kind(&args[0])
                    )));
                }
                let r = match name.as_str() {
                    "abs" => x.abs(),
                    "floor" => x.floor(),
                    "ceil" => x.ceil(),
                    "round" => x.round(),
                    "sin" => x.sin(),
                    _ => x.cos(),
                };
                self.stack.push(Value::Float(r));
                return Ok(());
            }
            "min" | "max" => {
                if !is_num(&args[0]) || !is_num(&args[1]) {
                    return Err(VmError(format!(
                        "{name} needs numbers, got {} and {}",
                        kind(&args[0]),
                        kind(&args[1])
                    )));
                }
                let (x, y) = (as_float(&args[0]), as_float(&args[1]));
                let r = if name == "min" { x.min(y) } else { x.max(y) };
                self.stack.push(Value::Float(r));
                return Ok(());
            }
            "clamp" => {
                // (v, lo, hi), in source order on the stack.
                if !is_num(&args[0]) || !is_num(&args[1]) || !is_num(&args[2]) {
                    return Err(VmError(format!(
                        "clamp needs numbers, got {}, {}, {}",
                        kind(&args[0]),
                        kind(&args[1]),
                        kind(&args[2])
                    )));
                }
                let (v, lo, hi) = (as_float(&args[0]), as_float(&args[1]), as_float(&args[2]));
                if hi < lo {
                    return Err(VmError(format!("clamp needs lo <= hi, got {lo}..{hi}")));
                }
                self.stack.push(Value::Float(v.clamp(lo, hi)));
                return Ok(());
            }
            "bytes" => {
                // bytes(n): a zero-filled byte buffer. Buffers carry
                // reference semantics -- the value pushed shares the
                // stored buffer.
                let Value::Int(n) = &args[0] else {
                    return Err(VmError("bytes(n) needs an int".into()));
                };
                if !(0..=65_536).contains(n) {
                    return Err(VmError(format!("bytes: size {n} out of 0..65536")));
                }
                self.stack
                    .push(Value::Bytes(std::sync::Arc::new(std::sync::Mutex::new(
                        vec![0u8; *n as usize],
                    ))));
                return Ok(());
            }
            "len" => match &args[0] {
                Value::Bytes(b) => {
                    let n = b.lock().expect("buffer poisoned").len() as i64;
                    self.stack.push(Value::Int(n));
                    return Ok(());
                }
                Value::Str(s) => {
                    let n = s.chars().count() as i64;
                    self.stack.push(Value::Int(n));
                    return Ok(());
                }
                other => {
                    return Err(VmError(format!(
                        "len needs a buffer or string, got {}",
                        kind(other)
                    )));
                }
            },
            other => {
                // Not a builtin: the host's extension hook (external
                // simulation components, node-runtime functions) gets the
                // call next. Some(value) pushes the result, None means
                // the name is unknown to the host too.
                if let Some(f) = self.host_extern.as_mut() {
                    match f(other, &args).map_err(VmError)? {
                        Some(v) => {
                            self.stack.push(v);
                            return Ok(());
                        }
                        None => {
                            return Err(VmError(format!(
                                "host function '{other}' not implemented"
                            )));
                        }
                    }
                }
                return Err(VmError(format!("host function '{other}' not implemented")));
            }
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
        Value::Bytes(_) => "buffer",
    }
}

fn values_eq(a: &Value, b: &Value) -> bool {
    match (a, b) {
        // Ints and floats compare across their types: 1 == 1.0.
        (Value::Int(_) | Value::Float(_), Value::Int(_) | Value::Float(_)) => {
            as_float(a) == as_float(b)
        }
        (Value::Bytes(x), Value::Bytes(y)) => {
            *x.lock().expect("buffer poisoned") == *y.lock().expect("buffer poisoned")
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
    use super::*;

    fn out(src: &str) -> Vec<String> {
        let script = compile(src).unwrap();
        let mut vm = super::Vm::new(script);
        vm.run()
            .unwrap_or_else(|e| panic!("script '{src}' failed: {e}"));
        vm.output
    }

    #[test]
    fn print_joins_arguments_with_spaces() {
        assert_eq!(out("print(\"a\", 1, 2.5, true);"), ["a 1 2.5 true"]);
    }

    #[test]
    fn stimulus_math_builtins_work_and_string_plus_concatenates() {
        assert_eq!(out("print(abs(0 - 2.5));"), ["2.5"]);
        assert_eq!(
            out("print(min(3, 1.5), max(3, 1.5));"),
            ["1.5 3.0"],
            "math builtins answer in floats"
        );
        // All math builtins answer in floats (the honest type for a
        // stimulus language; printing keeps the ".0").
        assert_eq!(
            out("print(floor(1.9), ceil(1.1), round(1.5));"),
            ["1.0 2.0 2.0"]
        );
        assert_eq!(out("print(clamp(3500, 0, 3000));"), ["3000.0"]);
        assert_eq!(out("print(\"rpm: \" + 1200);"), ["rpm: 1200"]);
        assert_eq!(out("print(1 + \" rpm\");"), ["1 rpm"]);
        assert_eq!(out("print(sin(0));"), ["0.0"]);
    }

    #[test]
    fn math_builtins_reject_non_numbers() {
        let script = compile("print(abs(\"x\"));").unwrap();
        let mut vm = Vm::new(script);
        let e = vm.run().unwrap_err();
        assert!(e.to_string().contains("number"), "{e}");
    }

    #[test]
    fn byte_buffers_are_reference_typed_and_indexable() {
        let src = r#"
            let buf = bytes(8);
            buf[0] = 0xAB;
            buf[7] = 255;
            print(buf[0], buf[7], len(buf));
            send(0x300, buf);
        "#;
        let script = compile(src).unwrap();
        let mut vm = Vm::new(script);
        vm.run().unwrap();
        assert_eq!(vm.output, ["171 255 8"]);
        assert_eq!(
            vm.outbox,
            [(0x300, vec![0xAB, 0, 0, 0, 0, 0, 0, 255])],
            "the whole buffer goes out as the payload"
        );
    }

    #[test]
    fn buffer_indexing_bounds_are_checked() {
        let script = compile("let b = bytes(2); print(b[5]);").unwrap();
        let mut vm = Vm::new(script);
        let e = vm.run().unwrap_err();
        assert!(e.to_string().contains("outside"), "{e}");
    }

    #[test]
    fn buffers_passed_to_functions_share_their_contents() {
        let src = r#"
            let buf = bytes(2);
            fn poke() {
                buf[1] = 9;
            }
            poke();
            print(buf[0], buf[1]);
        "#;
        assert_eq!(out(src), ["0 9"]);
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
