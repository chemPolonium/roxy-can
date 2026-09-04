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
    Stmt(Stmt),
}

/// An event handler declaration. The body runs when the node runtime (S2)
/// delivers the event; here it only needs to compile.
#[derive(Debug)]
pub struct OnDecl {
    pub kind: OnKind,
    pub body: Vec<Stmt>,
    pub line: u32,
}

#[derive(Debug)]
pub enum OnKind {
    /// Measurement start, once.
    Start,
    /// A frame with this identifier arrived on the node's channel.
    Message { id: u32 },
    /// A periodic tick every `period_ms` milliseconds.
    Timer { period_ms: u64 },
}

#[derive(Debug)]
pub struct FnDecl {
    pub name: String,
    pub params: Vec<String>,
    pub body: Vec<Stmt>,
}

#[derive(Debug)]
pub enum Stmt {
    Let(String, Expr),
    Assign(String, Expr),
    If {
        cond: Expr,
        then: Vec<Stmt>,
        els: Option<Vec<Stmt>>,
    },
    While {
        cond: Expr,
        body: Vec<Stmt>,
    },
    /// Desugared `for`: init, condition (None = true), step, body.
    For {
        init: Option<Box<Stmt>>,
        cond: Option<Expr>,
        step: Option<Box<Stmt>>,
        body: Vec<Stmt>,
    },
    Return(Option<Expr>),
    Block(Vec<Stmt>),
    Expr(Expr),
}

#[derive(Debug)]
pub enum Expr {
    Int(i64),
    Float(f64),
    Bool(bool),
    Str(String),
    Ident(String),
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
}

pub fn parse(toks: Vec<Token>) -> Result<Program, ScriptError> {
    let mut p = P {
        toks,
        pos: 0,
        fn_depth: 0,
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
}

impl P {
    fn err<T>(&self, msg: &str) -> Result<T, ScriptError> {
        let line = self.toks.get(self.pos).map_or(1, |t| t.line);
        Err(ScriptError {
            line,
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
            let line = self.toks.get(self.pos).map_or(1, |t| t.line);
            self.err(&format!(
                "expected {what}, found {:?}",
                self.toks.get(self.pos).map(|t| &t.tok)
            ))
            .map_err(|mut e| {
                e.line = line;
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
        let line = self.toks.get(self.pos).map_or(1, |t| t.line);
        self.expect(&Tok::On, "'on'")?;
        let word = self.ident("'start', 'message' or 'timer'")?;
        let kind = match word.as_str() {
            "start" => OnKind::Start,
            "message" => {
                let id = self.id_literal()?;
                OnKind::Message { id }
            }
            "timer" => {
                let period_ms = self.period_literal()?;
                OnKind::Timer { period_ms }
            }
            other => {
                return self.err(&format!("unknown event '{other}' (start, message, timer)"));
            }
        };
        self.fn_depth += 1;
        let body = self.block()?;
        self.fn_depth -= 1;
        Ok(OnDecl { kind, body, line })
    }

    /// A CAN identifier: a non-negative integer fitting in 29 bits of a
    /// standard id (extended ids come with extended frame support).
    fn id_literal(&mut self) -> Result<u32, ScriptError> {
        let line = self.toks.get(self.pos).map_or(1, |t| t.line);
        match self.toks.get(self.pos).map(|t| t.tok.clone()) {
            Some(Tok::Int(n)) if (0..=0x7FF).contains(&n) => {
                self.advance();
                Ok(n as u32)
            }
            Some(Tok::Int(n)) => Err(ScriptError {
                line,
                msg: format!("id {n:#x} out of the standard 11-bit range"),
            }),
            _ => self.err("expected a message id"),
        }
    }

    /// A timer period in milliseconds: a positive integer.
    fn period_literal(&mut self) -> Result<u64, ScriptError> {
        let line = self.toks.get(self.pos).map_or(1, |t| t.line);
        match self.toks.get(self.pos).map(|t| t.tok.clone()) {
            Some(Tok::Int(n)) if n > 0 => {
                self.advance();
                Ok(n as u64)
            }
            Some(Tok::Int(0)) => Err(ScriptError {
                line,
                msg: "timer period must be positive".to_string(),
            }),
            _ => self.err("expected a timer period in milliseconds"),
        }
    }

    fn block(&mut self) -> Result<Vec<Stmt>, ScriptError> {
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

    fn stmt(&mut self) -> Result<Stmt, ScriptError> {
        if self.eat(&Tok::Let) {
            let name = self.ident("variable name")?;
            self.expect(&Tok::Assign, "'=' in a let")?;
            let expr = self.expr()?;
            self.expect(&Tok::Semi, "';'")?;
            return Ok(Stmt::Let(name, expr));
        }
        if self.eat(&Tok::If) {
            return self.if_stmt();
        }
        if self.eat(&Tok::While) {
            self.expect(&Tok::LParen, "'('")?;
            let cond = self.expr()?;
            self.expect(&Tok::RParen, "')'")?;
            let body = self.block()?;
            return Ok(Stmt::While { cond, body });
        }
        if self.eat(&Tok::For) {
            return self.for_stmt();
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
        // Assignment or a bare expression, told apart by the next token.
        if matches!(self.toks.get(self.pos).map(|t| &t.tok), Some(Tok::Ident(_)))
            && matches!(
                self.toks.get(self.pos + 1).map(|t| &t.tok),
                Some(Tok::Assign)
            )
        {
            let name = self.ident("variable name")?;
            self.expect(&Tok::Assign, "'='")?;
            let expr = self.expr()?;
            self.expect(&Tok::Semi, "';'")?;
            return Ok(Stmt::Assign(name, expr));
        }
        let expr = self.expr()?;
        self.expect(&Tok::Semi, "';'")?;
        Ok(Stmt::Expr(expr))
    }

    fn if_stmt(&mut self) -> Result<Stmt, ScriptError> {
        self.expect(&Tok::LParen, "'('")?;
        let cond = self.expr()?;
        self.expect(&Tok::RParen, "')'")?;
        let then = self.block()?;
        let els = if self.eat(&Tok::Else) {
            if self.at(&Tok::If) {
                Some(vec![self.if_stmt()?])
            } else {
                Some(self.block()?)
            }
        } else {
            None
        };
        Ok(Stmt::If { cond, then, els })
    }

    fn for_stmt(&mut self) -> Result<Stmt, ScriptError> {
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
        Ok(Stmt::For {
            init,
            cond,
            step,
            body,
        })
    }

    /// A statement without the trailing `;` -- only what a for-header
    /// slot accepts: a let, an assignment, or an expression.
    fn simple_stmt(&mut self) -> Result<Stmt, ScriptError> {
        if self.eat(&Tok::Let) {
            let name = self.ident("variable name")?;
            self.expect(&Tok::Assign, "'=' in a let")?;
            return Ok(Stmt::Let(name, self.expr()?));
        }
        if matches!(self.toks.get(self.pos).map(|t| &t.tok), Some(Tok::Ident(_)))
            && matches!(
                self.toks.get(self.pos + 1).map(|t| &t.tok),
                Some(Tok::Assign)
            )
        {
            let name = self.ident("variable name")?;
            self.expect(&Tok::Assign, "'='")?;
            return Ok(Stmt::Assign(name, self.expr()?));
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
        let mut lhs = self.eq_expr()?;
        while self.eat(&Tok::And) {
            let rhs = self.eq_expr()?;
            lhs = Expr::Binary(BinOp::And, Box::new(lhs), Box::new(rhs));
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
        let mut lhs = self.term()?;
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
        let e = self.primary()?;
        let Expr::Ident(name) = e else {
            return Ok(e);
        };
        if !self.at(&Tok::LParen) {
            return Ok(Expr::Ident(name));
        }
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
        Ok(Expr::Call(name, args))
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
        };
        match p.stmt().unwrap() {
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
        };
        match p.stmt().unwrap() {
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
        };
        assert!(p.stmt().is_err());
    }
}
