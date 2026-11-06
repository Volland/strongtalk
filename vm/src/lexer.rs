//! Tokenizer for Strongtalk source: Smalltalk-80 syntax plus `<Type>` annotations,
//! `^<Type>` return types and `{{primitive...}}` calls.

use num_bigint::BigInt;
use num_traits::Num;

#[derive(Debug, Clone, PartialEq)]
pub enum Tok {
    Ident(String),
    Keyword(String),
    Binary(String),
    Int(i64),
    Big(BigInt),
    Float(f64),
    Char(char),
    Str(String),
    Sym(String),
    LitArray,  // #(
    ByteArray, // #[
    Assign,
    Caret,
    Colon,
    Dot,
    Semi,
    LParen,
    RParen,
    LBrack,
    RBrack,
    LBrace,
    RBrace,
    LPrim,
    RPrim,
    Eof,
}

#[derive(Debug, Clone)]
pub struct Token {
    pub tok: Tok,
    pub pos: usize,
    pub end: usize,
}

#[derive(Debug, Clone)]
pub struct SyntaxError {
    pub msg: String,
    pub pos: usize,
}

pub fn is_binary_char(c: char) -> bool {
    matches!(c, '+' | '-' | '*' | '/' | '\\' | '<' | '>' | '=' | '~' | '@' | '%' | '&' | '?' | ',')
}

pub fn tokenize(src: &str) -> Result<Vec<Token>, SyntaxError> {
    let chars: Vec<(usize, char)> = src.char_indices().collect();
    let n = chars.len();
    let mut i = 0usize;
    let mut out: Vec<Token> = Vec::new();
    let at = |i: usize| -> char { if i < n { chars[i].1 } else { '\0' } };
    let off = |i: usize| -> usize { if i < n { chars[i].0 } else { src.len() } };

    while i < n {
        let c = at(i);
        if c.is_whitespace() {
            i += 1;
            continue;
        }
        if c == '"' {
            let start = i;
            i += 1;
            while i < n && at(i) != '"' {
                i += 1;
            }
            if i >= n {
                return Err(SyntaxError { msg: "unterminated comment".into(), pos: off(start) });
            }
            i += 1;
            continue;
        }
        let start = i;
        let tok: Tok;
        if c.is_alphabetic() || c == '_' && at(i + 1).is_alphanumeric() {
            while i < n && (at(i).is_alphanumeric() || at(i) == '_') {
                i += 1;
            }
            if at(i) == ':' && at(i + 1) != '=' {
                i += 1;
                tok = Tok::Keyword(collect(&chars, start, i));
            } else {
                tok = Tok::Ident(collect(&chars, start, i));
            }
        } else if c.is_ascii_digit()
            || (c == '-' && at(i + 1).is_ascii_digit() && !prev_is_operand(&out))
        {
            let (t, ni) = lex_number(&chars, i)?;
            tok = t;
            i = ni;
        } else if c == '$' {
            if i + 1 >= n {
                return Err(SyntaxError { msg: "character expected after $".into(), pos: off(i) });
            }
            tok = Tok::Char(at(i + 1));
            i += 2;
        } else if c == '\'' {
            let (s, ni) = lex_string(&chars, i, src)?;
            tok = Tok::Str(s);
            i = ni;
        } else if c == '#' {
            let d = at(i + 1);
            if d == '(' {
                tok = Tok::LitArray;
                i += 2;
            } else if d == '[' {
                tok = Tok::ByteArray;
                i += 2;
            } else if d == '\'' {
                let (s, ni) = lex_string(&chars, i + 1, src)?;
                tok = Tok::Sym(s);
                i = ni;
            } else if d.is_alphabetic() || d == '_' {
                i += 1;
                let s0 = i;
                while i < n && (at(i).is_alphanumeric() || at(i) == '_' || at(i) == ':') {
                    i += 1;
                }
                tok = Tok::Sym(collect(&chars, s0, i));
            } else if is_binary_char(d) || d == '|' {
                i += 1;
                let s0 = i;
                while i < n && (is_binary_char(at(i)) || at(i) == '|') {
                    i += 1;
                }
                tok = Tok::Sym(collect(&chars, s0, i));
            } else if d == '#' {
                // ##foo  is the same as #foo
                i += 1;
                continue;
            } else {
                return Err(SyntaxError { msg: "bad literal after #".into(), pos: off(i) });
            }
        } else if c == ':' {
            if at(i + 1) == '=' {
                tok = Tok::Assign;
                i += 2;
            } else {
                tok = Tok::Colon;
                i += 1;
            }
        } else if c == '_' {
            tok = Tok::Assign;
            i += 1;
        } else if c == '^' {
            tok = Tok::Caret;
            i += 1;
        } else if c == '.' {
            tok = Tok::Dot;
            i += 1;
        } else if c == ';' {
            tok = Tok::Semi;
            i += 1;
        } else if c == '(' {
            tok = Tok::LParen;
            i += 1;
        } else if c == ')' {
            tok = Tok::RParen;
            i += 1;
        } else if c == '[' {
            tok = Tok::LBrack;
            i += 1;
        } else if c == ']' {
            tok = Tok::RBrack;
            i += 1;
        } else if c == '{' {
            if at(i + 1) == '{' {
                tok = Tok::LPrim;
                i += 2;
            } else {
                tok = Tok::LBrace;
                i += 1;
            }
        } else if c == '}' {
            if at(i + 1) == '}' && prim_open(&out) {
                tok = Tok::RPrim;
                i += 2;
            } else {
                tok = Tok::RBrace;
                i += 1;
            }
        } else if c == '|' {
            tok = Tok::Binary("|".into());
            i += 1;
        } else if is_binary_char(c) {
            while i < n && is_binary_char(at(i)) {
                // a '-' followed by a digit after another operator starts a negative literal
                if i > start && at(i) == '-' && at(i + 1).is_ascii_digit() {
                    break;
                }
                i += 1;
            }
            tok = Tok::Binary(collect(&chars, start, i));
        } else {
            return Err(SyntaxError { msg: format!("unexpected character '{}'", c), pos: off(i) });
        }
        out.push(Token { tok, pos: off(start), end: off(i) });
    }
    out.push(Token { tok: Tok::Eof, pos: src.len(), end: src.len() });
    Ok(out)
}

fn prim_open(out: &[Token]) -> bool {
    let mut depth = 0i32;
    for t in out {
        match t.tok {
            Tok::LPrim => depth += 1,
            Tok::RPrim => depth -= 1,
            _ => {}
        }
    }
    depth > 0
}

fn prev_is_operand(out: &[Token]) -> bool {
    match out.last() {
        None => false,
        Some(t) => matches!(
            t.tok,
            Tok::Ident(_)
                | Tok::Int(_)
                | Tok::Big(_)
                | Tok::Float(_)
                | Tok::Char(_)
                | Tok::Str(_)
                | Tok::Sym(_)
                | Tok::RParen
                | Tok::RBrack
                | Tok::RBrace
                | Tok::RPrim
        ),
    }
}

fn collect(chars: &[(usize, char)], a: usize, b: usize) -> String {
    chars[a..b].iter().map(|x| x.1).collect()
}

fn lex_string(chars: &[(usize, char)], start: usize, src: &str) -> Result<(String, usize), SyntaxError> {
    let n = chars.len();
    let mut i = start + 1;
    let mut s = String::new();
    loop {
        if i >= n {
            let pos = if start < n { chars[start].0 } else { src.len() };
            return Err(SyntaxError { msg: "unterminated string".into(), pos });
        }
        let c = chars[i].1;
        if c == '\'' {
            if i + 1 < n && chars[i + 1].1 == '\'' {
                s.push('\'');
                i += 2;
                continue;
            }
            i += 1;
            break;
        }
        s.push(c);
        i += 1;
    }
    Ok((s, i))
}

fn lex_number(chars: &[(usize, char)], start: usize) -> Result<(Tok, usize), SyntaxError> {
    let n = chars.len();
    let at = |i: usize| -> char { if i < n { chars[i].1 } else { '\0' } };
    let mut i = start;
    let neg = at(i) == '-';
    if neg {
        i += 1;
    }
    let ds = i;
    while at(i).is_ascii_digit() {
        i += 1;
    }
    let mut digits = collect(chars, ds, i);
    let mut radix = 10u32;
    if at(i) == 'r' {
        radix = digits.parse::<u32>().unwrap_or(10);
        if !(2..=36).contains(&radix) {
            return Err(SyntaxError { msg: "bad radix".into(), pos: chars[start].0 });
        }
        i += 1;
        let rs = i;
        while at(i).is_digit(radix) || (at(i).is_ascii_uppercase() && at(i).is_digit(radix)) {
            i += 1;
        }
        digits = collect(chars, rs, i);
    }
    let mut is_float = false;
    let mut frac = String::new();
    if radix == 10 && at(i) == '.' && at(i + 1).is_ascii_digit() {
        is_float = true;
        i += 1;
        let fs = i;
        while at(i).is_ascii_digit() {
            i += 1;
        }
        frac = collect(chars, fs, i);
    }
    let mut exp: i32 = 0;
    let mut has_exp = false;
    if matches!(at(i), 'e' | 'd' | 'q')
        && (at(i + 1).is_ascii_digit() || (at(i + 1) == '-' && at(i + 2).is_ascii_digit()))
    {
        has_exp = true;
        i += 1;
        let es = i;
        if at(i) == '-' {
            i += 1;
        }
        while at(i).is_ascii_digit() {
            i += 1;
        }
        exp = collect(chars, es, i).parse::<i32>().unwrap_or(0);
    }
    if is_float || (has_exp && exp < 0) {
        let text = format!("{}{}.{}e{}", if neg { "-" } else { "" }, digits, if frac.is_empty() { "0" } else { &frac }, exp);
        let f: f64 = text.parse().unwrap_or(0.0);
        return Ok((Tok::Float(f), i));
    }
    let mut big = BigInt::from_str_radix(&digits, radix)
        .map_err(|_| SyntaxError { msg: "bad number".into(), pos: chars[start].0 })?;
    if has_exp {
        big *= BigInt::from(10).pow(exp as u32);
    }
    if neg {
        big = -big;
    }
    match i64::try_from(&big) {
        Ok(v) => Ok((Tok::Int(v), i)),
        Err(_) => Ok((Tok::Big(big), i)),
    }
}
