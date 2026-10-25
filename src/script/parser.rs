//! Recursive-descent parser: tokens to the AST. Precedence climbs in the
//! usual C order; `for` is desugared here into init/while/step so the
//! compiler only ever sees `while`.

use super::ScriptError;
use super::lexer::{Tok, Token};

#[derive(Debug)]
pub struct Program {
    pub items: Vec<Item>,
}

#[derive(Debug)]
pub enum Item {
    Fn(FnDecl),
    On(OnDecl),
    Stmt(SpannedStmt),
}

/// A statement tagged with its source position: the compiler builds
/// per-chunk line tables from these so runtime errors can name the line,
/// and compile errors name the statement's column.
#[derive(Debug)]
pub struct SpannedStmt {
    pub line: u32,
    pub col: u32,
    pub stmt: Stmt,
}

/// An event handler declaration. The body runs when the node runtime (S2)
/// delivers the event; here it only needs to compile.
#[derive(Debug)]
pub struct OnDecl {
    pub kind: OnKind,
    pub body: Vec<SpannedStmt>,
    pub line: u32,
    pub col: u32,
}

#[derive(Debug)]
pub enum OnKind {
    /// Measurement start, once.
    Start,
    /// A frame with this identifier arrived on the node's channel.
    Message { id: u32 },
    /// A frame whose DBC message name is `name` arrived. The node resolves
    /// the name (and its standard/extended class) from its channel's
    /// database at start, so the script does not repeat the numeric id.
    NamedMessage { name: String },
    /// An **extended** frame with this identifier arrived. Numeric ids
    /// inside the standard range can be extended frames too (rare but
    /// legal); this form addresses them where plain `on message` cannot.
    ExtendedMessage { id: u32 },
    /// Every frame on the node's channel, whatever its id: the gateway /
    /// logger shape. `on message *`.
    AnyMessage,
    /// An error frame arrived on the node's channel (`on errorFrame`).
    ErrorFrame,
    /// A periodic tick every `period_ms` milliseconds.
    Timer { period_ms: u64 },
    /// A named one-shot: idle until `set_timer(name, ms)` arms it from
    /// any handler, then fires once on its handler.
    Oneshot { name: String },
    /// A FlexRay frame arrived in this slot (`on fr slot 13`), optionally on
    /// one named cluster (`on fr 0 slot 13`). The slot is the address because
    /// it is what the schedule and the received row agree on: a static slot is
    /// held by several frames in turn, so a frame name would fire on only some
    /// of the arrivals the user can see in Trace.
    FrSlot { bus: Option<u8>, slot: u16 },
}

#[derive(Debug)]
pub struct FnDecl {
    pub name: String,
    pub params: Vec<String>,
    pub body: Vec<SpannedStmt>,
}

#[derive(Debug)]
pub enum Stmt {
    Let(String, Expr),
    Assign(String, Expr),
    /// `name[i] = v` -- byte-buffer element assignment. The container is
    /// a plain variable; buffers carry reference semantics, so the store
    /// mutates the one shared buffer.
    AssignIndex(String, Expr, Expr),
    If {
        cond: Expr,
        then: Vec<SpannedStmt>,
        els: Option<Vec<SpannedStmt>>,
    },
    While {
        cond: Expr,
        body: Vec<SpannedStmt>,
    },
    /// Desugared `for`: init, condition (None = true), step, body.
    For {
        init: Option<Box<SpannedStmt>>,
        cond: Option<Expr>,
        step: Option<Box<SpannedStmt>>,
        body: Vec<SpannedStmt>,
    },
    Return(Option<Expr>),
    Break,
    Continue,
    Block(Vec<SpannedStmt>),
    Expr(Expr),
}

#[derive(Debug, Clone)]
pub enum Expr {
    Int(i64),
    Float(f64),
    Bool(bool),
    Str(String),
    Ident(String),
    /// `container[index]` -- byte buffers for now.
    Index(Box<Expr>, Box<Expr>),
    Unary(UnOp, Box<Expr>),
    Binary(BinOp, Box<Expr>, Box<Expr>),
    Call(String, Vec<Expr>),
}

#[derive(Debug, Clone, Copy)]
pub enum UnOp {
    Neg,
    Not,
}

#[derive(Debug, Clone, Copy)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    And,
    Or,
    /// Pure integer bit arithmetic: no floats, no short circuit, no
    /// effect on the static send/derive sets (R2 语言增量).
    BitAnd,
    BitOr,
    BitXor,
    Shl,
    Shr,
}

impl BinOp {
    /// The operator token family that stores back through compound
    /// assignment: `x |= e` compiles as `x = x | e`.
    pub fn assign_token(self) -> Option<Tok> {
        Some(match self {
            BinOp::Add => Tok::AssignAdd,
            BinOp::Sub => Tok::AssignSub,
            BinOp::Mul => Tok::AssignMul,
            BinOp::Div => Tok::AssignDiv,
            BinOp::Mod => Tok::AssignMod,
            BinOp::BitAnd => Tok::AssignBitAnd,
            BinOp::BitOr => Tok::AssignBitOr,
            BinOp::BitXor => Tok::AssignBitXor,
            BinOp::Shl => Tok::AssignShl,
            BinOp::Shr => Tok::AssignShr,
            _ => return None,
        })
    }
}

pub fn parse(toks: Vec<Token>) -> Result<Program, ScriptError> {
    let mut p = P {
        toks,
        pos: 0,
        fn_depth: 0,
        switch_temps: 0,
    };
    let mut items = Vec::new();
    while !p.at(&Tok::Eof) {
        if p.at(&Tok::Fn) {
            items.push(Item::Fn(p.fn_decl()?));
        } else if p.at(&Tok::On) {
            items.push(Item::On(p.on_decl()?));
        } else {
            items.push(Item::Stmt(p.stmt()?));
        }
    }
    Ok(Program { items })
}

struct P {
    toks: Vec<Token>,
    pos: usize,
    /// Nonzero while parsing a function body: `return` and nested `fn`
    /// are only legal where this says so.
    fn_depth: u32,
    /// Counter for the hidden `switch` subject temps (`__switch0`, ...),
    /// so nested switches never share one.
    switch_temps: usize,
}

/// `FR3` / `ETH0`: a reserved bus qualifier, prefix plus digits. These
/// names cannot be plain variables; `FR3.Signal` is how a FlexRay signal
/// is read.
fn bus_qualifier(name: &str, prefix: &str) -> Option<u32> {
    let rest = name.strip_prefix(prefix)?;
    if rest.is_empty() || !rest.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    rest.parse().ok()
}

/// Why a name cannot be a variable/function/parameter. The dot reads are
/// resolved by looking at the name on the left of the dot, so those names
/// must stay unambiguous in every scope.
fn reserved_identifier(name: &str) -> Option<&'static str> {
    match name {
        "this" => Some("`this` is the triggering frame context"),
        "node" => Some("`node` is the script's own context"),
        "FR" | "ETH" => Some("`FR` / `ETH` start a bus selector"),
        _ if bus_qualifier(name, "FR").is_some() => Some("`FRn` is a FlexRay bus selector"),
        _ if bus_qualifier(name, "ETH").is_some() => Some("`ETHn` is an Ethernet bus selector"),
        _ => None,
    }
}

/// What can follow an identifier at statement position.
#[derive(Clone, Copy, PartialEq)]
enum AssignAhead {
    /// `x = v` or `x op= v`.
    Plain,
    /// `x[i] = v` or `x[i] op= v`.
    Index,
}

impl P {
    fn err<T>(&self, msg: &str) -> Result<T, ScriptError> {
        let at = self.toks.get(self.pos);
        Err(ScriptError {
            line: at.map_or(1, |t| t.line),
            col: at.map(|t| t.col),
            msg: msg.to_string(),
        })
    }

    fn at(&self, tok: &Tok) -> bool {
        self.toks.get(self.pos).is_some_and(|t| &t.tok == tok)
    }

    fn advance(&mut self) -> Token {
        let t = self.toks[self.pos].clone();
        if self.pos < self.toks.len() - 1 {
            self.pos += 1;
        }
        t
    }

    fn eat(&mut self, tok: &Tok) -> bool {
        if self.at(tok) {
            self.advance();
            true
        } else {
            false
        }
    }

    fn expect(&mut self, tok: &Tok, what: &str) -> Result<(), ScriptError> {
        if self.at(tok) {
            self.advance();
            Ok(())
        } else {
            let at = self.toks.get(self.pos);
            let line = at.map_or(1, |t| t.line);
            let col = at.map(|t| t.col);
            self.err(&format!(
                "expected {what}, found {:?}",
                self.toks.get(self.pos).map(|t| &t.tok)
            ))
            .map_err(|mut e| {
                e.line = line;
                e.col = col;
                e
            })
        }
    }

    fn ident(&mut self, what: &str) -> Result<String, ScriptError> {
        match self.toks.get(self.pos).map(|t| t.tok.clone()) {
            Some(Tok::Ident(name)) => {
                self.advance();
                Ok(name)
            }
            _ => self.err(&format!("expected {what}")),
        }
    }

    /// A name the script itself introduces: a variable, function or
    /// parameter. Reserved context/bus names are refused here so a later
    /// `name.member` read can never be shadowed by them.
    fn var_ident(&mut self, what: &str) -> Result<String, ScriptError> {
        let name = self.ident(what)?;
        if let Some(why) = reserved_identifier(&name) {
            return self.err(&format!("`{name}` cannot be used as {what}: {why}"));
        }
        Ok(name)
    }

    /// One name after a bus qualifier: `.Signal` or `["Signal Name"]`.
    fn member(&mut self, what: &str) -> Result<String, ScriptError> {
        if self.eat(&Tok::Dot) {
            return self.ident(what);
        }
        if let Some(name) = self.literal_bracket() {
            return Ok(name);
        }
        self.err(&format!("expected {what}"))
    }

    /// True when the next tokens are exactly `["literal"]` -- the bracket
    /// form of a named read, as opposed to an ordinary index expression.
    fn at_literal_bracket(&self) -> bool {
        matches!(self.toks.get(self.pos).map(|t| &t.tok), Some(Tok::LBracket))
            && matches!(
                self.toks.get(self.pos + 1).map(|t| &t.tok),
                Some(Tok::Str(_))
            )
            && matches!(
                self.toks.get(self.pos + 2).map(|t| &t.tok),
                Some(Tok::RBracket)
            )
    }

    /// Consumes `["literal"]` and returns the string; leaves the stream
    /// untouched otherwise.
    fn literal_bracket(&mut self) -> Option<String> {
        if !self.at_literal_bracket() {
            return None;
        }
        self.advance(); // [
        let Some(Tok::Str(name)) = self.toks.get(self.pos).map(|t| t.tok.clone()) else {
            return None;
        };
        self.advance(); // "name"
        self.advance(); // ]
        Some(name)
    }

    /// `this`: sugar over the triggering frame's context. It never becomes
    /// a runtime object -- each field compiles to the existing host calls,
    /// so `this` costs nothing at run time and stays out of the value model.
    fn this_expr(&mut self) -> Result<Expr, ScriptError> {
        if self.eat(&Tok::Dot) {
            let field = self.ident("a `this` field after '.'")?;
            return match field.as_str() {
                "bus" => Ok(Expr::Call("frame_bus".to_string(), vec![])),
                "id" => Ok(Expr::Call("frame_can_id".to_string(), vec![])),
                "slot" => Ok(Expr::Call("frame_slot".to_string(), vec![])),
                "len" => Ok(Expr::Call("frame_dlc".to_string(), vec![])),
                "cycle" => Ok(Expr::Call("frame_cycle".to_string(), vec![])),
                "signal" => {
                    self.expect(&Tok::LParen, "'(' after this.signal")?;
                    let Some(Tok::Str(name)) = self.toks.get(self.pos).map(|t| t.tok.clone())
                    else {
                        return self.err("this.signal needs a signal name in quotes");
                    };
                    self.advance();
                    self.expect(&Tok::RParen, "')'")?;
                    Ok(Expr::Call(
                        "frame_signal".to_string(),
                        vec![Expr::Str(name)],
                    ))
                }
                _ => self.err(
                    "unknown `this` field; use bus, id, slot, len, cycle, \
                     signal(\"Name\") or this[index]",
                ),
            };
        }
        // `this[index]` is the payload byte; `this["Name"]` is the signal
        // with that name, the same literal-name rule the other reads use.
        if self.eat(&Tok::LBracket) {
            if let Some(Tok::Str(name)) = self.toks.get(self.pos).map(|t| t.tok.clone())
                && matches!(
                    self.toks.get(self.pos + 1).map(|t| &t.tok),
                    Some(Tok::RBracket)
                )
            {
                self.advance(); // "name"
                self.advance(); // ]
                return Ok(Expr::Call(
                    "frame_signal".to_string(),
                    vec![Expr::Str(name)],
                ));
            }
            let index = self.expr()?;
            self.expect(&Tok::RBracket, "']'")?;
            return Ok(Expr::Call("frame_byte".to_string(), vec![index]));
        }
        self.err(
            "`this` only describes the triggering frame; write this.bus, \
             this.id, this.signal(\"Name\") or this[index]",
        )
    }

    /// True when a statement begins with a named signal read of any
    /// spelling (`M.Sig`, `M["Sig"]`, `FR0.Sig`, `ETH0.Frame.Sig`, ...)
    /// followed by an assignment operator. Those reads are read-only.
    fn signal_write_ahead(&self) -> bool {
        let tok = |i: usize| self.toks.get(self.pos + i).map(|t| &t.tok);
        if !matches!(tok(0), Some(Tok::Ident(_))) {
            return false;
        }
        let mut i = 1;
        let mut qualified = false;
        loop {
            match tok(i) {
                Some(Tok::Dot) if matches!(tok(i + 1), Some(Tok::Ident(_))) => {
                    i += 2;
                    qualified = true;
                }
                Some(Tok::LBracket)
                    if matches!(tok(i + 1), Some(Tok::Str(_)))
                        && matches!(tok(i + 2), Some(Tok::RBracket)) =>
                {
                    i += 3;
                    qualified = true;
                }
                _ => break,
            }
        }
        qualified
            && matches!(
                tok(i),
                Some(
                    Tok::Assign
                        | Tok::AssignAdd
                        | Tok::AssignSub
                        | Tok::AssignMul
                        | Tok::AssignDiv
                        | Tok::AssignMod
                        | Tok::AssignBitAnd
                        | Tok::AssignBitOr
                        | Tok::AssignBitXor
                        | Tok::AssignShl
                        | Tok::AssignShr
                )
            )
    }

    fn fn_decl(&mut self) -> Result<FnDecl, ScriptError> {
        self.expect(&Tok::Fn, "'fn'")?;
        let name = self.var_ident("function name")?;
        self.expect(&Tok::LParen, "'('")?;
        let mut params = Vec::new();
        if !self.at(&Tok::RParen) {
            loop {
                params.push(self.var_ident("parameter name")?);
                if !self.eat(&Tok::Comma) {
                    break;
                }
            }
        }
        self.expect(&Tok::RParen, "')'")?;
        self.fn_depth += 1;
        let body = self.block()?;
        self.fn_depth -= 1;
        Ok(FnDecl { name, params, body })
    }

    /// `on start { }` / `on message 0x100 { }` / `on timer 100 { }` -- a
    /// node's event handlers. The event word is matched by text so that
    /// `message` and `timer` stay usable as ordinary variable names.
    fn on_decl(&mut self) -> Result<OnDecl, ScriptError> {
        let at = self.toks.get(self.pos);
        let line = at.map_or(1, |t| t.line);
        let col = at.map_or(1, |t| t.col);
        self.expect(&Tok::On, "'on'")?;
        let word = self.ident("'start', 'message', 'timer', 'extended', 'errorFrame' or 'fr'")?;
        let kind = match word.as_str() {
            "start" => OnKind::Start,
            "extended" => {
                // `on extended message <id>`: an extended frame with this
                // numeric id. The word after must be `message`.
                let word = self.ident("'message'")?;
                if word != "message" {
                    return self.err("expected 'message' after 'extended'");
                }
                OnKind::ExtendedMessage {
                    id: self.id_literal()?,
                }
            }
            "message" => {
                // `on message *`: every frame, whatever its id.
                // `on message 0x100`: by numeric id.
                // `on message EngineStatus` / `on message "Engine Status"`:
                // by DBC name; the node resolves id and frame class at start.
                if self.eat(&Tok::Star) {
                    OnKind::AnyMessage
                } else {
                    match self.toks.get(self.pos).map(|t| t.tok.clone()) {
                        Some(Tok::Int(_)) => OnKind::Message {
                            id: self.id_literal()?,
                        },
                        Some(Tok::Ident(name)) => {
                            self.advance();
                            OnKind::NamedMessage { name }
                        }
                        Some(Tok::Str(_)) => OnKind::NamedMessage {
                            name: self.string_literal()?,
                        },
                        _ => {
                            return self.err("expected a message id, a DBC message name, or '*'");
                        }
                    }
                }
            }
            "timer" => {
                // A number declares a periodic tick; a string names a
                // one-shot that `set_timer` arms at runtime.
                match self.toks.get(self.pos).map(|t| t.tok.clone()) {
                    Some(Tok::Str(_)) => OnKind::Oneshot {
                        name: self.string_literal()?,
                    },
                    _ => OnKind::Timer {
                        period_ms: self.period_literal()?,
                    },
                }
            }
            "errorframe" | "errorFrame" => OnKind::ErrorFrame,
            "fr" => {
                // `on fr slot 13` / `on fr 0 slot 13`. The cluster is optional
                // because a one-cluster setup is the common case, but it has to
                // be writable: slot 13 on two clusters is two different
                // arrivals, and a node sits on a CAN channel, so there is no
                // "its own FlexRay side" to infer.
                let bus = match self.toks.get(self.pos).map(|t| t.tok.clone()) {
                    Some(Tok::Int(n)) if (0..=255).contains(&n) => {
                        self.advance();
                        Some(n as u8)
                    }
                    Some(Tok::Int(n)) => {
                        return self.err(&format!("cluster index {n} out of range (0..255)"));
                    }
                    _ => None,
                };
                let word = self.ident("'slot'")?;
                if word != "slot" {
                    return self
                        .err("expected 'slot' after 'fr' (on fr slot 13 / on fr 0 slot 13)");
                }
                OnKind::FrSlot {
                    bus,
                    slot: self.slot_literal()?,
                }
            }
            other => {
                return self.err(&format!(
                    "unknown event '{other}' (start, message, extended message, timer, errorFrame, fr slot)"
                ));
            }
        };
        self.fn_depth += 1;
        let body = self.block()?;
        self.fn_depth -= 1;
        Ok(OnDecl {
            kind,
            body,
            line,
            col,
        })
    }

    /// A CAN identifier. Within the standard range it names standard
    /// frames; beyond 0x7FF it names a 29-bit extended frame (the same
    /// rule `send` uses to pick the frame class).
    fn id_literal(&mut self) -> Result<u32, ScriptError> {
        let at = self.toks.get(self.pos);
        let line = at.map_or(1, |t| t.line);
        let col = at.map_or(1, |t| t.col);
        match self.toks.get(self.pos).map(|t| t.tok.clone()) {
            Some(Tok::Int(n)) if (0..=0x1FFF_FFFF).contains(&n) => {
                self.advance();
                Ok(n as u32)
            }
            Some(Tok::Int(n)) => Err(ScriptError {
                line,
                col: Some(col),
                msg: format!("id {n:#x} out of the 29-bit extended range"),
            }),
            _ => self.err("expected a message id"),
        }
    }

    /// A FlexRay slot number, as `on fr slot` addresses one. Bounded by what a
    /// row carries (u16), not by a schedule: the compiler never sees the
    /// description, and a slot the cluster never uses simply never fires.
    fn slot_literal(&mut self) -> Result<u16, ScriptError> {
        let at = self.toks.get(self.pos);
        let line = at.map_or(1, |t| t.line);
        let col = at.map_or(1, |t| t.col);
        match self.toks.get(self.pos).map(|t| t.tok.clone()) {
            Some(Tok::Int(n)) if (0..=u16::MAX as i64).contains(&n) => {
                self.advance();
                Ok(n as u16)
            }
            Some(Tok::Int(n)) => Err(ScriptError {
                line,
                col: Some(col),
                msg: format!("slot {n} out of range (0..{})", u16::MAX),
            }),
            _ => self.err("expected a FlexRay slot number"),
        }
    }

    /// A timer period: a positive integer in milliseconds, or a duration
    /// literal (`250ms`, `1s`, `1.5s`).
    fn period_literal(&mut self) -> Result<u64, ScriptError> {
        let at = self.toks.get(self.pos);
        let line = at.map_or(1, |t| t.line);
        let col = at.map_or(1, |t| t.col);
        match self.toks.get(self.pos).map(|t| t.tok.clone()) {
            Some(Tok::Int(n)) => {
                self.advance();
                if let Some(ms) = self.duration_tail(n as f64, line, col)? {
                    return Ok(ms);
                }
                if n <= 0 {
                    return Err(ScriptError {
                        line,
                        col: Some(col),
                        msg: "timer period must be positive".to_string(),
                    });
                }
                Ok(n as u64)
            }
            Some(Tok::Float(x)) => {
                self.advance();
                match self.duration_tail(x, line, col)? {
                    Some(ms) => Ok(ms),
                    None => self.err("expected 'ms' or 's' after a fractional timer period"),
                }
            }
            _ => self.err("expected a timer period (milliseconds, 250ms or 1s)"),
        }
    }

    /// A `ms`/`s` suffix outside a timer-period position is not a value; say
    /// where duration literals are accepted instead of "expected ')'"-style
    /// noise.
    fn reject_stray_duration(&self) -> Result<(), ScriptError> {
        if let Some(Tok::Ident(unit)) = self.toks.get(self.pos).map(|t| t.tok.clone())
            && (unit == "ms" || unit == "s")
        {
            return self.err(
                "duration literals are only valid for timer periods \
                 (on timer, set_timer, set_period)",
            );
        }
        Ok(())
    }

    /// Consumes an optional `ms`/`s` suffix after a duration number and
    /// returns whole milliseconds. `None` means no suffix was present.
    fn duration_tail(
        &mut self,
        number: f64,
        line: u32,
        col: u32,
    ) -> Result<Option<u64>, ScriptError> {
        let unit = match self.toks.get(self.pos).map(|t| t.tok.clone()) {
            Some(Tok::Ident(unit)) if unit == "ms" || unit == "s" => unit,
            _ => return Ok(None),
        };
        self.advance();
        let ms = if unit == "s" { number * 1000.0 } else { number };
        if !ms.is_finite() || ms < 1.0 || ms.fract() != 0.0 {
            return Err(ScriptError {
                line,
                col: Some(col),
                msg: "timer periods are whole milliseconds (e.g. 250ms, 1s)".to_string(),
            });
        }
        Ok(Some(ms as u64))
    }

    /// A string literal: a one-shot timer's name.
    fn string_literal(&mut self) -> Result<String, ScriptError> {
        match self.toks.get(self.pos).map(|t| t.tok.clone()) {
            Some(Tok::Str(s)) => {
                self.advance();
                Ok(s)
            }
            _ => self.err("expected a timer name string"),
        }
    }

    fn block(&mut self) -> Result<Vec<SpannedStmt>, ScriptError> {
        self.expect(&Tok::LBrace, "'{'")?;
        let mut stmts = Vec::new();
        while !self.at(&Tok::RBrace) {
            if self.at(&Tok::Eof) {
                return self.err("unexpected end of input inside a block");
            }
            if self.at(&Tok::Fn) || self.at(&Tok::On) {
                return self.err("declarations may only appear at top level");
            }
            stmts.push(self.stmt()?);
        }
        self.expect(&Tok::RBrace, "'}'")?;
        Ok(stmts)
    }

    fn stmt(&mut self) -> Result<SpannedStmt, ScriptError> {
        let at = self.toks.get(self.pos);
        let line = at.map_or(1, |t| t.line);
        let col = at.map_or(1, |t| t.col);
        let stmt = self.stmt_inner()?;
        Ok(SpannedStmt { line, col, stmt })
    }

    fn stmt_inner(&mut self) -> Result<Stmt, ScriptError> {
        // Signal reads are read-only: writes carry a frame's raw bytes,
        // which only `set_sig` / `set_fr_sig` can shape. Point at the real
        // form instead of a generic lvalue error. Every named spelling is
        // covered: `M.Sig`, `M["Sig"]`, `FR0.Sig`, `ETH0.Frame.Sig` and
        // their bracket variants.
        if self.signal_write_ahead() {
            return self.err(
                "signal reads are read-only: write CAN signals through \
                 set_sig(buffer, id, \"Name\", value) and FlexRay signals \
                 through set_fr_sig(bus, slot, \"Name\", value)",
            );
        }
        if self.eat(&Tok::Let) {
            let name = self.var_ident("variable name")?;
            self.expect(&Tok::Assign, "'=' in a let")?;
            let expr = self.expr()?;
            self.expect(&Tok::Semi, "';'")?;
            return Ok(Stmt::Let(name, expr));
        }
        if self.eat(&Tok::If) {
            return Ok(self.if_stmt()?.stmt);
        }
        if self.eat(&Tok::While) {
            self.expect(&Tok::LParen, "'('")?;
            let cond = self.expr()?;
            self.expect(&Tok::RParen, "')'")?;
            let body = self.block()?;
            return Ok(Stmt::While { cond, body });
        }
        if self.eat(&Tok::For) {
            return Ok(self.for_stmt()?.stmt);
        }
        if self.eat(&Tok::Switch) {
            return self.switch_stmt();
        }
        if self.eat(&Tok::Return) {
            if self.at(&Tok::Semi) {
                self.advance();
                if self.fn_depth == 0 {
                    return self.err("'return' outside a function");
                }
                return Ok(Stmt::Return(None));
            }
            let expr = self.expr()?;
            self.expect(&Tok::Semi, "';'")?;
            if self.fn_depth == 0 {
                return self.err("'return' outside a function");
            }
            return Ok(Stmt::Return(Some(expr)));
        }
        if self.at(&Tok::LBrace) {
            return Ok(Stmt::Block(self.block()?));
        }
        if self.eat(&Tok::Break) {
            self.expect(&Tok::Semi, "';'")?;
            return Ok(Stmt::Break);
        }
        if self.eat(&Tok::Continue) {
            self.expect(&Tok::Semi, "';'")?;
            return Ok(Stmt::Continue);
        }
        // Assignment or a bare expression, told apart by the next token.
        // `name = v`, `name op= v`, and `name[i] (op=)= v` (buffer
        // element store) are the shapes; the compound forms desugar
        // here, so the compiler only ever sees plain stores.
        if matches!(self.toks.get(self.pos).map(|t| &t.tok), Some(Tok::Ident(_)))
            && self.assign_ahead().is_some()
        {
            let stmt = self.assign_stmt()?;
            self.expect(&Tok::Semi, "';'")?;
            return Ok(stmt);
        }
        let expr = self.expr()?;
        self.expect(&Tok::Semi, "';'")?;
        Ok(Stmt::Expr(expr))
    }

    /// The shapes an identifier at statement position can start: a plain
    /// store, an index store, or a compound store. The lookahead only
    /// peeks; [`P::assign_stmt`] consumes.
    fn assign_ahead(&self) -> Option<(AssignAhead, Option<BinOp>)> {
        let second = self.toks.get(self.pos + 1).map(|t| t.tok.clone())?;
        let (shape, op) = match second {
            Tok::Assign => (AssignAhead::Plain, None),
            Tok::LBracket => (AssignAhead::Index, None),
            Tok::AssignAdd => (AssignAhead::Plain, Some(BinOp::Add)),
            Tok::AssignSub => (AssignAhead::Plain, Some(BinOp::Sub)),
            Tok::AssignMul => (AssignAhead::Plain, Some(BinOp::Mul)),
            Tok::AssignDiv => (AssignAhead::Plain, Some(BinOp::Div)),
            Tok::AssignMod => (AssignAhead::Plain, Some(BinOp::Mod)),
            Tok::AssignBitAnd => (AssignAhead::Plain, Some(BinOp::BitAnd)),
            Tok::AssignBitOr => (AssignAhead::Plain, Some(BinOp::BitOr)),
            Tok::AssignBitXor => (AssignAhead::Plain, Some(BinOp::BitXor)),
            Tok::AssignShl => (AssignAhead::Plain, Some(BinOp::Shl)),
            Tok::AssignShr => (AssignAhead::Plain, Some(BinOp::Shr)),
            _ => return None,
        };
        Some((shape, op))
    }

    /// Parses one assignment (no trailing `;`): `x = v`, `x op= v`
    /// (compiled as `x = x op v`), and the index forms of both.
    fn assign_stmt(&mut self) -> Result<Stmt, ScriptError> {
        let Some((shape, op)) = self.assign_ahead() else {
            return self.err("expected an assignment");
        };
        let name = self.var_ident("variable name")?;
        if shape == AssignAhead::Index {
            self.advance();
            let idx = self.expr()?;
            self.expect(&Tok::RBracket, "']'")?;
            // The compound operator (if any) follows the closing bracket
            // -- past the index expression, beyond the shallow peek. The
            // store target reads back through the same index, so
            // `x[i] op= v` compiles as `x[i] = x[i] op v`.
            let op = self.assign_op_here();
            let value = if let Some(op) = op {
                let lhs = Expr::Index(Box::new(Expr::Ident(name.clone())), Box::new(idx.clone()));
                self.advance();
                let rhs = self.expr()?;
                Expr::Binary(op, Box::new(lhs), Box::new(rhs))
            } else {
                self.expect(&Tok::Assign, "'='")?;
                self.expr()?
            };
            return Ok(Stmt::AssignIndex(name, idx, value));
        }
        let expr = if let Some(op) = op {
            let lhs = Expr::Ident(name.clone());
            self.advance();
            let rhs = self.expr()?;
            Expr::Binary(op, Box::new(lhs), Box::new(rhs))
        } else {
            self.advance();
            self.expr()?
        };
        Ok(Stmt::Assign(name, expr))
    }

    /// The compound-assignment operator at the current position, if one
    /// sits there.
    fn assign_op_here(&self) -> Option<BinOp> {
        let op = self.toks.get(self.pos).map(|t| t.tok.clone())?;
        Some(match op {
            Tok::AssignAdd => BinOp::Add,
            Tok::AssignSub => BinOp::Sub,
            Tok::AssignMul => BinOp::Mul,
            Tok::AssignDiv => BinOp::Div,
            Tok::AssignMod => BinOp::Mod,
            Tok::AssignBitAnd => BinOp::BitAnd,
            Tok::AssignBitOr => BinOp::BitOr,
            Tok::AssignBitXor => BinOp::BitXor,
            Tok::AssignShl => BinOp::Shl,
            Tok::AssignShr => BinOp::Shr,
            _ => return None,
        })
    }

    /// `switch (subject) { case label: body ... default: body }`,
    /// desugared here into an if-chain -- each case is exclusive, there
    /// is no fallthrough and no case-level `break` (loops keep `break`).
    /// A plain identifier subject re-reads the variable per comparison;
    /// anything else (a call, an expression) is captured in a hidden
    /// temp first so it evaluates exactly once.
    fn switch_stmt(&mut self) -> Result<Stmt, ScriptError> {
        let head = self.toks.get(self.pos);
        let head_line = head.map_or(1, |t| t.line);
        let head_col = head.map_or(1, |t| t.col);
        self.expect(&Tok::LParen, "'('")?;
        let subject = self.expr()?;
        self.expect(&Tok::RParen, "')'")?;
        self.expect(&Tok::LBrace, "'{'")?;
        let mut cases: Vec<(Expr, u32, u32, Vec<SpannedStmt>)> = Vec::new();
        let mut default: Option<Vec<SpannedStmt>> = None;
        while !self.at(&Tok::RBrace) {
            if self.at(&Tok::Eof) {
                return self.err("unexpected end of input inside a switch");
            }
            let at = self.toks.get(self.pos);
            let (line, col) = (at.map_or(1, |t| t.line), at.map_or(1, |t| t.col));
            if self.eat(&Tok::Default) {
                if default.is_some() {
                    return self.err("duplicate default case");
                }
                self.expect(&Tok::Colon, "':'")?;
                let body = self.block()?;
                // Case and default bodies wrap in their own block, so
                // locals declared in one clause stay scoped to it.
                default = Some(vec![SpannedStmt {
                    line,
                    col,
                    stmt: Stmt::Block(body),
                }]);
            } else if self.eat(&Tok::Case) {
                let label = self.expr()?;
                self.expect(&Tok::Colon, "':'")?;
                let body = self.block()?;
                cases.push((label, line, col, body));
            } else {
                return self.err("expected 'case' or 'default' inside a switch");
            }
        }
        self.expect(&Tok::RBrace, "'}'")?;
        // Decide the comparison subject before folding: a plain
        // identifier re-reads the variable per case; anything else (a
        // call, an expression) is captured in a hidden temp first so it
        // evaluates exactly once.
        let (capture, compare) = match &subject {
            Expr::Ident(_) => (None, subject.clone()),
            _ => {
                let temp = format!("__switch{}", self.switch_temps);
                self.switch_temps += 1;
                let capture = SpannedStmt {
                    line: head_line,
                    col: head_col,
                    stmt: Stmt::Let(temp.clone(), subject),
                };
                (Some(capture), Expr::Ident(temp))
            }
        };
        // The chain folds from the last case backwards: each case becomes
        // `if (subject == label) { body } else <rest>`.
        let mut rest = default;
        for (label, line, col, body) in cases.into_iter().rev() {
            let cond = Expr::Binary(BinOp::Eq, Box::new(compare.clone()), Box::new(label));
            let stmt = Stmt::If {
                cond,
                then: vec![SpannedStmt {
                    line,
                    col,
                    stmt: Stmt::Block(body),
                }],
                els: rest,
            };
            rest = Some(vec![SpannedStmt { line, col, stmt }]);
        }
        let mut chain_stmts = rest.unwrap_or_default();
        if let Some(capture) = capture {
            chain_stmts.insert(0, capture);
        }
        Ok(Stmt::Block(chain_stmts))
    }

    fn if_stmt(&mut self) -> Result<SpannedStmt, ScriptError> {
        let at = self.toks.get(self.pos);
        let line = at.map_or(1, |t| t.line);
        let col = at.map_or(1, |t| t.col);
        self.expect(&Tok::LParen, "'('")?;
        let cond = self.expr()?;
        self.expect(&Tok::RParen, "')'")?;
        let then = self.block()?;
        let els = if self.eat(&Tok::Else) {
            // Consume the `if` token before recursing: `if_stmt()` starts
            // at the `(` of the condition.
            if self.eat(&Tok::If) {
                Some(vec![self.if_stmt()?])
            } else {
                Some(self.block()?)
            }
        } else {
            None
        };
        let stmt = Stmt::If { cond, then, els };
        Ok(SpannedStmt { line, col, stmt })
    }

    fn for_stmt(&mut self) -> Result<SpannedStmt, ScriptError> {
        let at = self.toks.get(self.pos);
        let line = at.map_or(1, |t| t.line);
        let col = at.map_or(1, |t| t.col);
        self.expect(&Tok::LParen, "'('")?;
        let init = if self.at(&Tok::Semi) {
            None
        } else {
            Some(Box::new(self.simple_stmt()?))
        };
        self.expect(&Tok::Semi, "';' after the for initializer")?;
        let cond = if self.at(&Tok::Semi) {
            None
        } else {
            Some(self.expr()?)
        };
        self.expect(&Tok::Semi, "';' after the for condition")?;
        let step = if self.at(&Tok::RParen) {
            None
        } else {
            Some(Box::new(self.simple_stmt()?))
        };
        self.expect(&Tok::RParen, "')'")?;
        let body = self.block()?;
        let stmt = Stmt::For {
            init,
            cond,
            step,
            body,
        };
        Ok(SpannedStmt { line, col, stmt })
    }

    /// A statement without the trailing `;` -- only what a for-header
    /// slot accepts: a let, an assignment, or an expression.
    fn simple_stmt(&mut self) -> Result<SpannedStmt, ScriptError> {
        let at = self.toks.get(self.pos);
        let line = at.map_or(1, |t| t.line);
        let col = at.map_or(1, |t| t.col);
        let stmt = self.simple_stmt_inner()?;
        Ok(SpannedStmt { line, col, stmt })
    }

    fn simple_stmt_inner(&mut self) -> Result<Stmt, ScriptError> {
        if self.eat(&Tok::Let) {
            let name = self.var_ident("variable name")?;
            self.expect(&Tok::Assign, "'=' in a let")?;
            return Ok(Stmt::Let(name, self.expr()?));
        }
        if matches!(self.toks.get(self.pos).map(|t| &t.tok), Some(Tok::Ident(_)))
            && self.assign_ahead().is_some()
        {
            return self.assign_stmt();
        }
        Ok(Stmt::Expr(self.expr()?))
    }

    fn expr(&mut self) -> Result<Expr, ScriptError> {
        self.or_expr()
    }

    fn or_expr(&mut self) -> Result<Expr, ScriptError> {
        let mut lhs = self.and_expr()?;
        while self.eat(&Tok::Or) {
            let rhs = self.and_expr()?;
            lhs = Expr::Binary(BinOp::Or, Box::new(lhs), Box::new(rhs));
        }
        Ok(lhs)
    }

    fn and_expr(&mut self) -> Result<Expr, ScriptError> {
        let mut lhs = self.bitor_expr()?;
        while self.eat(&Tok::And) {
            let rhs = self.bitor_expr()?;
            lhs = Expr::Binary(BinOp::And, Box::new(lhs), Box::new(rhs));
        }
        Ok(lhs)
    }

    /// Bitwise `|` -- looser than `^` (the C ladder), and both are pure
    /// integer arithmetic with no short circuit.
    fn bitor_expr(&mut self) -> Result<Expr, ScriptError> {
        let mut lhs = self.bitxor_expr()?;
        while self.eat(&Tok::BitOr) {
            let rhs = self.bitxor_expr()?;
            lhs = Expr::Binary(BinOp::BitOr, Box::new(lhs), Box::new(rhs));
        }
        Ok(lhs)
    }

    fn bitxor_expr(&mut self) -> Result<Expr, ScriptError> {
        let mut lhs = self.bitand_expr()?;
        while self.eat(&Tok::BitXor) {
            let rhs = self.bitand_expr()?;
            lhs = Expr::Binary(BinOp::BitXor, Box::new(lhs), Box::new(rhs));
        }
        Ok(lhs)
    }

    fn bitand_expr(&mut self) -> Result<Expr, ScriptError> {
        let mut lhs = self.eq_expr()?;
        while self.eat(&Tok::BitAnd) {
            let rhs = self.eq_expr()?;
            lhs = Expr::Binary(BinOp::BitAnd, Box::new(lhs), Box::new(rhs));
        }
        Ok(lhs)
    }

    fn eq_expr(&mut self) -> Result<Expr, ScriptError> {
        let mut lhs = self.cmp_expr()?;
        loop {
            let op = if self.eat(&Tok::Eq) {
                BinOp::Eq
            } else if self.eat(&Tok::Ne) {
                BinOp::Ne
            } else {
                break;
            };
            let rhs = self.cmp_expr()?;
            lhs = Expr::Binary(op, Box::new(lhs), Box::new(rhs));
        }
        Ok(lhs)
    }

    fn cmp_expr(&mut self) -> Result<Expr, ScriptError> {
        let mut lhs = self.shift_expr()?;
        loop {
            let op = if self.eat(&Tok::Lt) {
                BinOp::Lt
            } else if self.eat(&Tok::Le) {
                BinOp::Le
            } else if self.eat(&Tok::Gt) {
                BinOp::Gt
            } else if self.eat(&Tok::Ge) {
                BinOp::Ge
            } else {
                break;
            };
            let rhs = self.shift_expr()?;
            lhs = Expr::Binary(op, Box::new(lhs), Box::new(rhs));
        }
        Ok(lhs)
    }

    /// `<<` / `>>` bind tighter than comparisons, looser than `+` (the
    /// C ladder), so `1 << i + 1` reads as `1 << (i + 1)`.
    fn shift_expr(&mut self) -> Result<Expr, ScriptError> {
        let mut lhs = self.term()?;
        loop {
            let op = if self.eat(&Tok::Shl) {
                BinOp::Shl
            } else if self.eat(&Tok::Shr) {
                BinOp::Shr
            } else {
                break;
            };
            let rhs = self.term()?;
            lhs = Expr::Binary(op, Box::new(lhs), Box::new(rhs));
        }
        Ok(lhs)
    }

    fn term(&mut self) -> Result<Expr, ScriptError> {
        let mut lhs = self.factor()?;
        loop {
            let op = if self.eat(&Tok::Plus) {
                BinOp::Add
            } else if self.eat(&Tok::Minus) {
                BinOp::Sub
            } else {
                break;
            };
            let rhs = self.factor()?;
            lhs = Expr::Binary(op, Box::new(lhs), Box::new(rhs));
        }
        Ok(lhs)
    }

    fn factor(&mut self) -> Result<Expr, ScriptError> {
        let mut lhs = self.unary()?;
        loop {
            let op = if self.eat(&Tok::Star) {
                BinOp::Mul
            } else if self.eat(&Tok::Slash) {
                BinOp::Div
            } else if self.eat(&Tok::Percent) {
                BinOp::Mod
            } else {
                break;
            };
            let rhs = self.unary()?;
            lhs = Expr::Binary(op, Box::new(lhs), Box::new(rhs));
        }
        Ok(lhs)
    }

    fn unary(&mut self) -> Result<Expr, ScriptError> {
        if self.eat(&Tok::Minus) {
            return Ok(Expr::Unary(UnOp::Neg, Box::new(self.unary()?)));
        }
        if self.eat(&Tok::Not) {
            return Ok(Expr::Unary(UnOp::Not, Box::new(self.unary()?)));
        }
        self.call()
    }

    fn call(&mut self) -> Result<Expr, ScriptError> {
        let mut e = self.primary()?;
        // Postfix: calls and byte-buffer indexing chain off a primary.
        loop {
            match self.toks.get(self.pos).map(|t| t.tok.clone()) {
                Some(Tok::LParen) => {
                    let Expr::Ident(name) = e else {
                        return self.err("only named functions can be called");
                    };
                    self.advance();
                    let mut args = Vec::new();
                    if !self.at(&Tok::RParen) {
                        loop {
                            // `set_timer("name", 1s)` / `set_period(250ms)`:
                            // a duration literal becomes whole milliseconds
                            // before the VM sees it, so those builtins keep
                            // their integer contract.
                            let duration_position = (name == "set_timer" && args.len() == 1)
                                || (name == "set_period" && args.is_empty());
                            let at = self.toks.get(self.pos);
                            let (line, col) =
                                at.map(|token| (token.line, token.col)).unwrap_or((1, 1));
                            match self.toks.get(self.pos).map(|t| t.tok.clone()) {
                                Some(Tok::Int(n)) if duration_position => {
                                    self.advance();
                                    args.push(match self.duration_tail(n as f64, line, col)? {
                                        Some(ms) => Expr::Int(ms as i64),
                                        None => Expr::Int(n),
                                    });
                                }
                                Some(Tok::Float(x)) if duration_position => {
                                    self.advance();
                                    args.push(match self.duration_tail(x, line, col)? {
                                        Some(ms) => Expr::Int(ms as i64),
                                        None => Expr::Float(x),
                                    });
                                }
                                _ => args.push(self.expr()?),
                            }
                            if !self.eat(&Tok::Comma) {
                                break;
                            }
                        }
                    }
                    self.expect(&Tok::RParen, "')' after the arguments")?;
                    e = Expr::Call(name, args);
                }
                Some(Tok::LBracket) => {
                    self.advance();
                    let idx = self.expr()?;
                    self.expect(&Tok::RBracket, "']'")?;
                    e = Expr::Index(Box::new(e), Box::new(idx));
                }
                // A path separator in the Rust/C++ sense would make the
                // read look like a type/module path; this language has one
                // read path, and `::` stays inside sysvar keys.
                Some(Tok::Colon)
                    if matches!(
                        self.toks.get(self.pos + 1).map(|t| &t.tok),
                        Some(Tok::Colon)
                    ) =>
                {
                    return self.err(
                        "`::` is not an operator; use a dot read such as \
                         Message.Signal — `::` only appears inside system-variable \
                         keys like \"ns::name\"",
                    );
                }
                _ => break,
            }
        }
        Ok(e)
    }

    fn primary(&mut self) -> Result<Expr, ScriptError> {
        match self.toks.get(self.pos).map(|t| t.tok.clone()) {
            Some(Tok::Int(n)) => {
                self.advance();
                self.reject_stray_duration()?;
                Ok(Expr::Int(n))
            }
            Some(Tok::Float(x)) => {
                self.advance();
                self.reject_stray_duration()?;
                Ok(Expr::Float(x))
            }
            Some(Tok::Str(s)) => {
                self.advance();
                Ok(Expr::Str(s))
            }
            Some(Tok::True) => {
                self.advance();
                Ok(Expr::Bool(true))
            }
            Some(Tok::False) => {
                self.advance();
                Ok(Expr::Bool(false))
            }
            Some(Tok::Ident(name)) => {
                self.advance();
                // `this` / `node` are contexts, not values: they only make
                // sense with a field behind them. `this` is the triggering
                // frame, `node` the script itself.
                if name == "this" {
                    return self.this_expr();
                }
                if name == "node" {
                    if self.eat(&Tok::Dot) {
                        let field = self.ident("a `node` field after '.'")?;
                        return match field.as_str() {
                            "bus" => Ok(Expr::Call("node_bus".to_string(), vec![])),
                            _ => self.err("unknown `node` field; node.bus is the script's own bus"),
                        };
                    }
                    if self.at(&Tok::LBracket) {
                        return self.err(
                            "`node` has only node.bus; read the triggering frame with `this`",
                        );
                    }
                }
                // `FR.Name.Signal` / `FR["Name"]["Signal"]`: select the
                // cluster by the name its description carries. The numeric
                // `FR0` form stays for disambiguation.
                if name == "FR" && (self.at(&Tok::Dot) || self.at_literal_bracket()) {
                    let bus = self.member("a cluster name after 'FR'")?;
                    let signal = self.member("a signal name after the cluster name")?;
                    return Ok(Expr::Call(
                        "fr_sig_bus".to_string(),
                        vec![Expr::Str(bus), Expr::Str(signal)],
                    ));
                }
                // `ETH.Name.Frame.Signal`, with bracket spellings for names
                // that are not identifiers.
                if name == "ETH" && (self.at(&Tok::Dot) || self.at_literal_bracket()) {
                    let bus = self.member("a cluster name after 'ETH'")?;
                    let frame = self.member("a frame name after the cluster name")?;
                    let signal = self.member("a signal name after the frame name")?;
                    return Ok(Expr::Call(
                        "eth_sig_bus".to_string(),
                        vec![Expr::Str(bus), Expr::Str(frame), Expr::Str(signal)],
                    ));
                }
                // `FRn.Signal` / `FRn["Signal Name"]`: the cluster number
                // carries the bus, the host resolves the name to a slot
                // against that cluster's description at start.
                if let Some(bus) = bus_qualifier(&name, "FR") {
                    if bus > u32::from(u8::MAX) {
                        return self.err("the FlexRay bus index must be 0..255");
                    }
                    if !self.at(&Tok::Dot) && !self.at_literal_bracket() {
                        return self.err(&format!(
                            "{name} is a bus qualifier; read a signal as \
                             {name}.Signal or {name}[\"Signal Name\"]"
                        ));
                    }
                    let signal = self.member("a signal name after '.'")?;
                    return Ok(Expr::Call(
                        "fr_sig_named".to_string(),
                        vec![Expr::Int(bus as i64), Expr::Str(signal)],
                    ));
                }
                // `ETHn.Frame.Signal` / `ETHn["Frame"]["Signal Name"]`: two
                // names after the bus, both literal so the host can verify
                // them against the ARXML at start.
                if let Some(bus) = bus_qualifier(&name, "ETH") {
                    if bus > u32::from(u8::MAX) {
                        return self.err("the Ethernet bus index must be 0..255");
                    }
                    if !self.at(&Tok::Dot) && !self.at_literal_bracket() {
                        return self.err(&format!(
                            "{name} is a bus qualifier; read a signal as \
                             {name}.Frame.Signal or {name}[\"Frame\"][\"Signal Name\"]"
                        ));
                    }
                    let frame = self.member("a frame name after '.'")?;
                    let signal = self.member("a signal name after the frame name")?;
                    return Ok(Expr::Call(
                        "eth_sig".to_string(),
                        vec![Expr::Int(bus as i64), Expr::Str(frame), Expr::Str(signal)],
                    ));
                }
                // `Message.Signal` keeps both names as literals so the host
                // can resolve and check them against the node's DBC at start.
                // A non-literal bracket index still falls through to ordinary
                // byte-buffer indexing.
                if self.at(&Tok::Dot) || self.at_literal_bracket() {
                    let signal = self.member("a signal name after '.'")?;
                    return Ok(Expr::Call(
                        "sig_named".to_string(),
                        vec![Expr::Str(name), Expr::Str(signal)],
                    ));
                }
                Ok(Expr::Ident(name))
            }
            Some(Tok::LParen) => {
                self.advance();
                let e = self.expr()?;
                self.expect(&Tok::RParen, "')'")?;
                Ok(e)
            }
            // The old CAPL-style accessors were removed in favour of the
            // dot/bracket forms and the plain functions. Name the replacement
            // so a legacy script fails with a migration hint, not a scanner
            // mystery.
            Some(Tok::Dollar) => self.err(
                "$Message::Signal was removed; use Message.Signal, or \
                 Message[\"Signal Name\"] when the name is not an identifier",
            ),
            Some(Tok::At) => self.err(
                "@sysvar:: was removed; use sys_get(\"ns::name\") and \
                 sys_set(\"ns::name\", value)",
            ),
            _ => self.err("expected an expression"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn expr_of(src: &str) -> Expr {
        let toks = super::super::lexer::lex(src).unwrap();
        let mut p = P {
            toks,
            pos: 0,
            fn_depth: 0,
            switch_temps: 0,
        };
        p.expr().unwrap()
    }

    #[test]
    fn a_dotted_read_desugars_into_the_named_signal_form() {
        let script = crate::script::compile("on start { print(EngineStatus.RPM); }").unwrap();
        assert_eq!(
            script.named_signal_refs,
            vec![("EngineStatus".to_string(), "RPM".to_string())],
            "the host resolves the pair against the node's DBC"
        );
        let Expr::Call(name, args) = expr_of("EngineStatus.RPM") else {
            panic!("a dotted read must compile to a host call");
        };
        assert_eq!(name, "sig_named");
        assert!(
            matches!(
                args.as_slice(),
                [Expr::Str(message), Expr::Str(signal)]
                    if message == "EngineStatus" && signal == "RPM"
            ),
            "the names stay literal for the host's DBC resolution"
        );
    }

    #[test]
    fn bus_qualified_reads_desugar_into_host_calls() {
        let script = crate::script::compile(
            r#"
                on start {
                    print(FR1.CarSpeed);
                    print(FR1["Car Speed"]);
                    print(ETH0.FrameA.Speed);
                    print(ETH0["Frame A"]["Speed X"]);
                }
            "#,
        )
        .unwrap();
        assert_eq!(
            script.fr_named_refs,
            vec![(1, "CarSpeed".to_string()), (1, "Car Speed".to_string())]
        );
        assert_eq!(
            script.eth_named_refs,
            vec![
                (0, "FrameA".to_string(), "Speed".to_string()),
                (0, "Frame A".to_string(), "Speed X".to_string()),
            ]
        );
        let Expr::Call(name, args) = expr_of("FR1.CarSpeed") else {
            panic!("a FlexRay read must compile to a host call");
        };
        assert_eq!(name, "fr_sig_named");
        assert!(
            matches!(
                args.as_slice(),
                [Expr::Int(bus), Expr::Str(signal)]
                    if *bus == 1 && signal == "CarSpeed"
            ),
            "the bus and name stay literal for the host's description lookup"
        );
        let Expr::Call(name, args) = expr_of("ETH0.FrameA.Speed") else {
            panic!("an Ethernet read must compile to a host call");
        };
        assert_eq!(name, "eth_sig");
        assert!(matches!(
            args.as_slice(),
            [Expr::Int(0), Expr::Str(frame), Expr::Str(signal)]
                if frame == "FrameA" && signal == "Speed"
        ));
    }

    #[test]
    fn named_cluster_reads_desugar_into_host_calls() {
        let script = crate::script::compile(
            r#"
                on start {
                    print(FR.PowerTrain.CarSpeed);
                    print(FR["Power Train"]["Car Speed"]);
                    print(ETH.Body.FrameA.Speed);
                    print(ETH["Body"]["Frame A"]["Speed X"]);
                }
            "#,
        )
        .unwrap();
        assert_eq!(
            script.fr_bus_refs,
            vec![
                ("PowerTrain".to_string(), "CarSpeed".to_string()),
                ("Power Train".to_string(), "Car Speed".to_string()),
            ]
        );
        assert_eq!(
            script.eth_bus_refs,
            vec![
                (
                    "Body".to_string(),
                    "FrameA".to_string(),
                    "Speed".to_string()
                ),
                (
                    "Body".to_string(),
                    "Frame A".to_string(),
                    "Speed X".to_string()
                ),
            ]
        );
        let Expr::Call(name, _) = expr_of("FR.PowerTrain.CarSpeed") else {
            panic!("a named-cluster read must compile to a host call");
        };
        assert_eq!(name, "fr_sig_bus");
        let Expr::Call(name, args) = expr_of("ETH.Body.FrameA.Speed") else {
            panic!("a named-cluster Ethernet read must compile to a host call");
        };
        assert_eq!(name, "eth_sig_bus");
        assert!(matches!(
            args.as_slice(),
            [Expr::Str(bus), Expr::Str(frame), Expr::Str(signal)]
                if bus == "Body" && frame == "FrameA" && signal == "Speed"
        ));
    }

    /// `FR0` on its own is a bus qualifier, not a variable: say what to
    /// write instead of failing later at codegen.
    #[test]
    fn a_bus_qualifier_alone_names_the_required_form() {
        for (src, want) in [
            ("FR0", "FR0.Signal"),
            ("ETH0", "ETH0.Frame.Signal"),
            ("FR300.Speed", "0..255"),
            ("ETH256.Frame.Speed", "0..255"),
        ] {
            let error = crate::script::compile(&format!("on start {{ print({src}); }}"))
                .unwrap_err()
                .to_string();
            assert!(error.contains(want), "{src}: {error}");
        }
    }

    #[test]
    fn bus_qualified_writes_point_at_the_write_api() {
        for src in [
            "FR0.Speed = 1;",
            "FR0[\"Speed\"] = 1;",
            "ETH0.FrameA.Speed = 1;",
            "ETH0[\"Frame A\"][\"Speed\"] = 1;",
        ] {
            let error = crate::script::compile(&format!("on start {{ {src} }}"))
                .unwrap_err()
                .to_string();
            assert!(
                error.contains("read-only") && error.contains("set_fr_sig"),
                "{src}: {error}"
            );
        }
    }

    #[test]
    fn this_describes_the_triggering_frame() {
        for (src, want) in [
            ("this.bus", "frame_bus"),
            ("this.id", "frame_can_id"),
            ("this.slot", "frame_slot"),
            ("this.len", "frame_dlc"),
            ("this.cycle", "frame_cycle"),
            ("this.signal(\"RPM\")", "frame_signal"),
            ("this[\"Signal Name\"]", "frame_signal"),
            ("this[0]", "frame_byte"),
            ("node.bus", "node_bus"),
        ] {
            let Expr::Call(name, _) = expr_of(src) else {
                panic!("{src} must compile to a host call");
            };
            assert_eq!(name, want, "{src}");
        }
        for src in [
            "this",
            "this.nope",
            "node.nope",
            "node[\"x\"]",
            "this.signal(0)",
        ] {
            let error = crate::script::compile(&format!("on start {{ print({src}); }}"))
                .unwrap_err()
                .to_string();
            assert!(
                error.contains("this") || error.contains("node"),
                "{src}: {error}"
            );
        }
    }

    /// `.` resolves its left name against a context or a database, so those
    /// names must not be shadowed by the script's own declarations.
    #[test]
    fn context_and_bus_names_cannot_be_declared() {
        for (src, want) in [
            ("let this = 1;", "`this`"),
            ("let node = 1;", "`node`"),
            ("let FR = 1;", "`FR`"),
            ("let FR0 = 1;", "`FR0`"),
            ("let ETH2 = 1;", "`ETH2`"),
            ("fn ETH() {}", "`ETH`"),
            ("fn f(FR0) {}", "`FR0`"),
        ] {
            let error = crate::script::compile(src).unwrap_err().to_string();
            assert!(error.contains(want), "{src}: {error}");
        }
        assert!(
            crate::script::compile(
                "let Frame = 1; let Ethernet = 2; fn f(Frame) { return Frame; }"
            )
            .is_ok(),
            "only the exact qualifier shapes are reserved"
        );
        let error = crate::script::compile("on start { print(EngineStatus::RPM); }")
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("`::` is not an operator") && error.contains("Message.Signal"),
            "{error}"
        );
    }

    #[test]
    fn a_dotted_write_points_at_set_sig() {
        let error = crate::script::compile("on start { EngineStatus.RPM = 1; }")
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("set_sig") && error.contains("read-only"),
            "{error}"
        );
    }

    #[test]
    fn precedence_shapes_the_tree() {
        // 1 + 2 * 3 => Add(1, Mul(2, 3))
        let e = expr_of("1 + 2 * 3");
        match e {
            Expr::Binary(BinOp::Add, l, r) => {
                assert!(matches!(*l, Expr::Int(1)));
                assert!(matches!(*r, Expr::Binary(BinOp::Mul, _, _)));
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn comparison_binds_looser_than_arithmetic() {
        let e = expr_of("1 + 1 == 2");
        assert!(matches!(e, Expr::Binary(BinOp::Eq, _, _)));
    }

    #[test]
    fn call_parses_arguments() {
        let e = expr_of("f(1, x)");
        match e {
            Expr::Call(name, args) => {
                assert_eq!(name, "f");
                assert_eq!(args.len(), 2);
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn for_is_desugared_with_optional_slots() {
        let toks = super::super::lexer::lex("for (let i = 0; i < 3; i = i + 1) { }").unwrap();
        let mut p = P {
            toks,
            pos: 0,
            fn_depth: 0,
            switch_temps: 0,
        };
        match p.stmt().unwrap().stmt {
            Stmt::For {
                init: Some(_),
                cond: Some(_),
                step: Some(_),
                body,
            } => assert!(body.is_empty()),
            other => panic!("{other:?}"),
        }
        let toks = super::super::lexer::lex("for (; ;) { }").unwrap();
        let mut p = P {
            toks,
            pos: 0,
            fn_depth: 0,
            switch_temps: 0,
        };
        match p.stmt().unwrap().stmt {
            Stmt::For {
                init: None,
                cond: None,
                step: None,
                ..
            } => {}
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn return_outside_a_function_is_rejected() {
        let toks = super::super::lexer::lex("return 1;").unwrap();
        let mut p = P {
            toks,
            pos: 0,
            fn_depth: 0,
            switch_temps: 0,
        };
        assert!(p.stmt().is_err());
    }

    /// The C ladder: shift binds tighter than comparison, bitwise or/and
    /// sit between logical and and equality.
    #[test]
    fn bitwise_precedence_follows_the_c_ladder() {
        let e = expr_of("1 & 2 == 2");
        match e {
            // `1 & (2 == 2)` -- comparison binds tighter than `&`, and
            // the parser does not fold constants.
            Expr::Binary(BinOp::BitAnd, _, r) => {
                assert!(matches!(*r, Expr::Binary(BinOp::Eq, _, _)));
            }
            other => panic!("{other:?}"),
        }
        let e = expr_of("1 + 1 << 2");
        match e {
            // `(1 + 1) << 2` -- shift binds looser than addition.
            Expr::Binary(BinOp::Shl, l, _) => {
                assert!(matches!(*l, Expr::Binary(BinOp::Add, _, _)));
            }
            other => panic!("{other:?}"),
        }
        let e = expr_of("a | b ^ c & d");
        match e {
            // `a | (b ^ (c & d))`.
            Expr::Binary(BinOp::BitOr, _, r) => match *r {
                Expr::Binary(BinOp::BitXor, _, r) => {
                    assert!(matches!(*r, Expr::Binary(BinOp::BitAnd, _, _)));
                }
                other => panic!("{other:?}"),
            },
            other => panic!("{other:?}"),
        }
    }

    /// Compound stores desugar to plain assignment with the operator on
    /// the left, including the buffer-index form.
    #[test]
    fn compound_assignment_desugars() {
        let toks = super::super::lexer::lex("x += 1;").unwrap();
        let mut p = P {
            toks,
            pos: 0,
            fn_depth: 0,
            switch_temps: 0,
        };
        match p.stmt().unwrap().stmt {
            Stmt::Assign(name, Expr::Binary(BinOp::Add, l, r)) => {
                assert_eq!(name, "x");
                assert!(matches!(*l, Expr::Ident(ref n) if n == "x"));
                assert!(matches!(*r, Expr::Int(1)));
            }
            other => panic!("{other:?}"),
        }
        let toks = super::super::lexer::lex("buf[0] |= 0x80;").unwrap();
        let mut p = P {
            toks,
            pos: 0,
            fn_depth: 0,
            switch_temps: 0,
        };
        match p.stmt().unwrap().stmt {
            Stmt::AssignIndex(name, _, Expr::Binary(BinOp::BitOr, l, _)) => {
                assert_eq!(name, "buf");
                assert!(matches!(*l, Expr::Index(_, _)));
            }
            other => panic!("{other:?}"),
        }
    }

    /// `switch` desugars to an if-chain: a plain identifier subject is
    /// compared directly, anything else is captured in a hidden temp
    /// declared before the chain.
    #[test]
    fn switch_desugars_to_a_conditional_chain() {
        let toks = super::super::lexer::lex(
            "switch (state) { case 1: { print(1); } case 2: { print(2); } default: { print(0); } }",
        )
        .unwrap();
        let mut p = P {
            toks,
            pos: 0,
            fn_depth: 0,
            switch_temps: 0,
        };
        let stmt = p.stmt().unwrap().stmt;
        let Stmt::Block(stmts) = stmt else {
            panic!("a block wrapping the chain")
        };
        assert_eq!(stmts.len(), 1, "identifier subject needs no temp");
        let Stmt::If { cond, then, els } = &stmts[0].stmt else {
            panic!("the chain head is an if")
        };
        assert!(matches!(cond, Expr::Binary(BinOp::Eq, _, _)));
        assert_eq!(then.len(), 1);
        assert!(els.is_some(), "the second case nests under else");
        let els = els.as_ref().unwrap();
        let Stmt::If { els, .. } = &els[0].stmt else {
            panic!("the chain continues")
        };
        let els = els.as_ref().unwrap();
        assert!(matches!(els[0].stmt, Stmt::Block(_)), "default lands last");

        // A call subject gets exactly one hidden capture.
        let toks =
            super::super::lexer::lex("switch (frame_byte(0)) { case 1: { } case 2: { } }").unwrap();
        let mut p = P {
            toks,
            pos: 0,
            fn_depth: 0,
            switch_temps: 0,
        };
        let Stmt::Block(stmts) = p.stmt().unwrap().stmt else {
            panic!("a block")
        };
        assert!(
            matches!(stmts[0].stmt, Stmt::Let(ref n, _) if n.starts_with("__switch")),
            "the hidden temp declares the subject"
        );
        assert_eq!(stmts.len(), 2, "capture plus the chain head");
    }

    /// Bracket access is the escape hatch for DBC names that are not
    /// identifiers; it desugars exactly like the dot form.
    #[test]
    fn a_bracket_read_handles_non_identifier_signal_names() {
        let script =
            crate::script::compile("on start { print(EngineStatus[\"Engine Speed\"]); }").unwrap();
        assert_eq!(
            script.named_signal_refs,
            vec![("EngineStatus".to_string(), "Engine Speed".to_string())]
        );
        let Expr::Call(name, args) = expr_of("EngineStatus[\"Engine Speed\"]") else {
            panic!("a bracket read must compile to a host call");
        };
        assert_eq!(name, "sig_named");
        assert!(
            matches!(
                args.as_slice(),
                [Expr::Str(message), Expr::Str(signal)]
                    if message == "EngineStatus" && signal == "Engine Speed"
            ),
            "the names stay literal for the host's DBC resolution"
        );
    }

    #[test]
    fn a_bracket_write_points_at_set_sig() {
        let error = crate::script::compile("on start { EngineStatus[\"RPM\"] = 1; }")
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("set_sig") && error.contains("read-only"),
            "{error}"
        );
    }

    #[test]
    fn timer_units_compile_to_whole_milliseconds() {
        let script =
            crate::script::compile("on timer 1s { } on timer 250ms { } on timer 1.5s { }").unwrap();
        let periods: Vec<u64> = script
            .handlers
            .iter()
            .filter_map(|handler| match handler.kind {
                crate::script::HandlerKind::Timer { period_ms } => Some(period_ms),
                _ => None,
            })
            .collect();
        assert_eq!(periods, vec![1_000, 250, 1_500]);

        let Expr::Call(_, args) = expr_of("set_timer(\"resp\", 1.5s)") else {
            panic!("a call");
        };
        assert!(matches!(&args[1], Expr::Int(1_500)));
        let Expr::Call(_, args) = expr_of("set_period(500ms)") else {
            panic!("a call");
        };
        assert!(matches!(&args[0], Expr::Int(500)));

        for source in [
            "on timer 0.5ms { }",
            "on start { set_timer(\"x\", 0.5ms); }",
            "on start { let t = 1s; }",
            "on start { print(ramp(0, 1, 1s)); }",
        ] {
            assert!(
                crate::script::compile(source).is_err(),
                "{source} must not compile"
            );
        }
    }

    #[test]
    fn a_named_message_handler_takes_an_identifier_or_a_string() {
        let script =
            crate::script::compile("on message EngineStatus { } on message \"Engine Cmd\" { }")
                .unwrap();
        assert_eq!(
            script.handlers[0].kind,
            crate::script::HandlerKind::NamedMessage {
                name: "EngineStatus".to_string()
            }
        );
        assert_eq!(
            script.handlers[1].kind,
            crate::script::HandlerKind::NamedMessage {
                name: "Engine Cmd".to_string()
            }
        );
        let error =
            crate::script::compile("on message EngineStatus { } on message EngineStatus { }")
                .unwrap_err()
                .to_string();
        assert!(error.contains("duplicate handler"), "{error}");
    }

    /// The CAPL-style spellings are gone; the errors name the modern forms.
    #[test]
    fn legacy_accessor_forms_point_at_their_replacements() {
        for source in [
            "on start { print($EngineStatus::RPM); }",
            "on start { $EngineStatus::RPM = 5; }",
        ] {
            let error = crate::script::compile(source).unwrap_err().to_string();
            assert!(error.contains("Message.Signal"), "{source}: {error}");
        }
        for source in [
            "on start { print(@sysvar::Demo::Setpoint); }",
            "on start { @sysvar::Demo::Setpoint = 5; }",
        ] {
            let error = crate::script::compile(source).unwrap_err().to_string();
            assert!(error.contains("sys_get"), "{source}: {error}");
        }
    }
}
