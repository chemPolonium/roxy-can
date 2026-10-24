//! Derived channels: one expression over other signals, sampled on the bus
//! timeline by the core. The grammar matches the web editor: numbers,
//! `[signal]` / `[Bus:signal]` references, `+ - * /`, comparisons, and
//! `abs` / `sqrt` / `min` / `max`.
use serde::{Deserialize, Serialize};

/// One reference the editor resolved when the definition was saved: the text
/// as it appears inside `[...]` and the exact series key it named. Pinning the
/// key here keeps the core's choice identical to the editor's, even when a
/// second bus later carries a signal with the same name.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct DerivedSource {
    pub name: String,
    pub key: String,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct DerivedDef {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub unit: String,
    pub expression: String,
    /// State observer band cuts, ascending. Empty means no bands.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub cuts: Vec<f64>,
    /// One name per band (`cuts.len() + 1` when present).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub names: Vec<String>,
    /// Reference text -> series key, resolved by the editor against the chart
    /// it was watching. Older projects have none and fall back to name
    /// resolution in the core.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sources: Vec<DerivedSource>,
}

impl DerivedDef {
    pub fn has_rule(&self) -> bool {
        !self.cuts.is_empty()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Op {
    Add,
    Sub,
    Mul,
    Div,
    Gt,
    Lt,
    Ge,
    Le,
    Eq,
    Ne,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Expr {
    Num(f64),
    Ref(String),
    Neg(Box<Expr>),
    Binary(Op, Box<Expr>, Box<Expr>),
    Call(String, Vec<Expr>),
}

#[derive(Clone, Debug, PartialEq)]
enum Token {
    Num(f64),
    Ref(String),
    Name(String),
    Op(Op),
    LParen,
    RParen,
    Comma,
    End,
}

fn tokenize(source: &str) -> Result<Vec<Token>, String> {
    let bytes = source.as_bytes();
    let mut out = Vec::new();
    let mut i = 0usize;
    while i < bytes.len() {
        let ch = bytes[i] as char;
        if ch.is_ascii_whitespace() {
            i += 1;
            continue;
        }
        let pair = source.get(i..i + 2).unwrap_or("");
        let op = match pair {
            ">=" => Some(Op::Ge),
            "<=" => Some(Op::Le),
            "==" => Some(Op::Eq),
            "!=" => Some(Op::Ne),
            _ => None,
        };
        if let Some(op) = op {
            out.push(Token::Op(op));
            i += 2;
            continue;
        }
        let single = match ch {
            '+' => Some(Op::Add),
            '-' => Some(Op::Sub),
            '*' => Some(Op::Mul),
            '/' => Some(Op::Div),
            '>' => Some(Op::Gt),
            '<' => Some(Op::Lt),
            _ => None,
        };
        if let Some(op) = single {
            out.push(Token::Op(op));
            i += 1;
            continue;
        }
        match ch {
            '(' => {
                out.push(Token::LParen);
                i += 1;
                continue;
            }
            ')' => {
                out.push(Token::RParen);
                i += 1;
                continue;
            }
            ',' => {
                out.push(Token::Comma);
                i += 1;
                continue;
            }
            '[' => {
                let Some(end) = source[i + 1..].find(']') else {
                    return Err("missing ]".to_string());
                };
                let name = source[i + 1..i + 1 + end].trim();
                if name.is_empty() {
                    return Err("empty [signal]".to_string());
                }
                out.push(Token::Ref(name.to_string()));
                i += end + 2;
                continue;
            }
            _ => {}
        }
        if ch.is_ascii_digit() || ch == '.' {
            let start = i;
            let mut digits = 0usize;
            while i < bytes.len() && bytes[i].is_ascii_digit() {
                i += 1;
                digits += 1;
            }
            if i < bytes.len() && bytes[i] == b'.' {
                i += 1;
                while i < bytes.len() && bytes[i].is_ascii_digit() {
                    i += 1;
                    digits += 1;
                }
            }
            if digits == 0 {
                return Err(format!("unexpected '{ch}'"));
            }
            let text = &source[start..i];
            let value = text
                .parse::<f64>()
                .map_err(|_| format!("invalid number '{text}'"))?;
            out.push(Token::Num(value));
            continue;
        }
        if ch.is_ascii_alphabetic() || ch == '_' {
            let start = i;
            i += 1;
            while i < bytes.len()
                && ((bytes[i] as char).is_ascii_alphanumeric() || bytes[i] == b'_')
            {
                i += 1;
            }
            out.push(Token::Name(source[start..i].to_string()));
            continue;
        }
        return Err(format!("unexpected '{ch}'"));
    }
    out.push(Token::End);
    Ok(out)
}

struct Parser {
    tokens: Vec<Token>,
    index: usize,
}

impl Parser {
    fn peek(&self) -> &Token {
        &self.tokens[self.index]
    }

    fn take(&mut self, expected: Token) -> Result<(), String> {
        if self.peek() == &expected {
            self.index += 1;
            Ok(())
        } else {
            Err(format!("expected {expected:?}"))
        }
    }

    fn parse(mut self) -> Result<Expr, String> {
        let node = self.expression()?;
        if self.peek() != &Token::End {
            return Err("unexpected trailing input".to_string());
        }
        Ok(node)
    }

    fn expression(&mut self) -> Result<Expr, String> {
        let mut node = self.additive()?;
        while let Token::Op(op @ (Op::Gt | Op::Lt | Op::Ge | Op::Le | Op::Eq | Op::Ne)) =
            self.peek()
        {
            let op = *op;
            self.index += 1;
            node = Expr::Binary(op, Box::new(node), Box::new(self.additive()?));
        }
        Ok(node)
    }

    fn additive(&mut self) -> Result<Expr, String> {
        let mut node = self.term()?;
        while let Token::Op(op @ (Op::Add | Op::Sub)) = self.peek() {
            let op = *op;
            self.index += 1;
            node = Expr::Binary(op, Box::new(node), Box::new(self.term()?));
        }
        Ok(node)
    }

    fn term(&mut self) -> Result<Expr, String> {
        let mut node = self.factor()?;
        while let Token::Op(op @ (Op::Mul | Op::Div)) = self.peek() {
            let op = *op;
            self.index += 1;
            node = Expr::Binary(op, Box::new(node), Box::new(self.factor()?));
        }
        Ok(node)
    }

    fn factor(&mut self) -> Result<Expr, String> {
        match self.peek().clone() {
            Token::Op(Op::Sub) => {
                self.index += 1;
                Ok(Expr::Neg(Box::new(self.factor()?)))
            }
            Token::Num(value) => {
                self.index += 1;
                Ok(Expr::Num(value))
            }
            Token::Ref(name) => {
                self.index += 1;
                Ok(Expr::Ref(name))
            }
            Token::LParen => {
                self.index += 1;
                let node = self.expression()?;
                self.take(Token::RParen)?;
                Ok(node)
            }
            Token::Name(name) => {
                self.index += 1;
                self.take(Token::LParen)?;
                let mut args = Vec::new();
                if self.peek() != &Token::RParen {
                    args.push(self.expression()?);
                    while self.peek() == &Token::Comma {
                        self.index += 1;
                        args.push(self.expression()?);
                    }
                }
                self.take(Token::RParen)?;
                Ok(Expr::Call(name, args))
            }
            _ => Err("expected a value".to_string()),
        }
    }
}

fn check_calls(expr: &Expr) -> Result<(), String> {
    match expr {
        Expr::Call(name, args) => {
            if !matches!(name.as_str(), "abs" | "sqrt" | "min" | "max") {
                return Err(format!("unknown function {name}()"));
            }
            if name != "min" && name != "max" && args.len() != 1 {
                return Err(format!("{name}() takes one argument"));
            }
            if args.is_empty() {
                return Err(format!("{name}() takes arguments"));
            }
            for arg in args {
                check_calls(arg)?;
            }
            Ok(())
        }
        Expr::Neg(value) => check_calls(value),
        Expr::Binary(_, left, right) => {
            check_calls(left)?;
            check_calls(right)
        }
        Expr::Num(_) | Expr::Ref(_) => Ok(()),
    }
}

fn collect_refs<'a>(expr: &'a Expr, out: &mut Vec<&'a str>) {
    match expr {
        Expr::Ref(name) => {
            if !out.contains(&name.as_str()) {
                out.push(name);
            }
        }
        Expr::Neg(value) => collect_refs(value, out),
        Expr::Binary(_, left, right) => {
            collect_refs(left, out);
            collect_refs(right, out);
        }
        Expr::Call(_, args) => {
            for arg in args {
                collect_refs(arg, out);
            }
        }
        Expr::Num(_) => {}
    }
}

pub fn references(expr: &Expr) -> Vec<&str> {
    let mut out = Vec::new();
    collect_refs(expr, &mut out);
    out
}

/// Parses and validates one expression. A definition has to reference at
/// least one signal; a constant lane would just be a line at a value.
pub fn parse(source: &str) -> Result<Expr, String> {
    let expr = Parser {
        tokens: tokenize(source)?,
        index: 0,
    }
    .parse()?;
    check_calls(&expr)?;
    if references(&expr).is_empty() {
        return Err("reference at least one [signal]".to_string());
    }
    Ok(expr)
}

/// Evaluates against the values the caller resolves for each reference
/// text. `None` from the closure or a failed operation (division by zero,
/// `sqrt` of a negative, a non-finite result) drops the sample.
pub fn eval(expr: &Expr, value: &mut impl FnMut(&str) -> Option<f64>) -> Option<f64> {
    let result = match expr {
        Expr::Num(value) => *value,
        Expr::Ref(name) => value(name)?,
        Expr::Neg(inner) => -eval(inner, value)?,
        Expr::Call(name, args) => {
            let values: Vec<f64> = args
                .iter()
                .map(|arg| eval(arg, value))
                .collect::<Option<Vec<_>>>()?;
            match name.as_str() {
                "abs" => values[0].abs(),
                "sqrt" if values[0] >= 0.0 => values[0].sqrt(),
                "min" => values.iter().copied().fold(f64::INFINITY, f64::min),
                "max" => values.iter().copied().fold(f64::NEG_INFINITY, f64::max),
                _ => return None,
            }
        }
        Expr::Binary(op, left, right) => {
            let left = eval(left, value)?;
            let right = eval(right, value)?;
            match op {
                Op::Add => left + right,
                Op::Sub => left - right,
                Op::Mul => left * right,
                Op::Div if right != 0.0 => left / right,
                Op::Div => return None,
                Op::Gt => (left > right) as i32 as f64,
                Op::Lt => (left < right) as i32 as f64,
                Op::Ge => (left >= right) as i32 as f64,
                Op::Le => (left <= right) as i32 as f64,
                Op::Eq => (left == right) as i32 as f64,
                Op::Ne => (left != right) as i32 as f64,
            }
        }
    };
    result.is_finite().then_some(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(source: &str, values: &[(&str, f64)]) -> Option<f64> {
        let expr = parse(source).expect("expression parses");
        eval(&expr, &mut |name| {
            values
                .iter()
                .find(|(candidate, _)| *candidate == name)
                .map(|(_, value)| *value)
        })
    }

    #[test]
    fn arithmetic_and_operators_use_the_web_grammar() {
        assert_eq!(run("[A] + 2 * [B]", &[("A", 1.0), ("B", 3.0)]), Some(7.0));
        assert_eq!(run("-[A] / 4", &[("A", 10.0)]), Some(-2.5));
        assert_eq!(run("[A] > 1.5", &[("A", 2.0)]), Some(1.0));
        assert_eq!(run("[A] != 2", &[("A", 2.0)]), Some(0.0));
    }

    #[test]
    fn calls_and_failures_drop_the_sample() {
        assert_eq!(
            run("max([A], [B], 5)", &[("A", 1.0), ("B", 4.0)]),
            Some(5.0)
        );
        assert_eq!(run("sqrt([A])", &[("A", 9.0)]), Some(3.0));
        assert_eq!(run("sqrt([A])", &[("A", -1.0)]), None);
        assert_eq!(run("[A] / [B]", &[("A", 1.0), ("B", 0.0)]), None);
        assert_eq!(run("[A] / [B]", &[("A", 1.0)]), None);
    }

    #[test]
    fn reference_order_and_validation_are_stable() {
        let expr = parse("[B] + [A] * [B]").unwrap();
        assert_eq!(references(&expr), vec!["B", "A"]);
        assert_eq!(
            parse("1 + 2").unwrap_err(),
            "reference at least one [signal]"
        );
        assert!(parse("abs([A], [B])").is_err());
        assert!(parse("[A").is_err());
        assert!(parse("[A] +").is_err());
        assert_eq!(parse("[A]").unwrap(), Expr::Ref("A".to_string()));
    }

    #[test]
    fn a_definition_round_trips_through_json() {
        let def = DerivedDef {
            id: "d1".into(),
            name: "Error".into(),
            unit: "km/h".into(),
            expression: "[A] - [B]".into(),
            cuts: vec![10.0],
            names: vec!["ok".into(), "high".into()],
            sources: Vec::new(),
        };
        let text = serde_json::to_string(&def).unwrap();
        let back: DerivedDef = serde_json::from_str(&text).unwrap();
        assert_eq!(back, def);
    }
}
