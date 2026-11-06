//! Turns syntax trees into resolved executable nodes: variables become frame slots or
//! instance-variable indexes, and common control messages on literal blocks are inlined.

use crate::ast::*;
use crate::object::*;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

pub enum Node {
    Const(Value),
    LitArray(Rc<Lit>),
    SelfRef,
    Local(u16, u16),
    SetLocal(u16, u16, Box<Node>),
    IVar(usize),
    SetIVar(usize, Box<Node>),
    ClassIVar(Rc<str>),
    SetClassIVar(Rc<str>, Box<Node>),
    /// A global or class variable, looked up when evaluated.
    Name(Rc<str>, Option<Rc<Class>>),
    SetName(Rc<str>, Option<Rc<Class>>, Box<Node>),
    Ws(Rc<RefCell<HashMap<String, Value>>>, Rc<str>),
    SetWs(Rc<RefCell<HashMap<String, Value>>>, Rc<str>, Box<Node>),
    Send(Box<Node>, Sym, Vec<Node>),
    SuperSend(Rc<Class>, bool, Sym, Vec<Node>),
    Cascade(Box<Node>, Vec<(Sym, Vec<Node>)>),
    Block(Rc<BlockCode>),
    Seq(Vec<Node>),
    Return(Box<Node>),
    Brace(Vec<Node>),
    Prim(Sym, Option<Box<Node>>, Vec<Node>),
    If(Box<Node>, bool, Box<Node>, Option<Box<Node>>),
    IfNil(Box<Node>, bool, Option<(u16, u16)>, Box<Node>, Option<Box<Node>>),
    And(Box<Node>, Box<Node>),
    Or(Box<Node>, Box<Node>),
    While(Box<Node>, bool, Option<Box<Node>>),
    ToDo { from: Box<Node>, to: Box<Node>, by: Option<Box<Node>>, var: (u16, u16), body: Box<Node> },
    Repeat(Box<Node>),
    TimesRepeat(Box<Node>, Box<Node>),
}

pub struct CompileError {
    pub msg: String,
    pub pos: usize,
}

struct Scope {
    /// Visible names in this real frame: (name, slot).
    names: Vec<(String, u16)>,
    nslots: u16,
}

pub struct Compiler<'a> {
    pub class: Option<Rc<Class>>,
    pub meta: bool,
    pub ivars: Vec<String>,
    pub class_ivars: Vec<String>,
    pub workspace: Option<Rc<RefCell<HashMap<String, Value>>>>,
    scopes: Vec<Scope>,
    pub interner: &'a mut dyn FnMut(&str) -> Sym,
    pub in_block: usize,
}

pub type CResult<T> = Result<T, CompileError>;

impl<'a> Compiler<'a> {
    pub fn new(
        class: Option<Rc<Class>>,
        meta: bool,
        workspace: Option<Rc<RefCell<HashMap<String, Value>>>>,
        interner: &'a mut dyn FnMut(&str) -> Sym,
    ) -> Compiler<'a> {
        let (ivars, class_ivars) = match &class {
            Some(c) if !meta => (c.all_ivars().into_iter().map(|v| v.name).collect(), vec![]),
            Some(c) => (vec![], c.all_class_ivars().into_iter().map(|v| v.name).collect()),
            None => (vec![], vec![]),
        };
        Compiler { class, meta, ivars, class_ivars, workspace, scopes: vec![], interner, in_block: 0 }
    }

    fn sym(&mut self, s: &str) -> Sym {
        (self.interner)(s)
    }

    pub fn compile_method(&mut self, m: &MethodAst) -> CResult<MethodTree> {
        self.scopes.push(Scope { names: vec![], nslots: 0 });
        for p in &m.params {
            self.declare(&p.name, p.pos)?;
        }
        for t in &m.temps {
            self.declare(&t.name, t.pos)?;
        }
        let mut nodes = self.compile_stmts(&m.body)?;
        // A method's final `^expr` (or the implicit `^self`) is simply the value of its body:
        // evaluating it in tail position saves a level of machine stack per call.
        match nodes.pop() {
            Some(Node::Return(e)) if matches!(m.body.last(), Some(Stmt::Return(..))) => nodes.push(*e),
            Some(other) => {
                nodes.push(other);
                nodes.push(Node::SelfRef);
            }
            None => nodes.push(Node::SelfRef),
        }
        let sc = self.scopes.pop().unwrap();
        Ok(MethodTree { nargs: m.params.len(), nslots: sc.nslots as usize, body: Node::Seq(nodes) })
    }

    /// A do-it returns the value of its last statement.
    pub fn compile_doit(&mut self, temps: &[Var], body: &[Stmt]) -> CResult<MethodTree> {
        self.scopes.push(Scope { names: vec![], nslots: 0 });
        for t in temps {
            self.declare(&t.name, t.pos)?;
        }
        let mut nodes = Vec::new();
        let n = body.len();
        for (i, s) in body.iter().enumerate() {
            match s {
                Stmt::Expr(e) if i + 1 == n => nodes.push(self.expr(e)?),
                Stmt::Expr(e) => nodes.push(self.expr(e)?),
                Stmt::Return(e, _) => nodes.push(Node::Return(Box::new(self.expr(e)?))),
            }
        }
        if n == 0 {
            nodes.push(Node::Return(Box::new(Node::Const(Value::Nil))));
        }
        let sc = self.scopes.pop().unwrap();
        Ok(MethodTree { nargs: 0, nslots: sc.nslots as usize, body: Node::Seq(nodes) })
    }

    fn declare(&mut self, name: &str, pos: usize) -> CResult<(u16, u16)> {
        let sc = self.scopes.last_mut().unwrap();
        if matches!(name, "self" | "super" | "thisContext" | "nil" | "true" | "false") {
            return Err(CompileError { msg: format!("'{}' cannot be used as a variable name", name), pos });
        }
        let slot = sc.nslots;
        sc.nslots += 1;
        sc.names.push((name.to_string(), slot));
        Ok((0, slot))
    }

    fn compile_stmts(&mut self, body: &[Stmt]) -> CResult<Vec<Node>> {
        let mut out = Vec::new();
        for s in body {
            out.push(match s {
                Stmt::Expr(e) => self.expr(e)?,
                Stmt::Return(e, _) => Node::Return(Box::new(self.expr(e)?)),
            });
        }
        Ok(out)
    }

    fn lookup(&self, name: &str) -> Option<(u16, u16)> {
        for (depth, sc) in self.scopes.iter().rev().enumerate() {
            if let Some((_, slot)) = sc.names.iter().rev().find(|(n, _)| n == name) {
                return Some((depth as u16, *slot));
            }
        }
        None
    }

    fn var(&mut self, name: &str, pos: usize) -> CResult<Node> {
        if name == "self" {
            return Ok(Node::SelfRef);
        }
        if name == "super" {
            return Err(CompileError { msg: "'super' can only be used to send a message".into(), pos });
        }
        if name == "thisContext" {
            return Ok(Node::Const(Value::Nil));
        }
        if let Some((d, s)) = self.lookup(name) {
            return Ok(Node::Local(d, s));
        }
        if let Some(i) = self.ivars.iter().rposition(|n| n == name) {
            return Ok(Node::IVar(i));
        }
        if self.class_ivars.iter().any(|n| n == name) {
            return Ok(Node::ClassIVar(self.sym(name)));
        }
        let starts_lower = name.chars().next().map(|c| c.is_lowercase()).unwrap_or(false);
        if starts_lower {
            if let Some(ws) = &self.workspace {
                return Ok(Node::Ws(ws.clone(), self.sym(name)));
            }
        }
        Ok(Node::Name(self.sym(name), self.class.clone()))
    }

    fn assign(&mut self, name: &str, val: Node, pos: usize) -> CResult<Node> {
        if let Some((d, s)) = self.lookup(name) {
            return Ok(Node::SetLocal(d, s, Box::new(val)));
        }
        if let Some(i) = self.ivars.iter().rposition(|n| n == name) {
            return Ok(Node::SetIVar(i, Box::new(val)));
        }
        if self.class_ivars.iter().any(|n| n == name) {
            return Ok(Node::SetClassIVar(self.sym(name), Box::new(val)));
        }
        if matches!(name, "self" | "super" | "nil" | "true" | "false" | "thisContext") {
            return Err(CompileError { msg: format!("cannot assign to '{}'", name), pos });
        }
        let starts_lower = name.chars().next().map(|c| c.is_lowercase()).unwrap_or(false);
        if starts_lower {
            if let Some(ws) = &self.workspace {
                return Ok(Node::SetWs(ws.clone(), self.sym(name), Box::new(val)));
            }
            if self.class.as_ref().and_then(|c| c.find_class_var_owner(name)).is_none() {
                return Err(CompileError { msg: format!("undeclared variable '{}'", name), pos });
            }
        }
        Ok(Node::SetName(self.sym(name), self.class.clone(), Box::new(val)))
    }

    fn expr(&mut self, e: &Expr) -> CResult<Node> {
        match e {
            Expr::Lit(l, _) => Ok(match l {
                Lit::Array(_) | Lit::Bytes(_) | Lit::Str(_) => Node::LitArray(Rc::new(l.clone())),
                _ => Node::Const(lit_value(l, self.interner)),
            }),
            Expr::Var(n, p) => self.var(n, *p),
            Expr::Assign(n, v, p) => {
                let val = self.expr(v)?;
                self.assign(n, val, *p)
            }
            Expr::Send { recv, sel, args, pos } => self.send(recv, sel, args, *pos),
            Expr::Cascade { recv, msgs } => {
                if matches!(&**recv, Expr::Var(n, _) if n == "super") {
                    return Err(CompileError { msg: "cannot cascade to super".into(), pos: recv.pos() });
                }
                let r = self.expr(recv)?;
                let mut ms = Vec::new();
                for (sel, args, _) in msgs {
                    let mut a = Vec::new();
                    for x in args {
                        a.push(self.expr(x)?);
                    }
                    ms.push((self.sym(sel), a));
                }
                Ok(Node::Cascade(Box::new(r), ms))
            }
            Expr::Block(b) => self.block(b),
            Expr::Brace(items, _) => {
                let mut v = Vec::new();
                for x in items {
                    v.push(self.expr(x)?);
                }
                Ok(Node::Brace(v))
            }
            Expr::Guaranteed(_, e, _) => self.expr(e),
            Expr::Prim { recv, sel, args, .. } => {
                let r = match recv {
                    Some(r) => Some(Box::new(self.expr(r)?)),
                    None => None,
                };
                let mut a = Vec::new();
                for x in args {
                    a.push(self.expr(x)?);
                }
                Ok(Node::Prim(self.sym(sel), r, a))
            }
        }
    }

    pub fn block(&mut self, b: &BlockAst) -> CResult<Node> {
        self.scopes.push(Scope { names: vec![], nslots: 0 });
        self.in_block += 1;
        for p in &b.params {
            self.declare(&p.name, p.pos)?;
        }
        for t in &b.temps {
            self.declare(&t.name, t.pos)?;
        }
        let r = self.block_body(&b.body);
        self.in_block -= 1;
        let sc = self.scopes.pop().unwrap();
        let body = r?;
        Ok(Node::Block(Rc::new(BlockCode { nparams: b.params.len(), nslots: sc.nslots as usize, body })))
    }

    /// The statements of a block; the value is the last expression (nil when empty).
    fn block_body(&mut self, body: &[Stmt]) -> CResult<Node> {
        if body.is_empty() {
            return Ok(Node::Const(Value::Nil));
        }
        let mut v = self.compile_stmts(body)?;
        if v.len() == 1 {
            return Ok(v.pop().unwrap());
        }
        Ok(Node::Seq(v))
    }

    fn is_literal_block(e: &Expr, nparams: usize) -> bool {
        matches!(e, Expr::Block(b) if b.params.len() == nparams && b.ret.is_none())
    }

    /// Compiles a literal block's body into the current frame (no closure is created).
    fn inline_block(&mut self, e: &Expr) -> CResult<Node> {
        let b = match e {
            Expr::Block(b) => b,
            _ => unreachable!(),
        };
        let mark = self.scopes.last().unwrap().names.len();
        for t in &b.temps {
            self.declare(&t.name, t.pos)?;
        }
        // temporaries of an inlined block start out nil on each entry
        let mut resets = Vec::new();
        for t in &b.temps {
            let (_, s) = self.lookup(&t.name).unwrap();
            resets.push(Node::SetLocal(0, s, Box::new(Node::Const(Value::Nil))));
        }
        let r = self.block_body(&b.body);
        self.scopes.last_mut().unwrap().names.truncate(mark);
        let body = r?;
        if resets.is_empty() {
            Ok(body)
        } else {
            resets.push(body);
            Ok(Node::Seq(resets))
        }
    }

    fn inline_block_with_arg(&mut self, e: &Expr) -> CResult<((u16, u16), Node)> {
        let b = match e {
            Expr::Block(b) => b,
            _ => unreachable!(),
        };
        let mark = self.scopes.last().unwrap().names.len();
        let slot = self.declare(&b.params[0].name, b.params[0].pos)?;
        for t in &b.temps {
            self.declare(&t.name, t.pos)?;
        }
        let r = self.block_body(&b.body);
        self.scopes.last_mut().unwrap().names.truncate(mark);
        Ok((slot, r?))
    }

    fn send(&mut self, recv: &Expr, sel: &str, args: &[Expr], pos: usize) -> CResult<Node> {
        if let Expr::Var(n, _) = recv {
            if n == "super" {
                let class = match &self.class {
                    Some(c) => c.clone(),
                    None => return Err(CompileError { msg: "'super' outside a method".into(), pos }),
                };
                let mut a = Vec::new();
                for x in args {
                    a.push(self.expr(x)?);
                }
                return Ok(Node::SuperSend(class, self.meta, self.sym(sel), a));
            }
        }
        // Inlined control structures: the messages the original VM's symbol table singles out
        // (sub_40f100: ifTrue:, ifFalse:, ifTrue:ifFalse:, ifFalse:ifTrue:, and:, or:,
        // whileTrue[:], whileFalse[:], to:by:do:); to:do: is to:by:do: with a step of 1.
        match (sel, args.len()) {
            ("ifTrue:", 1) | ("ifFalse:", 1) if Self::is_literal_block(&args[0], 0) => {
                let c = self.expr(recv)?;
                let t = self.inline_block(&args[0])?;
                return Ok(Node::If(Box::new(c), sel == "ifTrue:", Box::new(t), None));
            }
            ("ifTrue:ifFalse:", 2) | ("ifFalse:ifTrue:", 2)
                if Self::is_literal_block(&args[0], 0) && Self::is_literal_block(&args[1], 0) =>
            {
                let c = self.expr(recv)?;
                let t = self.inline_block(&args[0])?;
                let f = self.inline_block(&args[1])?;
                return Ok(Node::If(Box::new(c), sel == "ifTrue:ifFalse:", Box::new(t), Some(Box::new(f))));
            }
            ("and:", 1) | ("or:", 1) if Self::is_literal_block(&args[0], 0) => {
                let c = self.expr(recv)?;
                let b = self.inline_block(&args[0])?;
                return Ok(if sel == "and:" { Node::And(Box::new(c), Box::new(b)) } else { Node::Or(Box::new(c), Box::new(b)) });
            }
            ("whileTrue:", 1) | ("whileFalse:", 1)
                if Self::is_literal_block(recv, 0) && Self::is_literal_block(&args[0], 0) =>
            {
                let c = self.inline_block(recv)?;
                let b = self.inline_block(&args[0])?;
                return Ok(Node::While(Box::new(c), sel == "whileTrue:", Some(Box::new(b))));
            }
            ("whileTrue", 0) | ("whileFalse", 0) if Self::is_literal_block(recv, 0) => {
                let c = self.inline_block(recv)?;
                return Ok(Node::While(Box::new(c), sel == "whileTrue", None));
            }
            ("to:do:", 2) if Self::is_literal_block(&args[1], 1) => {
                let from = self.expr(recv)?;
                let to = self.expr(&args[0])?;
                let (var, body) = self.inline_block_with_arg(&args[1])?;
                return Ok(Node::ToDo { from: Box::new(from), to: Box::new(to), by: None, var, body: Box::new(body) });
            }
            ("to:by:do:", 3) if Self::is_literal_block(&args[2], 1) => {
                let from = self.expr(recv)?;
                let to = self.expr(&args[0])?;
                let by = self.expr(&args[1])?;
                let (var, body) = self.inline_block_with_arg(&args[2])?;
                return Ok(Node::ToDo { from: Box::new(from), to: Box::new(to), by: Some(Box::new(by)), var, body: Box::new(body) });
            }
            _ => {}
        }
        let r = self.expr(recv)?;
        let mut a = Vec::with_capacity(args.len());
        for x in args {
            a.push(self.expr(x)?);
        }
        Ok(Node::Send(Box::new(r), self.sym(sel), a))
    }
}

pub fn lit_value(l: &Lit, interner: &mut dyn FnMut(&str) -> Sym) -> Value {
    match l {
        Lit::Nil => Value::Nil,
        Lit::True => Value::Bool(true),
        Lit::False => Value::Bool(false),
        Lit::Int(i) => Value::int(*i),
        Lit::Big(b) => Value::big(b.clone()),
        Lit::Float(f) => Value::Float(*f),
        Lit::Char(c) => Value::Char(*c),
        Lit::Str(s) => Value::str(s),
        Lit::Sym(s) => Value::Sym(interner(s)),
        Lit::Array(_) | Lit::Bytes(_) => Value::Nil, // built by the interpreter (needs the Array class)
    }
}
