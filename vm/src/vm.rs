//! The bytecode interpreter loop. Smalltalk activations live in `Interp::vframes` (on the
//! heap), so a deep Smalltalk recursion does not grow the machine stack.

use crate::codegen::{Code, Op};
use crate::interp::{Interp, Unwind, R};
use crate::object::*;
use std::cell::RefCell;
use std::rc::Rc;

pub struct VFrame {
    pub code: Rc<Code>,
    pub pc: usize,
    /// Height of the operand stack when the activation started.
    pub base: usize,
    pub frame: Rc<Frame>,
    pub rcv: Value,
    /// The method activation that `^` returns from.
    pub home: u64,
    /// This activation's id (0 for blocks).
    pub id: u64,
    pub method: Option<Rc<Method>>,
}

/// Either a value right away (primitives, natives) or "a new activation was pushed".
pub type D = Result<Option<Value>, Unwind>;

fn frame_at(f: &Rc<Frame>, depth: u16) -> &Rc<Frame> {
    let mut fr = f;
    for _ in 0..depth {
        fr = fr.parent.as_ref().expect("frame depth");
    }
    fr
}

impl Interp {
    pub fn push_method_frame(&mut self, m: Rc<Method>, code: Rc<Code>, rcv: Value, mut args: Vec<Value>) -> D {
        if args.len() != code.nparams {
            return self.signal_error("Error", &format!("wrong number of arguments for #{}", m.selector)).map(Some);
        }
        if self.vframes.len() >= self.max_depth {
            return self.stack_overflow().map(Some);
        }
        let id = self.next_id();
        args.resize(code.nslots, Value::Nil);
        let frame = Rc::new(Frame { slots: RefCell::new(args), parent: None });
        self.vframes.push(VFrame { code, pc: 0, base: self.ostack.len(), frame, rcv, home: id, id, method: Some(m) });
        Ok(None)
    }

    pub fn push_block_frame(&mut self, b: &Rc<Closure>, mut args: Vec<Value>) -> D {
        if args.len() != b.code.nparams {
            let msg = format!("wrong number of arguments: this block takes {} argument(s), got {}", b.code.nparams, args.len());
            return self.signal_error("Error", &msg).map(Some);
        }
        if self.vframes.len() >= self.max_depth {
            return self.stack_overflow().map(Some);
        }
        args.resize(b.code.nslots, Value::Nil);
        let frame = Rc::new(Frame { slots: RefCell::new(args), parent: Some(b.outer.clone()) });
        self.vframes.push(VFrame {
            code: b.code.clone(),
            pc: 0,
            base: self.ostack.len(),
            frame,
            rcv: b.receiver.clone(),
            home: b.home,
            id: 0,
            method: None,
        });
        Ok(None)
    }

    pub fn dispatch_method(&mut self, m: Rc<Method>, rcv: Value, args: Vec<Value>) -> D {
        match &m.body {
            MethodBody::Native(f) => {
                let f = *f;
                f(self, rcv, args).map(Some)
            }
            MethodBody::Code(c) => {
                let code = c.code.clone();
                self.push_method_frame(m, code, rcv, args)
            }
        }
    }

    /// Sends a message: answers the result, or pushes the activation that will compute it.
    pub fn dispatch(&mut self, rcv: Value, sel: &Sym, args: Vec<Value>) -> D {
        self.sends += 1;
        if args.len() == 1 {
            match (&rcv, &args[0]) {
                (Value::Int(a), Value::Int(b)) => {
                    if let Some(v) = self.fast_int(*a, sel, *b) {
                        return Ok(Some(v));
                    }
                }
                (Value::Float(a), Value::Float(b)) => {
                    if let Some(v) = self.fast_float(*a, sel, *b) {
                        return Ok(Some(v));
                    }
                }
                (Value::Float(a), Value::Int(b)) => {
                    if let Some(v) = self.fast_float(*a, sel, *b as f64) {
                        return Ok(Some(v));
                    }
                }
                (Value::Int(a), Value::Float(b)) => {
                    if let Some(v) = self.fast_float(*a as f64, sel, *b) {
                        return Ok(Some(v));
                    }
                }
                _ => {}
            }
        }
        if let Value::Block(b) = &rcv {
            let n = args.len();
            if b.code.nparams == n
                && ((n == 0 && Rc::ptr_eq(sel, &self.s.value))
                    || (n == 1 && Rc::ptr_eq(sel, &self.s.value1))
                    || (n == 2 && Rc::ptr_eq(sel, &self.s.value2))
                    || (n == 3 && Rc::ptr_eq(sel, &self.s.value3))
                    || (n == 4 && Rc::ptr_eq(sel, &self.s.value4)))
            {
                let b = b.clone();
                return self.push_block_frame(&b, args);
            }
        }
        let (cls, meta) = self.behavior_of(&rcv);
        match self.lookup(&cls, meta, sel) {
            Some(m) => self.dispatch_method(m, rcv, args),
            None => self.dispatch_dnu(rcv, sel, args),
        }
    }

    #[inline(never)]
    fn dispatch_dnu(&mut self, rcv: Value, sel: &Sym, args: Vec<Value>) -> D {
        let (cls, meta) = self.behavior_of(&rcv);
        let dnu = self.s.dnu.clone();
        if Rc::ptr_eq(sel, &dnu) {
            return Err(self.hard_error(&format!("{} does not understand #doesNotUnderstand:", cls.name())));
        }
        match self.lookup(&cls, meta, &dnu) {
            Some(m) => {
                // As Delta::does_not_understand (0x43a170) builds it: receiver at +7,
                // selector at +11, arguments at +15.
                let msg = self.new_object(&self.core.message.clone());
                if let Value::Obj(o) = &msg {
                    o.set(0, rcv.clone());
                    o.set(1, Value::Sym(sel.clone()));
                    o.set(2, self.array(args));
                }
                self.dispatch_method(m, rcv, vec![msg])
            }
            None => {
                let who = if meta { format!("{} class", cls.name()) } else { cls.name() };
                Err(self.hard_error(&format!("{} does not understand #{}", who, sel)))
            }
        }
    }

    fn dispatch_super(&mut self, rcv: Value, sel: &Sym, args: Vec<Value>, cls: &Rc<Class>, meta: bool) -> D {
        let m = match cls.superclass() {
            Some(s) => self.lookup_from(Some(s), meta, sel),
            None if meta => self.lookup_from(Some(self.core.class.clone()), false, sel),
            None => None,
        };
        match m {
            Some(m) => self.dispatch_method(m, rcv, args),
            None => self.dispatch_dnu(rcv, sel, args),
        }
    }

    /// Runs activations until the one at `base` returns.
    pub fn run(&mut self, base: usize) -> R {
        if self.run_depth >= self.max_run_depth {
            self.drop_frames(base);
            return Err(self.hard_error("Stack overflow: too many nested exception handlers or callbacks"));
        }
        self.run_depth += 1;
        let r = loop {
            match self.step(base) {
                Ok(v) => break Ok(v),
                Err(u) => match self.unwind(base, u) {
                    Ok(Some(v)) => break Ok(v),
                    Ok(None) => continue,
                    Err(u) => break Err(u),
                },
            }
        };
        self.run_depth -= 1;
        r
    }

    fn drop_frames(&mut self, base: usize) {
        if base < self.vframes.len() {
            let fb = self.vframes[base].base;
            self.vframes.truncate(base);
            self.ostack.truncate(fb);
        }
    }

    /// Delivers a non-local return to its home activation if it is in this run; otherwise
    /// abandons this run's activations and passes the unwind outwards.
    fn unwind(&mut self, base: usize, u: Unwind) -> Result<Option<Value>, Unwind> {
        if let Unwind::Return(h, v) = u {
            if let Some(j) = (base..self.vframes.len()).rev().find(|&j| self.vframes[j].id == h) {
                let fb = self.vframes[j].base;
                self.vframes.truncate(j);
                self.ostack.truncate(fb);
                if j == base {
                    return Ok(Some(v));
                }
                self.ostack.push(v);
                return Ok(None);
            }
            self.drop_frames(base);
            return Err(Unwind::Return(h, v));
        }
        self.drop_frames(base);
        Err(u)
    }

    fn pop_value(&mut self) -> Value {
        self.ostack.pop().unwrap_or(Value::Nil)
    }

    fn take_args(&mut self, n: usize) -> (Value, Vec<Value>) {
        let at = self.ostack.len() - n;
        let args = self.ostack.split_off(at);
        let rcv = self.pop_value();
        (rcv, args)
    }

    fn deliver(&mut self, d: D) -> Result<(), Unwind> {
        if let Some(v) = d? {
            self.ostack.push(v);
        }
        Ok(())
    }

    /// Executes bytecodes until the activation at `base` returns (Ok) or something unwinds (Err).
    fn step(&mut self, base: usize) -> R {
        loop {
            let fi = self.vframes.len() - 1;
            let f = &mut self.vframes[fi];
            let op = f.code.ops[f.pc];
            f.pc += 1;
            match op {
                Op::PushConst(k) => {
                    let v = f.code.consts[k as usize].clone();
                    self.ostack.push(v);
                }
                Op::PushNil => self.ostack.push(Value::Nil),
                Op::PushSelf => {
                    let v = f.rcv.clone();
                    self.ostack.push(v);
                }
                Op::PushLocal(d, s) => {
                    let v = frame_at(&f.frame, d).slots.borrow()[s as usize].clone();
                    self.ostack.push(v);
                }
                Op::StoreLocal(d, s) => {
                    let v = self.ostack.last().cloned().unwrap_or(Value::Nil);
                    frame_at(&f.frame, d).slots.borrow_mut()[s as usize] = v;
                }
                Op::PushIVar(i) => {
                    let v = match &f.rcv {
                        Value::Obj(o) => o.get(i as usize),
                        _ => Value::Nil,
                    };
                    self.ostack.push(v);
                }
                Op::StoreIVar(i) => {
                    let v = self.ostack.last().cloned().unwrap_or(Value::Nil);
                    let ok = if let Value::Obj(o) = &f.rcv {
                        o.set(i as usize, v);
                        true
                    } else {
                        false
                    };
                    if !ok {
                        self.signal_error("Error", "this object has no instance variables to assign")?;
                    }
                }
                Op::PushClassIVar(k) => {
                    let v = match &f.rcv {
                        Value::Class(c) => c.class_ivar_values.borrow().get(&*f.code.syms[k as usize]).cloned().unwrap_or(Value::Nil),
                        _ => Value::Nil,
                    };
                    self.ostack.push(v);
                }
                Op::StoreClassIVar(k) => {
                    let v = self.ostack.last().cloned().unwrap_or(Value::Nil);
                    if let Value::Class(c) = &f.rcv {
                        c.class_ivar_values.borrow_mut().insert(f.code.syms[k as usize].to_string(), v);
                    }
                }
                Op::PushName(k) => {
                    let (name, cls) = f.code.names[k as usize].clone();
                    let v = self.lookup_name(&name, &cls)?;
                    self.ostack.push(v);
                }
                Op::StoreName(k) => {
                    let (name, cls) = f.code.names[k as usize].clone();
                    let v = self.ostack.last().cloned().unwrap_or(Value::Nil);
                    let owner = cls.as_ref().and_then(|c| c.find_class_var_owner(&name));
                    match owner {
                        Some(o) => {
                            o.class_var_values.borrow_mut().insert(name.to_string(), v);
                        }
                        None => {
                            self.globals.insert(name.to_string(), v);
                        }
                    }
                }
                Op::PushWs(k) => {
                    let v = match &f.code.ws {
                        Some(ws) => ws.borrow().get(&*f.code.syms[k as usize]).cloned().unwrap_or(Value::Nil),
                        None => Value::Nil,
                    };
                    self.ostack.push(v);
                }
                Op::StoreWs(k) => {
                    let v = self.ostack.last().cloned().unwrap_or(Value::Nil);
                    if let Some(ws) = &f.code.ws {
                        ws.borrow_mut().insert(f.code.syms[k as usize].to_string(), v);
                    }
                }
                Op::Pop => {
                    self.ostack.pop();
                }
                Op::Dup => {
                    let v = self.ostack.last().cloned().unwrap_or(Value::Nil);
                    self.ostack.push(v);
                }
                Op::Send(k, n) => {
                    let sel = f.code.syms[k as usize].clone();
                    if n == 1 {
                        let len = self.ostack.len();
                        let fast = match (&self.ostack[len - 2], &self.ostack[len - 1]) {
                            (Value::Int(a), Value::Int(b)) => self.fast_int(*a, &sel, *b),
                            (Value::Float(a), Value::Float(b)) => self.fast_float(*a, &sel, *b),
                            _ => None,
                        };
                        if let Some(v) = fast {
                            self.sends += 1;
                            self.ostack.truncate(len - 2);
                            self.ostack.push(v);
                            continue;
                        }
                    }
                    let (rcv, args) = self.take_args(n as usize);
                    let d = self.dispatch(rcv, &sel, args);
                    self.deliver(d)?;
                }
                Op::SuperSend(k, n, c, meta) => {
                    let sel = f.code.syms[k as usize].clone();
                    let cls = f.code.supers[c as usize].clone();
                    let (rcv, args) = self.take_args(n as usize);
                    let d = self.dispatch_super(rcv, &sel, args, &cls, meta);
                    self.deliver(d)?;
                }
                Op::MakeBlock(k) => {
                    let b = Closure { code: f.code.blocks[k as usize].clone(), outer: f.frame.clone(), receiver: f.rcv.clone(), home: f.home };
                    self.ostack.push(Value::Block(Rc::new(b)));
                }
                Op::MakeArray(n) => {
                    let at = self.ostack.len() - n as usize;
                    let items = self.ostack.split_off(at);
                    let a = self.array(items);
                    self.ostack.push(a);
                }
                Op::PushLit(k) => {
                    let l = f.code.lits[k as usize].clone();
                    let v = self.lit_to_value(&l);
                    self.ostack.push(v);
                }
                Op::Jump(t) => f.pc = t as usize,
                Op::JumpIfFalse(t) | Op::JumpIfTrue(t) => {
                    let want = matches!(op, Op::JumpIfTrue(_));
                    match self.ostack.pop() {
                        Some(Value::Bool(b)) => {
                            if b == want {
                                self.vframes[fi].pc = t as usize;
                            }
                        }
                        other => {
                            // a resumed NonBooleanReceiver error counts as false
                            self.non_boolean(other.unwrap_or(Value::Nil))?;
                            if !want {
                                self.vframes[fi].pc = t as usize;
                            }
                        }
                    }
                }
                Op::JumpIfNil(t) | Op::JumpIfNotNil(t) => {
                    let is_nil = matches!(self.ostack.pop(), Some(Value::Nil) | None);
                    if is_nil == matches!(op, Op::JumpIfNil(_)) {
                        f.pc = t as usize;
                    }
                }
                Op::Return => {
                    let v = self.pop_value();
                    let f = &self.vframes[fi];
                    if f.id == f.home {
                        let fb = f.base;
                        self.vframes.pop();
                        self.ostack.truncate(fb);
                        if fi == base {
                            return Ok(v);
                        }
                        self.ostack.push(v);
                    } else {
                        let h = f.home;
                        if !self.vframes.iter().any(|x| x.id == h) {
                            let r = self.signal_error(
                                "Error",
                                "BlockContext>>cannotReturn: the method that created this block has already returned",
                            );
                            r?;
                            self.ostack.push(v);
                            continue;
                        }
                        return Err(Unwind::Return(h, v));
                    }
                }
                Op::BlockEnd => {
                    let v = self.pop_value();
                    let fb = self.vframes[fi].base;
                    self.vframes.pop();
                    self.ostack.truncate(fb);
                    if fi == base {
                        return Ok(v);
                    }
                    self.ostack.push(v);
                }
                Op::Prim(k, n) => {
                    let name = f.code.syms[k as usize].clone();
                    let (rcv, args) = self.take_args(n as usize);
                    let d = self.primitive(&name, rcv, args);
                    self.deliver(d)?;
                }
                Op::LoopTest(i, lim, step) => {
                    let (iv, lv, sv) = {
                        let s = f.frame.slots.borrow();
                        (s[i as usize].clone(), s[lim as usize].clone(), s[step as usize].clone())
                    };
                    let b = self.loop_test(iv, lv, sv)?;
                    self.ostack.push(Value::Bool(b));
                }
            }
        }
    }

    fn loop_test(&mut self, i: Value, lim: Value, step: Value) -> Result<bool, Unwind> {
        match (&i, &lim, &step) {
            (Value::Int(a), Value::Int(b), Value::Int(s)) if *s != 0 => return Ok(if *s > 0 { a <= b } else { a >= b }),
            (_, _, Value::Int(0)) => {
                self.signal_error("Error", "the step of a loop must not be zero")?;
                return Ok(false);
            }
            _ => {}
        }
        let lt = self.s.lt.clone();
        let neg = matches!(self.send(step, &lt, vec![Value::Int(0)])?, Value::Bool(true));
        let cmp = if neg { self.s.ge.clone() } else { self.s.le.clone() };
        Ok(matches!(self.send(i, &cmp, vec![lim])?, Value::Bool(true)))
    }

    /// Primitives that only send messages or run blocks are handled here, without nesting the
    /// interpreter; the rest are in `prims`.
    fn primitive(&mut self, name: &Sym, rcv: Value, mut args: Vec<Value>) -> D {
        match &**name {
            "primitivePerform:ifFail:"
            | "primitivePerform:with:ifFail:"
            | "primitivePerform:with:with:ifFail:"
            | "primitivePerform:with:with:with:ifFail:" => {
                args.pop();
                let sel = args.remove(0);
                if let Some(s) = self.selector_of(&sel) {
                    return self.dispatch(rcv, &s, args);
                }
            }
            "primitivePerform:arguments:ifFail:" => {
                if let Some(s) = self.selector_of(&args[0]) {
                    if let Value::Obj(o) = &args[1] {
                        if let Some(ix) = &o.indexed {
                            let a = ix.borrow().clone();
                            return self.dispatch(rcv, &s, a);
                        }
                    }
                }
            }
            "primitiveValueWithArguments:ifFail:" => {
                if let (Value::Block(b), Value::Obj(o)) = (&rcv, &args[0]) {
                    if let Some(ix) = &o.indexed {
                        if ix.borrow().len() == b.code.nparams {
                            let a = ix.borrow().clone();
                            let b = b.clone();
                            return self.push_block_frame(&b, a);
                        }
                    }
                }
            }
            _ => {}
        }
        crate::prims::call(self, name, rcv, args).map(Some)
    }

    fn selector_of(&mut self, v: &Value) -> Option<Sym> {
        match v {
            Value::Sym(s) => Some(self.intern(s)),
            Value::Str(s) => {
                let st: String = s.borrow().iter().collect();
                Some(self.intern(&st))
            }
            _ => None,
        }
    }
}
