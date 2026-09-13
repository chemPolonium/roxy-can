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

    fn fn_decl(&mut self) -> Result<FnDecl, ScriptError> {
        self.expect(&Tok::Fn, "'fn'")?;
        let name = self.ident("function name")?;
        self.expect(&Tok::LParen, "'('")?;
        let mut params = Vec::new();
        if !self.at(&Tok::RParen) {
            loop {
                params.push(self.ident("parameter name")?);
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
        let word = self.ident("'start', 'message', 'timer', 'extended' or 'errorFrame'")?;
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
                if self.eat(&Tok::Star) {
                    OnKind::AnyMessage
                } else {
                    let id = self.id_literal()?;
                    OnKind::Message { id }
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
            other => {
                return self.err(&format!(
                    "unknown event '{other}' (start, message, extended message, timer, errorFrame)"
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

    /// A timer period in milliseconds: a positive integer.
    fn period_literal(&mut self) -> Result<u64, ScriptError> {
        let at = self.toks.get(self.pos);
        let line = at.map_or(1, |t| t.line);
        let col = at.map_or(1, |t| t.col);
        match self.toks.get(self.pos).map(|t| t.tok.clone()) {
            Some(Tok::Int(n)) if n > 0 => {
                self.advance();
                Ok(n as u64)
            }
            Some(Tok::Int(0)) => Err(ScriptError {
                line,
                col: Some(col),
                msg: "timer period must be positive".to_string(),
            }),
            _ => self.err("expected a timer period in milliseconds"),
        }
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
        if self.eat(&Tok::Let) {
            let name = self.ident("variable name")?;
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
        let name = self.ident("variable name")?;
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
                let lhs = Expr::Index(
                    Box::new(Expr::Ident(name.clone())),
                    Box::new(idx.clone()),
                );
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
            let cond = Expr::Binary(
                BinOp::Eq,
                Box::new(compare.clone()),
                Box::new(label),
            );
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
            let name = self.ident("variable name")?;
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
                            args.push(self.expr()?);
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
                _ => break,
            }
        }
        Ok(e)
    }

    fn primary(&mut self) -> Result<Expr, ScriptError> {
        match self.toks.get(self.pos).map(|t| t.tok.clone()) {
            Some(Tok::Int(n)) => {
                self.advance();
                Ok(Expr::Int(n))
            }
            Some(Tok::Float(x)) => {
                self.advance();
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
                Ok(Expr::Ident(name))
            }
            Some(Tok::LParen) => {
                self.advance();
                let e = self.expr()?;
                self.expect(&Tok::RParen, "')'")?;
                Ok(e)
            }
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
        let toks = super::super::lexer::lex(
            "switch (frame_byte(0)) { case 1: { } case 2: { } }",
        )
        .unwrap();
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
}
