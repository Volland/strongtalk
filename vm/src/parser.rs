//! Recursive-descent parser for Strongtalk methods and do-its.

use crate::ast::*;
use crate::lexer::{tokenize, SyntaxError, Tok, Token};
use std::rc::Rc;

pub struct Parser {
    toks: Vec<Token>,
    i: usize,
}

type PResult<T> = Result<T, SyntaxError>;

impl Parser {
    pub fn new(src: &str) -> PResult<Parser> {
        Ok(Parser { toks: tokenize(src)?, i: 0 })
    }

    fn peek(&self) -> &Tok {
        &self.toks[self.i].tok
    }
    fn peek_at(&self, k: usize) -> &Tok {
        let j = (self.i + k).min(self.toks.len() - 1);
        &self.toks[j].tok
    }
    fn pos(&self) -> usize {
        self.toks[self.i].pos
    }
    fn next(&mut self) -> Tok {
        let t = self.toks[self.i].tok.clone();
        if self.i < self.toks.len() - 1 {
            self.i += 1;
        }
        t
    }
    fn err<T>(&self, msg: &str) -> PResult<T> {
        Err(SyntaxError { msg: msg.to_string(), pos: self.pos() })
    }
    fn is_bin(&self, s: &str) -> bool {
        matches!(self.peek(), Tok::Binary(b) if b == s)
    }
    fn at_type_open(&self) -> bool {
        self.is_bin("<")
    }
    fn expect(&mut self, t: Tok, what: &str) -> PResult<()> {
        if *self.peek() == t {
            self.next();
            Ok(())
        } else {
            self.err(&format!("{} expected", what))
        }
    }

    // ---------- types ----------

    /// Parses `<Type>`; the current token must be `<`.
    pub fn parse_type_annotation(&mut self) -> PResult<TypeExpr> {
        self.next(); // <
        let t = self.parse_type()?;
        if self.is_bin(">") {
            self.next();
            Ok(t)
        } else {
            self.err("'>' expected to close the type")
        }
    }

    fn parse_type(&mut self) -> PResult<TypeExpr> {
        let mut alts = vec![self.parse_type_term()?];
        while self.is_bin("|") {
            self.next();
            alts.push(self.parse_type_term()?);
        }
        Ok(if alts.len() == 1 { alts.pop().unwrap() } else { TypeExpr::Union(alts) })
    }

    fn parse_type_term(&mut self) -> PResult<TypeExpr> {
        let mut t = match self.peek().clone() {
            Tok::Ident(n) => {
                self.next();
                if *self.peek() == Tok::LBrack {
                    self.next();
                    let mut args = Vec::new();
                    if *self.peek() != Tok::RBrack {
                        args.push(self.parse_type()?);
                        while self.is_bin(",") {
                            self.next();
                            args.push(self.parse_type()?);
                        }
                    }
                    self.expect(Tok::RBrack, "']'")?;
                    TypeExpr::Generic(n, args)
                } else {
                    TypeExpr::Name(n)
                }
            }
            Tok::LBrack => {
                self.next();
                let mut args = Vec::new();
                let mut ret = None;
                while *self.peek() != Tok::RBrack {
                    if *self.peek() == Tok::Caret {
                        self.next();
                        ret = Some(Box::new(self.parse_type()?));
                    } else if *self.peek() == Tok::Colon {
                        self.next();
                        args.push(self.parse_type()?);
                    } else {
                        args.push(self.parse_type()?);
                    }
                    if self.is_bin(",") {
                        self.next();
                    } else if *self.peek() != Tok::RBrack {
                        return self.err("',' or ']' expected in block type");
                    }
                }
                self.next();
                TypeExpr::Block(args, ret)
            }
            Tok::LParen => {
                self.next();
                let t = self.parse_type()?;
                self.expect(Tok::RParen, "')'")?;
                t
            }
            _ => return self.err("type name expected"),
        };
        while matches!(self.peek(), Tok::Ident(n) if n == "class") {
            self.next();
            t = TypeExpr::Meta(Box::new(t));
        }
        Ok(t)
    }

    fn opt_type(&mut self) -> PResult<Option<TypeExpr>> {
        if self.at_type_open() {
            Ok(Some(self.parse_type_annotation()?))
        } else {
            Ok(None)
        }
    }

    // ---------- methods ----------

    pub fn parse_method(&mut self) -> PResult<MethodAst> {
        let mut params = Vec::new();
        let selector;
        match self.peek().clone() {
            Tok::Ident(n) => {
                self.next();
                selector = n;
            }
            Tok::Binary(b) => {
                self.next();
                let p = self.pos();
                let name = self.ident("argument name")?;
                let ty = self.opt_type()?;
                params.push(Var { name, ty, pos: p });
                selector = b;
            }
            Tok::Keyword(_) => {
                let mut sel = String::new();
                while let Tok::Keyword(k) = self.peek().clone() {
                    self.next();
                    sel.push_str(&k);
                    let p = self.pos();
                    let name = self.ident("argument name")?;
                    let ty = self.opt_type()?;
                    params.push(Var { name, ty, pos: p });
                }
                selector = sel;
            }
            _ => return self.err("method header expected"),
        }
        let mut ret = None;
        if *self.peek() == Tok::Caret && matches!(self.peek_at(1), Tok::Binary(b) if b == "<") {
            self.next();
            ret = Some(self.parse_type_annotation()?);
        }
        // Strongtalk inference clauses `{where ...}` are accepted and ignored.
        if *self.peek() == Tok::LBrace && matches!(self.peek_at(1), Tok::Ident(w) if w == "where") {
            while !matches!(self.peek(), Tok::RBrace | Tok::Eof) {
                self.next();
            }
            self.next();
        }
        let header_end = self.toks[self.i.saturating_sub(1)].end;
        let temps = self.parse_temps()?;
        let body = self.parse_statements()?;
        if *self.peek() != Tok::Eof {
            return self.err("end of method expected (missing '.'?)");
        }
        Ok(MethodAst { selector, params, ret, temps, body, header_end })
    }

    /// A do-it: optional temporaries and statements.
    pub fn parse_doit(&mut self) -> PResult<(Vec<Var>, Vec<Stmt>)> {
        let temps = self.parse_temps()?;
        let body = self.parse_statements()?;
        if *self.peek() != Tok::Eof {
            return self.err("end of expression expected (missing '.'?)");
        }
        Ok((temps, body))
    }

    fn ident(&mut self, what: &str) -> PResult<String> {
        match self.peek().clone() {
            Tok::Ident(n) => {
                self.next();
                Ok(n)
            }
            _ => self.err(&format!("{} expected", what)),
        }
    }

    fn parse_temps(&mut self) -> PResult<Vec<Var>> {
        let mut temps = Vec::new();
        if self.is_bin("|") {
            self.next();
            while let Tok::Ident(n) = self.peek().clone() {
                let p = self.pos();
                self.next();
                let ty = self.opt_type()?;
                temps.push(Var { name: n, ty, pos: p });
            }
            if !self.is_bin("|") {
                return self.err("'|' expected to end the temporaries");
            }
            self.next();
        }
        Ok(temps)
    }

    fn parse_statements(&mut self) -> PResult<Vec<Stmt>> {
        let mut out = Vec::new();
        loop {
            while *self.peek() == Tok::Dot {
                self.next();
            }
            match self.peek() {
                Tok::Eof | Tok::RBrack | Tok::RPrim => break,
                Tok::Caret => {
                    let p = self.pos();
                    self.next();
                    let e = self.parse_expr()?;
                    out.push(Stmt::Return(e, p));
                    while *self.peek() == Tok::Dot {
                        self.next();
                    }
                    break;
                }
                _ => {
                    let e = self.parse_expr()?;
                    out.push(Stmt::Expr(e));
                    if *self.peek() == Tok::Dot {
                        continue;
                    }
                    break;
                }
            }
        }
        Ok(out)
    }

    fn parse_expr(&mut self) -> PResult<Expr> {
        if let (Tok::Ident(n), Tok::Assign) = (self.peek().clone(), self.peek_at(1).clone()) {
            let p = self.pos();
            self.next();
            self.next();
            let v = self.parse_expr()?;
            return Ok(Expr::Assign(n, Box::new(v), p));
        }
        let e = self.parse_keyword_expr()?;
        if *self.peek() != Tok::Semi {
            return Ok(e);
        }
        let (recv, first) = match e {
            Expr::Send { recv, sel, args, pos } => (recv, (sel, args, pos)),
            _ => return self.err("cascade needs a message before ';'"),
        };
        let mut msgs = vec![first];
        while *self.peek() == Tok::Semi {
            self.next();
            msgs.push(self.parse_cascade_msg()?);
        }
        Ok(Expr::Cascade { recv, msgs })
    }

    fn parse_cascade_msg(&mut self) -> PResult<(String, Vec<Expr>, usize)> {
        let p = self.pos();
        match self.peek().clone() {
            Tok::Ident(n) => {
                self.next();
                Ok((n, vec![], p))
            }
            Tok::Binary(b) => {
                self.next();
                let a = self.parse_unary_expr()?;
                Ok((b, vec![a], p))
            }
            Tok::Keyword(_) => {
                let mut sel = String::new();
                let mut args = Vec::new();
                while let Tok::Keyword(k) = self.peek().clone() {
                    self.next();
                    sel.push_str(&k);
                    args.push(self.parse_binary_expr()?);
                }
                Ok((sel, args, p))
            }
            _ => self.err("message expected after ';'"),
        }
    }

    fn parse_keyword_expr(&mut self) -> PResult<Expr> {
        let recv = self.parse_binary_expr()?;
        if let Tok::Keyword(_) = self.peek() {
            let p = self.pos();
            let mut sel = String::new();
            let mut args = Vec::new();
            while let Tok::Keyword(k) = self.peek().clone() {
                self.next();
                sel.push_str(&k);
                args.push(self.parse_binary_expr()?);
            }
            return Ok(Expr::Send { recv: Box::new(recv), sel, args, pos: p });
        }
        Ok(recv)
    }

    fn parse_binary_expr(&mut self) -> PResult<Expr> {
        let mut e = self.parse_unary_expr()?;
        while let Tok::Binary(b) = self.peek().clone() {
            let p = self.pos();
            self.next();
            let a = self.parse_unary_expr()?;
            e = Expr::Send { recv: Box::new(e), sel: b, args: vec![a], pos: p };
        }
        Ok(e)
    }

    fn parse_unary_expr(&mut self) -> PResult<Expr> {
        let mut e = self.parse_primary()?;
        while let Tok::Ident(n) = self.peek().clone() {
            let p = self.pos();
            self.next();
            e = Expr::Send { recv: Box::new(e), sel: n, args: vec![], pos: p };
        }
        Ok(e)
    }

    fn parse_primary(&mut self) -> PResult<Expr> {
        let p = self.pos();
        match self.peek().clone() {
            Tok::Ident(n) => {
                self.next();
                if n == "guaranteed" && self.at_type_open() {
                    let t = self.parse_type_annotation()?;
                    let e = self.parse_unary_expr()?;
                    return Ok(Expr::Guaranteed(t, Box::new(e), p));
                }
                Ok(match n.as_str() {
                    "nil" => Expr::Lit(Lit::Nil, p),
                    "true" => Expr::Lit(Lit::True, p),
                    "false" => Expr::Lit(Lit::False, p),
                    _ => Expr::Var(n, p),
                })
            }
            Tok::Int(v) => {
                self.next();
                Ok(Expr::Lit(Lit::Int(v), p))
            }
            Tok::Big(v) => {
                self.next();
                Ok(Expr::Lit(Lit::Big(v), p))
            }
            Tok::Float(v) => {
                self.next();
                Ok(Expr::Lit(Lit::Float(v), p))
            }
            Tok::Char(c) => {
                self.next();
                Ok(Expr::Lit(Lit::Char(c), p))
            }
            Tok::Str(s) => {
                self.next();
                Ok(Expr::Lit(Lit::Str(s), p))
            }
            Tok::Sym(s) => {
                self.next();
                Ok(Expr::Lit(Lit::Sym(s), p))
            }
            Tok::LitArray => {
                self.next();
                Ok(Expr::Lit(self.parse_lit_array_body()?, p))
            }
            Tok::ByteArray => {
                self.next();
                Ok(Expr::Lit(self.parse_byte_array_body()?, p))
            }
            Tok::LParen => {
                self.next();
                let e = self.parse_expr()?;
                self.expect(Tok::RParen, "')'")?;
                Ok(e)
            }
            Tok::LBrack => Ok(Expr::Block(Rc::new(self.parse_block()?))),
            Tok::LBrace => {
                self.next();
                let mut items = Vec::new();
                loop {
                    while *self.peek() == Tok::Dot {
                        self.next();
                    }
                    if *self.peek() == Tok::RBrace {
                        self.next();
                        break;
                    }
                    items.push(self.parse_expr()?);
                    match self.peek() {
                        Tok::Dot => {}
                        Tok::RBrace => {}
                        _ => return self.err("'.' or '}' expected"),
                    }
                }
                Ok(Expr::Brace(items, p))
            }
            Tok::LPrim => {
                self.next();
                let e = self.parse_prim(p)?;
                self.expect(Tok::RPrim, "'}}'")?;
                Ok(e)
            }
            Tok::Eof => self.err("expression expected"),
            Tok::Binary(b) if b == "<" => self.err("unexpected '<' (a type annotation is only allowed after a variable name)"),
            _ => self.err("expression expected"),
        }
    }

    fn parse_prim(&mut self, p: usize) -> PResult<Expr> {
        let recv = match self.peek() {
            Tok::Keyword(_) => None,
            Tok::Ident(n) if n.starts_with("primitive") => None,
            _ => Some(Box::new(self.parse_primary()?)),
        };
        let mut sel = String::new();
        let mut args = Vec::new();
        match self.peek().clone() {
            Tok::Ident(n) => {
                self.next();
                sel = n;
            }
            Tok::Keyword(_) => {
                while let Tok::Keyword(k) = self.peek().clone() {
                    self.next();
                    sel.push_str(&k);
                    args.push(self.parse_binary_expr()?);
                }
            }
            _ => return self.err("primitive name expected"),
        }
        Ok(Expr::Prim { recv, sel, args, pos: p })
    }

    fn parse_block(&mut self) -> PResult<BlockAst> {
        let p = self.pos();
        self.next(); // [
        let mut params = Vec::new();
        while *self.peek() == Tok::Colon {
            self.next();
            let pp = self.pos();
            let name = self.ident("block argument name")?;
            let ty = self.opt_type()?;
            params.push(Var { name, ty, pos: pp });
        }
        let mut ret = None;
        if *self.peek() == Tok::Caret && matches!(self.peek_at(1), Tok::Binary(b) if b == "<") {
            self.next();
            ret = Some(self.parse_type_annotation()?);
        }
        if !params.is_empty() || ret.is_some() {
            if self.is_bin("|") {
                self.next();
            } else if *self.peek() != Tok::RBrack {
                return self.err("'|' expected after block arguments");
            }
        }
        let temps = self.parse_temps()?;
        let body = self.parse_statements()?;
        self.expect(Tok::RBrack, "']'")?;
        Ok(BlockAst { params, temps, ret, body, pos: p })
    }

    fn parse_lit_array_body(&mut self) -> PResult<Lit> {
        let mut items = Vec::new();
        loop {
            let t = self.next();
            let lit = match t {
                Tok::RParen => break,
                Tok::Eof => return self.err("')' expected to end the literal array"),
                Tok::Int(v) => Lit::Int(v),
                Tok::Big(v) => Lit::Big(v),
                Tok::Float(v) => Lit::Float(v),
                Tok::Char(c) => Lit::Char(c),
                Tok::Str(s) => Lit::Str(s),
                Tok::Sym(s) => Lit::Sym(s),
                Tok::Ident(n) => match n.as_str() {
                    "nil" => Lit::Nil,
                    "true" => Lit::True,
                    "false" => Lit::False,
                    _ => Lit::Sym(n),
                },
                Tok::Keyword(k) => {
                    let mut s = k;
                    while let Tok::Keyword(k2) = self.peek().clone() {
                        self.next();
                        s.push_str(&k2);
                    }
                    Lit::Sym(s)
                }
                Tok::Binary(b) => {
                    if b == "-" {
                        match self.peek().clone() {
                            Tok::Int(v) => {
                                self.next();
                                Lit::Int(-v)
                            }
                            Tok::Float(v) => {
                                self.next();
                                Lit::Float(-v)
                            }
                            Tok::Big(v) => {
                                self.next();
                                Lit::Big(-v)
                            }
                            _ => Lit::Sym(b),
                        }
                    } else {
                        Lit::Sym(b)
                    }
                }
                Tok::LParen | Tok::LitArray => self.parse_lit_array_body()?,
                Tok::ByteArray | Tok::LBrack => self.parse_byte_array_body()?,
                Tok::Caret => Lit::Sym("^".into()),
                _ => return self.err("literal expected in literal array"),
            };
            items.push(lit);
        }
        Ok(Lit::Array(items))
    }

    fn parse_byte_array_body(&mut self) -> PResult<Lit> {
        let mut bytes = Vec::new();
        loop {
            match self.next() {
                Tok::RBrack => break,
                Tok::Int(v) if (0..=255).contains(&v) => bytes.push(v as u8),
                _ => return self.err("byte (0-255) expected in byte array"),
            }
        }
        Ok(Lit::Bytes(bytes))
    }
}

/// Parses an instance-variable declaration string such as `'x <Int> y <Number> z'`.
pub fn parse_var_decls(src: &str) -> PResult<Vec<Var>> {
    let mut p = Parser::new(src)?;
    let mut out = Vec::new();
    loop {
        match p.peek().clone() {
            Tok::Eof => break,
            Tok::Ident(n) => {
                let pos = p.pos();
                p.next();
                let ty = p.opt_type()?;
                out.push(Var { name: n, ty, pos });
            }
            Tok::Binary(b) if b == "," => {
                p.next();
            }
            _ => return p.err("variable name expected"),
        }
    }
    Ok(out)
}

/// Parses a type written without the angle brackets (`Collection[E]`).
pub fn parse_type_text(src: &str) -> PResult<TypeExpr> {
    let mut p = Parser::new(src)?;
    let t = p.parse_type()?;
    if *p.peek() != Tok::Eof {
        return p.err("end of type expected");
    }
    Ok(t)
}

/// Splits a method chunk's selector out without a full parse (used for messages).
pub fn quick_selector(src: &str) -> Option<String> {
    let mut p = Parser::new(src).ok()?;
    p.parse_method().ok().map(|m| m.selector)
}
