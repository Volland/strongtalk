//! The object memory: values, classes, methods, frames and closures.

use crate::ast::{MethodAst, TypeExpr, Var};
use crate::compiler::Node;
use num_bigint::BigInt;
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

pub type Sym = Rc<str>;

#[derive(Clone)]
pub enum Value {
    Nil,
    Bool(bool),
    Int(i64),
    Big(Rc<BigInt>),
    Float(f64),
    Char(char),
    Str(Rc<RefCell<Vec<char>>>),
    Sym(Sym),
    Obj(Rc<Object>),
    Block(Rc<Closure>),
    Class(Rc<Class>),
    Meta(Rc<Class>),
}

/// SmallInteger range of the original 32-bit VM: oops carry a 2-bit tag (00 = SmallInteger,
/// 01 = heap object) and values are decoded with `sar 2` (see MECHANICS.md, 0x4ccc5a/0x4cccd6),
/// so SmallIntegers hold 30-bit signed values.
pub const SMI_MAX: i64 = (1 << 29) - 1;
pub const SMI_MIN: i64 = -(1 << 29);

impl Value {
    pub fn str(s: &str) -> Value {
        Value::Str(Rc::new(RefCell::new(s.chars().collect())))
    }
    pub fn from_chars(v: Vec<char>) -> Value {
        Value::Str(Rc::new(RefCell::new(v)))
    }
    pub fn int(i: i64) -> Value {
        if (SMI_MIN..=SMI_MAX).contains(&i) {
            Value::Int(i)
        } else {
            Value::Big(Rc::new(BigInt::from(i)))
        }
    }
    pub fn big(b: BigInt) -> Value {
        match i64::try_from(&b) {
            Ok(i) if (SMI_MIN..=SMI_MAX).contains(&i) => Value::Int(i),
            _ => Value::Big(Rc::new(b)),
        }
    }
    pub fn is_nil(&self) -> bool {
        matches!(self, Value::Nil)
    }
    pub fn identical(&self, o: &Value) -> bool {
        match (self, o) {
            (Value::Nil, Value::Nil) => true,
            (Value::Bool(a), Value::Bool(b)) => a == b,
            (Value::Int(a), Value::Int(b)) => a == b,
            (Value::Big(a), Value::Big(b)) => Rc::ptr_eq(a, b) || a == b,
            (Value::Float(a), Value::Float(b)) => a.to_bits() == b.to_bits(),
            (Value::Char(a), Value::Char(b)) => a == b,
            (Value::Str(a), Value::Str(b)) => Rc::ptr_eq(a, b),
            (Value::Sym(a), Value::Sym(b)) => a == b,
            (Value::Obj(a), Value::Obj(b)) => Rc::ptr_eq(a, b),
            (Value::Block(a), Value::Block(b)) => Rc::ptr_eq(a, b),
            (Value::Class(a), Value::Class(b)) => Rc::ptr_eq(a, b),
            (Value::Meta(a), Value::Meta(b)) => Rc::ptr_eq(a, b),
            _ => false,
        }
    }
    pub fn identity_hash(&self) -> i64 {
        let p = match self {
            Value::Nil => 1,
            Value::Bool(b) => 2 + *b as usize,
            Value::Int(i) => return *i,
            Value::Big(b) => return (b.iter_u32_digits().next().unwrap_or(0) & 0x3fff_ffff) as i64,
            Value::Float(f) => (f.to_bits() ^ (f.to_bits() >> 32)) as usize,
            Value::Char(c) => *c as usize,
            Value::Str(s) => Rc::as_ptr(s) as *const u8 as usize,
            Value::Sym(s) => {
                let mut h: u64 = 1469598103934665603;
                for b in s.bytes() {
                    h = (h ^ b as u64).wrapping_mul(1099511628211);
                }
                h as usize
            }
            Value::Obj(o) => Rc::as_ptr(o) as *const u8 as usize,
            Value::Block(b) => Rc::as_ptr(b) as *const u8 as usize,
            Value::Class(c) => Rc::as_ptr(c) as *const u8 as usize,
            Value::Meta(c) => (Rc::as_ptr(c) as *const u8 as usize) ^ 0x5555,
        };
        ((p >> 3) & 0x3fff_ffff) as i64
    }
}

pub struct Object {
    pub class: RefCell<Rc<Class>>,
    pub slots: RefCell<Vec<Value>>,
    pub indexed: Option<RefCell<Vec<Value>>>,
}

impl Object {
    pub fn get(&self, i: usize) -> Value {
        self.slots.borrow().get(i).cloned().unwrap_or(Value::Nil)
    }
    pub fn set(&self, i: usize, v: Value) {
        let mut s = self.slots.borrow_mut();
        if i >= s.len() {
            s.resize(i + 1, Value::Nil);
        }
        s[i] = v;
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Format {
    /// Named instance variables only.
    Plain,
    /// Named instance variables plus indexed slots (Array and subclasses).
    Indexed,
    /// Values the VM represents directly (SmallInteger, String, Block...).
    Special,
}

pub struct Class {
    pub name: RefCell<String>,
    pub superclass: RefCell<Option<Rc<Class>>>,
    /// The superclass as written, with generic arguments (`Collection[E]`).
    pub super_text: RefCell<String>,
    pub generics: RefCell<Vec<Var>>,
    pub ivars: RefCell<Vec<Var>>,
    pub class_ivars: RefCell<Vec<Var>>,
    pub class_ivar_values: RefCell<HashMap<String, Value>>,
    pub class_vars: RefCell<Vec<(String, Option<TypeExpr>)>>,
    pub class_var_values: RefCell<HashMap<String, Value>>,
    pub methods: RefCell<HashMap<Sym, Rc<Method>>>,
    pub class_methods: RefCell<HashMap<Sym, Rc<Method>>>,
    pub format: Cell<Format>,
    pub comment: RefCell<String>,
    pub group: RefCell<String>,
    pub is_protocol: Cell<bool>,
}

impl Class {
    pub fn new(name: &str, sup: Option<Rc<Class>>, format: Format) -> Rc<Class> {
        Rc::new(Class {
            super_text: RefCell::new(sup.as_ref().map(|s| s.name.borrow().clone()).unwrap_or_else(|| "nil".into())),
            name: RefCell::new(name.to_string()),
            superclass: RefCell::new(sup),
            generics: RefCell::new(vec![]),
            ivars: RefCell::new(vec![]),
            class_ivars: RefCell::new(vec![]),
            class_ivar_values: RefCell::new(HashMap::new()),
            class_vars: RefCell::new(vec![]),
            class_var_values: RefCell::new(HashMap::new()),
            methods: RefCell::new(HashMap::new()),
            class_methods: RefCell::new(HashMap::new()),
            format: Cell::new(format),
            comment: RefCell::new(String::new()),
            group: RefCell::new(String::new()),
            is_protocol: Cell::new(false),
        })
    }
    pub fn name(&self) -> String {
        self.name.borrow().clone()
    }
    pub fn superclass(&self) -> Option<Rc<Class>> {
        self.superclass.borrow().clone()
    }
    /// All named instance variables including inherited ones, in slot order.
    pub fn all_ivars(&self) -> Vec<Var> {
        let mut v = match self.superclass() {
            Some(s) => s.all_ivars(),
            None => vec![],
        };
        v.extend(self.ivars.borrow().iter().cloned());
        v
    }
    pub fn all_class_ivars(&self) -> Vec<Var> {
        let mut v = match self.superclass() {
            Some(s) => s.all_class_ivars(),
            None => vec![],
        };
        v.extend(self.class_ivars.borrow().iter().cloned());
        v
    }
    pub fn inherits_from(self: &Rc<Class>, other: &Rc<Class>) -> bool {
        let mut c = Some(self.clone());
        while let Some(k) = c {
            if Rc::ptr_eq(&k, other) {
                return true;
            }
            c = k.superclass();
        }
        false
    }
    pub fn find_class_var_owner(self: &Rc<Class>, name: &str) -> Option<Rc<Class>> {
        let mut c = Some(self.clone());
        while let Some(k) = c {
            if k.class_vars.borrow().iter().any(|(n, _)| n == name) {
                return Some(k);
            }
            c = k.superclass();
        }
        None
    }
    pub fn method_dict(&self, meta: bool) -> &RefCell<HashMap<Sym, Rc<Method>>> {
        if meta {
            &self.class_methods
        } else {
            &self.methods
        }
    }
}

pub type NativeFn = fn(&mut crate::interp::Interp, Value, Vec<Value>) -> crate::interp::R;

pub enum MethodBody {
    Code(Rc<MethodCode>),
    Native(NativeFn),
}

/// A compiled method body before code generation.
pub struct MethodTree {
    pub nargs: usize,
    pub nslots: usize,
    pub body: Node,
}

pub struct MethodCode {
    pub nargs: usize,
    pub code: Rc<crate::codegen::Code>,
}

pub struct Method {
    pub selector: Sym,
    pub class: Rc<Class>,
    pub meta: bool,
    pub category: String,
    pub source: String,
    pub signature: String,
    pub ast: Option<Rc<MethodAst>>,
    pub body: MethodBody,
}

pub struct Frame {
    pub slots: RefCell<Vec<Value>>,
    pub parent: Option<Rc<Frame>>,
}

pub struct BlockCode {
    pub nparams: usize,
    pub nslots: usize,
    pub body: Node,
}

pub struct Closure {
    pub code: Rc<crate::codegen::Code>,
    pub outer: Rc<Frame>,
    pub receiver: Value,
    pub home: u64,
}
