//! Turns resolved syntax trees (`Node`) into stack bytecode. The interpreter runs bytecode in a
//! loop with Smalltalk activations kept on the heap, so call depth is not limited by the
//! machine stack (which is small inside a browser).

use crate::ast::Lit;
use crate::compiler::Node;
use crate::object::*;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

#[derive(Clone, Copy, Debug)]
pub enum Op {
    PushConst(u32),
    /// Literal arrays and strings: a fresh copy every time (strings can be changed).
    PushLit(u32),
    PushSelf,
    PushNil,
    PushLocal(u16, u16),
    StoreLocal(u16, u16),
    PushIVar(u16),
    StoreIVar(u16),
    PushClassIVar(u32),
    StoreClassIVar(u32),
    PushName(u32),
    StoreName(u32),
    PushWs(u32),
    StoreWs(u32),
    Pop,
    Dup,
    Send(u32, u8),
    /// selector, argument count, class (index into `supers`), class side
    SuperSend(u32, u8, u32, bool),
    MakeBlock(u32),
    MakeArray(u32),
    Jump(u32),
    JumpIfFalse(u32),
    JumpIfTrue(u32),
    /// Pops the top; jumps when it is (or is not) nil.
    JumpIfNil(u32),
    JumpIfNotNil(u32),
    /// `^` : return the top from the home method (a non-local return inside a block).
    Return,
    /// End of a block: the top is the block's value.
    BlockEnd,
    /// primitive name, argument count, whether a receiver was pushed
    Prim(u32, u8),
    /// Loop test for to:do: — counter, limit, step slots; pushes a Boolean.
    LoopTest(u16, u16, u16),
}

pub struct Code {
    pub ops: Vec<Op>,
    pub consts: Vec<Value>,
    pub syms: Vec<Sym>,
    pub names: Vec<(Rc<str>, Option<Rc<Class>>)>,
    pub ws: Option<Rc<RefCell<HashMap<String, Value>>>>,
    pub blocks: Vec<Rc<Code>>,
    pub lits: Vec<Rc<Lit>>,
    pub supers: Vec<Rc<Class>>,
    pub nparams: usize,
    pub nslots: usize,
}

struct Gen {
    code: Code,
    /// slots allocated for loop counters in the frame being generated
    extra: usize,
}

impl Gen {
    fn emit(&mut self, op: Op) -> usize {
        self.code.ops.push(op);
        self.code.ops.len() - 1
    }
    fn here(&self) -> u32 {
        self.code.ops.len() as u32
    }
    fn patch(&mut self, at: usize) {
        let to = self.here();
        self.code.ops[at] = match self.code.ops[at] {
            Op::Jump(_) => Op::Jump(to),
            Op::JumpIfFalse(_) => Op::JumpIfFalse(to),
            Op::JumpIfTrue(_) => Op::JumpIfTrue(to),
            Op::JumpIfNil(_) => Op::JumpIfNil(to),
            Op::JumpIfNotNil(_) => Op::JumpIfNotNil(to),
            other => other,
        };
    }
    fn konst(&mut self, v: Value) -> u32 {
        self.code.consts.push(v);
        self.code.consts.len() as u32 - 1
    }
    fn sym(&mut self, s: &Sym) -> u32 {
        if let Some(i) = self.code.syms.iter().position(|x| Rc::ptr_eq(x, s)) {
            return i as u32;
        }
        self.code.syms.push(s.clone());
        self.code.syms.len() as u32 - 1
    }
    fn hidden(&mut self) -> u16 {
        let s = self.code.nslots + self.extra;
        self.extra += 1;
        s as u16
    }

    fn node(&mut self, n: &Node) {
        match n {
            Node::Const(Value::Nil) => {
                self.emit(Op::PushNil);
            }
            Node::Const(v) => {
                let k = self.konst(v.clone());
                self.emit(Op::PushConst(k));
            }
            Node::LitArray(l) => {
                self.code.lits.push(l.clone());
                let k = self.code.lits.len() as u32 - 1;
                self.emit(Op::PushLit(k));
            }
            Node::SelfRef => {
                self.emit(Op::PushSelf);
            }
            Node::Local(d, s) => {
                self.emit(Op::PushLocal(*d, *s));
            }
            Node::SetLocal(d, s, e) => {
                self.node(e);
                self.emit(Op::StoreLocal(*d, *s));
            }
            Node::IVar(i) => {
                self.emit(Op::PushIVar(*i as u16));
            }
            Node::SetIVar(i, e) => {
                self.node(e);
                self.emit(Op::StoreIVar(*i as u16));
            }
            Node::ClassIVar(n) => {
                let k = self.sym(n);
                self.emit(Op::PushClassIVar(k));
            }
            Node::SetClassIVar(n, e) => {
                self.node(e);
                let k = self.sym(n);
                self.emit(Op::StoreClassIVar(k));
            }
            Node::Name(n, c) => {
                self.code.names.push((n.clone(), c.clone()));
                let k = self.code.names.len() as u32 - 1;
                self.emit(Op::PushName(k));
            }
            Node::SetName(n, c, e) => {
                self.node(e);
                self.code.names.push((n.clone(), c.clone()));
                let k = self.code.names.len() as u32 - 1;
                self.emit(Op::StoreName(k));
            }
            Node::Ws(ws, n) => {
                self.code.ws = Some(ws.clone());
                let k = self.sym(n);
                self.emit(Op::PushWs(k));
            }
            Node::SetWs(ws, n, e) => {
                self.node(e);
                self.code.ws = Some(ws.clone());
                let k = self.sym(n);
                self.emit(Op::StoreWs(k));
            }
            Node::Send(r, sel, args) => {
                self.node(r);
                for a in args {
                    self.node(a);
                }
                let k = self.sym(sel);
                self.emit(Op::Send(k, args.len() as u8));
            }
            Node::SuperSend(cls, meta, sel, args) => {
                self.emit(Op::PushSelf);
                for a in args {
                    self.node(a);
                }
                let k = self.sym(sel);
                self.code.supers.push(cls.clone());
                let c = self.code.supers.len() as u32 - 1;
                self.emit(Op::SuperSend(k, args.len() as u8, c, *meta));
            }
            Node::Cascade(r, msgs) => {
                self.node(r);
                let last = msgs.len().saturating_sub(1);
                for (i, (sel, args)) in msgs.iter().enumerate() {
                    if i < last {
                        self.emit(Op::Dup);
                    }
                    for a in args {
                        self.node(a);
                    }
                    let k = self.sym(sel);
                    self.emit(Op::Send(k, args.len() as u8));
                    if i < last {
                        self.emit(Op::Pop);
                    }
                }
            }
            Node::Block(b) => {
                let code = block_code(b.nparams, b.nslots, &b.body);
                self.code.blocks.push(Rc::new(code));
                let k = self.code.blocks.len() as u32 - 1;
                self.emit(Op::MakeBlock(k));
            }
            Node::Seq(v) => {
                if v.is_empty() {
                    self.emit(Op::PushNil);
                }
                for (i, x) in v.iter().enumerate() {
                    self.node(x);
                    if i + 1 < v.len() {
                        self.emit(Op::Pop);
                    }
                }
            }
            Node::Return(e) => {
                self.node(e);
                self.emit(Op::Return);
            }
            Node::Brace(items) => {
                for x in items {
                    self.node(x);
                }
                self.emit(Op::MakeArray(items.len() as u32));
            }
            Node::Prim(name, r, args) => {
                match r {
                    Some(r) => self.node(r),
                    None => {
                        self.emit(Op::PushNil);
                    }
                }
                for a in args {
                    self.node(a);
                }
                let k = self.sym(name);
                self.emit(Op::Prim(k, args.len() as u8));
            }
            Node::If(c, when, then, els) => {
                self.node(c);
                let j = self.emit(if *when { Op::JumpIfFalse(0) } else { Op::JumpIfTrue(0) });
                self.node(then);
                let end = self.emit(Op::Jump(0));
                self.patch(j);
                match els {
                    Some(e) => self.node(e),
                    None => {
                        self.emit(Op::PushNil);
                    }
                }
                self.patch(end);
            }
            Node::IfNil(c, nil_first, slot, a, b) => {
                self.node(c);
                self.emit(Op::Dup);
                if *nil_first {
                    // ifNil: a [ifNotNil: b]
                    let j = self.emit(Op::JumpIfNotNil(0));
                    self.emit(Op::Pop);
                    self.node(a);
                    match b {
                        None => self.patch(j),
                        Some(b) => {
                            let end = self.emit(Op::Jump(0));
                            self.patch(j);
                            if let Some((d, s)) = slot {
                                self.emit(Op::StoreLocal(*d, *s));
                            }
                            self.emit(Op::Pop);
                            self.node(b);
                            self.patch(end);
                        }
                    }
                } else {
                    // ifNotNil: a  (nil stays as the value)
                    let j = self.emit(Op::JumpIfNil(0));
                    if let Some((d, s)) = slot {
                        self.emit(Op::StoreLocal(*d, *s));
                    }
                    self.emit(Op::Pop);
                    self.node(a);
                    self.patch(j);
                }
            }
            Node::And(a, b) => {
                self.node(a);
                let f = self.emit(Op::JumpIfFalse(0));
                self.node(b);
                let end = self.emit(Op::Jump(0));
                self.patch(f);
                let k = self.konst(Value::Bool(false));
                self.emit(Op::PushConst(k));
                self.patch(end);
            }
            Node::Or(a, b) => {
                self.node(a);
                let t = self.emit(Op::JumpIfTrue(0));
                self.node(b);
                let end = self.emit(Op::Jump(0));
                self.patch(t);
                let k = self.konst(Value::Bool(true));
                self.emit(Op::PushConst(k));
                self.patch(end);
            }
            Node::While(cond, expect, body) => {
                let top = self.here();
                self.node(cond);
                let exit = self.emit(if *expect { Op::JumpIfFalse(0) } else { Op::JumpIfTrue(0) });
                if let Some(b) = body {
                    self.node(b);
                    self.emit(Op::Pop);
                }
                self.emit(Op::Jump(top));
                self.patch(exit);
                self.emit(Op::PushNil);
            }
            Node::Repeat(b) => {
                let top = self.here();
                self.node(b);
                self.emit(Op::Pop);
                self.emit(Op::Jump(top));
                self.emit(Op::PushNil);
            }
            Node::TimesRepeat(n, b) => {
                let (i, lim, step) = (self.hidden(), self.hidden(), self.hidden());
                self.node(n);
                self.emit(Op::StoreLocal(0, lim));
                self.emit(Op::Pop);
                let one = self.konst(Value::Int(1));
                self.emit(Op::PushConst(one));
                self.emit(Op::StoreLocal(0, step));
                self.emit(Op::StoreLocal(0, i));
                self.emit(Op::Pop);
                self.counted_loop(i, lim, step, b);
                self.emit(Op::PushLocal(0, lim));
            }
            Node::ToDo { from, to, by, var, body } => {
                let (start, lim, step) = (self.hidden(), self.hidden(), self.hidden());
                let (d, v) = *var;
                debug_assert_eq!(d, 0);
                self.node(from);
                self.emit(Op::StoreLocal(0, start));
                self.emit(Op::StoreLocal(0, v));
                self.emit(Op::Pop);
                self.node(to);
                self.emit(Op::StoreLocal(0, lim));
                self.emit(Op::Pop);
                match by {
                    Some(b) => self.node(b),
                    None => {
                        let one = self.konst(Value::Int(1));
                        self.emit(Op::PushConst(one));
                    }
                }
                self.emit(Op::StoreLocal(0, step));
                self.emit(Op::Pop);
                self.counted_loop(v, lim, step, body);
                self.emit(Op::PushLocal(0, start));
            }
        }
    }

    fn counted_loop(&mut self, i: u16, lim: u16, step: u16, body: &Node) {
        let top = self.here();
        self.emit(Op::LoopTest(i, lim, step));
        let exit = self.emit(Op::JumpIfFalse(0));
        self.node(body);
        self.emit(Op::Pop);
        self.emit(Op::PushLocal(0, i));
        self.emit(Op::PushLocal(0, step));
        let plus = Rc::from("+");
        let k = self.sym_named(plus);
        self.emit(Op::Send(k, 1));
        self.emit(Op::StoreLocal(0, i));
        self.emit(Op::Pop);
        self.emit(Op::Jump(top));
        self.patch(exit);
    }

    fn sym_named(&mut self, s: Sym) -> u32 {
        if let Some(i) = self.code.syms.iter().position(|x| **x == *s) {
            return i as u32;
        }
        self.code.syms.push(s);
        self.code.syms.len() as u32 - 1
    }

    fn finish(mut self) -> Code {
        self.code.nslots += self.extra;
        self.code
    }
}

fn empty(nparams: usize, nslots: usize) -> Gen {
    Gen {
        code: Code {
            ops: Vec::new(),
            consts: Vec::new(),
            syms: Vec::new(),
            names: Vec::new(),
            ws: None,
            blocks: Vec::new(),
            lits: Vec::new(),
            supers: Vec::new(),
            nparams,
            nslots,
        },
        extra: 0,
    }
}

/// A method or do-it body: its value is returned from the method.
pub fn method_code(nargs: usize, nslots: usize, body: &Node) -> Code {
    let mut g = empty(nargs, nslots);
    g.node(body);
    g.emit(Op::Return);
    g.finish()
}

pub fn block_code(nparams: usize, nslots: usize, body: &Node) -> Code {
    let mut g = empty(nparams, nslots);
    g.node(body);
    g.emit(Op::BlockEnd);
    g.finish()
}

/// Interns the selectors of generated code with the interpreter's symbol table, so that
/// selector identity comparisons (fast paths, the method cache) work.
pub fn intern_all(code: &mut Code, intern: &mut dyn FnMut(&str) -> Sym) {
    for s in code.syms.iter_mut() {
        *s = intern(s);
    }
    for b in code.blocks.iter_mut() {
        if let Some(bm) = Rc::get_mut(b) {
            intern_all(bm, intern);
        }
    }
}
