//! The interpreter: message sending, blocks, non-local return, exceptions and file-in.

use crate::ast::{Lit, MethodAst};
use crate::compiler::{lit_value, Compiler};
use crate::object::*;
use crate::parser::Parser;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

pub struct ErrorInfo {
    pub message: String,
    pub class_name: String,
    pub stack: Vec<String>,
}

pub enum Unwind {
    Return(u64, Value),
    ExcReturn(u64, Value),
    Retry(u64),
    Resume(u64, Value),
    Error(Box<ErrorInfo>),
}

pub type R = Result<Value, Unwind>;

pub struct Handler {
    pub id: u64,
    pub class: Value,
    pub block: Value,
    pub disabled: u32,
}

pub struct Core {
    pub object: Rc<Class>,
    pub behavior: Rc<Class>,
    pub class: Rc<Class>,
    pub metaclass: Rc<Class>,
    pub undefined: Rc<Class>,
    pub boolean: Rc<Class>,
    pub true_: Rc<Class>,
    pub false_: Rc<Class>,
    pub magnitude: Rc<Class>,
    pub number: Rc<Class>,
    pub integer: Rc<Class>,
    pub small_int: Rc<Class>,
    pub large_int: Rc<Class>,
    pub float: Rc<Class>,
    pub character: Rc<Class>,
    pub collection: Rc<Class>,
    pub seq: Rc<Class>,
    pub arrayed: Rc<Class>,
    pub array: Rc<Class>,
    pub string: Rc<Class>,
    pub symbol: Rc<Class>,
    pub block: Rc<Class>,
    pub message: Rc<Class>,
    pub system: Rc<Class>,
    pub class_decl: Rc<Class>,
    pub mirror: Rc<Class>,
    pub method_reader: Rc<Class>,
    pub byte_array: Rc<Class>,
}

pub struct Syms {
    pub plus: Sym,
    pub minus: Sym,
    pub times: Sym,
    pub divide: Sym,
    pub lt: Sym,
    pub gt: Sym,
    pub le: Sym,
    pub ge: Sym,
    pub eq: Sym,
    pub ne: Sym,
    pub eqeq: Sym,
    pub value: Sym,
    pub value1: Sym,
    pub value2: Sym,
    pub value3: Sym,
    pub value4: Sym,
    pub dnu: Sym,
    pub print_string: Sym,
    pub signal: Sym,
    pub handles: Sym,
    pub default_action: Sym,
    pub message_text_set: Sym,
    pub description: Sym,
}

pub enum Event {
    Inspect(usize),
    Canvas(usize),
    ClearTranscript,
    Browse(String),
}

pub struct CanvasBuf {
    pub width: usize,
    pub height: usize,
    pub title: String,
    pub pixels: Vec<u32>,
}

#[derive(Default)]
pub struct FileInReport {
    pub methods: usize,
    pub doits: usize,
    pub errors: Vec<String>,
}

/// FxHash: the method cache is hit on every send, and its keys are already well mixed.
#[derive(Default, Clone, Copy)]
pub struct FxHasher(u64);
impl std::hash::Hasher for FxHasher {
    fn finish(&self) -> u64 {
        self.0
    }
    fn write(&mut self, bytes: &[u8]) {
        for b in bytes {
            self.0 = (self.0.rotate_left(5) ^ *b as u64).wrapping_mul(0x51_7c_c1_b7_27_22_0a_95);
        }
    }
    fn write_usize(&mut self, i: usize) {
        self.0 = (self.0.rotate_left(5) ^ i as u64).wrapping_mul(0x51_7c_c1_b7_27_22_0a_95);
    }
    fn write_u8(&mut self, i: u8) {
        self.write_usize(i as usize)
    }
}
pub type FxBuild = std::hash::BuildHasherDefault<FxHasher>;

pub struct Interp {
    pub globals: HashMap<String, Value>,
    pub core: Core,
    pub s: Syms,
    symtab: HashMap<String, Sym>,
    cache: HashMap<(usize, bool, usize), Option<Rc<Method>>, FxBuild>,
    pub vframes: Vec<crate::vm::VFrame>,
    pub ostack: Vec<Value>,
    next_id: u64,
    pub run_depth: usize,
    pub max_run_depth: usize,
    pub max_depth: usize,
    /// The configured limit (max_depth is raised briefly while an overflow is signalled).
    pub depth_limit: usize,
    pub handlers: Vec<Handler>,
    pub events: Vec<Event>,
    pub transcript_sink: Option<Box<dyn FnMut(&str)>>,
    pub transcript_buf: String,
    pub changes: Vec<String>,
    pub record_changes: bool,
    pub inspected: Vec<Value>,
    pub canvases: Vec<CanvasBuf>,
    pub workspaces: HashMap<String, Rc<RefCell<HashMap<String, Value>>>>,
    pub prims: HashMap<&'static str, crate::prims::PrimFn>,
    pub rng: u64,
    pub sends: u64,
}

impl Interp {
    pub fn new() -> Interp {
        let object = Class::new("Object", None, Format::Plain);
        let mk = |n: &str, s: &Rc<Class>, f: Format| Class::new(n, Some(s.clone()), f);
        let behavior = mk("Behavior", &object, Format::Special);
        let class = mk("Class", &behavior, Format::Special);
        let metaclass = mk("Metaclass", &behavior, Format::Special);
        let undefined = mk("UndefinedObject", &object, Format::Special);
        let boolean = mk("Boolean", &object, Format::Plain);
        let true_ = mk("True", &boolean, Format::Special);
        let false_ = mk("False", &boolean, Format::Special);
        let magnitude = mk("Magnitude", &object, Format::Plain);
        let number = mk("Number", &magnitude, Format::Plain);
        let integer = mk("Integer", &number, Format::Plain);
        let small_int = mk("SmallInteger", &integer, Format::Special);
        let large_int = mk("LargeInteger", &integer, Format::Special);
        let float = mk("Float", &number, Format::Special);
        let character = mk("Character", &magnitude, Format::Special);
        let collection = mk("Collection", &object, Format::Plain);
        let seq = mk("SequenceableCollection", &collection, Format::Plain);
        let arrayed = mk("ArrayedCollection", &seq, Format::Plain);
        let array = mk("Array", &arrayed, Format::Indexed);
        let string = mk("String", &arrayed, Format::Special);
        let symbol = mk("Symbol", &string, Format::Special);
        let byte_array = mk("ByteArray", &arrayed, Format::Indexed);
        let block = mk("BlockClosure", &object, Format::Special);
        let message = mk("Message", &object, Format::Plain);
        let system = mk("SystemDictionary", &object, Format::Plain);
        let class_decl = mk("ClassDeclaration", &object, Format::Plain);
        let mirror = mk("ClassMirror", &object, Format::Plain);
        let method_reader = mk("MethodReader", &object, Format::Plain);
        let core = Core {
            object, behavior, class, metaclass, undefined, boolean, true_, false_, magnitude, number, integer,
            small_int, large_int, float, character, collection, seq, arrayed, array, string, symbol, block,
            message, system, class_decl, mirror, method_reader, byte_array,
        };
        let mut symtab: HashMap<String, Sym> = HashMap::new();
        let mut sy = |s: &str| -> Sym {
            let r: Sym = Rc::from(s);
            symtab.insert(s.to_string(), r.clone());
            r
        };
        let s = Syms {
            plus: sy("+"),
            minus: sy("-"),
            times: sy("*"),
            divide: sy("/"),
            lt: sy("<"),
            gt: sy(">"),
            le: sy("<="),
            ge: sy(">="),
            eq: sy("="),
            ne: sy("~="),
            eqeq: sy("=="),
            value: sy("value"),
            value1: sy("value:"),
            value2: sy("value:value:"),
            value3: sy("value:value:value:"),
            value4: sy("value:value:value:value:"),
            dnu: sy("doesNotUnderstand:"),
            print_string: sy("printString"),
            signal: sy("signal"),
            handles: sy("handles:"),
            default_action: sy("defaultAction"),
            message_text_set: sy("messageText:"),
            description: sy("description"),
        };
        let mut it = Interp {
            globals: HashMap::new(),
            core,
            s,
            symtab,
            cache: HashMap::default(),
            vframes: Vec::new(),
            ostack: Vec::new(),
            next_id: 1,
            run_depth: 0,
            max_run_depth: 2_000,
            max_depth: 100_000,
            depth_limit: 100_000,
            handlers: Vec::new(),
            events: Vec::new(),
            transcript_sink: None,
            transcript_buf: String::new(),
            changes: Vec::new(),
            record_changes: false,
            inspected: Vec::new(),
            canvases: Vec::new(),
            workspaces: HashMap::new(),
            prims: crate::prims::table(),
            rng: 0x2545F4914F6CDD1D,
            sends: 0,
        };
        let c = &it.core;
        let all = vec![
            c.object.clone(), c.behavior.clone(), c.class.clone(), c.metaclass.clone(), c.undefined.clone(),
            c.boolean.clone(), c.true_.clone(), c.false_.clone(), c.magnitude.clone(), c.number.clone(),
            c.integer.clone(), c.small_int.clone(), c.large_int.clone(), c.float.clone(), c.character.clone(),
            c.collection.clone(), c.seq.clone(), c.arrayed.clone(), c.array.clone(), c.string.clone(),
            c.symbol.clone(), c.block.clone(), c.message.clone(), c.system.clone(), c.class_decl.clone(),
            c.mirror.clone(), c.method_reader.clone(), c.byte_array.clone(),
        ];
        for k in all {
            *k.group.borrow_mut() = "kernel".into();
            it.globals.insert(k.name(), Value::Class(k));
        }
        it.core.message.ivars.borrow_mut().extend(crate::parser::parse_var_decls("receiver <Object> selector <Symbol> arguments <Array>").unwrap_or_default());
        let sys = it.new_object(&it.core.system.clone());
        it.globals.insert("Smalltalk".into(), sys.clone());
        it.globals.insert("Delta".into(), sys);
        crate::natives::install(&mut it);
        it
    }

    pub fn intern(&mut self, s: &str) -> Sym {
        if let Some(x) = self.symtab.get(s) {
            return x.clone();
        }
        let r: Sym = Rc::from(s);
        self.symtab.insert(s.to_string(), r.clone());
        r
    }

    pub fn flush_cache(&mut self) {
        self.cache.clear();
    }

    pub fn new_object(&self, cls: &Rc<Class>) -> Value {
        let n = cls.all_ivars().len();
        let indexed = if cls.format.get() == Format::Indexed { Some(RefCell::new(Vec::new())) } else { None };
        Value::Obj(Rc::new(Object { class: RefCell::new(cls.clone()), slots: RefCell::new(vec![Value::Nil; n]), indexed }))
    }

    pub fn new_indexed(&self, cls: &Rc<Class>, items: Vec<Value>) -> Value {
        let n = cls.all_ivars().len();
        Value::Obj(Rc::new(Object {
            class: RefCell::new(cls.clone()),
            slots: RefCell::new(vec![Value::Nil; n]),
            indexed: Some(RefCell::new(items)),
        }))
    }

    pub fn array(&self, items: Vec<Value>) -> Value {
        self.new_indexed(&self.core.array.clone(), items)
    }

    pub fn class_named(&self, name: &str) -> Option<Rc<Class>> {
        match self.globals.get(name) {
            Some(Value::Class(c)) => Some(c.clone()),
            _ => None,
        }
    }

    /// The class (and side) that answers messages sent to `v`.
    pub fn behavior_of(&self, v: &Value) -> (Rc<Class>, bool) {
        let c = &self.core;
        match v {
            Value::Nil => (c.undefined.clone(), false),
            Value::Bool(true) => (c.true_.clone(), false),
            Value::Bool(false) => (c.false_.clone(), false),
            Value::Int(_) => (c.small_int.clone(), false),
            Value::Big(_) => (c.large_int.clone(), false),
            Value::Float(_) => (c.float.clone(), false),
            Value::Char(_) => (c.character.clone(), false),
            Value::Str(_) => (c.string.clone(), false),
            Value::Sym(_) => (c.symbol.clone(), false),
            Value::Obj(o) => (o.class.borrow().clone(), false),
            Value::Block(_) => (c.block.clone(), false),
            Value::Class(k) => (k.clone(), true),
            Value::Meta(_) => (c.metaclass.clone(), false),
        }
    }

    /// The object `v class` answers.
    pub fn class_value_of(&self, v: &Value) -> Value {
        match v {
            Value::Class(k) => Value::Meta(k.clone()),
            _ => Value::Class(self.behavior_of(v).0),
        }
    }

    #[inline(never)]
    pub fn lookup(&mut self, cls: &Rc<Class>, meta: bool, sel: &Sym) -> Option<Rc<Method>> {
        let key = (Rc::as_ptr(cls) as *const u8 as usize, meta, Rc::as_ptr(sel) as *const u8 as usize);
        if let Some(m) = self.cache.get(&key) {
            return m.clone();
        }
        let m = self.lookup_from(Some(cls.clone()), meta, sel);
        self.cache.insert(key, m.clone());
        m
    }

    pub fn lookup_from(&self, start: Option<Rc<Class>>, meta: bool, sel: &Sym) -> Option<Rc<Method>> {
        let mut c = start;
        while let Some(k) = c {
            if let Some(m) = k.method_dict(meta).borrow().get(sel) {
                return Some(m.clone());
            }
            c = k.superclass();
        }
        if meta {
            return self.lookup_from(Some(self.core.class.clone()), false, sel);
        }
        None
    }

    pub fn responds_to(&mut self, v: &Value, sel: &Sym) -> bool {
        let (c, m) = self.behavior_of(v);
        self.lookup(&c, m, sel).is_some()
    }

    pub fn send(&mut self, rcv: Value, sel: &Sym, args: Vec<Value>) -> R {
        let base = self.vframes.len();
        match self.dispatch(rcv, sel, args)? {
            Some(v) => Ok(v),
            None => self.run(base),
        }
    }

    pub fn next_id(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    pub fn send_str(&mut self, rcv: Value, sel: &str, args: Vec<Value>) -> R {
        let s = self.intern(sel);
        self.send(rcv, &s, args)
    }

    #[inline(never)]
    pub fn fast_int(&self, a: i64, sel: &Sym, b: i64) -> Option<Value> {
        let s = &self.s;
        let p = |x: &Sym| Rc::ptr_eq(sel, x);
        if p(&s.plus) {
            Some(Value::int(a + b))
        } else if p(&s.minus) {
            Some(Value::int(a - b))
        } else if p(&s.lt) {
            Some(Value::Bool(a < b))
        } else if p(&s.gt) {
            Some(Value::Bool(a > b))
        } else if p(&s.le) {
            Some(Value::Bool(a <= b))
        } else if p(&s.ge) {
            Some(Value::Bool(a >= b))
        } else if p(&s.eq) || p(&s.eqeq) {
            Some(Value::Bool(a == b))
        } else if p(&s.ne) {
            Some(Value::Bool(a != b))
        } else if p(&s.times) {
            Some(match a.checked_mul(b) {
                Some(v) => Value::int(v),
                None => Value::big(num_bigint::BigInt::from(a) * num_bigint::BigInt::from(b)),
            })
        } else {
            None
        }
    }

    #[inline(never)]
    pub fn fast_float(&self, a: f64, sel: &Sym, b: f64) -> Option<Value> {
        let s = &self.s;
        let p = |x: &Sym| Rc::ptr_eq(sel, x);
        if p(&s.plus) {
            Some(Value::Float(a + b))
        } else if p(&s.minus) {
            Some(Value::Float(a - b))
        } else if p(&s.times) {
            Some(Value::Float(a * b))
        } else if p(&s.divide) && b != 0.0 {
            Some(Value::Float(a / b))
        } else if p(&s.lt) {
            Some(Value::Bool(a < b))
        } else if p(&s.gt) {
            Some(Value::Bool(a > b))
        } else if p(&s.le) {
            Some(Value::Bool(a <= b))
        } else if p(&s.ge) {
            Some(Value::Bool(a >= b))
        } else if p(&s.eq) {
            Some(Value::Bool(a == b))
        } else if p(&s.ne) {
            Some(Value::Bool(a != b))
        } else {
            None
        }
    }

    pub fn invoke(&mut self, m: &Rc<Method>, rcv: Value, args: Vec<Value>) -> R {
        let base = self.vframes.len();
        match self.dispatch_method(m.clone(), rcv, args)? {
            Some(v) => Ok(v),
            None => self.run(base),
        }
    }

    pub fn call_block(&mut self, b: &Rc<Closure>, args: Vec<Value>) -> R {
        let base = self.vframes.len();
        match self.push_block_frame(b, args)? {
            Some(v) => Ok(v),
            None => self.run(base),
        }
    }

    /// Calls a block with as many of `args` as it takes (for fail blocks and handlers).
    pub fn call_flexible(&mut self, v: &Value, args: Vec<Value>) -> R {
        match v {
            Value::Block(b) => {
                let n = b.code.nparams;
                let mut a = args;
                a.truncate(n);
                while a.len() < n {
                    a.push(Value::Nil);
                }
                let b = b.clone();
                self.call_block(&b, a)
            }
            other => Ok(other.clone()),
        }
    }

    pub fn trace(&self) -> Vec<String> {
        let mut out = Vec::new();
        for f in self.vframes.iter().rev() {
            let label = match &f.method {
                Some(m) => method_label(m),
                None => match self.vframes.iter().find(|x| x.id == f.home && f.home != 0) {
                    Some(crate::vm::VFrame { method: Some(m), .. }) => format!("[] in {}", method_label(m)),
                    _ => "[] in Doit".to_string(),
                },
            };
            out.push(label);
            if out.len() >= 40 {
                out.push("...".into());
                break;
            }
        }
        out
    }

    /// Signals a catchable Error, with some room above the limit for its handlers to run.
    #[cold]
    #[inline(never)]
    pub fn stack_overflow(&mut self) -> R {
        if self.max_depth > self.depth_limit {
            return Err(self.overflow());
        }
        self.max_depth = self.depth_limit + 5_000;
        let r = self.signal_error("Error", "Stack overflow: the call chain is too deep (infinite recursion?)");
        self.max_depth = self.depth_limit;
        r
    }

    #[cold]
    #[inline(never)]
    pub fn overflow(&self) -> Unwind {
        self.hard_error("Stack overflow: the call chain is too deep (infinite recursion?)")
    }

    #[inline(never)]
    pub fn hard_error(&self, msg: &str) -> Unwind {
        Unwind::Error(Box::new(ErrorInfo { message: msg.to_string(), class_name: "Error".into(), stack: self.trace() }))
    }

    /// Signals a Smalltalk exception of the named class so handlers can catch it.
    #[inline(never)]
    pub fn signal_error(&mut self, class: &str, msg: &str) -> R {
        let cls = self.class_named(class).or_else(|| self.class_named("Error"));
        match cls {
            Some(c) if c.all_ivars().iter().any(|v| v.name == "messageText") => {
                let e = self.new_object(&c);
                let s = self.s.message_text_set.clone();
                self.send(e.clone(), &s, vec![Value::str(msg)])?;
                let sig = self.s.signal.clone();
                self.send(e, &sig, vec![])
            }
            _ => Err(self.hard_error(msg)),
        }
    }

    pub fn non_boolean(&mut self, v: Value) -> R {
        let (c, _) = self.behavior_of(&v);
        self.signal_error("Error", &format!("NonBooleanReceiver: proceed for truth (got a {})", c.name()))
    }

    pub fn lit_to_value(&mut self, l: &Lit) -> Value {
        match l {
            Lit::Array(items) => {
                let v: Vec<Value> = items.iter().map(|x| self.lit_to_value(x)).collect();
                self.array(v)
            }
            Lit::Bytes(b) => {
                let v = b.iter().map(|x| Value::Int(*x as i64)).collect();
                self.new_indexed(&self.core.byte_array.clone(), v)
            }
            other => {
                let mut f = |s: &str| self.intern(s);
                lit_value(other, &mut f)
            }
        }
    }

    pub fn lookup_name(&mut self, name: &Rc<str>, cls: &Option<Rc<Class>>) -> R {
        if let Some(c) = cls {
            if let Some(owner) = c.find_class_var_owner(name) {
                return Ok(owner.class_var_values.borrow().get(&**name).cloned().unwrap_or(Value::Nil));
            }
        }
        match self.globals.get(&**name) {
            Some(v) => Ok(v.clone()),
            None => self.signal_error("Error", &format!("undefined variable '{}'", name)),
        }
    }

    // ---------- exceptions ----------

    pub fn on_do(&mut self, body: Value, class: Value, handler: Value) -> R {
        let b = match &body {
            Value::Block(b) => b.clone(),
            _ => return self.signal_error("Error", "on:do: needs a block receiver"),
        };
        loop {
            let id = self.next_id();
            self.handlers.push(Handler { id, class: class.clone(), block: handler.clone(), disabled: 0 });
            let keep = self.handlers.len() - 1;
            let r = self.call_block(&b, vec![]);
            self.handlers.truncate(keep);
            match r {
                Err(Unwind::ExcReturn(h, v)) if h == id => return Ok(v),
                Err(Unwind::Retry(h)) if h == id => continue,
                other => return other,
            }
        }
    }

    pub fn ivar_index(&self, v: &Value, name: &str) -> Option<usize> {
        if let Value::Obj(o) = v {
            let cls = o.class.borrow().clone();
            return cls.all_ivars().iter().position(|x| x.name == name);
        }
        None
    }

    pub fn set_ivar(&self, v: &Value, name: &str, x: Value) {
        if let (Some(i), Value::Obj(o)) = (self.ivar_index(v, name), v) {
            o.set(i, x);
        }
    }

    pub fn get_ivar(&self, v: &Value, name: &str) -> Value {
        match (self.ivar_index(v, name), v) {
            (Some(i), Value::Obj(o)) => o.get(i),
            _ => Value::Nil,
        }
    }

    pub fn signal(&mut self, exc: Value) -> R {
        let mut i = self.handlers.len();
        let handles = self.s.handles.clone();
        while i > 0 {
            i -= 1;
            if self.handlers[i].disabled > 0 {
                continue;
            }
            let hclass = self.handlers[i].class.clone();
            let ok = self.send(hclass, &handles, vec![exc.clone()])?;
            if matches!(ok, Value::Bool(true)) {
                let (hid, hblock) = (self.handlers[i].id, self.handlers[i].block.clone());
                let sid = self.next_id();
                self.set_ivar(&exc, "handlerId", Value::Int(hid as i64));
                self.set_ivar(&exc, "signalId", Value::Int(sid as i64));
                let top = self.handlers.len();
                for h in &mut self.handlers[i..top] {
                    h.disabled += 1;
                }
                let r = self.call_flexible(&hblock, vec![exc.clone()]);
                let top2 = top.min(self.handlers.len());
                for h in &mut self.handlers[i..top2] {
                    h.disabled = h.disabled.saturating_sub(1);
                }
                return match r {
                    Ok(v) => Err(Unwind::ExcReturn(hid, v)),
                    Err(Unwind::Resume(s, v)) if s == sid => Ok(v),
                    Err(e) => Err(e),
                };
            }
        }
        let da = self.s.default_action.clone();
        self.send(exc, &da, vec![])
    }

    pub fn unhandled(&mut self, exc: Value) -> Unwind {
        let d = self.s.description.clone();
        let text = match self.send(exc.clone(), &d, vec![]) {
            Ok(Value::Str(s)) => s.borrow().iter().collect::<String>(),
            _ => "an exception".to_string(),
        };
        let (cls, _) = self.behavior_of(&exc);
        let mut stack = self.trace();
        // drop the frames of the exception machinery itself
        while stack.first().map(|s| s.starts_with("Exception") || s.starts_with("Error") || s.contains(">>signal") || s.contains(">>defaultAction")).unwrap_or(false) && stack.len() > 1 {
            stack.remove(0);
        }
        Unwind::Error(Box::new(ErrorInfo { message: text, class_name: cls.name(), stack }))
    }

    // ---------- printing ----------

    pub fn print_string(&mut self, v: &Value) -> String {
        let ps = self.s.print_string.clone();
        match self.send(v.clone(), &ps, vec![]) {
            Ok(Value::Str(s)) => s.borrow().iter().collect(),
            Ok(Value::Sym(s)) => s.to_string(),
            _ => {
                let (c, m) = self.behavior_of(v);
                if m {
                    c.name()
                } else {
                    format!("a {}", c.name())
                }
            }
        }
    }

    pub fn transcript_write(&mut self, s: &str) {
        if let Some(f) = self.transcript_sink.as_mut() {
            f(s);
        } else {
            self.transcript_buf.push_str(s);
        }
    }

    // ---------- compiling ----------

    pub fn compile_method(&mut self, cls: &Rc<Class>, meta: bool, category: &str, source: &str) -> Result<Rc<Method>, (String, usize)> {
        let mut p = Parser::new(source).map_err(|e| (e.msg, e.pos))?;
        let ast = p.parse_method().map_err(|e| (e.msg, e.pos))?;
        let code = {
            let symtab = &mut self.symtab;
            let mut f = |s: &str| -> Sym {
                if let Some(x) = symtab.get(s) {
                    return x.clone();
                }
                let r: Sym = Rc::from(s);
                symtab.insert(s.to_string(), r.clone());
                r
            };
            let mut comp = Compiler::new(Some(cls.clone()), meta, None, &mut f);
            let tree = comp.compile_method(&ast).map_err(|e| (e.msg, e.pos))?;
            let mut code = crate::codegen::method_code(tree.nargs, tree.nslots, &tree.body);
            crate::codegen::intern_all(&mut code, &mut f);
            MethodCode { nargs: tree.nargs, code: Rc::new(code) }
        };
        let sel = self.intern(&ast.selector);
        let category = if category.is_empty() {
            cls.method_dict(meta).borrow().get(&sel).map(|m| m.category.clone()).unwrap_or_else(|| "as yet unclassified".into())
        } else {
            category.to_string()
        };
        let m = Rc::new(Method {
            selector: sel.clone(),
            class: cls.clone(),
            meta,
            category: category.clone(),
            source: source.trim_matches(|c| c == '\n' || c == '\r').to_string(),
            signature: ast.signature(),
            ast: Some(Rc::new(ast)),
            body: MethodBody::Code(Rc::new(code)),
        });
        cls.method_dict(meta).borrow_mut().insert(sel, m.clone());
        self.flush_cache();
        if self.record_changes {
            let side = if meta { " classSide" } else { "" };
            self.changes.push(format!(
                "! (Delta mirrorFor: #{}){} methodsFor: '{}' !\n{}\n! !\n",
                cls.name(),
                side,
                category.replace('\'', "''"),
                m.source.replace('!', "!!")
            ));
        }
        Ok(m)
    }

    /// Recompiles the methods of `cls` and its subclasses (after instance variables changed).
    pub fn recompile_tree(&mut self, cls: &Rc<Class>) -> Vec<String> {
        let mut errors = Vec::new();
        let classes: Vec<Rc<Class>> = self
            .globals
            .values()
            .filter_map(|v| match v {
                Value::Class(k) if k.inherits_from(cls) => Some(k.clone()),
                _ => None,
            })
            .collect();
        let rec = self.record_changes;
        self.record_changes = false;
        for k in classes {
            for meta in [false, true] {
                let ms: Vec<Rc<Method>> = k.method_dict(meta).borrow().values().cloned().collect();
                for m in ms {
                    if let MethodBody::Code(_) = m.body {
                        if let Err((e, _)) = self.compile_method(&k, meta, &m.category, &m.source) {
                            errors.push(format!("{}: {}", method_label(&m), e));
                        }
                    }
                }
            }
        }
        self.record_changes = rec;
        errors
    }

    pub fn workspace(&mut self, id: &str) -> Rc<RefCell<HashMap<String, Value>>> {
        self.workspaces.entry(id.to_string()).or_insert_with(|| Rc::new(RefCell::new(HashMap::new()))).clone()
    }

    /// Compiles and runs a do-it. Uncaught errors come back as `ErrorInfo`.
    pub fn evaluate(&mut self, source: &str, workspace: Option<&str>, receiver: Value) -> Result<Value, ErrorInfo> {
        let ws = workspace.map(|w| self.workspace(w));
        let mut p = Parser::new(source).map_err(|e| syntax_error(source, &e.msg, e.pos))?;
        let (temps, body) = p.parse_doit().map_err(|e| syntax_error(source, &e.msg, e.pos))?;
        let (cls, meta) = match &receiver {
            Value::Nil => (None, false),
            v => {
                let (c, m) = self.behavior_of(v);
                (Some(c), m)
            }
        };
        let code = {
            let symtab = &mut self.symtab;
            let mut f = |s: &str| -> Sym {
                if let Some(x) = symtab.get(s) {
                    return x.clone();
                }
                let r: Sym = Rc::from(s);
                symtab.insert(s.to_string(), r.clone());
                r
            };
            let mut comp = Compiler::new(cls, meta, ws, &mut f);
            let tree = comp.compile_doit(&temps, &body).map_err(|e| syntax_error(source, &e.msg, e.pos))?;
            let mut code = crate::codegen::method_code(0, tree.nslots, &tree.body);
            crate::codegen::intern_all(&mut code, &mut f);
            MethodCode { nargs: 0, code: Rc::new(code) }
        };
        let doit = Rc::new(Method {
            selector: self.intern("DoIt"),
            class: self.core.undefined.clone(),
            meta: false,
            category: String::new(),
            source: source.to_string(),
            signature: "DoIt".into(),
            ast: None,
            body: MethodBody::Code(Rc::new(code)),
        });
        let top = self.vframes.is_empty();
        if top {
            self.ostack.clear();
            self.run_depth = 0;
            self.handlers.clear();
        }
        let r = self.invoke(&doit, receiver, vec![]);
        if top {
            self.vframes.clear();
            self.ostack.clear();
            self.handlers.clear();
            self.run_depth = 0;
        }
        match r {
            Ok(v) => Ok(v),
            Err(Unwind::Error(e)) => Err(*e),
            Err(Unwind::Return(_, v)) => Ok(v),
            Err(_) => Err(ErrorInfo {
                message: "an exception handler was used after its on:do: had already finished".into(),
                class_name: "Error".into(),
                stack: vec![],
            }),
        }
    }

    /// Reads Smalltalk chunk format (`!`-separated), as Strongtalk's file-in does.
    pub fn file_in(&mut self, text: &str) -> FileInReport {
        let mut report = FileInReport::default();
        let mut reader: Option<(Rc<Class>, bool, String)> = None;
        for (chunk, line) in chunks(text) {
            if let Some((cls, meta, cat)) = &reader {
                if chunk.trim().is_empty() {
                    reader = None;
                    continue;
                }
                let (cls, meta, cat) = (cls.clone(), *meta, cat.clone());
                match self.compile_method(&cls, meta, &cat, &chunk) {
                    Ok(_) => report.methods += 1,
                    Err((e, pos)) => {
                        let sel = crate::parser::quick_selector(&chunk).unwrap_or_else(|| first_line(&chunk));
                        let l = line + chunk[..pos.min(chunk.len())].matches('\n').count();
                        report.errors.push(format!("line {}: {}{}>>{}: {}", l, cls.name(), if meta { " class" } else { "" }, sel, e));
                    }
                }
                continue;
            }
            if chunk.trim().is_empty() {
                continue;
            }
            match self.evaluate(&chunk, None, Value::Nil) {
                Ok(v) => {
                    report.doits += 1;
                    if let Value::Obj(o) = &v {
                        if Rc::ptr_eq(&o.class.borrow(), &self.core.method_reader) {
                            if let (Value::Class(c), Value::Bool(m), Value::Str(cat)) = (o.get(0), o.get(1), o.get(2)) {
                                let cat: String = cat.borrow().iter().collect();
                                reader = Some((c, m, cat));
                            }
                        }
                    }
                }
                Err(e) => report.errors.push(format!("line {}: {}", line, e.message)),
            }
        }
        report
    }

    pub fn now_ms() -> f64 {
        #[cfg(target_arch = "wasm32")]
        {
            js_sys::Date::now()
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            use std::time::{SystemTime, UNIX_EPOCH};
            SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs_f64() * 1000.0).unwrap_or(0.0)
        }
    }
}

impl Default for Interp {
    fn default() -> Self {
        Interp::new()
    }
}

pub fn method_label(m: &Method) -> String {
    format!("{}{}>>{}", m.class.name(), if m.meta { " class" } else { "" }, m.selector)
}

fn first_line(s: &str) -> String {
    s.trim().lines().next().unwrap_or("").chars().take(40).collect()
}

pub fn syntax_error(src: &str, msg: &str, pos: usize) -> ErrorInfo {
    let pos = pos.min(src.len());
    let line = src[..pos].matches('\n').count() + 1;
    ErrorInfo { message: format!("Syntax error (line {}): {}", line, msg), class_name: "SyntaxError".into(), stack: vec![format!("@{}", pos)] }
}

/// Splits chunk-format text into chunks with their starting line numbers.
pub fn chunks(text: &str) -> Vec<(String, usize)> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut line = 1usize;
    let mut start_line = 1usize;
    let mut it = text.chars().peekable();
    while let Some(ch) = it.next() {
        if ch == '!' {
            if it.peek() == Some(&'!') {
                it.next();
                cur.push('!');
                continue;
            }
            out.push((std::mem::take(&mut cur), start_line));
            start_line = line;
            continue;
        }
        if ch == '\n' {
            line += 1;
        }
        cur.push(ch);
    }
    if !cur.trim().is_empty() {
        out.push((cur, start_line));
    }
    out
}

pub fn method_ast(m: &Method) -> Option<Rc<MethodAst>> {
    m.ast.clone()
}
