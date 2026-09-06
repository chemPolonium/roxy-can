//! The stack VM: executes a compiled [`Script`]. Frames are cheap and
//! capped; every executed instruction decrements a budget so a runaway
//! loop (S2: a runaway node callback) can never wedge the host.

use super::{HostInput, Op, Script, Value};
use crate::sim::{SrcKind, ValueSrc, eval_phys};

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

/// Timer control requested by the running handler. `SetPeriod`/`Stop`
/// concern the running `on timer` body itself; `Arm`/`Cancel` address a
/// named one-shot and are meaningful from any handler. The node runtime
/// applies these after the run.
#[derive(Clone, Debug)]
pub enum TimerOp {
    SetPeriod(u64),
    Stop,
    Arm { name: String, ms: u64 },
    Cancel { name: String },
}

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
    /// Timer control queued by `set_period` / `stop_timer` inside the
    /// running handler. The node runtime drains and applies these after
    /// the run.
    pub timer_ops: Vec<TimerOp>,
    /// The triggering frame's payload bytes, set by the node runtime
    /// before `on message` handlers run. Empty outside of frame events.
    pub frame_bytes: Vec<u8>,
    /// The triggering frame's identifier, set alongside `frame_bytes`.
    /// What `frame_id()` reads; 0 outside of frame events.
    pub frame_id: u32,
    /// xorshift64 state for `random()`; re-seedable via `srand`.
    pub rng: u64,
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
            timer_ops: Vec::new(),
            frame_bytes: Vec::new(),
            frame_id: 0,
            rng: 0x9E37_79B9_7F4A_7C15,
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
            let (chunk, ip) = (frame.chunk, frame.ip);
            let Some(op) = self.script.functions[chunk].code.get(ip) else {
                return Err(VmError("code ran off its chunk".into()));
            };
            let op = *op;
            self.frames.last_mut().expect("frame").ip += 1;
            if let Err(e) = self.exec_op(op) {
                // Runtime errors name the line of the instruction that
                // produced them, via the chunk's line table.
                let line = self.line_at(chunk, ip);
                return Err(VmError(match line {
                    Some(line) => format!("line {line}: {}", e.0),
                    None => e.0,
                }));
            }
            if self.frames.is_empty() {
                self.stack.pop();
                return Ok(());
            }
        }
    }

    /// The source line recorded for `ip` in `chunk`, if any mark exists.
    fn line_at(&self, chunk: usize, ip: usize) -> Option<u32> {
        self.script.functions[chunk]
            .lines
            .iter()
            .rev()
            .find(|(op, _)| *op as usize <= ip)
            .map(|(_, line)| *line)
    }

    fn exec_op(&mut self, op: Op) -> Result<(), VmError> {
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
                // Floats truncate, matching `send`: a waveform value can
                // flow straight into a buffer element.
                let byte = match value {
                    Value::Int(n) => n,
                    Value::Float(f) if f.is_finite() => f.trunc() as i64,
                    other => {
                        return Err(VmError(format!(
                            "buffer elements must be ints, got {}",
                            kind(&other)
                        )));
                    }
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
                // constant pool. Registered extern components claim
                // the name first; otherwise the host's per-node extern
                // hook answers. Unclaimed names are runtime errors --
                // the compiler cannot know what the host registers.
                let Value::Str(name) = self.script.constants[c as usize].clone() else {
                    return Err(VmError("extern name constant must be a string".into()));
                };
                if self.stack.len() < argc as usize {
                    return Err(VmError("stack underflow in extern call".into()));
                }
                let args: Vec<Value> = self.stack.split_off(self.stack.len() - argc as usize);
                match super::extern_lookup(&name) {
                    // A registered extern owns the name, so its Ok(None)
                    // means "returns nothing": nil it is.
                    Some(f) => match f(&args) {
                        Ok(Some(v)) => self.stack.push(v),
                        Ok(None) => self.stack.push(Value::Nil),
                        Err(e) => return Err(VmError(e)),
                    },
                    // The per-node hook claims what it knows; an
                    // unclaimed name (its Ok(None)) is a runtime error.
                    None => {
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
            }
        }
        Ok(())
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
        // Numeric ordering for int/float; lexicographic for strings.
        let ord = match (&a, &b) {
            (Value::Str(x), Value::Str(y)) => x.cmp(y),
            _ => {
                let (x, y) = (as_float(&a), as_float(&b));
                match x.partial_cmp(&y) {
                    Some(o) => o,
                    None => {
                        return Err(VmError(format!(
                            "cannot order {} and {}",
                            kind(&a),
                            kind(&b)
                        )));
                    }
                }
            }
        };
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
                    // Single-buffer form: the buffer IS the payload. Up to
                    // 64 bytes; beyond 8 the core sends the frame as FD.
                    match &args[1] {
                        Value::Bytes(b) => {
                            let buf = b.lock().expect("buffer poisoned").clone();
                            if buf.len() > 64 {
                                return Err(VmError(format!(
                                    "send: payload up to 64 bytes, got {}",
                                    buf.len()
                                )));
                            }
                            buf
                        }
                        _ => unreachable!(),
                    }
                } else {
                    if args.len() - 1 > 8 {
                        return Err(VmError(
                            "send: at most 8 data bytes for a classic frame".into(),
                        ));
                    }
                    let mut data = Vec::with_capacity(args.len() - 1);
                    for b in &args[1..] {
                        // Floats truncate: a waveform value flows straight
                        // into the payload, `send(0x100, floor(ramp(...)))`
                        // and `send(0x100, ramp(...))` agree.
                        let n = match b {
                            Value::Int(n) => *n,
                            Value::Float(f) if f.is_finite() => f.trunc() as i64,
                            other => {
                                return Err(VmError(format!(
                                    "send: data byte must be 0..255, got {}",
                                    kind(other)
                                )));
                            }
                        };
                        if !(0..=255).contains(&n) {
                            return Err(VmError(format!(
                                "send: data byte must be 0..255, got {n}"
                            )));
                        }
                        data.push(n as u8);
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
            "set_period" => {
                // Timer control for the RUNNING `on timer` handler: the
                // node runtime applies queued ops after the run.
                let Value::Int(ms) = args[0] else {
                    return Err(VmError("set_period needs an int".into()));
                };
                if ms <= 0 {
                    return Err(VmError("set_period: period must be positive".into()));
                }
                self.timer_ops.push(TimerOp::SetPeriod(ms as u64));
            }
            "stop_timer" => {
                self.timer_ops.push(TimerOp::Stop);
            }
            "set_timer" => {
                // set_timer("name", ms): arms the named one-shot. It
                // fires once, `ms` from now, on `on timer "name"`;
                // arming again (even from that handler) restarts it.
                let (Value::Str(timer), Value::Int(ms)) = (&args[0], &args[1]) else {
                    return Err(VmError(
                        "set_timer(\"name\", ms) needs a string and an int".into(),
                    ));
                };
                if *ms <= 0 {
                    return Err(VmError("set_timer: delay must be positive".into()));
                }
                self.timer_ops.push(TimerOp::Arm {
                    name: timer.clone(),
                    ms: *ms as u64,
                });
            }
            "cancel_timer" => {
                let Value::Str(timer) = &args[0] else {
                    return Err(VmError("cancel_timer(\"name\") needs a string".into()));
                };
                self.timer_ops.push(TimerOp::Cancel {
                    name: timer.clone(),
                });
            }
            "frame_byte" => {
                let Value::Int(n) = args[0] else {
                    return Err(VmError("frame_byte needs an int index".into()));
                };
                let n = usize::try_from(n).unwrap_or(usize::MAX);
                match self.frame_bytes.get(n) {
                    Some(b) => self.stack.push(Value::Int(*b as i64)),
                    None => {
                        return Err(VmError(format!(
                            "frame_byte: index {n} outside 0..{}",
                            self.frame_bytes.len()
                        )));
                    }
                }
                return Ok(());
            }
            "frame_dlc" => {
                self.stack.push(Value::Int(self.frame_bytes.len() as i64));
                return Ok(());
            }
            "frame_id" => {
                // The triggering frame's own identifier: what a wildcard
                // `on message *` handler needs to filter or forward.
                self.stack.push(Value::Int(self.frame_id as i64));
                return Ok(());
            }
            "random" => {
                let lo = as_float(&args[0]);
                let hi = as_float(&args[1]);
                if hi < lo {
                    return Err(VmError(format!("random needs lo <= hi, got {lo}..{hi}")));
                }
                // xorshift64: deterministic for a given call sequence, so
                // a stimulus stays reproducible run over run.
                self.rng ^= self.rng << 13;
                self.rng ^= self.rng >> 7;
                self.rng ^= self.rng << 17;
                let unit = (self.rng >> 11) as f64 / (1u64 << 53) as f64;
                self.stack.push(Value::Float(lo + unit * (hi - lo)));
                return Ok(());
            }
            "srand" => {
                let Value::Int(seed) = args[0] else {
                    return Err(VmError("srand needs an int".into()));
                };
                self.rng = seed as u64;
            }
            "ramp" | "triangle" | "square" | "counter" => {
                // Periodic shapes share one evaluator with the TX
                // generator, so a scripted wave matches a generator wave
                // sample for sample. Square is sim's two-slot step.
                let lo = as_float(&args[0]);
                let hi = as_float(&args[1]);
                let period = as_float(&args[2]);
                if period <= 0.0 {
                    return Err(VmError(format!("{name}: period must be positive")));
                }
                let kind = match name.as_str() {
                    "ramp" => SrcKind::Ramp,
                    "triangle" => SrcKind::Triangle,
                    "square" => SrcKind::Step,
                    _ => SrcKind::Counter,
                };
                let src = ValueSrc {
                    // A sub-microsecond period is meaningless on CAN; hold
                    // the sim fallback (one second) rather than zeroing.
                    period_us: (period * 1_000_000.0).round().max(1.0) as u64,
                    ..ValueSrc::new("wave", kind, lo, hi)
                };
                let t_us = (self.host_input.now_s.max(0.0) * 1_000_000.0) as u64;
                self.stack.push(Value::Float(eval_phys(&src, t_us)));
                return Ok(());
            }
            "sine_wave" => {
                // sine_wave(offset, amplitude, period_s): a sine centred
                // at `offset` with peak-to-peak `2 * amplitude`.
                let offset = as_float(&args[0]);
                let amplitude = as_float(&args[1]);
                let period = as_float(&args[2]);
                if period <= 0.0 {
                    return Err(VmError("sine_wave: period must be positive".into()));
                }
                let phase = (self.host_input.now_s % period) / period * std::f64::consts::TAU;
                self.stack
                    .push(Value::Float(offset + amplitude * phase.sin()));
                return Ok(());
            }
            // Bitwise operations: int-only, essential for CAN field
            // extraction and construction.
            "bit_and" | "bit_or" | "bit_xor" => {
                let (Value::Int(a), Value::Int(b)) = (&args[0], &args[1]) else {
                    return Err(VmError(format!("{name} needs two ints")));
                };
                let r = match name.as_str() {
                    "bit_and" => a & b,
                    "bit_or" => a | b,
                    _ => a ^ b,
                };
                self.stack.push(Value::Int(r));
                return Ok(());
            }
            "bit_not" => {
                let Value::Int(a) = args[0] else {
                    return Err(VmError("bit_not needs an int".into()));
                };
                self.stack.push(Value::Int(!a));
                return Ok(());
            }
            "bit_shl" | "bit_shr" => {
                let (Value::Int(a), Value::Int(n)) = (&args[0], &args[1]) else {
                    return Err(VmError(format!("{name} needs two ints")));
                };
                let (a, n) = (*a, *n as u32 % 64);
                let r = if name == "bit_shl" {
                    ((a as u64) << n) as i64
                } else {
                    ((a as u64) >> n) as i64
                };
                self.stack.push(Value::Int(r));
                return Ok(());
            }
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
    fn random_stays_in_range_and_srand_replays() {
        let src = "srand(42); print(random(0, 10)); print(random(0, 10));";
        let a = out(src);
        let b = out(src);
        assert_eq!(a, b, "same seed, same sequence");
        for v in &a {
            let x: f64 = v
                .parse()
                .unwrap_or_else(|e| panic!("random output '{v}' not numeric: {e}"));
            assert!(
                (0.0..=10.0).contains(&x),
                "random output {x} outside the range"
            );
        }
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

    /// The periodic shapes must match the TX generator sample for
    /// sample: both evaluate through `sim::eval_phys`.
    #[test]
    fn periodic_waves_share_the_generator_evaluator() {
        let script = compile(
            "print(ramp(0, 100, 1)); \
             print(triangle(0, 100, 1)); \
             print(square(0, 100, 1)); \
             print(counter(0, 15, 1.6));",
        )
        .unwrap();
        let mut vm = Vm::new(script);
        vm.host_input.now_s = 0.25;
        vm.run().unwrap();
        assert_eq!(
            vm.output,
            ["25.0", "50.0", "0.0", "2.0"],
            "ramp rises, triangle peaks at half, square holds lo in the first half, counter is one step in"
        );
        // The sine builtin keeps its centred parameterisation.
        let script = compile("print(sine_wave(10, 5, 1));").unwrap();
        let mut vm = Vm::new(script);
        vm.host_input.now_s = 0.25;
        vm.run().unwrap();
        assert_eq!(vm.output, ["15.0"]);
    }

    #[test]
    fn waves_reject_a_non_positive_period() {
        for wave in ["ramp", "triangle", "square", "counter", "sine_wave"] {
            let src = format!("print({wave}(0, 100, 0));");
            let mut vm = Vm::new(compile(&src).unwrap());
            let e = vm.run().unwrap_err();
            assert!(e.to_string().contains("positive"), "{wave}: {e}");
        }
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
