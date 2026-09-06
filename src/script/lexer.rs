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
    Assign,
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
                        if chars.get(i + 1) == Some(&'=') {
                            (Tok::Le, 2)
                        } else {
                            (Tok::Lt, 1)
                        }
                    }
                    '>' => {
                        if chars.get(i + 1) == Some(&'=') {
                            (Tok::Ge, 2)
                        } else {
                            (Tok::Gt, 1)
                        }
                    }
                    '&' if chars.get(i + 1) == Some(&'&') => (Tok::And, 2),
                    '|' if chars.get(i + 1) == Some(&'|') => (Tok::Or, 2),
                    '+' => (Tok::Plus, 1),
                    '-' => (Tok::Minus, 1),
                    '*' => (Tok::Star, 1),
                    '/' => (Tok::Slash, 1),
                    '%' => (Tok::Percent, 1),
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
}
