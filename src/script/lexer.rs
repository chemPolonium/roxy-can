//! Tokenizer: source text to a flat token vector with line numbers.
//! Hand-rolled and single-pass; the language is small enough that a
//! lexer table would outlive its usefulness.

use super::ScriptError;

#[derive(Clone, Debug, PartialEq)]
pub enum Tok {
    Ident(String),
    Int(i64),
    Float(f64),
    Str(String),
    // Keywords.
    Let,
    Fn,
    If,
    Else,
    While,
    For,
    Return,
    Break,
    Continue,
    Switch,
    Case,
    Default,
    True,
    False,
    /// Event handler introducer (`on start` / `on message 0x100` /
    /// `on timer 100`). The event word itself stays an identifier so
    /// ordinary variables may still be called `message` or `timer`.
    On,
    // Punctuation and operators.
    LParen,
    RParen,
    LBrace,
    RBrace,
    LBracket,
    RBracket,
    Comma,
    Semi,
    Colon,
    /// `$Message::Signal` -- the CANoe-style signal read.
    Dollar,
    /// `@sysvar::ns::name` -- the CANoe-style system variable access.
    At,
    Assign,
    /// `+=` and friends: compound stores desugar to plain assignment.
    AssignAdd,
    AssignSub,
    AssignMul,
    AssignDiv,
    AssignMod,
    AssignBitAnd,
    AssignBitOr,
    AssignBitXor,
    AssignShl,
    AssignShr,
    Plus,
    Minus,
    Star,
    Slash,
    Percent,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    And,
    Or,
    Not,
    BitAnd,
    BitOr,
    BitXor,
    Shl,
    Shr,
    Eof,
}

#[derive(Clone, Debug)]
pub struct Token {
    pub tok: Tok,
    pub line: u32,
    /// 1-based character column of the token's first character.
    pub col: u32,
}

fn keyword(word: &str) -> Option<Tok> {
    Some(match word {
        "let" => Tok::Let,
        "fn" => Tok::Fn,
        "if" => Tok::If,
        "else" => Tok::Else,
        "while" => Tok::While,
        "for" => Tok::For,
        "return" => Tok::Return,
        "break" => Tok::Break,
        "continue" => Tok::Continue,
        "switch" => Tok::Switch,
        "case" => Tok::Case,
        "default" => Tok::Default,
        "true" => Tok::True,
        "false" => Tok::False,
        "on" => Tok::On,
        _ => return None,
    })
}

fn is_ident_start(c: char) -> bool {
    c.is_ascii_alphabetic() || c == '_'
}

fn is_ident_part(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

pub fn lex(src: &str) -> Result<Vec<Token>, ScriptError> {
    let chars: Vec<char> = src.chars().collect();
    let mut toks = Vec::new();
    let mut i = 0usize;
    let mut line = 1u32;
    let mut col = 1u32;
    let err = |line: u32, col: u32, msg: &str| ScriptError {
        line,
        col: Some(col),
        msg: msg.to_string(),
    };

    while i < chars.len() {
        let c = chars[i];
        let tok_col = col;
        match c {
            '\n' => {
                line += 1;
                col = 1;
                i += 1;
            }
            ' ' | '\t' | '\r' => {
                col += 1;
                i += 1;
            }
            '/' if i + 1 < chars.len() && chars[i + 1] == '/' => {
                while i < chars.len() && chars[i] != '\n' {
                    i += 1;
                }
            }
            '/' if i + 1 < chars.len() && chars[i + 1] == '*' => {
                let open_line = line;
                let open_col = col;
                col += 2;
                i += 2;
                loop {
                    match chars.get(i) {
                        None => return Err(err(open_line, open_col, "unterminated comment")),
                        Some('*') if chars.get(i + 1) == Some(&'/') => {
                            col += 2;
                            i += 2;
                            break;
                        }
                        Some('\n') => {
                            line += 1;
                            col = 1;
                            i += 1;
                        }
                        Some(_) => {
                            col += 1;
                            i += 1;
                        }
                    }
                }
            }
            '0'..='9' => {
                // Hex literals (0x1F) -- CAN identifiers are written in
                // hex everywhere, so the lexer speaks them natively.
                if c == '0' && matches!(chars.get(i + 1), Some('x') | Some('X')) {
                    let open_line = line;
                    let open_col = col;
                    col += 2;
                    i += 2;
                    let start = i;
                    while i < chars.len() && chars[i].is_ascii_hexdigit() {
                        col += 1;
                        i += 1;
                    }
                    let text: String = chars[start..i].iter().collect();
                    if text.is_empty() {
                        return Err(err(open_line, open_col, "malformed hex literal"));
                    }
                    let n = i64::from_str_radix(&text, 16)
                        .map_err(|_| err(open_line, open_col, "hex literal out of range"))?;
                    toks.push(Token {
                        tok: Tok::Int(n),
                        line,
                        col: tok_col,
                    });
                    continue;
                }
                let start = i;
                while i < chars.len() && chars[i].is_ascii_digit() {
                    col += 1;
                    i += 1;
                }
                let is_float =
                    i + 1 < chars.len() && chars[i] == '.' && chars[i + 1].is_ascii_digit();
                if is_float {
                    col += 1;
                    i += 1;
                    while i < chars.len() && chars[i].is_ascii_digit() {
                        col += 1;
                        i += 1;
                    }
                }
                let text: String = chars[start..i].iter().collect();
                let tok = if is_float {
                    Tok::Float(
                        text.parse::<f64>()
                            .map_err(|_| err(line, tok_col, "malformed float"))?,
                    )
                } else {
                    Tok::Int(
                        text.parse::<i64>()
                            .map_err(|_| err(line, tok_col, "integer out of range"))?,
                    )
                };
                toks.push(Token {
                    tok,
                    line,
                    col: tok_col,
                });
            }
            '"' => {
                let open_line = line;
                let open_col = col;
                col += 1;
                i += 1;
                let mut s = String::new();
                loop {
                    match chars.get(i) {
                        None => return Err(err(open_line, open_col, "unterminated string")),
                        Some('"') => {
                            col += 1;
                            i += 1;
                            break;
                        }
                        Some('\n') => return Err(err(line, col, "newline in string")),
                        Some('\\') => {
                            col += 1;
                            i += 1;
                            let esc = chars
                                .get(i)
                                .ok_or_else(|| err(open_line, open_col, "unterminated string"))?;
                            s.push(match esc {
                                'n' => '\n',
                                't' => '\t',
                                'r' => '\r',
                                '"' => '"',
                                '\\' => '\\',
                                other => {
                                    return Err(err(
                                        line,
                                        col,
                                        &format!("unknown escape '\\{other}'"),
                                    ));
                                }
                            });
                            col += 1;
                            i += 1;
                        }
                        Some(c) => {
                            s.push(*c);
                            col += 1;
                            i += 1;
                        }
                    }
                }
                toks.push(Token {
                    tok: Tok::Str(s),
                    line,
                    col: tok_col,
                });
            }
            c if is_ident_start(c) => {
                let start = i;
                while i < chars.len() && is_ident_part(chars[i]) {
                    col += 1;
                    i += 1;
                }
                let word: String = chars[start..i].iter().collect();
                let tok = keyword(&word).unwrap_or(Tok::Ident(word));
                toks.push(Token {
                    tok,
                    line,
                    col: tok_col,
                });
            }
            _ => {
                let (tok, len) = match c {
                    '(' => (Tok::LParen, 1),
                    ')' => (Tok::RParen, 1),
                    '{' => (Tok::LBrace, 1),
                    '}' => (Tok::RBrace, 1),
                    '[' => (Tok::LBracket, 1),
                    ']' => (Tok::RBracket, 1),
                    ',' => (Tok::Comma, 1),
                    ';' => (Tok::Semi, 1),
                    ':' => (Tok::Colon, 1),
                    '$' => (Tok::Dollar, 1),
                    '@' => (Tok::At, 1),
                    '=' => {
                        if chars.get(i + 1) == Some(&'=') {
                            (Tok::Eq, 2)
                        } else {
                            (Tok::Assign, 1)
                        }
                    }
                    '!' => {
                        if chars.get(i + 1) == Some(&'=') {
                            (Tok::Ne, 2)
                        } else {
                            (Tok::Not, 1)
                        }
                    }
                    '<' => {
                        // Longest match first: `<=`, `<<=`, `<<`, `<`.
                        if chars.get(i + 1) == Some(&'=') {
                            (Tok::Le, 2)
                        } else if chars.get(i + 1) == Some(&'<') {
                            if chars.get(i + 2) == Some(&'=') {
                                (Tok::AssignShl, 3)
                            } else {
                                (Tok::Shl, 2)
                            }
                        } else {
                            (Tok::Lt, 1)
                        }
                    }
                    '>' => {
                        // `>=`, `>>=`, `>>`, `>` -- longest first.
                        if chars.get(i + 1) == Some(&'=') {
                            (Tok::Ge, 2)
                        } else if chars.get(i + 1) == Some(&'>') {
                            if chars.get(i + 2) == Some(&'=') {
                                (Tok::AssignShr, 3)
                            } else {
                                (Tok::Shr, 2)
                            }
                        } else {
                            (Tok::Gt, 1)
                        }
                    }
                    // `&&`, `&=`, `&` -- the lone `&` is bitwise and.
                    '&' => match chars.get(i + 1) {
                        Some('&') => (Tok::And, 2),
                        Some('=') => (Tok::AssignBitAnd, 2),
                        _ => (Tok::BitAnd, 1),
                    },
                    '|' => match chars.get(i + 1) {
                        Some('|') => (Tok::Or, 2),
                        Some('=') => (Tok::AssignBitOr, 2),
                        _ => (Tok::BitOr, 1),
                    },
                    '^' => {
                        if chars.get(i + 1) == Some(&'=') {
                            (Tok::AssignBitXor, 2)
                        } else {
                            (Tok::BitXor, 1)
                        }
                    }
                    '+' => {
                        if chars.get(i + 1) == Some(&'=') {
                            (Tok::AssignAdd, 2)
                        } else {
                            (Tok::Plus, 1)
                        }
                    }
                    '-' => {
                        if chars.get(i + 1) == Some(&'=') {
                            (Tok::AssignSub, 2)
                        } else {
                            (Tok::Minus, 1)
                        }
                    }
                    '*' => {
                        if chars.get(i + 1) == Some(&'=') {
                            (Tok::AssignMul, 2)
                        } else {
                            (Tok::Star, 1)
                        }
                    }
                    '/' => {
                        if chars.get(i + 1) == Some(&'=') {
                            (Tok::AssignDiv, 2)
                        } else {
                            (Tok::Slash, 1)
                        }
                    }
                    '%' => {
                        if chars.get(i + 1) == Some(&'=') {
                            (Tok::AssignMod, 2)
                        } else {
                            (Tok::Percent, 1)
                        }
                    }
                    other => {
                        return Err(err(line, col, &format!("unexpected character '{other}'")));
                    }
                };
                col += len as u32;
                i += len;
                toks.push(Token {
                    tok,
                    line,
                    col: tok_col,
                });
            }
        }
    }
    toks.push(Token {
        tok: Tok::Eof,
        line,
        col,
    });
    Ok(toks)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(src: &str) -> Vec<Tok> {
        lex(src).unwrap().into_iter().map(|t| t.tok).collect()
    }

    #[test]
    fn tokens_in_order() {
        let toks = kinds("let x_1 = 1.5;\nx_1 = x_1 + 2;");
        assert_eq!(
            toks,
            vec![
                Tok::Let,
                Tok::Ident("x_1".into()),
                Tok::Assign,
                Tok::Float(1.5),
                Tok::Semi,
                Tok::Ident("x_1".into()),
                Tok::Assign,
                Tok::Ident("x_1".into()),
                Tok::Plus,
                Tok::Int(2),
                Tok::Semi,
                Tok::Eof,
            ]
        );
    }

    #[test]
    fn comments_and_escapes_and_lines() {
        let toks = lex("// gone\n/* multi\nline */ print(\"a\\nb\");").unwrap();
        assert_eq!(toks[0].line, 3);
        assert_eq!(toks[0].tok, Tok::Ident("print".into()));
        assert_eq!(toks[1].tok, Tok::LParen);
        assert_eq!(toks[2].tok, Tok::Str("a\nb".into()));
        assert_eq!(toks.last().unwrap().tok, Tok::Eof);
    }

    #[test]
    fn errors_carry_lines() {
        let e = lex("let s = \"oops").unwrap_err();
        assert_eq!(e.line, 1);
        let e = lex("let s = 1;\n/* never closed").unwrap_err();
        assert_eq!(e.line, 2);
    }

    #[test]
    fn tokens_carry_columns() {
        let toks = lex("  let x;\n    send(0x100);").unwrap();
        assert_eq!((toks[0].line, toks[0].col), (1, 3), "'let'");
        assert_eq!((toks[1].line, toks[1].col), (1, 7), "'x'");
        assert_eq!((toks[3].line, toks[3].col), (2, 5), "'send' after newline");
        assert_eq!((toks[5].line, toks[5].col), (2, 10), "hex literal");
        let e = lex("let s = \"oops").unwrap_err();
        assert_eq!(e.col, Some(9), "the offending string literal's column");
    }

    /// Longest match wins: the logical and compound forms shadow the
    /// single-character bitwise operators.
    #[test]
    fn bitwise_and_compound_operators_lex() {
        assert_eq!(
            kinds("a & b | c ^ d << e >> f"),
            vec![
                Tok::Ident("a".into()),
                Tok::BitAnd,
                Tok::Ident("b".into()),
                Tok::BitOr,
                Tok::Ident("c".into()),
                Tok::BitXor,
                Tok::Ident("d".into()),
                Tok::Shl,
                Tok::Ident("e".into()),
                Tok::Shr,
                Tok::Ident("f".into()),
                Tok::Eof,
            ]
        );
        assert_eq!(
            kinds("x += 1; x -= 2; x *= 3; x /= 4; x %= 5;"),
            vec![
                Tok::Ident("x".into()),
                Tok::AssignAdd,
                Tok::Int(1),
                Tok::Semi,
                Tok::Ident("x".into()),
                Tok::AssignSub,
                Tok::Int(2),
                Tok::Semi,
                Tok::Ident("x".into()),
                Tok::AssignMul,
                Tok::Int(3),
                Tok::Semi,
                Tok::Ident("x".into()),
                Tok::AssignDiv,
                Tok::Int(4),
                Tok::Semi,
                Tok::Ident("x".into()),
                Tok::AssignMod,
                Tok::Int(5),
                Tok::Semi,
                Tok::Eof,
            ]
        );
        assert_eq!(
            kinds("x &= a; x |= b; x ^= c; x <<= d; x >>= e;"),
            vec![
                Tok::Ident("x".into()),
                Tok::AssignBitAnd,
                Tok::Ident("a".into()),
                Tok::Semi,
                Tok::Ident("x".into()),
                Tok::AssignBitOr,
                Tok::Ident("b".into()),
                Tok::Semi,
                Tok::Ident("x".into()),
                Tok::AssignBitXor,
                Tok::Ident("c".into()),
                Tok::Semi,
                Tok::Ident("x".into()),
                Tok::AssignShl,
                Tok::Ident("d".into()),
                Tok::Semi,
                Tok::Ident("x".into()),
                Tok::AssignShr,
                Tok::Ident("e".into()),
                Tok::Semi,
                Tok::Eof,
            ]
        );
        // The two-character logical forms still beat the single ones.
        assert_eq!(kinds("a && b || c"), vec![Tok::Ident("a".into()), Tok::And, Tok::Ident("b".into()), Tok::Or, Tok::Ident("c".into()), Tok::Eof]);
        // The switch punctuation.
        assert_eq!(
            kinds("switch (x) { case 1: break; default: }"),
            vec![
                Tok::Switch,
                Tok::LParen,
                Tok::Ident("x".into()),
                Tok::RParen,
                Tok::LBrace,
                Tok::Case,
                Tok::Int(1),
                Tok::Colon,
                Tok::Break,
                Tok::Semi,
                Tok::Default,
                Tok::Colon,
                Tok::RBrace,
                Tok::Eof,
            ]
        );
    }
}
