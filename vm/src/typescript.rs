//! Strongtalk → TypeScript translator.
//!
//! Translates either a file in chunk ("file-in") format with class definitions, or workspace
//! code (a do-it), into one self-contained TypeScript file:
//!
//! * classes become TypeScript classes (class-side methods become `static`), Strongtalk type
//!   annotations become TypeScript types (`<Int>` → `number`, `<Array[Str]>` → `string[]`,
//!   `<[Int, ^Bool]>` → `(a: number) => boolean`, `<Self>` → `this`),
//! * control-flow messages with literal blocks (`ifTrue:`, `whileTrue:`, `to:do:`, `do:`,
//!   `on:do:` ...) become `if`, `while`, `for` and `try` statements,
//! * well-known messages become their JavaScript equivalents (`collect:` → `map`,
//!   `inject:into:` → `reduce`, `printString`, 1-based `at:`), using the declared types to choose
//!   the direct form; where the receiver's type is unknown a small runtime helper decides,
//! * a `^` inside a block that is not inlined returns from the enclosing method, as in Smalltalk.
//!
//! The runtime prelude is emitted only as far as the translated code uses it.

use crate::ast::*;
use crate::interp::chunks;
use crate::parser::{parse_type_text, parse_var_decls, Parser};
use serde_json::json;
use std::collections::{BTreeSet, HashMap, HashSet};

/// The result of a translation.
#[derive(Debug, Clone, Default)]
pub struct Translation {
    pub ok: bool,
    pub code: String,
    pub warnings: Vec<String>,
    pub error: Option<String>,
    pub line: usize,
    pub column: usize,
    pub classes: Vec<String>,
}

impl Translation {
    pub fn to_json(&self) -> String {
        json!({
            "ok": self.ok, "code": self.code, "warnings": self.warnings, "error": self.error,
            "line": self.line, "column": self.column, "classes": self.classes,
        })
        .to_string()
    }
}

/// Translates Strongtalk source (a file-in or a do-it) to TypeScript.
pub fn translate(src: &str) -> Translation {
    let mut t = Tr::default();
    t.checked_division = src.contains("ZeroDivide");
    if looks_like_file_in(src) {
        t.file_in(src)
    } else {
        t.doit_unit(src)
    }
}

fn looks_like_file_in(src: &str) -> bool {
    src.contains("Delta define:") || src.contains("methodsFor:") || src.contains("subclass:") && src.contains("instanceVariableNames:")
}

// ------------------------------------------------------------------ expressions and types

#[derive(Clone, Debug, PartialEq)]
enum Ty {
    Any,
    Num,
    Str,
    Char,
    Bool,
    Nil,
    Arr(Box<Ty>),
    Set,
    Map,
    Block,
    Stream,
    /// an instance of a class
    Obj(String),
    /// a class object
    Class(String),
}

const P_LOW: u8 = 1;
const P_ASSIGN: u8 = 2;
const P_COND: u8 = 3;
const P_OR: u8 = 4;
const P_AND: u8 = 5;
const P_EQ: u8 = 9;
const P_REL: u8 = 10;
const P_ADD: u8 = 12;
const P_MUL: u8 = 13;
const P_EXP: u8 = 14;
const P_UNARY: u8 = 15;
const P_POST: u8 = 18;
const P_ATOM: u8 = 20;

#[derive(Clone, Debug)]
struct E {
    code: String,
    ty: Ty,
    prec: u8,
}

fn ex(code: impl Into<String>, ty: Ty, prec: u8) -> E {
    E { code: code.into(), ty, prec }
}

impl E {
    /// The code, parenthesized when its precedence is below `min`.
    fn at(&self, min: u8) -> String {
        if self.prec < min {
            format!("({})", self.code)
        } else {
            self.code.clone()
        }
    }
    /// The code as the target of `.member`.
    fn obj(&self) -> String {
        if self.prec < P_POST || self.code.chars().next().map_or(false, |c| c.is_ascii_digit()) {
            format!("({})", self.code)
        } else {
            self.code.clone()
        }
    }
    /// Can be evaluated twice without side effects or cost.
    fn simple(&self) -> bool {
        self.prec >= P_ATOM
            && (self.code == "this"
                || self.code.chars().all(|c| c.is_alphanumeric() || c == '_' || c == '$' || c == '.')
                || self.code.starts_with('\'') && !self.code[1..].contains('\''))
    }
}

const JS_RESERVED: &[&str] = &[
    "break", "case", "catch", "class", "const", "continue", "debugger", "default", "delete", "do", "else", "enum",
    "export", "extends", "finally", "for", "function", "if", "import", "in", "instanceof", "new", "return",
    "switch", "this", "throw", "try", "typeof", "var", "void", "while", "with", "yield", "let", "static",
    "implements", "interface", "package", "private", "protected", "public", "await", "arguments", "eval",
    "undefined", "NaN", "Infinity", "async", "of", "type", "any", "number", "string", "boolean",
];

const JS_GLOBAL_CLASSES: &[&str] = &[
    "Object", "Array", "String", "Number", "Boolean", "Symbol", "Map", "Set", "Date", "Error", "Function", "Math",
    "JSON", "Promise", "RegExp", "Proxy", "Reflect", "Intl", "BigInt", "WeakMap", "WeakSet", "Iterator",
];

/// Class names the translation maps or provides itself.
const KNOWN_GLOBALS: &[&str] = &[
    "Transcript", "Smalltalk", "OrderedCollection", "Array", "Bag", "SortedCollection", "List", "Set", "IdentitySet",
    "Dictionary", "IdentityDictionary", "String", "Symbol", "Character", "WriteStream", "ReadWriteStream", "Interval",
    "Time", "Error", "Exception", "ZeroDivide", "MessageNotUnderstood", "Warning", "NotFound", "SubscriptOutOfBounds",
    "Random", "Object", "Int", "Integer", "SmallInteger", "Number", "Float", "Str", "Bool", "Boolean", "Self",
    "Instance", "Nil", "UndefinedObject", "Block", "BlockClosure", "Collection", "SeqCltn", "SequenceableCollection",
    "ReadSequence", "Sequence", "ArrayedCollection", "KeyedCollection", "Stream", "ReadStream", "Fraction",
    "LargeInteger", "Magnitude", "Char", "Sym", "Num", "Dynamic", "ReadString", "SmallInt",
];

/// Messages every object understands: mapped even for classes defined in the translation.
const OBJECT_PROTOCOL: &[&str] = &[
    "subclassResponsibility", "shouldNotImplement", "printString", "displayString", "printNl", "displayNl", "isNil",
    "notNil", "ifNil:", "yourself", "class", "isKindOf:", "isMemberOf:", "respondsTo:", "=", "==", "~=", "~~", "->",
    "halt", "inspect", "deepCopy", "perform:", "isString", "isNumber", "isSymbol",
];

const STATIC_RESERVED: &[&str] = &["name", "length", "caller", "arguments", "prototype", "call", "apply", "bind"];

fn ident(name: &str) -> String {
    if JS_RESERVED.contains(&name) {
        format!("{}_", name)
    } else {
        name.to_string()
    }
}

fn cap(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
        None => String::new(),
    }
}

fn js_str(s: &str) -> String {
    let mut o = String::from("'");
    for c in s.chars() {
        match c {
            '\\' => o.push_str("\\\\"),
            '\'' => o.push_str("\\'"),
            '\n' => o.push_str("\\n"),
            '\r' => o.push_str("\\r"),
            '\t' => o.push_str("\\t"),
            c if (c as u32) < 0x20 => o.push_str(&format!("\\u{:04x}", c as u32)),
            c => o.push(c),
        }
    }
    o.push('\'');
    o
}

fn indent(lines: Vec<String>) -> Vec<String> {
    lines
        .into_iter()
        .map(|l| l.split('\n').map(|s| if s.is_empty() { String::new() } else { format!("  {}", s) }).collect::<Vec<_>>().join("\n"))
        .collect()
}

fn block_of(e: &Expr) -> Option<&BlockAst> {
    match e {
        Expr::Block(b) => Some(b),
        _ => None,
    }
}

/// A block with no parameters whose value is one expression without returns.
fn simple_block(e: &Expr) -> bool {
    match block_of(e) {
        Some(b) => b.params.is_empty() && b.temps.is_empty() && (b.body.is_empty() || b.body.len() == 1 && matches!(&b.body[0], Stmt::Expr(x) if !is_stmt_construct(x))),
        None => false,
    }
}

fn lit_block_args(args: &[Expr], nparams: &[usize]) -> bool {
    args.len() == nparams.len() && args.iter().zip(nparams).all(|(a, n)| block_of(a).map_or(false, |b| b.params.len() == *n))
}

/// Messages that translate to statements (loops, try) rather than expressions.
fn is_stmt_construct(e: &Expr) -> bool {
    match e {
        Expr::Send { recv, sel, args, .. } => match sel.as_str() {
            "whileTrue:" | "whileFalse:" | "whileTrue" | "whileFalse" => block_of(recv).is_some(),
            "to:do:" | "to:by:do:" | "timesRepeat:" | "do:" | "reverseDo:" | "doWithIndex:" | "keysAndValuesDo:" | "do:separatedBy:" => {
                args.last().and_then(block_of).is_some()
            }
            "on:do:" | "ensure:" | "ifCurtailed:" => block_of(recv).is_some(),
            "ifTrue:" | "ifFalse:" | "ifTrue:ifFalse:" | "ifFalse:ifTrue:" => {
                args.iter().all(|a| block_of(a).is_some()) && !args.iter().all(simple_block)
            }
            _ => false,
        },
        Expr::Cascade { .. } => false,
        _ => false,
    }
}

fn is_error_send(e: &Expr) -> bool {
    matches!(e, Expr::Send { recv, sel, .. } if sel == "error:" && matches!(&**recv, Expr::Var(n, _) if n == "self"))
}

fn always_returns(stmts: &[Stmt]) -> bool {
    match stmts.last() {
        Some(Stmt::Return(..)) => true,
        Some(Stmt::Expr(e)) if is_error_send(e) => true,
        Some(Stmt::Expr(Expr::Send { sel, args, .. })) if (sel == "ifTrue:ifFalse:" || sel == "ifFalse:ifTrue:") => {
            args.iter().all(|a| block_of(a).map_or(false, |b| always_returns(&b.body)))
        }
        _ => false,
    }
}

// ------------------------------------------------------------------ the translator

#[derive(Clone, Default)]
struct ClassDef {
    name: String,
    superclass: Option<String>,
    generics: Vec<String>,
    ivars: Vec<Var>,
    class_vars: Vec<Var>,
    comment: Option<String>,
    /// (class side?, source, line)
    methods: Vec<(bool, String, usize)>,
}

#[derive(Default, Clone)]
struct Cx {
    class: Option<String>,
    meta: bool,
    generics: Vec<String>,
    scopes: Vec<HashMap<String, (String, Ty)>>,
    closure: usize,
    nlr: bool,
    doit: bool,
}

#[derive(Default)]
struct Tr {
    classes: Vec<ClassDef>,
    /// class name → (class side?, selector) → declared result type
    defined: HashMap<String, HashMap<(bool, String), Option<TypeExpr>>>,
    /// every selector defined in this unit, both sides
    selectors: HashSet<String>,
    /// (class side?, selector) for every method defined in this unit
    side_selectors: HashSet<(bool, String)>,
    /// selectors translated as plain method calls without a library mapping
    unmapped: BTreeSet<String>,
    /// library classes referenced but not part of this translation
    unknown_classes: BTreeSet<String>,
    /// mangled names of instance methods (instance variables with these names are renamed)
    method_names: HashSet<String>,
    warnings: Vec<String>,
    helpers: BTreeSet<&'static str>,
    cx: Cx,
    tmp: usize,
    /// the program handles ZeroDivide: divisions must raise it, as in Smalltalk
    checked_division: bool,
}

impl Tr {
    fn warn(&mut self, w: String) {
        if !self.warnings.contains(&w) && self.warnings.len() < 60 {
            self.warnings.push(w);
        }
    }
    fn fresh(&mut self, base: &str) -> String {
        self.tmp += 1;
        format!("${}{}", base, self.tmp)
    }
    fn use_helper(&mut self, h: &'static str) {
        self.helpers.insert(h);
    }
    fn class_def(&self, name: &str) -> Option<&ClassDef> {
        self.classes.iter().find(|c| c.name == name)
    }
    fn class_ts(&self, name: &str) -> String {
        if self.class_def(name).is_some() && JS_GLOBAL_CLASSES.contains(&name) {
            format!("St{}", name)
        } else {
            name.to_string()
        }
    }
    /// The superclass chain within this unit, starting with `name`.
    fn chain(&self, name: &str) -> Vec<String> {
        let mut out = vec![];
        let mut cur = Some(name.to_string());
        while let Some(c) = cur {
            if out.contains(&c) {
                break;
            }
            let next = self.class_def(&c).and_then(|d| d.superclass.clone()).map(|s| s.split('[').next().unwrap_or("").trim().to_string());
            out.push(c);
            cur = next;
        }
        out
    }
    fn lookup_method(&self, class: &str, meta: bool, sel: &str) -> Option<Option<TypeExpr>> {
        for c in self.chain(class) {
            if let Some(m) = self.defined.get(&c).and_then(|d| d.get(&(meta, sel.to_string()))) {
                return Some(m.clone());
            }
        }
        None
    }

    // -------------------------------------------------------------- selector names

    fn method_name(&self, sel: &str, meta: bool) -> String {
        self.method_name_ex(sel, meta, true)
    }

    /// `twin`: rename a one-keyword selector `x:` to `setX` when the same side also defines a
    /// unary `x` (TypeScript has one namespace per side for both).
    fn method_name_ex(&self, sel: &str, meta: bool, twin: bool) -> String {
        let base = match sel {
            "+" => "plus".into(),
            "-" => "minus".into(),
            "*" => "times".into(),
            "/" => "dividedBy".into(),
            "//" => "intDiv".into(),
            "\\\\" => "mod".into(),
            "<" => "lessThan".into(),
            ">" => "greaterThan".into(),
            "<=" => "lessOrEqual".into(),
            ">=" => "greaterOrEqual".into(),
            "=" => "equals".into(),
            "~=" => "notEquals".into(),
            "==" => "same".into(),
            "~~" => "notSame".into(),
            "," => "concat".into(),
            "@" => "point".into(),
            "->" => "association".into(),
            "new:" => "newSize".into(),
            s if s.ends_with(':') => {
                let parts: Vec<&str> = s.split(':').filter(|p| !p.is_empty()).collect();
                if twin && parts.len() == 1 && self.side_selectors.contains(&(meta, parts[0].to_string())) {
                    format!("set{}", cap(parts[0]))
                } else {
                    let mut n = parts[0].to_string();
                    for p in &parts[1..] {
                        n.push_str(&cap(p));
                    }
                    n
                }
            }
            s if s.chars().next().map_or(false, |c| c.is_alphabetic() || c == '_') => s.to_string(),
            s => format!("op{}", s.chars().map(|c| format!("{:x}", c as u32)).collect::<String>()),
        };
        if base == "constructor" || meta && STATIC_RESERVED.contains(&base.as_str()) {
            format!("{}_", base)
        } else {
            base
        }
    }

    // -------------------------------------------------------------- types

    fn ts_type(&mut self, t: &TypeExpr) -> String {
        match t {
            TypeExpr::Name(n) => self.ts_type_name(n, &[]),
            TypeExpr::Generic(n, args) => {
                let a: Vec<String> = args.iter().map(|a| self.ts_type(a)).collect();
                self.ts_type_name(n, &a)
            }
            TypeExpr::Union(ts) => {
                let mut parts: Vec<String> = vec![];
                for t in ts {
                    let s = self.ts_type(t);
                    if !parts.contains(&s) {
                        parts.push(s);
                    }
                }
                parts.join(" | ")
            }
            TypeExpr::Block(args, ret) => {
                let ps: Vec<String> = args.iter().enumerate().map(|(i, a)| format!("arg{}: {}", i + 1, self.ts_type(a))).collect();
                let r = ret.as_ref().map(|r| self.ts_type(r)).unwrap_or_else(|| "any".into());
                format!("(({}) => {})", ps.join(", "), r)
            }
            TypeExpr::Meta(t) => format!("typeof {}", self.ts_type(t)),
        }
    }

    fn ts_type_name(&mut self, n: &str, args: &[String]) -> String {
        let elem = || args.first().cloned().unwrap_or_else(|| "any".into());
        match n {
            "Int" | "Integer" | "SmallInteger" | "SmallInt" | "LargeInteger" | "Number" | "Float" | "Fraction" | "Magnitude" | "Num" => "number".into(),
            "Str" | "String" | "Symbol" | "Sym" | "Character" | "Char" | "ReadString" => "string".into(),
            "Bool" | "Boolean" => "boolean".into(),
            "Object" | "Dynamic" => "any".into(),
            "Nil" | "UndefinedObject" => "null".into(),
            "Self" => {
                if self.cx.meta {
                    format!("typeof {}", self.class_ts(&self.cx.class.clone().unwrap_or_default()))
                } else {
                    "this".into()
                }
            }
            "Instance" => {
                let c = self.cx.class.clone().unwrap_or_else(|| "any".into());
                let g = self.class_def(&c).map(|d| d.generics.len()).unwrap_or(0);
                let name = self.class_ts(&c);
                if g > 0 {
                    format!("{}<{}>", name, vec!["any"; g].join(", "))
                } else {
                    name
                }
            }
            "Block" | "BlockClosure" => "Function".into(),
            "Array" | "OrderedCollection" | "Collection" | "SeqCltn" | "SequenceableCollection" | "ReadSequence" | "Sequence"
            | "ArrayedCollection" | "SortedCollection" | "Bag" | "Interval" | "List"
                if self.class_def(n).is_none() =>
            {
                let e = elem();
                if e.contains(' ') { format!("({})[]", e) } else { format!("{}[]", e) }
            }
            "Set" | "IdentitySet" if self.class_def(n).is_none() => format!("Set<{}>", elem()),
            "Dictionary" | "IdentityDictionary" | "KeyedCollection" if self.class_def(n).is_none() => {
                let k = args.first().cloned().unwrap_or_else(|| "any".into());
                let v = args.get(1).cloned().unwrap_or_else(|| "any".into());
                format!("Map<{}, {}>", k, v)
            }
            "WriteStream" | "ReadWriteStream" | "Stream" | "ReadStream" if self.class_def(n).is_none() => {
                self.use_helper("WriteStream");
                "WriteStream".into()
            }
            "Exception" | "Error" | "ZeroDivide" | "MessageNotUnderstood" if self.class_def(n).is_none() => "Error".into(),
            g if self.cx.generics.iter().any(|x| x == g) => {
                if self.cx.meta { "any".into() } else { g.to_string() }
            }
            other => {
                if self.class_def(other).is_none() && !KNOWN_GLOBALS.contains(&other) && other.chars().next().map_or(false, |c| c.is_uppercase()) && other.len() > 1 {
                    self.unknown_classes.insert(other.to_string());
                }
                let name = self.class_ts(other);
                if args.is_empty() {
                    let g = self.class_def(other).map(|d| d.generics.len()).unwrap_or(0);
                    if g > 0 { format!("{}<{}>", name, vec!["any"; g].join(", ")) } else { name }
                } else {
                    format!("{}<{}>", name, args.join(", "))
                }
            }
        }
    }

    fn ty_of(&self, t: &TypeExpr) -> Ty {
        let name_ty = |n: &str, args: &[TypeExpr]| -> Ty {
            match n {
                "Int" | "Integer" | "SmallInteger" | "SmallInt" | "LargeInteger" | "Number" | "Float" | "Fraction" | "Num" => Ty::Num,
                "Str" | "String" | "Symbol" | "Sym" | "ReadString" => Ty::Str,
                "Character" | "Char" => Ty::Char,
                "Bool" | "Boolean" => Ty::Bool,
                "Nil" | "UndefinedObject" => Ty::Nil,
                "Block" | "BlockClosure" => Ty::Block,
                "Array" | "OrderedCollection" | "Collection" | "SeqCltn" | "SequenceableCollection" | "ReadSequence" | "Sequence"
                | "ArrayedCollection" | "SortedCollection" | "Bag" | "Interval" | "List"
                    if self.class_def(n).is_none() =>
                {
                    Ty::Arr(Box::new(args.first().map(|a| self.ty_of(a)).unwrap_or(Ty::Any)))
                }
                "Set" | "IdentitySet" if self.class_def(n).is_none() => Ty::Set,
                "Dictionary" | "IdentityDictionary" | "KeyedCollection" if self.class_def(n).is_none() => Ty::Map,
                "WriteStream" | "ReadWriteStream" | "Stream" if self.class_def(n).is_none() => Ty::Stream,
                "Self" => match &self.cx.class {
                    Some(c) if self.cx.meta => Ty::Class(c.clone()),
                    Some(c) => Ty::Obj(c.clone()),
                    None => Ty::Any,
                },
                "Instance" => self.cx.class.clone().map(Ty::Obj).unwrap_or(Ty::Any),
                "Object" | "Dynamic" => Ty::Any,
                other if self.class_def(other).is_some() => Ty::Obj(other.to_string()),
                other if ["Error", "Exception", "ZeroDivide", "MessageNotUnderstood"].contains(&other) => Ty::Obj(other.to_string()),
                _ => Ty::Any,
            }
        };
        match t {
            TypeExpr::Name(n) => name_ty(n, &[]),
            TypeExpr::Generic(n, a) => name_ty(n, a),
            TypeExpr::Block(..) => Ty::Block,
            TypeExpr::Meta(t) => match &**t {
                TypeExpr::Name(n) => Ty::Class(n.clone()),
                _ => Ty::Any,
            },
            TypeExpr::Union(ts) => {
                let tys: Vec<Ty> = ts.iter().map(|t| self.ty_of(t)).filter(|t| *t != Ty::Nil).collect();
                if tys.len() == 1 { tys[0].clone() } else { Ty::Any }
            }
        }
    }

    fn param_decl(&mut self, v: &Var, infer: bool) -> (String, Ty, String) {
        let name = ident(&v.name);
        match &v.ty {
            Some(t) => {
                let ty = self.ty_of(t);
                let ts = self.ts_type(t);
                (name.clone(), ty, format!("{}: {}", name, ts))
            }
            None => (name.clone(), Ty::Any, if infer { name } else { format!("{}: any", ident(&v.name)) }),
        }
    }

    // -------------------------------------------------------------- scopes and variables

    fn push_scope(&mut self) {
        self.cx.scopes.push(HashMap::new());
    }
    fn pop_scope(&mut self) {
        self.cx.scopes.pop();
    }
    fn bind(&mut self, name: &str, code: String, ty: Ty) {
        if self.cx.scopes.is_empty() {
            self.push_scope();
        }
        self.cx.scopes.last_mut().unwrap().insert(name.to_string(), (code, ty));
    }

    fn declare_temps(&mut self, temps: &[Var], out: &mut Vec<String>) {
        for v in temps {
            let name = ident(&v.name);
            match &v.ty {
                Some(t) => {
                    let ty = self.ty_of(t);
                    let ts = self.ts_type(t);
                    self.bind(&v.name, name.clone(), ty);
                    out.push(format!("let {}: {};", name, ts));
                }
                None => {
                    self.bind(&v.name, name.clone(), Ty::Any);
                    out.push(format!("let {};", name));
                }
            }
        }
    }

    fn ivar(&self, name: &str) -> Option<(String, Ty)> {
        let class = self.cx.class.clone()?;
        for c in self.chain(&class) {
            if let Some(d) = self.class_def(&c) {
                if !self.cx.meta {
                    if let Some(v) = d.ivars.iter().find(|v| v.name == name) {
                        let ts = self.ivar_ts(&v.name);
                        return Some((format!("this.{}", ts), v.ty.as_ref().map(|t| self.ty_of(t)).unwrap_or(Ty::Any)));
                    }
                }
                if let Some(v) = d.class_vars.iter().find(|v| v.name == name) {
                    let ts = self.ivar_ts(&v.name);
                    return Some((format!("{}.{}", self.class_ts(&c), ts), v.ty.as_ref().map(|t| self.ty_of(t)).unwrap_or(Ty::Any)));
                }
            }
        }
        None
    }

    fn ivar_ts(&self, name: &str) -> String {
        if self.method_names.contains(name) || STATIC_RESERVED.contains(&name) {
            format!("_{}", name)
        } else {
            name.to_string()
        }
    }

    fn var(&mut self, name: &str) -> E {
        match name {
            "self" => {
                return match &self.cx.class {
                    Some(c) if self.cx.meta => ex("this", Ty::Class(c.clone()), P_ATOM),
                    Some(c) => ex("this", Ty::Obj(c.clone()), P_ATOM),
                    None => {
                        self.warn("`self` in a workspace has no meaning in TypeScript; translated as `undefined`".into());
                        ex("undefined", Ty::Nil, P_ATOM)
                    }
                }
            }
            "thisContext" => {
                self.warn("thisContext has no TypeScript equivalent".into());
                return ex("undefined", Ty::Any, P_ATOM);
            }
            _ => {}
        }
        for s in self.cx.scopes.iter().rev() {
            if let Some((code, ty)) = s.get(name) {
                return ex(code.clone(), ty.clone(), P_ATOM);
            }
        }
        if let Some((code, ty)) = self.ivar(name) {
            return ex(code, ty, P_POST);
        }
        match name {
            "Transcript" => {
                self.use_helper("Transcript");
                ex("Transcript", Ty::Obj("Transcript".into()), P_ATOM)
            }
            "Smalltalk" => ex("(globalThis as any)", Ty::Class("Smalltalk".into()), P_ATOM),
            n if n.chars().next().map_or(false, |c| c.is_uppercase()) => {
                match n {
                    "ZeroDivide" | "MessageNotUnderstood" | "Warning" | "NotFound" | "SubscriptOutOfBounds" if self.class_def(n).is_none() => {
                        self.use_helper("exceptions")
                    }
                    "WriteStream" | "ReadWriteStream" if self.class_def(n).is_none() => self.use_helper("WriteStream"),
                    "Random" if self.class_def(n).is_none() => self.use_helper("Random"),
                    _ => {}
                }
                if self.class_def(n).is_none() && !KNOWN_GLOBALS.contains(&n) && !self.cx.generics.iter().any(|g| g == n) {
                    self.unknown_classes.insert(n.to_string());
                }
                let code = match n {
                    "Exception" if self.class_def(n).is_none() => "Error".to_string(),
                    "ReadWriteStream" => "WriteStream".to_string(),
                    _ => self.class_ts(n),
                };
                ex(code, Ty::Class(n.to_string()), P_ATOM)
            }
            n => {
                self.warn(format!("`{}` is not declared here; it is translated as a free variable", n));
                ex(ident(n), Ty::Any, P_ATOM)
            }
        }
    }

    fn assign_target(&mut self, name: &str) -> (String, Option<Ty>) {
        for s in self.cx.scopes.iter().rev() {
            if let Some((code, ty)) = s.get(name) {
                return (code.clone(), Some(ty.clone()));
            }
        }
        if let Some((code, ty)) = self.ivar(name) {
            return (code, Some(ty));
        }
        if name.chars().next().map_or(false, |c| c.is_uppercase()) {
            return (format!("(globalThis as any).{}", name), None);
        }
        self.warn(format!("assignment to undeclared variable `{}`", name));
        (ident(name), None)
    }

    // -------------------------------------------------------------- literals

    fn lit(&mut self, l: &Lit) -> E {
        match l {
            Lit::Nil => ex("null", Ty::Nil, P_ATOM),
            Lit::True => ex("true", Ty::Bool, P_ATOM),
            Lit::False => ex("false", Ty::Bool, P_ATOM),
            Lit::Int(i) => ex(i.to_string(), Ty::Num, if *i < 0 { P_UNARY } else { P_ATOM }),
            Lit::Big(b) => {
                self.warn("integers beyond 2^53 become BigInt literals (`123n`), which do not mix with number arithmetic".into());
                ex(format!("{}n", b), Ty::Any, P_ATOM)
            }
            Lit::Float(f) => {
                let s = if f.is_finite() { format!("{:?}", f) } else if f.is_nan() { "NaN".into() } else if *f > 0.0 { "Infinity".into() } else { "-Infinity".into() };
                let neg = s.starts_with('-');
                ex(s, Ty::Num, if neg { P_UNARY } else { P_ATOM })
            }
            Lit::Char(c) => ex(js_str(&c.to_string()), Ty::Char, P_ATOM),
            Lit::Str(s) => ex(js_str(s), Ty::Str, P_ATOM),
            Lit::Sym(s) => ex(js_str(s), Ty::Str, P_ATOM),
            Lit::Array(items) => {
                let parts: Vec<E> = items.iter().map(|i| self.lit(i)).collect();
                let elem = if !parts.is_empty() && parts.iter().all(|p| p.ty == parts[0].ty) { parts[0].ty.clone() } else { Ty::Any };
                ex(format!("[{}]", parts.iter().map(|p| p.code.clone()).collect::<Vec<_>>().join(", ")), Ty::Arr(Box::new(elem)), P_ATOM)
            }
            Lit::Bytes(bs) => ex(format!("new Uint8Array([{}])", bs.iter().map(|b| b.to_string()).collect::<Vec<_>>().join(", ")), Ty::Any, P_POST),
        }
    }

    // -------------------------------------------------------------- expressions

    fn expr(&mut self, e: &Expr) -> E {
        match e {
            Expr::Lit(l, _) => self.lit(l),
            Expr::Var(n, _) => self.var(n),
            Expr::Assign(n, v, _) => {
                let val = self.expr(v);
                let (target, ty) = self.assign_target(n);
                if ty == Some(Ty::Any) && val.ty != Ty::Any && val.ty != Ty::Nil {
                    // remember what an untyped temporary holds, to choose direct translations later
                    if let Some(s) = self.cx.scopes.iter_mut().rev().find(|s| s.contains_key(n.as_str())) {
                        if let Some(entry) = s.get_mut(n.as_str()) {
                            entry.1 = val.ty.clone();
                        }
                    }
                }
                ex(format!("{} = {}", target, val.at(P_LOW)), val.ty, P_ASSIGN)
            }
            Expr::Send { recv, sel, args, .. } => self.send(recv, sel, args),
            Expr::Cascade { recv, msgs } => self.cascade_expr(recv, msgs),
            Expr::Block(b) => self.closure(b, false),
            Expr::Brace(items, _) => {
                let parts: Vec<String> = items.iter().map(|i| self.expr(i).at(P_LOW)).collect();
                ex(format!("[{}]", parts.join(", ")), Ty::Arr(Box::new(Ty::Any)), P_ATOM)
            }
            Expr::Prim { sel, args, .. } => {
                self.warn(format!("the VM primitive {} has no TypeScript equivalent", sel));
                self.use_helper("prim");
                let a: Vec<String> = args.iter().filter(|a| block_of(a).is_none()).map(|a| self.expr(a).at(P_LOW)).collect();
                ex(format!("$primitive({}, [{}])", js_str(sel), a.join(", ")), Ty::Any, P_POST)
            }
            Expr::Guaranteed(t, e, _) => {
                let v = self.expr(e);
                let ts = self.ts_type(t);
                ex(format!("{} as {}", v.at(P_REL + 1), ts), self.ty_of(t), P_REL)
            }
        }
    }

    /// The value of a literal block that is evaluated in place (an inlined block).
    fn block_value(&mut self, e: &Expr) -> E {
        let b = match block_of(e) {
            Some(b) => b,
            None => {
                let f = self.expr(e);
                return ex(format!("{}()", f.obj()), Ty::Any, P_POST);
            }
        };
        if b.body.is_empty() {
            return ex("null", Ty::Nil, P_ATOM);
        }
        if simple_block(e) {
            if let Stmt::Expr(x) = &b.body[0] {
                return self.expr(x);
            }
        }
        let b = b.clone();
        self.iife(|t| t.inline_block(&b, Tail::Return))
    }

    fn iife(&mut self, f: impl FnOnce(&mut Self) -> Vec<String>) -> E {
        self.cx.closure += 1;
        let lines = f(self);
        self.cx.closure -= 1;
        ex(format!("(() => {{\n{}\n}})()", indent(lines).join("\n")), Ty::Any, P_POST)
    }

    /// A block used as a value: an arrow function.
    fn closure(&mut self, b: &BlockAst, infer: bool) -> E {
        self.cx.closure += 1;
        self.push_scope();
        let mut ps = vec![];
        for p in &b.params {
            let (name, ty, decl) = self.param_decl(p, infer);
            self.bind(&p.name, name, ty);
            ps.push(decl);
        }
        let ret = b.ret.as_ref().map(|r| format!(": {}", self.ts_type(r))).unwrap_or_default();
        let head = format!("({}){} =>", ps.join(", "), ret);
        let code = if b.body.is_empty() && b.temps.is_empty() {
            format!("{} null", head)
        } else if b.temps.is_empty() && b.body.len() == 1 && matches!(&b.body[0], Stmt::Expr(x) if !is_stmt_construct(x) && !matches!(x, Expr::Cascade{..})) {
            let x = match &b.body[0] {
                Stmt::Expr(x) => x.clone(),
                _ => unreachable!(),
            };
            let v = self.expr(&x);
            let c = v.at(P_LOW);
            if c.starts_with('{') { format!("{} ({})", head, c) } else { format!("{} {}", head, c) }
        } else {
            let lines = self.body(&b.temps, &b.body, Tail::Return);
            format!("{} {{\n{}\n}}", head, indent(lines).join("\n"))
        };
        self.pop_scope();
        self.cx.closure -= 1;
        ex(code, Ty::Block, P_LOW)
    }

    /// A block passed to a collection method: parameters typed from the element type.
    fn fn_arg(&mut self, a: &Expr, elem: &Ty) -> E {
        match block_of(a) {
            Some(b) => {
                let b2 = b.clone();
                // let TypeScript infer untyped parameters from the array method
                let mut e = self.closure_with_elem(&b2, elem);
                e.prec = P_LOW;
                e
            }
            None => self.expr(a),
        }
    }

    fn closure_with_elem(&mut self, b: &BlockAst, elem: &Ty) -> E {
        // like `closure`, but untyped parameters get the collection's element type for later choices
        self.cx.closure += 1;
        self.push_scope();
        let mut ps = vec![];
        for p in &b.params {
            let (name, ty, decl) = self.param_decl(p, true);
            let ty = if ty == Ty::Any && p.ty.is_none() { elem.clone() } else { ty };
            self.bind(&p.name, name, ty);
            ps.push(decl);
        }
        let head = format!("({}) =>", ps.join(", "));
        let code = if b.body.is_empty() && b.temps.is_empty() {
            format!("{} null", head)
        } else if b.temps.is_empty() && b.body.len() == 1 && matches!(&b.body[0], Stmt::Expr(x) if !is_stmt_construct(x) && !matches!(x, Expr::Cascade{..})) {
            let x = match &b.body[0] {
                Stmt::Expr(x) => x.clone(),
                _ => unreachable!(),
            };
            let v = self.expr(&x);
            let c = v.at(P_LOW);
            if c.starts_with('{') { format!("{} ({})", head, c) } else { format!("{} {}", head, c) }
        } else {
            let lines = self.body(&b.temps, &b.body, Tail::Return);
            format!("{} {{\n{}\n}}", head, indent(lines).join("\n"))
        };
        self.pop_scope();
        self.cx.closure -= 1;
        ex(code, Ty::Block, P_LOW)
    }

    fn cascade_expr(&mut self, recv: &Expr, msgs: &[(String, Vec<Expr>, usize)]) -> E {
        let r = self.expr(recv);
        let tmp = self.fresh("c");
        let rt = ex(tmp.clone(), r.ty.clone(), P_ATOM);
        let mut lines = vec![];
        for (i, (sel, args, _)) in msgs.iter().enumerate() {
            let v = self.send_e(rt.clone(), sel, args);
            if i + 1 == msgs.len() {
                lines.push(format!("return {};", v.code));
            } else {
                lines.push(format!("{};", v.code));
            }
        }
        let ty = r.ty.clone();
        ex(format!("(({}) => {{\n{}\n}})({})", tmp, indent(lines).join("\n"), r.at(P_LOW)), ty, P_POST)
    }

    fn not(&self, c: &E) -> String {
        if c.prec == P_UNARY {
            if let Some(rest) = c.code.strip_prefix("!(") {
                if let Some(inner) = rest.strip_suffix(')') {
                    if balanced_parens(inner) {
                        return inner.to_string();
                    }
                }
            }
            if let Some(rest) = c.code.strip_prefix('!') {
                if !rest.contains(' ') {
                    return rest.to_string();
                }
            }
        }
        if c.prec == P_EQ {
            for (a, b) in [(" === ", " !== "), (" !== ", " === "), (" == ", " != "), (" != ", " == ")] {
                if c.code.matches(a).count() == 1 && !c.code.contains("&&") && !c.code.contains("||") {
                    return c.code.replace(a, b);
                }
            }
        }
        format!("!{}", c.at(P_UNARY))
    }

    // -------------------------------------------------------------- message sends

    fn send(&mut self, recv: &Expr, sel: &str, args: &[Expr]) -> E {
        if let Some(e) = self.control_expr(recv, sel, args) {
            return e;
        }
        if matches!(recv, Expr::Var(n, _) if n == "super") {
            return self.super_send(sel, args);
        }
        if is_error_send(&Expr::Send { recv: Box::new(recv.clone()), sel: sel.into(), args: args.to_vec(), pos: 0 }) {
            self.use_helper("error");
            let m = self.expr(&args[0]);
            return ex(format!("$error({})", m.at(P_LOW)), Ty::Any, P_POST);
        }
        // `a to: b` used as a collection
        let r = self.expr(recv);
        self.send_e(r, sel, args)
    }

    fn super_send(&mut self, sel: &str, args: &[Expr]) -> E {
        let class = self.cx.class.clone().unwrap_or_default();
        let sup = self.class_def(&class).and_then(|d| d.superclass.clone()).unwrap_or_else(|| "Object".into());
        let sup_in_unit = self.class_def(sup.split('[').next().unwrap_or("")).is_some();
        if self.cx.meta && (sel == "new" || sel == "basicNew") && !self.lookup_method(&sup, true, sel).is_some() {
            return ex("new this()", Ty::Obj(class), P_POST);
        }
        if !sup_in_unit && !self.cx.meta && (sel == "printOn:" || sel == "initialize") {
            // Object>>printOn: / initialize: nothing to call in TypeScript
            if sel == "printOn:" {
                let a = self.expr(&args[0]);
                self.use_helper("printString");
                return ex(format!("{}.nextPutAll($article(this.constructor.name))", a.obj()), Ty::Any, P_POST);
            }
            return ex("this", Ty::Obj(class), P_ATOM);
        }
        let name = self.method_name(sel, self.cx.meta);
        let a: Vec<String> = args.iter().map(|x| self.expr(x).at(P_LOW)).collect();
        let ty = self.lookup_method(&sup, self.cx.meta, sel).flatten().map(|t| self.ty_of(&t)).unwrap_or(Ty::Any);
        ex(format!("super.{}({})", name, a.join(", ")), ty, P_POST)
    }

    fn call(&mut self, r: &E, name: &str, args: &[Expr], ty: Ty) -> E {
        let a: Vec<String> = args.iter().map(|x| self.expr(x).at(P_LOW)).collect();
        ex(format!("{}.{}({})", r.obj(), name, a.join(", ")), ty, P_POST)
    }

    /// A send to an already translated receiver.
    fn send_e(&mut self, r: E, sel: &str, args: &[Expr]) -> E {
        // classes
        if let Ty::Class(c) = &r.ty {
            let c = c.clone();
            if let Some(e) = self.class_send(&c, &r, sel, args) {
                return e;
            }
        }
        // methods defined in this translation unit win over library mappings
        if let Ty::Obj(c) = &r.ty {
            let c = c.clone();
            if let Some(ret) = self.lookup_method(&c, false, sel) {
                let ty = match ret {
                    Some(t) => {
                        let saved = (self.cx.class.clone(), self.cx.meta);
                        self.cx.class = Some(c.clone());
                        self.cx.meta = false;
                        let ty = self.ty_of(&t);
                        self.cx.class = saved.0;
                        self.cx.meta = saved.1;
                        ty
                    }
                    None => Ty::Any,
                };
                let name = self.method_name(sel, false);
                return self.call(&r, &name, args, ty);
            }
        }
        if let Some(e) = self.mapped(&r, sel, args) {
            return e;
        }
        let prelude_recv = r.ty == Ty::Stream || r.ty == Ty::Obj("Transcript".into()) || r.ty == Ty::Obj("Random".into());
        let name = self.method_name_ex(sel, false, !prelude_recv);
        if !prelude_recv && !self.selectors.contains(sel) && !matches!(&r.ty, Ty::Obj(c) if self.class_def(c).is_some()) {
            self.unmapped.insert(sel.to_string());
        }
        self.call(&r, &name, args, Ty::Any)
    }

    fn class_send(&mut self, c: &str, r: &E, sel: &str, args: &[Expr]) -> Option<E> {
        let user = self.class_def(c).is_some();
        if user {
            if (sel == "new" || sel == "basicNew") && self.lookup_method(c, true, "new").is_none() {
                let ts = self.class_ts(c);
                return Some(ex(format!("new {}()", ts), Ty::Obj(c.to_string()), P_POST));
            }
            let ret = self.lookup_method(c, true, sel);
            let ty = match ret.clone() {
                Some(Some(t)) => {
                    let saved = (self.cx.class.clone(), self.cx.meta);
                    self.cx.class = Some(c.to_string());
                    self.cx.meta = true;
                    let ty = self.ty_of(&t);
                    self.cx.class = saved.0;
                    self.cx.meta = saved.1;
                    ty
                }
                _ => Ty::Any,
            };
            if sel == "name" && ret.is_none() {
                return Some(ex(format!("{}.name", r.obj()), Ty::Str, P_POST));
            }
            let name = self.method_name(sel, true);
            return Some(self.call(r, &name, args, ty));
        }
        let a = |t: &mut Self, i: usize| t.expr(&args[i]);
        let arr = |code: String| ex(code, Ty::Arr(Box::new(Ty::Any)), P_ATOM);
        let res = match (c, sel) {
            ("OrderedCollection" | "Array" | "Bag" | "SortedCollection" | "List", "new") => arr("[]".into()),
            ("OrderedCollection" | "Bag" | "SortedCollection" | "List", "new:") => arr("[]".into()),
            ("Array", "new:") => {
                let n = a(self, 0);
                ex(format!("new Array({}).fill(null)", n.at(P_LOW)), Ty::Arr(Box::new(Ty::Any)), P_POST)
            }
            ("Array" | "OrderedCollection", "new:withAll:") => {
                let n = a(self, 0);
                let v = a(self, 1);
                ex(format!("new Array({}).fill({})", n.at(P_LOW), v.at(P_LOW)), Ty::Arr(Box::new(v.ty)), P_POST)
            }
            ("Array" | "OrderedCollection" | "Bag" | "List", s) if s.starts_with("with:") && s.split(':').filter(|p| !p.is_empty()).all(|p| p == "with") => {
                let items: Vec<E> = (0..args.len()).map(|i| a(self, i)).collect();
                let elem = if !items.is_empty() && items.iter().all(|i| i.ty == items[0].ty) { items[0].ty.clone() } else { Ty::Any };
                ex(format!("[{}]", items.iter().map(|i| i.at(P_LOW)).collect::<Vec<_>>().join(", ")), Ty::Arr(Box::new(elem)), P_ATOM)
            }
            ("Array" | "OrderedCollection" | "Bag" | "List", "withAll:") => {
                let v = a(self, 0);
                ex(format!("[...{}]", v.at(P_UNARY)), Ty::Arr(Box::new(Ty::Any)), P_ATOM)
            }
            ("Set" | "IdentitySet", "new" | "new:") => ex("new Set()", Ty::Set, P_POST),
            ("Set" | "IdentitySet", "withAll:") => {
                let v = a(self, 0);
                ex(format!("new Set({})", v.at(P_LOW)), Ty::Set, P_POST)
            }
            ("Dictionary" | "IdentityDictionary", "new" | "new:") => ex("new Map()", Ty::Map, P_POST),
            ("String" | "Symbol", "new") => ex("''", Ty::Str, P_ATOM),
            ("String", "new:") => {
                let n = a(self, 0);
                ex(format!("' '.repeat({})", n.at(P_LOW)), Ty::Str, P_POST)
            }
            ("String", "new:withAll:") => {
                let n = a(self, 0);
                let ch = a(self, 1);
                ex(format!("{}.repeat({})", ch.obj(), n.at(P_LOW)), Ty::Str, P_POST)
            }
            ("String" | "Character", "cr" | "lf" | "newLine") => ex("'\\n'", Ty::Str, P_ATOM),
            ("String" | "Character", "tab") => ex("'\\t'", Ty::Str, P_ATOM),
            ("String" | "Character", "space") => ex("' '", Ty::Str, P_ATOM),
            ("Character", "value:") => {
                let n = a(self, 0);
                ex(format!("String.fromCharCode({})", n.at(P_LOW)), Ty::Char, P_POST)
            }
            ("WriteStream" | "ReadWriteStream", "on:" | "with:") => {
                self.use_helper("WriteStream");
                let v = a(self, 0);
                let init = if sel == "with:" { v.at(P_LOW) } else { "''".into() };
                ex(format!("new WriteStream({})", init), Ty::Stream, P_POST)
            }
            ("Interval", "from:to:") => {
                self.use_helper("range");
                let x = a(self, 0);
                let y = a(self, 1);
                ex(format!("$range({}, {})", x.at(P_LOW), y.at(P_LOW)), Ty::Arr(Box::new(Ty::Num)), P_POST)
            }
            ("Smalltalk", "at:put:") => {
                let k = a(self, 0);
                let v = a(self, 1);
                ex(format!("(globalThis as any)[{}] = {}", k.code, v.at(P_LOW)), v.ty, P_ASSIGN)
            }
            ("Smalltalk", "at:") => {
                let k = a(self, 0);
                ex(format!("(globalThis as any)[{}]", k.code), Ty::Any, P_POST)
            }
            ("Smalltalk", "version") => ex("'Strongtalk 2.0 (TypeScript translation)'", Ty::Str, P_ATOM),
            ("Time", "millisecondsToRun:") => {
                self.use_helper("time");
                let f = self.fn_arg(&args[0], &Ty::Any);
                ex(format!("$millisecondsToRun({})", f.at(P_LOW)), Ty::Num, P_POST)
            }
            ("Time", "millisecondClockValue" | "millisecondClock" | "totalMilliseconds") => ex("Date.now()", Ty::Num, P_POST),
            ("Error" | "Exception" | "ZeroDivide" | "MessageNotUnderstood" | "Warning" | "NotFound" | "SubscriptOutOfBounds", "new") => {
                ex(format!("new {}()", r.code), Ty::Obj(c.into()), P_POST)
            }
            ("Error" | "Exception" | "ZeroDivide" | "MessageNotUnderstood" | "Warning" | "NotFound" | "SubscriptOutOfBounds", "signal" | "signal:" | "new:") => {
                self.use_helper("raise");
                let m = if args.is_empty() { String::new() } else { a(self, 0).at(P_LOW) };
                ex(format!("$raise(new {}({}))", r.code, m), Ty::Any, P_POST)
            }
            ("Random", "new") => ex("new Random()", Ty::Obj("Random".into()), P_POST),
            (_, "name") => ex(format!("{}.name", r.obj()), Ty::Str, P_POST),
            (_, "new") => ex(format!("new {}()", r.code), Ty::Obj(c.into()), P_POST),
            _ => return None,
        };
        Some(res)
    }

    fn num_binop(&mut self, r: &E, op: &str, arg: &Expr, prec: u8, ty: Ty) -> E {
        let b = self.expr(arg);
        // Smalltalk binary messages go left to right: the left operand may be at the same level
        let left = r.at(prec);
        let right = b.at(prec + 1);
        ex(format!("{} {} {}", left, op, right), ty, prec)
    }

    fn is_user_obj(&self, t: &Ty) -> bool {
        matches!(t, Ty::Obj(c) if self.class_def(c).is_some())
    }

    /// Library messages with a direct TypeScript/JavaScript equivalent.
    fn mapped(&mut self, r: &E, sel: &str, args: &[Expr]) -> Option<E> {
        let t = r.ty.clone();
        let is_arr = matches!(t, Ty::Arr(_));
        let elem = match &t {
            Ty::Arr(e) => (**e).clone(),
            Ty::Str => Ty::Char,
            _ => Ty::Any,
        };
        let seq = is_arr || t == Ty::Str;
        let any = t == Ty::Any;
        let user = self.is_user_obj(&t);
        // instances of classes defined here keep only the protocol every object has
        if user && !OBJECT_PROTOCOL.contains(&sel) {
            return None;
        }
        let a0 = |s: &mut Self| s.expr(&args[0]);
        let o = r.obj();
        let e = match sel {
            // arithmetic and comparison
            "/" if self.checked_division => {
                let b = self.expr(&args[0]);
                if b.code.parse::<f64>().map_or(true, |v| v == 0.0) {
                    self.use_helper("div");
                    ex(format!("$div({}, {})", r.at(P_LOW), b.at(P_LOW)), Ty::Num, P_POST)
                } else {
                    ex(format!("{} / {}", r.at(P_MUL), b.at(P_MUL + 1)), Ty::Num, P_MUL)
                }
            }
            "+" | "-" | "*" | "/" => {
                let b = self.expr(&args[0]);
                if sel == "+" && (t == Ty::Str || b.ty == Ty::Str) {
                    self.warn("`+` on strings is not concatenation in Smalltalk; check this translation".into());
                }
                let prec = if sel == "+" || sel == "-" { P_ADD } else { P_MUL };
                let ty = if t == Ty::Num && b.ty == Ty::Num { Ty::Num } else if t == Ty::Num || b.ty == Ty::Num { Ty::Num } else { Ty::Any };
                ex(format!("{} {} {}", r.at(prec), sel, b.at(prec + 1)), ty, prec)
            }
            "<" | ">" | "<=" | ">=" => self.num_binop(r, sel, &args[0], P_REL, Ty::Bool),
            "=" | "==" => self.num_binop(r, "===", &args[0], P_EQ, Ty::Bool),
            "~=" | "~~" => self.num_binop(r, "!==", &args[0], P_EQ, Ty::Bool),
            "//" => {
                let b = a0(self);
                ex(format!("Math.floor({} / {})", r.at(P_MUL), b.at(P_MUL + 1)), Ty::Num, P_POST)
            }
            "\\\\" => {
                self.use_helper("mod");
                let b = a0(self);
                ex(format!("$mod({}, {})", r.at(P_LOW), b.at(P_LOW)), Ty::Num, P_POST)
            }
            "rem:" => self.num_binop(r, "%", &args[0], P_MUL, Ty::Num),
            "quo:" => {
                let b = a0(self);
                ex(format!("Math.trunc({} / {})", r.at(P_MUL), b.at(P_MUL + 1)), Ty::Num, P_POST)
            }
            "raisedTo:" | "**" => {
                let b = a0(self);
                ex(format!("{} ** {}", r.at(P_EXP + 1), b.at(P_EXP)), Ty::Num, P_EXP)
            }
            "max:" | "min:" if !seq => {
                let b = a0(self);
                ex(format!("Math.{}({}, {})", &sel[..3], r.at(P_LOW), b.at(P_LOW)), Ty::Num, P_POST)
            }
            "between:and:" => {
                let lo = a0(self);
                let hi = self.expr(&args[1]);
                if r.simple() {
                    ex(format!("{} >= {} && {} <= {}", r.at(P_REL + 1), lo.at(P_REL + 1), r.at(P_REL + 1), hi.at(P_REL + 1)), Ty::Bool, P_AND)
                } else {
                    self.use_helper("between");
                    ex(format!("$between({}, {}, {})", r.at(P_LOW), lo.at(P_LOW), hi.at(P_LOW)), Ty::Bool, P_POST)
                }
            }
            "abs" | "sqrt" | "sin" | "cos" | "tan" | "exp" | "floor" | "sign" => ex(format!("Math.{}({})", sel, r.at(P_LOW)), Ty::Num, P_POST),
            "arcTan" | "arcSin" | "arcCos" => ex(format!("Math.a{}({})", &sel[3..].to_lowercase(), r.at(P_LOW)), Ty::Num, P_POST),
            "ln" => ex(format!("Math.log({})", r.at(P_LOW)), Ty::Num, P_POST),
            "log:" => {
                let b = a0(self);
                ex(format!("Math.log({}) / Math.log({})", r.at(P_LOW), b.at(P_LOW)), Ty::Num, P_MUL)
            }
            "ceiling" => ex(format!("Math.ceil({})", r.at(P_LOW)), Ty::Num, P_POST),
            "rounded" => ex(format!("Math.round({})", r.at(P_LOW)), Ty::Num, P_POST),
            "truncated" => ex(format!("Math.trunc({})", r.at(P_LOW)), Ty::Num, P_POST),
            "asInteger" if t != Ty::Str && t != Ty::Char => ex(format!("Math.trunc({})", r.at(P_LOW)), Ty::Num, P_POST),
            "asFloat" => ex(r.code.clone(), Ty::Num, r.prec),
            "negated" => ex(format!("-{}", r.at(P_UNARY)), Ty::Num, P_UNARY),
            "squared" => ex(format!("{} ** 2", r.at(P_EXP + 1)), Ty::Num, P_EXP),
            "reciprocal" => ex(format!("1 / {}", r.at(P_MUL + 1)), Ty::Num, P_MUL),
            "isZero" => ex(format!("{} === 0", r.at(P_EQ + 1)), Ty::Bool, P_EQ),
            "even" => ex(format!("{} % 2 === 0", r.at(P_MUL)), Ty::Bool, P_EQ),
            "odd" => ex(format!("{} % 2 !== 0", r.at(P_MUL)), Ty::Bool, P_EQ),
            "isInteger" => ex(format!("Number.isInteger({})", r.at(P_LOW)), Ty::Bool, P_POST),
            "isNumber" => ex(format!("typeof {} === 'number'", r.at(P_UNARY)), Ty::Bool, P_EQ),
            "isString" | "isSymbol" => ex(format!("typeof {} === 'string'", r.at(P_UNARY)), Ty::Bool, P_EQ),
            "factorial" => {
                self.use_helper("factorial");
                ex(format!("$factorial({})", r.at(P_LOW)), Ty::Any, P_POST)
            }
            "isPrime" => {
                self.use_helper("isPrime");
                ex(format!("$isPrime({})", r.at(P_LOW)), Ty::Bool, P_POST)
            }
            "gcd:" | "lcm:" => {
                self.use_helper("gcd");
                let b = a0(self);
                ex(format!("${}({}, {})", &sel[..3], r.at(P_LOW), b.at(P_LOW)), Ty::Num, P_POST)
            }
            "asCharacter" => ex(format!("String.fromCharCode({})", r.at(P_LOW)), Ty::Char, P_POST),
            "to:" => {
                self.use_helper("range");
                let b = a0(self);
                ex(format!("$range({}, {})", r.at(P_LOW), b.at(P_LOW)), Ty::Arr(Box::new(Ty::Num)), P_POST)
            }
            "to:by:" => {
                self.use_helper("range");
                let b = a0(self);
                let s = self.expr(&args[1]);
                ex(format!("$range({}, {}, {})", r.at(P_LOW), b.at(P_LOW), s.at(P_LOW)), Ty::Arr(Box::new(Ty::Num)), P_POST)
            }
            "@" => {
                let b = a0(self);
                ex(format!("{{ x: {}, y: {} }}", r.at(P_LOW), b.at(P_LOW)), Ty::Any, P_ATOM)
            }
            "printString:" | "radix:" | "printStringRadix:" => {
                let b = a0(self);
                ex(format!("{}.toString({})", o, b.at(P_LOW)), Ty::Str, P_POST)
            }
            "printPaddedWith:to:" => {
                let c = a0(self);
                let n = self.expr(&args[1]);
                ex(format!("String({}).padStart({}, {})", r.at(P_LOW), n.at(P_LOW), c.at(P_LOW)), Ty::Str, P_POST)
            }
            // booleans
            "not" => ex(self.not(r), Ty::Bool, P_UNARY),
            "&" => self.num_binop(r, "&&", &args[0], P_AND, Ty::Bool),
            "|" => self.num_binop(r, "||", &args[0], P_OR, Ty::Bool),
            "and:" | "or:" => {
                let op = if sel == "and:" { "&&" } else { "||" };
                let p = if sel == "and:" { P_AND } else { P_OR };
                let b = self.block_value(&args[0]);
                ex(format!("{} {} {}", r.at(p), op, b.at(p + 1)), Ty::Bool, p)
            }
            "xor:" => self.num_binop(r, "!==", &args[0], P_EQ, Ty::Bool),
            "eqv:" => self.num_binop(r, "===", &args[0], P_EQ, Ty::Bool),
            // nil
            "isNil" => ex(format!("{} == null", r.at(P_EQ + 1)), Ty::Bool, P_EQ),
            "notNil" => ex(format!("{} != null", r.at(P_EQ + 1)), Ty::Bool, P_EQ),
            "isEmptyOrNil" => {
                self.use_helper("isEmpty");
                ex(format!("{} == null || $isEmpty({})", r.at(P_EQ + 1), r.at(P_LOW)), Ty::Bool, P_OR)
            }
            "ifNil:" => {
                let b = self.block_value(&args[0]);
                ex(format!("{} ?? {}", r.at(P_EQ), b.at(P_EQ)), r.ty.clone(), P_OR)
            }
            // printing
            "printString" => {
                if t == Ty::Num {
                    ex(format!("String({})", r.at(P_LOW)), Ty::Str, P_POST)
                } else {
                    self.use_helper("printString");
                    ex(format!("$printString({})", r.at(P_LOW)), Ty::Str, P_POST)
                }
            }
            "displayString" | "asString" => match t {
                Ty::Str | Ty::Char => r.clone(),
                Ty::Num => ex(format!("String({})", r.at(P_LOW)), Ty::Str, P_POST),
                _ => {
                    self.use_helper("printString");
                    ex(format!("$displayString({})", r.at(P_LOW)), Ty::Str, P_POST)
                }
            },
            "printNl" => {
                self.use_helper("printString");
                ex(format!("console.log($printString({}))", r.at(P_LOW)), Ty::Nil, P_POST)
            }
            "displayNl" => {
                self.use_helper("printString");
                ex(format!("console.log($displayString({}))", r.at(P_LOW)), Ty::Nil, P_POST)
            }
            "printOn:" if !matches!(t, Ty::Obj(_)) => {
                self.use_helper("printString");
                let s = a0(self);
                ex(format!("{}.nextPutAll($printString({}))", s.obj(), r.at(P_LOW)), Ty::Any, P_POST)
            }
            // objects
            "yourself" => r.clone(),
            "class" => ex(format!("{}.constructor", o), Ty::Any, P_POST),
            "isKindOf:" => {
                let b = a0(self);
                ex(format!("{} instanceof {}", r.at(P_REL + 1), b.at(P_REL + 1)), Ty::Bool, P_REL)
            }
            "isMemberOf:" => {
                let b = a0(self);
                ex(format!("{}.constructor === {}", o, b.at(P_EQ + 1)), Ty::Bool, P_EQ)
            }
            "respondsTo:" => {
                let b = a0(self);
                ex(format!("typeof {}[{}] === 'function'", o, b.code), Ty::Bool, P_EQ)
            }
            "perform:" => {
                let b = a0(self);
                ex(format!("{}[{}]()", o, b.code), Ty::Any, P_POST)
            }
            "halt" => ex("(() => { debugger; })()", Ty::Nil, P_POST),
            "subclassResponsibility" | "shouldNotImplement" => {
                self.use_helper("error");
                ex(format!("$error({}.constructor.name + ' must implement this method')", o), Ty::Any, P_POST)
            }
            "copy" | "shallowCopy" if seq => {
                if t == Ty::Str { r.clone() } else { ex(format!("[...{}]", r.at(P_UNARY)), t.clone(), P_ATOM) }
            }
            "deepCopy" => ex(format!("structuredClone({})", r.at(P_LOW)), t.clone(), P_POST),
            "->" => {
                let b = a0(self);
                ex(format!("[{}, {}]", r.at(P_LOW), b.at(P_LOW)), Ty::Arr(Box::new(Ty::Any)), P_ATOM)
            }
            "inspect" => ex(format!("console.dir({})", r.at(P_LOW)), Ty::Nil, P_POST),
            // blocks
            "value" if t == Ty::Block || any && !self.selectors.contains("value") => ex(format!("{}()", r.obj()), Ty::Any, P_POST),
            "value:" | "value:value:" | "value:value:value:" | "value:value:value:value:" if t == Ty::Block || any => {
                let a: Vec<String> = args.iter().map(|x| self.expr(x).at(P_LOW)).collect();
                ex(format!("{}({})", r.obj(), a.join(", ")), Ty::Any, P_POST)
            }
            "valueWithArguments:" => {
                let b = a0(self);
                ex(format!("{}(...{})", r.obj(), b.at(P_UNARY)), Ty::Any, P_POST)
            }
            "numArgs" if t == Ty::Block => ex(format!("{}.length", o), Ty::Num, P_POST),
            // exceptions
            "signal" if !seq => {
                self.use_helper("raise");
                ex(format!("$raise({})", r.at(P_LOW)), Ty::Any, P_POST)
            }
            "signal:" if !seq => {
                self.use_helper("raise");
                let m = a0(self);
                ex(format!("$raise({}, {})", r.at(P_LOW), m.at(P_LOW)), Ty::Any, P_POST)
            }
            "messageText" | "description" if !seq => ex(format!("{}.message", o), Ty::Str, P_POST),
            "return:" | "resume:" => {
                self.warn(format!("`{}` in a handler becomes the handler's value", sel));
                a0(self)
            }
            "retry" => {
                self.warn("`retry` has no direct TypeScript equivalent".into());
                ex("undefined", Ty::Any, P_ATOM)
            }
            "pass" | "outer" | "signalAgain" => {
                self.use_helper("raise");
                ex(format!("$raise({})", r.at(P_LOW)), Ty::Any, P_POST)
            }
            // strings and characters
            "asUppercase" | "asLowercase" if t != Ty::Any || true => {
                let m = if sel == "asUppercase" { "toUpperCase" } else { "toLowerCase" };
                ex(format!("{}.{}()", o, m), if t == Ty::Char { Ty::Char } else { Ty::Str }, P_POST)
            }
            "asSymbol" => r.clone(),
            "asNumber" => ex(format!("Number({})", r.at(P_LOW)), Ty::Num, P_POST),
            "asInteger" | "value" | "asciiValue" | "codePoint" if t == Ty::Char || t == Ty::Str && sel != "value" => {
                if t == Ty::Char { ex(format!("{}.charCodeAt(0)", o), Ty::Num, P_POST) } else { ex(format!("parseInt({}, 10)", r.at(P_LOW)), Ty::Num, P_POST) }
            }
            "isVowel" => ex(format!("'aeiouAEIOU'.includes({})", r.at(P_LOW)), Ty::Bool, P_POST),
            "isLetter" => ex(format!("/\\p{{L}}/u.test({})", r.at(P_LOW)), Ty::Bool, P_POST),
            "isDigit" => ex(format!("/^\\d$/.test({})", r.at(P_LOW)), Ty::Bool, P_POST),
            "isUppercase" => ex(format!("/\\p{{Lu}}/u.test({})", r.at(P_LOW)), Ty::Bool, P_POST),
            "isLowercase" => ex(format!("/\\p{{Ll}}/u.test({})", r.at(P_LOW)), Ty::Bool, P_POST),
            "isSeparator" => ex(format!("/^\\s$/.test({})", r.at(P_LOW)), Ty::Bool, P_POST),
            "trimSeparators" | "trimBlanks" | "trimmed" | "withBlanksTrimmed" => ex(format!("{}.trim()", o), Ty::Str, P_POST),
            "substrings" => ex(format!("{}.trim().split(/\\s+/)", o), Ty::Arr(Box::new(Ty::Str)), P_POST),
            "substrings:" | "subStrings:" | "tokenize:" => {
                let b = a0(self);
                ex(format!("{}.split({})", o, b.at(P_LOW)), Ty::Arr(Box::new(Ty::Str)), P_POST)
            }
            "lines" => ex(format!("{}.split('\\n')", o), Ty::Arr(Box::new(Ty::Str)), P_POST),
            "includesSubstring:" | "includesSubString:" => {
                let b = a0(self);
                ex(format!("{}.includes({})", o, b.at(P_LOW)), Ty::Bool, P_POST)
            }
            "startsWith:" | "beginsWith:" | "endsWith:" => {
                let b = a0(self);
                let m = if sel == "endsWith:" { "endsWith" } else { "startsWith" };
                ex(format!("{}.{}({})", o, m, b.at(P_LOW)), Ty::Bool, P_POST)
            }
            "copyReplaceAll:with:" | "replaceAll:with:" if t == Ty::Str || any => {
                let b = a0(self);
                let c = self.expr(&args[1]);
                ex(format!("{}.replaceAll({}, {})", o, b.at(P_LOW), c.at(P_LOW)), Ty::Str, P_POST)
            }
            "join:" => {
                let b = a0(self);
                ex(format!("{}.join({})", b.obj(), r.at(P_LOW)), Ty::Str, P_POST)
            }
            "," => {
                let b = a0(self);
                if t == Ty::Str || t == Ty::Char || b.ty == Ty::Str {
                    ex(format!("{} + {}", r.at(P_ADD), b.at(P_ADD + 1)), Ty::Str, P_ADD)
                } else {
                    ex(format!("{}.concat({})", o, b.at(P_LOW)), t.clone(), P_POST)
                }
            }
            // collections
            "size" => match t {
                Ty::Arr(_) | Ty::Str => ex(format!("{}.length", o), Ty::Num, P_POST),
                Ty::Set | Ty::Map => ex(format!("{}.size", o), Ty::Num, P_POST),
                _ => {
                    self.use_helper("size");
                    ex(format!("$size({})", r.at(P_LOW)), Ty::Num, P_POST)
                }
            },
            "isEmpty" | "notEmpty" | "isEmptyCollection" => {
                let neg = sel == "notEmpty";
                match t {
                    Ty::Arr(_) | Ty::Str => ex(format!("{}.length {} 0", o, if neg { "!==" } else { "===" }), Ty::Bool, P_EQ),
                    Ty::Set | Ty::Map => ex(format!("{}.size {} 0", o, if neg { "!==" } else { "===" }), Ty::Bool, P_EQ),
                    _ => {
                        self.use_helper("isEmpty");
                        ex(format!("{}$isEmpty({})", if neg { "!" } else { "" }, r.at(P_LOW)), Ty::Bool, if neg { P_UNARY } else { P_POST })
                    }
                }
            }
            "at:" => {
                let i = a0(self);
                match t {
                    Ty::Arr(_) | Ty::Str => ex(format!("{}[{}]", o, one_based(&i)), elem.clone(), P_POST),
                    Ty::Map => ex(format!("{}.get({})", o, i.at(P_LOW)), Ty::Any, P_POST),
                    _ => {
                        self.use_helper("at");
                        ex(format!("$at({}, {})", r.at(P_LOW), i.at(P_LOW)), Ty::Any, P_POST)
                    }
                }
            }
            "at:put:" => {
                let i = a0(self);
                let v = self.expr(&args[1]);
                match t {
                    Ty::Arr(_) => ex(format!("{}[{}] = {}", o, one_based(&i), v.at(P_LOW)), v.ty, P_ASSIGN),
                    Ty::Map => ex(format!("{}.set({}, {})", o, i.at(P_LOW), v.at(P_LOW)), Ty::Map, P_POST),
                    _ => {
                        self.use_helper("atPut");
                        ex(format!("$atPut({}, {}, {})", r.at(P_LOW), i.at(P_LOW), v.at(P_LOW)), v.ty, P_POST)
                    }
                }
            }
            "at:ifAbsent:" => {
                let k = a0(self);
                let d = self.block_value(&args[1]);
                match t {
                    Ty::Map => ex(format!("{}.has({}) ? {}.get({}) : {}", o, k.at(P_LOW), o, k.at(P_LOW), d.at(P_COND)), Ty::Any, P_COND),
                    _ => {
                        self.use_helper("at");
                        ex(format!("$at({}, {}) ?? {}", r.at(P_LOW), k.at(P_LOW), d.at(P_EQ)), Ty::Any, P_OR)
                    }
                }
            }
            "add:" | "addLast:" => {
                let v = a0(self);
                match t {
                    Ty::Arr(_) => ex(format!("{}.push({})", o, v.at(P_LOW)), Ty::Num, P_POST),
                    Ty::Set => ex(format!("{}.add({})", o, v.at(P_LOW)), Ty::Set, P_POST),
                    Ty::Stream => ex(format!("{}.nextPut({})", o, v.at(P_LOW)), Ty::Stream, P_POST),
                    _ => {
                        self.use_helper("add");
                        ex(format!("$add({}, {})", r.at(P_LOW), v.at(P_LOW)), v.ty, P_POST)
                    }
                }
            }
            "addAll:" if is_arr || any => {
                let v = a0(self);
                ex(format!("{}.push(...{})", o, v.at(P_UNARY)), Ty::Num, P_POST)
            }
            "addFirst:" if is_arr || any => {
                let v = a0(self);
                ex(format!("{}.unshift({})", o, v.at(P_LOW)), Ty::Num, P_POST)
            }
            "removeFirst" if is_arr || any => ex(format!("{}.shift()", o), elem.clone(), P_POST),
            "removeLast" if is_arr || any => ex(format!("{}.pop()", o), elem.clone(), P_POST),
            "remove:" | "remove:ifAbsent:" if is_arr || t == Ty::Set || any => {
                let v = a0(self);
                if t == Ty::Set {
                    ex(format!("{}.delete({})", o, v.at(P_LOW)), Ty::Bool, P_POST)
                } else {
                    self.use_helper("remove");
                    ex(format!("$remove({}, {})", r.at(P_LOW), v.at(P_LOW)), Ty::Any, P_POST)
                }
            }
            "removeKey:" | "removeKey:ifAbsent:" if t == Ty::Map || any => {
                let v = a0(self);
                ex(format!("{}.delete({})", o, v.at(P_LOW)), Ty::Bool, P_POST)
            }
            "includes:" => {
                let v = a0(self);
                match t {
                    Ty::Arr(_) | Ty::Str => ex(format!("{}.includes({})", o, v.at(P_LOW)), Ty::Bool, P_POST),
                    Ty::Set => ex(format!("{}.has({})", o, v.at(P_LOW)), Ty::Bool, P_POST),
                    _ => {
                        self.use_helper("includes");
                        ex(format!("$includes({}, {})", r.at(P_LOW), v.at(P_LOW)), Ty::Bool, P_POST)
                    }
                }
            }
            "includesKey:" => {
                let v = a0(self);
                ex(format!("{}.has({})", o, v.at(P_LOW)), Ty::Bool, P_POST)
            }
            "keys" if t == Ty::Map || any => ex(format!("[...{}.keys()]", o), Ty::Arr(Box::new(Ty::Any)), P_ATOM),
            "values" if t == Ty::Map || any => ex(format!("[...{}.values()]", o), Ty::Arr(Box::new(Ty::Any)), P_ATOM),
            "first" if seq || any => if seq { ex(format!("{}[0]", o), elem.clone(), P_POST) } else { ex(format!("{}.at(0)", o), Ty::Any, P_POST) },
            "last" if seq || any => ex(format!("{}.at(-1)", o), elem.clone(), P_POST),
            "first:" if seq || any => {
                let n = a0(self);
                ex(format!("{}.slice(0, {})", o, n.at(P_LOW)), t.clone(), P_POST)
            }
            "last:" if seq || any => {
                let n = a0(self);
                ex(format!("{}.slice(-{})", o, n.at(P_UNARY)), t.clone(), P_POST)
            }
            "allButFirst" if seq || any => ex(format!("{}.slice(1)", o), t.clone(), P_POST),
            "allButLast" if seq || any => ex(format!("{}.slice(0, -1)", o), t.clone(), P_POST),
            "indexOf:" if seq || any => {
                let v = a0(self);
                ex(format!("{}.indexOf({}) + 1", o, v.at(P_LOW)), Ty::Num, P_ADD)
            }
            "copyFrom:to:" if seq || any => {
                let a = a0(self);
                let b = self.expr(&args[1]);
                let start = match a.code.parse::<i64>() {
                    Ok(n) => (n - 1).to_string(),
                    Err(_) => format!("{} - 1", a.at(P_ADD)),
                };
                ex(format!("{}.slice({}, {})", o, start, b.at(P_LOW)), t.clone(), P_POST)
            }
            "copyWith:" if seq || any => {
                let v = a0(self);
                if t == Ty::Str { ex(format!("{} + {}", r.at(P_ADD), v.at(P_ADD + 1)), Ty::Str, P_ADD) } else { ex(format!("[...{}, {}]", r.at(P_UNARY), v.at(P_LOW)), t.clone(), P_ATOM) }
            }
            "reversed" | "reverse" => match t {
                Ty::Str => ex(format!("[...{}].reverse().join('')", r.at(P_UNARY)), Ty::Str, P_POST),
                Ty::Arr(_) => ex(format!("[...{}].reverse()", r.at(P_UNARY)), t.clone(), P_POST),
                _ => {
                    self.use_helper("reversed");
                    ex(format!("$reversed({})", r.at(P_LOW)), Ty::Any, P_POST)
                }
            },
            "asArray" | "asOrderedCollection" | "asBag" => match t {
                Ty::Map => ex(format!("[...{}.values()]", o), Ty::Arr(Box::new(Ty::Any)), P_ATOM),
                _ => ex(format!("[...{}]", r.at(P_UNARY)), Ty::Arr(Box::new(elem.clone())), P_ATOM),
            },
            "asSet" | "asIdentitySet" => ex(format!("new Set({})", r.at(P_LOW)), Ty::Set, P_POST),
            "sort" if is_arr || any => {
                self.use_helper("sorted");
                ex(format!("$sort({})", r.at(P_LOW)), t.clone(), P_POST)
            }
            "asSortedCollection" | "asSortedArray" => {
                self.use_helper("sorted");
                ex(format!("$sorted({})", r.at(P_LOW)), Ty::Arr(Box::new(elem.clone())), P_POST)
            }
            "asSortedCollection:" | "sort:" | "sortBlock:" => {
                let f = self.fn_arg(&args[0], &elem);
                let two = block_of(&args[0]).map_or(false, |b| b.params.len() == 2);
                let cmp = match f.code.split_once(" => ") {
                    Some((params, body)) if two && !body.starts_with('{') && !body.contains('?') => format!("{} => ({} ? -1 : 1)", params, body),
                    _ => format!("(a, b) => (({})(a, b) ? -1 : 1)", f.code),
                };
                let base = if sel == "sort:" { o.clone() } else { format!("[...{}]", r.at(P_UNARY)) };
                ex(format!("{}.sort({})", base, cmp), Ty::Arr(Box::new(elem.clone())), P_POST)
            }
            "do:" if !matches!(t, Ty::Obj(_) | Ty::Class(_)) => {
                let f = self.fn_arg(&args[0], &elem);
                let tgt = if t == Ty::Map { format!("[...{}.values()]", o) } else if t == Ty::Str || t == Ty::Set { format!("[...{}]", r.at(P_UNARY)) } else { o.clone() };
                ex(format!("{}.forEach({})", tgt, f.at(P_LOW)), Ty::Nil, P_POST)
            }
            "collect:" | "select:" | "reject:" | "detect:" | "anySatisfy:" | "allSatisfy:" | "contains:" | "count:" | "detect:ifNone:" | "inject:into:"
                if !matches!(t, Ty::Obj(_) | Ty::Class(_)) =>
            {
                self.collection_hof(r, sel, args, &elem)
            }
            "with:collect:" if seq || any => {
                let other = a0(self);
                let f = self.fn_arg(&args[1], &elem);
                ex(format!("{}.map((x, i) => ({})(x, {}[i]))", o, f.code, other.obj()), Ty::Arr(Box::new(Ty::Any)), P_POST)
            }
            "sum" if seq || any => ex(format!("{}.reduce((a, b) => a + b, 0)", o), Ty::Num, P_POST),
            "max" if seq || any => ex(format!("Math.max(...{})", r.at(P_UNARY)), Ty::Num, P_POST),
            "min" if seq || any => ex(format!("Math.min(...{})", r.at(P_UNARY)), Ty::Num, P_POST),
            "occurrencesOf:" => {
                self.use_helper("occurrences");
                let v = a0(self);
                ex(format!("$occurrencesOf({}, {})", r.at(P_LOW), v.at(P_LOW)), Ty::Num, P_POST)
            }
            // streams
            "contents" if t == Ty::Stream => ex(format!("{}.contents()", o), Ty::Str, P_POST),
            "<<" => {
                self.use_helper("printString");
                let v = a0(self);
                ex(format!("{}.nextPutAll($displayString({}))", o, v.at(P_LOW)), t.clone(), P_POST)
            }
            _ => return None,
        };
        Some(e)
    }

    fn collection_hof(&mut self, r: &E, sel: &str, args: &[Expr], elem: &Ty) -> E {
        let t = &r.ty;
        let base = match t {
            Ty::Str | Ty::Set => format!("[...{}]", r.at(P_UNARY)),
            Ty::Map => format!("[...{}.values()]", r.obj()),
            _ => r.obj(),
        };
        let f = |s: &mut Self, i: usize| s.fn_arg(&args[i], elem);
        match sel {
            "collect:" => {
                let g = f(self, 0);
                let out_elem = Ty::Any;
                ex(format!("{}.map({})", base, g.code), Ty::Arr(Box::new(out_elem)), P_POST)
            }
            "select:" => {
                let g = f(self, 0);
                ex(format!("{}.filter({})", base, g.code), Ty::Arr(Box::new(elem.clone())), P_POST)
            }
            "reject:" => {
                let g = f(self, 0);
                let code = match g.code.split_once(" => ") {
                    Some((params, body)) if block_of(&args[0]).map_or(false, |b| b.params.len() == 1) && !body.starts_with('{') => {
                        format!("{} => !({})", params, body)
                    }
                    _ => format!("(x) => !({})(x)", g.code),
                };
                ex(format!("{}.filter({})", base, code), Ty::Arr(Box::new(elem.clone())), P_POST)
            }
            "detect:" => {
                let g = f(self, 0);
                ex(format!("{}.find({})!", base, g.code), elem.clone(), P_POST)
            }
            "detect:ifNone:" => {
                let g = f(self, 0);
                let d = self.block_value(&args[1]);
                ex(format!("{}.find({}) ?? {}", base, g.code, d.at(P_EQ)), elem.clone(), P_OR)
            }
            "anySatisfy:" | "contains:" => {
                let g = f(self, 0);
                ex(format!("{}.some({})", base, g.code), Ty::Bool, P_POST)
            }
            "allSatisfy:" => {
                let g = f(self, 0);
                ex(format!("{}.every({})", base, g.code), Ty::Bool, P_POST)
            }
            "count:" => {
                let g = f(self, 0);
                ex(format!("{}.filter({}).length", base, g.code), Ty::Num, P_POST)
            }
            "inject:into:" => {
                let init = self.expr(&args[0]);
                let g = self.fn_arg(&args[1], &Ty::Any);
                ex(format!("{}.reduce({}, {})", base, g.code, init.at(P_LOW)), init.ty.clone(), P_POST)
            }
            _ => unreachable!(),
        }
    }

    // -------------------------------------------------------------- control flow in expressions

    fn control_expr(&mut self, recv: &Expr, sel: &str, args: &[Expr]) -> Option<E> {
        match sel {
            "ifTrue:" | "ifFalse:" | "ifTrue:ifFalse:" | "ifFalse:ifTrue:" if args.iter().all(|a| block_of(a).map_or(false, |b| b.params.is_empty())) => {
                if args.iter().all(simple_block) {
                    let c = self.expr(recv);
                    let (t, f) = match sel {
                        "ifTrue:" => (self.block_value(&args[0]), ex("null", Ty::Nil, P_ATOM)),
                        "ifFalse:" => (ex("null", Ty::Nil, P_ATOM), self.block_value(&args[0])),
                        "ifTrue:ifFalse:" => (self.block_value(&args[0]), self.block_value(&args[1])),
                        _ => {
                            let f = self.block_value(&args[0]);
                            (self.block_value(&args[1]), f)
                        }
                    };
                    let ty = if t.ty == f.ty { t.ty.clone() } else { Ty::Any };
                    return Some(ex(format!("{} ? {} : {}", c.at(P_OR), t.at(P_ASSIGN), f.at(P_ASSIGN)), ty, P_COND));
                }
                let (recv, sel, args) = (recv.clone(), sel.to_string(), args.to_vec());
                Some(self.iife(|t| t.control_stmt(&recv, &sel, &args, Tail::Return).unwrap_or_default()))
            }
            "ifNotNil:" | "ifNil:ifNotNil:" | "ifNotNil:ifNil:" if args.iter().all(|a| block_of(a).is_some()) => {
                let (recv, sel, args) = (recv.clone(), sel.to_string(), args.to_vec());
                Some(self.iife(|t| t.control_stmt(&recv, &sel, &args, Tail::Return).unwrap_or_default()))
            }
            _ => {
                let e = Expr::Send { recv: Box::new(recv.clone()), sel: sel.to_string(), args: args.to_vec(), pos: 0 };
                if is_stmt_construct(&e) {
                    let (recv, sel, args) = (recv.clone(), sel.to_string(), args.to_vec());
                    return Some(self.iife(|t| t.control_stmt(&recv, &sel, &args, Tail::Return).unwrap_or_default()));
                }
                None
            }
        }
    }

    // -------------------------------------------------------------- statements

    fn body(&mut self, temps: &[Var], stmts: &[Stmt], tail: Tail) -> Vec<String> {
        let mut out = vec![];
        self.declare_temps(temps, &mut out);
        let n = stmts.len();
        for (i, s) in stmts.iter().enumerate() {
            let t = if i + 1 == n { tail } else { Tail::None };
            out.extend(self.stmt(s, t));
            if matches!(s, Stmt::Return(..)) {
                break;
            }
        }
        if n == 0 && tail == Tail::Return {
            out.push("return null;".into());
        }
        out
    }

    fn inline_block(&mut self, b: &BlockAst, tail: Tail) -> Vec<String> {
        self.push_scope();
        let lines = self.body(&b.temps, &b.body, tail);
        self.pop_scope();
        lines
    }

    fn stmt(&mut self, s: &Stmt, tail: Tail) -> Vec<String> {
        match s {
            Stmt::Return(e, _) => self.return_stmt(e),
            Stmt::Expr(e) => self.expr_stmt(e, tail),
        }
    }

    fn return_stmt(&mut self, e: &Expr) -> Vec<String> {
        if is_error_send(e) {
            return self.expr_stmt(e, Tail::None);
        }
        if self.cx.closure > 0 {
            self.cx.nlr = true;
            self.use_helper("nlr");
            let v = self.expr(e);
            return vec![format!("return $nlr.return({});", v.at(P_LOW))];
        }
        if self.cx.doit {
            return self.expr_stmt(e, Tail::DoIt);
        }
        if let Expr::Send { recv, sel, args, .. } = e {
            if matches!(sel.as_str(), "ifTrue:ifFalse:" | "ifFalse:ifTrue:") && args.iter().all(|a| block_of(a).is_some()) && !args.iter().all(simple_block) {
                if let Some(lines) = self.control_stmt(recv, sel, args, Tail::Return) {
                    return lines;
                }
            }
        }
        let v = self.expr(e);
        vec![format!("return {};", v.code)]
    }

    fn expr_stmt(&mut self, e: &Expr, tail: Tail) -> Vec<String> {
        match e {
            Expr::Send { recv, sel, args, .. } => {
                if is_error_send(e) {
                    let m = self.expr(&args[0]);
                    return vec![format!("throw new Error({});", m.at(P_LOW))];
                }
                if let Some(lines) = self.control_stmt(recv, sel, args, tail) {
                    return lines;
                }
                // `Error signal: 'x'` as a statement: a plain throw
                if matches!(sel.as_str(), "signal" | "signal:") {
                    if let Expr::Var(n, _) = &**recv {
                        if ["Error", "Exception", "ZeroDivide", "MessageNotUnderstood", "Warning", "NotFound"].contains(&n.as_str()) && self.class_def(n).is_none() {
                            let c = self.var(n);
                            let m = if args.is_empty() { String::new() } else { self.expr(&args[0]).at(P_LOW) };
                            return vec![format!("throw new {}({});", c.code, m)];
                        }
                    }
                }
            }
            Expr::Cascade { recv, msgs } => return self.cascade_stmts(recv, msgs, tail),
            _ => {}
        }
        let v = self.expr(e);
        match tail {
            Tail::Return => vec![format!("return {};", v.code)],
            Tail::DoIt if should_print(e) => {
                self.use_helper("printString");
                vec![format!("console.log($printString({}));", v.at(P_LOW))]
            }
            _ => {
                if v.prec == P_ATOM && !v.code.contains('(') {
                    return vec![]; // a bare variable or literal does nothing
                }
                let c = if v.code.starts_with('{') || v.code.starts_with("function") { format!("({})", v.code) } else { v.code };
                vec![format!("{};", c)]
            }
        }
    }

    fn cascade_stmts(&mut self, recv: &Expr, msgs: &[(String, Vec<Expr>, usize)], tail: Tail) -> Vec<String> {
        let r = self.expr(recv);
        let mut out = vec![];
        let target = if r.simple() {
            r.clone()
        } else {
            let tmp = self.fresh("c");
            out.push(format!("const {} = {};", tmp, r.at(P_LOW)));
            ex(tmp, r.ty.clone(), P_ATOM)
        };
        for (i, (sel, args, _)) in msgs.iter().enumerate() {
            let last = i + 1 == msgs.len();
            let v = self.send_e(target.clone(), sel, args);
            if last && tail == Tail::Return {
                out.push(format!("return {};", v.code));
            } else if last && tail == Tail::DoIt && !(target.code == "Transcript") && sel != "yourself" {
                self.use_helper("printString");
                out.push(format!("console.log($printString({}));", v.at(P_LOW)));
            } else if !(sel == "yourself" && v.code == target.code) {
                out.push(format!("{};", v.code));
            }
        }
        out
    }

    /// Control-flow messages with literal blocks as statements, or None.
    fn control_stmt(&mut self, recv: &Expr, sel: &str, args: &[Expr], tail: Tail) -> Option<Vec<String>> {
        let branch_tail = match tail {
            Tail::Return => Tail::Return,
            Tail::DoIt => Tail::DoIt,
            _ => Tail::None,
        };
        let nil_tail = |out: &mut Vec<String>| {
            if tail == Tail::Return {
                out.push("return null;".into());
            }
        };
        match sel {
            "ifTrue:" | "ifFalse:" | "ifTrue:ifFalse:" | "ifFalse:ifTrue:" if args.iter().all(|a| block_of(a).map_or(false, |b| b.params.is_empty())) => {
                let c = self.expr(recv);
                let (then_e, else_e, negate) = match sel {
                    "ifTrue:" => (&args[0], None, false),
                    "ifFalse:" => (&args[0], None, true),
                    "ifTrue:ifFalse:" => (&args[0], Some(&args[1]), false),
                    _ => (&args[1], Some(&args[0]), false),
                };
                let cond = if negate { self.not(&c) } else { c.code.clone() };
                let mut out = vec![format!("if ({}) {{", cond)];
                let tb = block_of(then_e).unwrap().clone();
                out.extend(indent(self.inline_block(&tb, branch_tail)));
                match else_e {
                    Some(ee) => {
                        let eb = block_of(ee).unwrap().clone();
                        // `else if` for a nested conditional
                        let nested = eb.temps.is_empty()
                            && eb.body.len() == 1
                            && matches!(&eb.body[0], Stmt::Expr(Expr::Send { sel, args, .. }) if matches!(sel.as_str(), "ifTrue:" | "ifTrue:ifFalse:" | "ifFalse:ifTrue:") && args.iter().all(|a| block_of(a).map_or(false, |b| b.params.is_empty())));
                        if nested {
                            if let Stmt::Expr(Expr::Send { recv, sel, args, .. }) = &eb.body[0] {
                                if let Some(lines) = self.control_stmt(recv, sel, args, tail) {
                                    let mut it = lines.into_iter();
                                    let first = it.next().unwrap_or_default();
                                    out.push(format!("}} else {}", first));
                                    out.extend(it);
                                    return Some(out);
                                }
                            }
                        }
                        out.push("} else {".into());
                        out.extend(indent(self.inline_block(&eb, branch_tail)));
                        out.push("}".into());
                    }
                    None => {
                        out.push("}".into());
                        nil_tail(&mut out);
                    }
                }
                Some(out)
            }
            "ifNil:" | "ifNotNil:" | "ifNil:ifNotNil:" | "ifNotNil:ifNil:" if args.iter().all(|a| block_of(a).is_some()) => {
                let v = self.expr(recv);
                let mut out = vec![];
                let target = if v.simple() {
                    v.clone()
                } else {
                    let tmp = self.fresh("v");
                    out.push(format!("const {} = {};", tmp, v.at(P_LOW)));
                    ex(tmp, v.ty.clone(), P_ATOM)
                };
                let (nil_b, notnil_b) = match sel {
                    "ifNil:" => (Some(&args[0]), None),
                    "ifNotNil:" => (None, Some(&args[0])),
                    "ifNil:ifNotNil:" => (Some(&args[0]), Some(&args[1])),
                    _ => (Some(&args[1]), Some(&args[0])),
                };
                let notnil_lines = |t: &mut Self, b: &Expr| -> Vec<String> {
                    let b = block_of(b).unwrap().clone();
                    t.push_scope();
                    if let Some(p) = b.params.first() {
                        t.bind(&p.name, target.code.clone(), target.ty.clone());
                    }
                    let l = t.body(&b.temps, &b.body, branch_tail);
                    t.pop_scope();
                    l
                };
                match (nil_b, notnil_b) {
                    (Some(n), None) => {
                        out.push(format!("if ({} == null) {{", target.code));
                        let nb = block_of(n).unwrap().clone();
                        out.extend(indent(self.inline_block(&nb, branch_tail)));
                        out.push("}".into());
                        if tail == Tail::Return {
                            out.push(format!("return {};", target.code));
                        }
                    }
                    (None, Some(nn)) => {
                        out.push(format!("if ({} != null) {{", target.code));
                        out.extend(indent(notnil_lines(self, nn)));
                        out.push("}".into());
                        nil_tail(&mut out);
                    }
                    (Some(n), Some(nn)) => {
                        out.push(format!("if ({} == null) {{", target.code));
                        let nb = block_of(n).unwrap().clone();
                        out.extend(indent(self.inline_block(&nb, branch_tail)));
                        out.push("} else {".into());
                        out.extend(indent(notnil_lines(self, nn)));
                        out.push("}".into());
                    }
                    _ => {}
                }
                Some(out)
            }
            "whileTrue:" | "whileFalse:" | "whileTrue" | "whileFalse" if block_of(recv).map_or(false, |b| b.params.is_empty()) && args.iter().all(|a| block_of(a).map_or(false, |b| b.params.is_empty())) => {
                let neg = sel.starts_with("whileFalse");
                let cb = block_of(recv).unwrap().clone();
                let mut out = vec![];
                let body_lines = |t: &mut Self| -> Vec<String> {
                    match args.first().and_then(block_of) {
                        Some(b) => {
                            let b = b.clone();
                            t.inline_block(&b, Tail::None)
                        }
                        None => vec![],
                    }
                };
                if cb.temps.is_empty() && cb.body.len() == 1 && matches!(&cb.body[0], Stmt::Expr(x) if !is_stmt_construct(x)) {
                    let c = match &cb.body[0] {
                        Stmt::Expr(x) => self.expr(x),
                        _ => unreachable!(),
                    };
                    let cond = if neg { self.not(&c) } else { c.code.clone() };
                    out.push(format!("while ({}) {{", cond));
                    out.extend(indent(body_lines(self)));
                    out.push("}".into());
                } else {
                    out.push("while (true) {".into());
                    self.push_scope();
                    let mut inner = vec![];
                    self.declare_temps(&cb.temps, &mut inner);
                    let n = cb.body.len();
                    for (i, s) in cb.body.iter().enumerate() {
                        if i + 1 == n {
                            if let Stmt::Expr(x) = s {
                                let c = self.expr(x);
                                let stop = if neg { c.code.clone() } else { self.not(&c) };
                                inner.push(format!("if ({}) break;", stop));
                            } else {
                                inner.extend(self.stmt(s, Tail::None));
                            }
                        } else {
                            inner.extend(self.stmt(s, Tail::None));
                        }
                    }
                    self.pop_scope();
                    inner.extend(body_lines(self));
                    out.extend(indent(inner));
                    out.push("}".into());
                }
                nil_tail(&mut out);
                Some(out)
            }
            "to:do:" | "to:by:do:" if lit_block_args(&args[args.len() - 1..], &[1]) => {
                let start = self.expr(recv);
                let end = self.expr(&args[0]);
                let step = if sel == "to:by:do:" { Some(self.expr(&args[1])) } else { None };
                let b = block_of(args.last().unwrap()).unwrap().clone();
                let mut out = vec![];
                self.push_scope();
                let v = ident(&b.params[0].name);
                self.bind(&b.params[0].name, v.clone(), Ty::Num);
                let (init_end, end_code) = if end.simple() {
                    (String::new(), end.code.clone())
                } else {
                    let n = self.fresh("end");
                    (format!(", {} = {}", n, end.at(P_LOW)), n)
                };
                let header = match &step {
                    None => format!("for (let {v} = {}{}; {v} <= {}; {v}++) {{", start.at(P_LOW), init_end, end_code),
                    Some(s) => match s.code.parse::<f64>() {
                        Ok(k) if k == 1.0 => format!("for (let {v} = {}{}; {v} <= {}; {v}++) {{", start.at(P_LOW), init_end, end_code),
                        Ok(k) if k == -1.0 => format!("for (let {v} = {}{}; {v} >= {}; {v}--) {{", start.at(P_LOW), init_end, end_code),
                        Ok(k) if k > 0.0 => format!("for (let {v} = {}{}; {v} <= {}; {v} += {}) {{", start.at(P_LOW), init_end, end_code, s.code),
                        Ok(k) if k < 0.0 => format!("for (let {v} = {}{}; {v} >= {}; {v} -= {}) {{", start.at(P_LOW), init_end, end_code, s.code.trim_start_matches('-')),
                        _ => {
                            let sn = if s.simple() { s.code.clone() } else { self.fresh("step") };
                            let init_step = if s.simple() { String::new() } else { format!(", {} = {}", sn, s.at(P_LOW)) };
                            format!("for (let {v} = {}{}{}; {sn} > 0 ? {v} <= {} : {v} >= {}; {v} += {sn}) {{", start.at(P_LOW), init_end, init_step, end_code, end_code)
                        }
                    },
                };
                out.push(header);
                let body = self.body(&b.temps, &b.body, Tail::None);
                self.pop_scope();
                out.extend(indent(body));
                out.push("}".into());
                nil_tail(&mut out);
                Some(out)
            }
            "timesRepeat:" if lit_block_args(args, &[0]) => {
                let n = self.expr(recv);
                let i = self.fresh("i");
                let b = block_of(&args[0]).unwrap().clone();
                let mut out = vec![if n.simple() {
                    format!("for (let {i} = 0; {i} < {}; {i}++) {{", n.code)
                } else {
                    let m = self.fresh("n");
                    format!("for (let {i} = 0, {m} = {}; {i} < {m}; {i}++) {{", n.at(P_LOW))
                }];
                out.extend(indent(self.inline_block(&b, Tail::None)));
                out.push("}".into());
                nil_tail(&mut out);
                Some(out)
            }
            "do:" | "reverseDo:" if lit_block_args(args, &[1]) => {
                let b = block_of(&args[0]).unwrap().clone();
                // (a to: b) do: [...] → a counting loop
                if sel == "do:" {
                    if let Expr::Send { recv: r2, sel: s2, args: a2, .. } = recv {
                        if s2 == "to:" || s2 == "to:by:" {
                            let mut a = vec![a2[0].clone()];
                            if s2 == "to:by:" {
                                a.push(a2[1].clone());
                            }
                            a.push(args[0].clone());
                            let s = if s2 == "to:" { "to:do:" } else { "to:by:do:" };
                            return self.control_stmt(r2, s, &a, tail);
                        }
                    }
                }
                let c = self.expr(recv);
                let elem = match &c.ty {
                    Ty::Arr(e) => (**e).clone(),
                    Ty::Str => Ty::Char,
                    _ => Ty::Any,
                };
                let coll = match (&c.ty, sel) {
                    (Ty::Map, _) => format!("{}.values()", c.obj()),
                    (_, "reverseDo:") => format!("[...{}].reverse()", c.at(P_UNARY)),
                    _ => c.at(P_LOW),
                };
                self.push_scope();
                let (name, ty, decl) = self.param_decl(&b.params[0], true);
                let ty = if ty == Ty::Any { elem } else { ty };
                self.bind(&b.params[0].name, name, ty);
                let decl = decl.split(':').next().unwrap_or("").trim().to_string();
                let mut out = vec![format!("for (const {} of {}) {{", decl, coll)];
                let body = self.body(&b.temps, &b.body, Tail::None);
                self.pop_scope();
                out.extend(indent(body));
                out.push("}".into());
                nil_tail(&mut out);
                Some(out)
            }
            "doWithIndex:" | "keysAndValuesDo:" if lit_block_args(args, &[2]) => {
                let b = block_of(&args[0]).unwrap().clone();
                let c = self.expr(recv);
                let mut out = vec![];
                self.push_scope();
                let p0 = ident(&b.params[0].name);
                let p1 = ident(&b.params[1].name);
                if c.ty == Ty::Map {
                    self.bind(&b.params[0].name, p0.clone(), Ty::Any);
                    self.bind(&b.params[1].name, p1.clone(), Ty::Any);
                    out.push(format!("for (const [{}, {}] of {}) {{", p0, p1, c.at(P_LOW)));
                } else {
                    let (elem_p, idx_p) = if sel == "doWithIndex:" { (&p0, &p1) } else { (&p1, &p0) };
                    let elem = match &c.ty {
                        Ty::Arr(e) => (**e).clone(),
                        _ => Ty::Any,
                    };
                    let (en, inn) = if sel == "doWithIndex:" { (&b.params[0].name, &b.params[1].name) } else { (&b.params[1].name, &b.params[0].name) };
                    self.bind(en, elem_p.clone(), elem);
                    self.bind(inn, idx_p.clone(), Ty::Num);
                    let k = self.fresh("k");
                    out.push(format!("for (const [{}, {}] of [...{}].entries()) {{", k, elem_p, c.at(P_UNARY)));
                    out.push(format!("  const {} = {} + 1;", idx_p, k));
                }
                let body = self.body(&b.temps, &b.body, Tail::None);
                self.pop_scope();
                out.extend(indent(body));
                out.push("}".into());
                nil_tail(&mut out);
                Some(out)
            }
            "do:separatedBy:" if lit_block_args(args, &[1, 0]) => {
                let b = block_of(&args[0]).unwrap().clone();
                let sb = block_of(&args[1]).unwrap().clone();
                let c = self.expr(recv);
                let k = self.fresh("k");
                self.push_scope();
                let p = ident(&b.params[0].name);
                self.bind(&b.params[0].name, p.clone(), Ty::Any);
                let mut out = vec![format!("for (const [{}, {}] of [...{}].entries()) {{", k, p, c.at(P_UNARY))];
                let sep = self.inline_block(&sb, Tail::None);
                let mut inner = vec![format!("if ({} > 0) {{", k)];
                inner.extend(indent(sep));
                inner.push("}".into());
                inner.extend(self.body(&b.temps, &b.body, Tail::None));
                self.pop_scope();
                out.extend(indent(inner));
                out.push("}".into());
                nil_tail(&mut out);
                Some(out)
            }
            "on:do:" if block_of(recv).map_or(false, |b| b.params.is_empty()) => {
                let body_b = block_of(recv).unwrap().clone();
                let classes = self.exception_classes(&args[0]);
                let mut out = vec!["try {".into()];
                out.extend(indent(self.inline_block(&body_b, branch_tail)));
                let (catch_var, handler): (String, Vec<String>) = match block_of(&args[1]) {
                    Some(h) => {
                        let h = h.clone();
                        self.push_scope();
                        let v = h.params.first().map(|p| ident(&p.name)).unwrap_or_else(|| "e".into());
                        if let Some(p) = h.params.first() {
                            self.bind(&p.name, v.clone(), Ty::Obj("Error".into()));
                        }
                        let l = self.body(&h.temps, &h.body, branch_tail);
                        self.pop_scope();
                        (v, l)
                    }
                    None => {
                        let f = self.expr(&args[1]);
                        let line = if tail == Tail::Return { format!("return {}(e);", f.obj()) } else { format!("{}(e);", f.obj()) };
                        ("e".into(), vec![line])
                    }
                };
                out.push(format!("}} catch ({}: any) {{", catch_var));
                let mut inner = vec![];
                if let Some(cs) = classes {
                    let test: Vec<String> = cs.iter().map(|c| format!("{} instanceof {}", catch_var, c)).collect();
                    inner.push(format!("if (!({})) throw {};", test.join(" || "), catch_var));
                }
                inner.extend(handler);
                out.extend(indent(inner));
                out.push("}".into());
                Some(out)
            }
            "ensure:" | "ifCurtailed:" if block_of(recv).map_or(false, |b| b.params.is_empty()) && lit_block_args(args, &[0]) => {
                let body_b = block_of(recv).unwrap().clone();
                let fin = block_of(&args[0]).unwrap().clone();
                let mut out = vec!["try {".into()];
                out.extend(indent(self.inline_block(&body_b, branch_tail)));
                if sel == "ensure:" {
                    out.push("} finally {".into());
                    out.extend(indent(self.inline_block(&fin, Tail::None)));
                } else {
                    out.push("} catch (e: any) {".into());
                    let mut inner = self.inline_block(&fin, Tail::None);
                    inner.push("throw e;".into());
                    out.extend(indent(inner));
                }
                out.push("}".into());
                Some(out)
            }
            _ => None,
        }
    }

    /// The exception classes an `on:` argument names, or None for "any error".
    fn exception_classes(&mut self, e: &Expr) -> Option<Vec<String>> {
        match e {
            Expr::Var(n, _) if (n == "Error" || n == "Exception") && self.class_def(n).is_none() => None,
            Expr::Var(n, _) => Some(vec![self.var(n).code]),
            Expr::Send { recv, sel, args, .. } if sel == "," => {
                let mut a = self.exception_classes(recv)?;
                a.extend(self.exception_classes(&args[0])?);
                Some(a)
            }
            other => Some(vec![self.expr(other).at(P_REL + 1)]),
        }
    }

    // -------------------------------------------------------------- units

    fn doit_unit(&mut self, src: &str) -> Translation {
        let parsed = Parser::new(src).and_then(|mut p| p.parse_doit());
        let (temps, stmts) = match parsed {
            Ok(x) => x,
            Err(e) => return error_result(src, &e.msg, e.pos),
        };
        self.cx = Cx { doit: true, ..Default::default() };
        self.push_scope();
        let mut lines = self.body(&temps, &stmts, Tail::DoIt);
        if self.cx.nlr {
            lines = wrap_nlr(lines, false);
        }
        if self.helpers.contains("Transcript") {
            lines.push("Transcript.flush();".into());
        }
        let decls = self.finish();
        let mut out = String::from("// Translated from Strongtalk to TypeScript.\n");
        let prelude = self.prelude();
        if !prelude.is_empty() {
            out.push_str(&prelude);
            out.push('\n');
        }
        out.push_str(&decls);
        out.push_str(&lines.join("\n"));
        out.push('\n');
        Translation { ok: true, code: out, warnings: self.warnings.clone(), ..Default::default() }
    }

    fn file_in(&mut self, src: &str) -> Translation {
        // pass 1: read definitions, comments and method sources
        let mut reader: Option<(String, bool)> = None;
        let mut doits: Vec<(String, usize)> = vec![];
        for (chunk, line) in chunks(src) {
            if let Some((class, meta)) = &reader {
                if chunk.trim().is_empty() {
                    reader = None;
                    continue;
                }
                let (class, meta) = (class.clone(), *meta);
                if let Some(c) = self.classes.iter_mut().find(|c| c.name == class) {
                    c.methods.push((meta, chunk.clone(), line));
                }
                continue;
            }
            if chunk.trim().is_empty() {
                continue;
            }
            let parsed = Parser::new(&chunk).and_then(|mut p| p.parse_doit());
            let stmts = match parsed {
                Ok((_, s)) => s,
                Err(e) => {
                    let mut r = error_result(&chunk, &e.msg, e.pos);
                    r.line += line - 1;
                    return r;
                }
            };
            let e = match stmts.as_slice() {
                [Stmt::Expr(e)] => e.clone(),
                _ => {
                    doits.push((chunk.clone(), line));
                    continue;
                }
            };
            if let Some(def) = definition(&e) {
                if let Some(c) = self.classes.iter_mut().find(|c| c.name == def.name) {
                    let methods = std::mem::take(&mut c.methods);
                    *c = ClassDef { methods, ..def };
                } else {
                    self.classes.push(def);
                }
                continue;
            }
            if let Some((class, meta)) = method_reader(&e) {
                if self.class_def(&class).is_none() {
                    self.classes.push(ClassDef { name: class.clone(), superclass: None, ..Default::default() });
                    self.warn(format!("{} is extended here but not defined; its methods become a class of their own", class));
                }
                reader = Some((class, meta));
                continue;
            }
            if let Some((class, comment)) = comment_of(&e) {
                if let Some(c) = self.classes.iter_mut().find(|c| c.name == class) {
                    c.comment = Some(comment);
                }
                continue;
            }
            if is_mirror_noise(&e) {
                continue;
            }
            doits.push((chunk.clone(), line));
        }
        // pass 2: method signatures, so sends can use declared types
        for c in self.classes.clone() {
            for (meta, src, _) in &c.methods {
                if let Ok(m) = Parser::new(src).and_then(|mut p| p.parse_method()) {
                    self.selectors.insert(m.selector.clone());
                    self.side_selectors.insert((*meta, m.selector.clone()));
                    self.defined.entry(c.name.clone()).or_default().insert((*meta, m.selector.clone()), m.ret.clone());
                }
            }
        }
        for c in self.classes.clone() {
            for (meta, src, _) in &c.methods {
                if !*meta {
                    if let Some(sel) = crate::parser::quick_selector(src) {
                        let n = self.method_name(&sel, false);
                        self.method_names.insert(n);
                    }
                }
            }
        }
        // pass 3: translate
        let order = self.class_order();
        let mut body = String::new();
        for name in &order {
            let c = self.class_def(name).cloned().unwrap();
            body.push_str(&self.class_code(&c));
            body.push('\n');
        }
        let mut top = vec![];
        for (chunk, line) in &doits {
            let parsed = Parser::new(chunk).and_then(|mut p| p.parse_doit());
            match parsed {
                Ok((temps, stmts)) => {
                    self.cx = Cx { doit: true, ..Default::default() };
                    self.push_scope();
                    let mut lines = self.body(&temps, &stmts, Tail::None);
                    if self.cx.nlr {
                        lines = wrap_nlr(lines, false);
                    }
                    top.extend(lines);
                }
                Err(e) => self.warn(format!("line {}: {}", line, e.msg)),
            }
        }
        if !top.is_empty() {
            if self.helpers.contains("Transcript") {
                top.push("Transcript.flush();".into());
            }
            body.push_str("// do-its from the file\n");
            body.push_str(&top.join("\n"));
            body.push('\n');
        }
        let decls = self.finish();
        let mut out = String::from("// Translated from Strongtalk to TypeScript.\n");
        let prelude = self.prelude();
        if !prelude.is_empty() {
            out.push_str(&prelude);
            out.push('\n');
        }
        if !decls.is_empty() {
            out.push_str(&decls);
            out.push('\n');
        }
        out.push_str(&body);
        Translation { ok: true, code: out, warnings: self.warnings.clone(), classes: order, ..Default::default() }
    }

    /// Superclasses before subclasses.
    fn class_order(&self) -> Vec<String> {
        let mut done: Vec<String> = vec![];
        let names: Vec<String> = self.classes.iter().map(|c| c.name.clone()).collect();
        fn visit(t: &Tr, n: &str, done: &mut Vec<String>, depth: usize) {
            if done.iter().any(|d| d == n) || depth > 64 {
                return;
            }
            if let Some(s) = t.class_def(n).and_then(|c| c.superclass.clone()) {
                let s = s.split('[').next().unwrap_or("").trim().to_string();
                if t.class_def(&s).is_some() {
                    visit(t, &s, done, depth + 1);
                }
            }
            done.push(n.to_string());
        }
        for n in &names {
            visit(self, n, &mut done, 0);
        }
        done
    }

    fn class_code(&mut self, c: &ClassDef) -> String {
        self.cx = Cx { class: Some(c.name.clone()), generics: c.generics.clone(), ..Default::default() };
        let mut out = String::new();
        if let Some(cm) = &c.comment {
            out.push_str(&doc_comment(cm, ""));
        }
        let name = self.class_ts(&c.name);
        let gens = if c.generics.is_empty() { String::new() } else { format!("<{}>", c.generics.join(", ")) };
        let ext = match &c.superclass {
            None => String::new(),
            Some(s) if s == "Object" || s.is_empty() => String::new(),
            Some(s) => {
                let base = s.split('[').next().unwrap_or("").trim().to_string();
                if self.class_def(&base).is_some() {
                    match parse_type_text(s) {
                        Ok(TypeExpr::Generic(n, args)) => {
                            let a: Vec<String> = args.iter().map(|a| self.ts_type(a)).collect();
                            format!(" extends {}<{}>", self.class_ts(&n), a.join(", "))
                        }
                        _ => {
                            let g = self.class_def(&base).map(|d| d.generics.len()).unwrap_or(0);
                            if g > 0 { format!(" extends {}<{}>", self.class_ts(&base), vec!["any"; g].join(", ")) } else { format!(" extends {}", self.class_ts(&base)) }
                        }
                    }
                } else if ["Error", "Exception"].contains(&base.as_str()) {
                    " extends Error".into()
                } else {
                    self.warn(format!("{}'s superclass {} is not part of this translation", c.name, base));
                    format!(" extends {}", base)
                }
            }
        };
        out.push_str(&format!("export class {}{}{} {{\n", name, gens, ext));
        let mut members: Vec<String> = vec![];
        for v in &c.class_vars {
            let n = self.ivar_ts(&v.name);
            let t = v.ty.as_ref().map(|t| self.ts_type(t)).unwrap_or_else(|| "any".into());
            members.push(format!("  static {}: {};", n, t));
        }
        for v in &c.ivars {
            let n = self.ivar_ts(&v.name);
            match &v.ty {
                Some(t) => {
                    let ts = self.ts_type(t);
                    members.push(format!("  {}!: {};", n, ts));
                }
                None => members.push(format!("  {}: any;", n)),
            }
        }
        if !members.is_empty() {
            out.push_str(&members.join("\n"));
            out.push('\n');
        }
        // class side first, then instance side
        let mut methods: Vec<(bool, String, usize)> = c.methods.iter().filter(|m| m.0).cloned().collect();
        methods.extend(c.methods.iter().filter(|m| !m.0).cloned());
        for (meta, src, line) in methods {
            match self.method_code(c, meta, &src) {
                Ok(code) => {
                    out.push('\n');
                    out.push_str(&code);
                }
                Err((msg, pos)) => {
                    let l = line + src[..pos.min(src.len())].matches('\n').count();
                    self.warn(format!("line {}: {}{}: {} (method skipped)", l, c.name, if meta { " class" } else { "" }, msg));
                    out.push_str(&format!("\n  // could not translate: {}\n", src.trim().lines().next().unwrap_or("")));
                }
            }
        }
        out.push_str("}\n");
        out
    }

    fn method_code(&mut self, c: &ClassDef, meta: bool, src: &str) -> Result<String, (String, usize)> {
        let m = Parser::new(src).and_then(|mut p| p.parse_method()).map_err(|e| (e.msg, e.pos))?;
        self.cx = Cx { class: Some(c.name.clone()), meta, generics: c.generics.clone(), ..Default::default() };
        self.push_scope();
        let mut ps = vec![];
        for p in &m.params {
            let (name, ty, decl) = self.param_decl(p, false);
            self.bind(&p.name, name, ty);
            ps.push(decl);
        }
        let ret = m.ret.as_ref().map(|r| {
            let t = self.ts_type(r);
            // Strongtalk lets nil stand for any type; TypeScript wants it said
            if returns_nil(&m.body) && !["any", "null", "this"].contains(&t.as_str()) && !t.ends_with("| null") {
                format!(": {} | null", t)
            } else {
                format!(": {}", t)
            }
        }).unwrap_or_default();
        let returns_self = match &m.ret {
            None => true,
            Some(TypeExpr::Name(n)) => n == "Self",
            _ => false,
        };
        let mut lines = self.body(&m.temps, &m.body, Tail::Method);
        if returns_self && !always_returns(&m.body) {
            lines.push("return this;".into());
        }
        if self.cx.nlr {
            lines = wrap_nlr(lines, true);
        }
        let name = self.method_name(&m.selector, meta);
        let mut out = String::new();
        if let Some(doc) = method_comment(src, m.header_end) {
            out.push_str(&doc_comment(&doc, "  "));
        }
        out.push_str(&format!("  {}{}({}){} {{\n", if meta { "static " } else { "" }, name, ps.join(", "), ret));
        for l in indent(indent(lines)) {
            out.push_str(&l);
            out.push('\n');
        }
        out.push_str("  }\n");
        Ok(out)
    }

    // -------------------------------------------------------------- the runtime prelude

    /// Notes gathered over the whole unit, and declarations for classes it does not define.
    fn finish(&mut self) -> String {
        if !self.unmapped.is_empty() {
            let names: Vec<String> = self.unmapped.iter().take(14).map(|s| format!("#{}", s)).collect();
            let more = if self.unmapped.len() > 14 { format!(" and {} more", self.unmapped.len() - 14) } else { String::new() };
            self.warn(format!("no library mapping for {}{}: translated as method calls", names.join(", "), more));
        }
        if self.unknown_classes.is_empty() {
            return String::new();
        }
        let names: Vec<String> = self.unknown_classes.iter().cloned().collect();
        self.warn(format!("{} come from the Strongtalk library and are not part of this translation", names.join(", ")));
        let mut s = String::from("// Strongtalk library classes this code uses, which are not translated here:\n");
        for n in names {
            s.push_str(&format!("declare const {n}: any;\ntype {n} = any;\n"));
        }
        s
    }

    fn prelude(&self) -> String {
        let mut want: BTreeSet<&str> = self.helpers.clone().into_iter().collect();
        loop {
            let before = want.len();
            for (name, deps, _) in HELPERS {
                if want.contains(name) {
                    for d in *deps {
                        want.insert(d);
                    }
                }
            }
            if want.len() == before {
                break;
            }
        }
        let parts: Vec<&str> = HELPERS.iter().filter(|(n, _, _)| want.contains(n)).map(|(_, _, code)| *code).collect();
        if parts.is_empty() {
            return String::new();
        }
        format!("// ---- Strongtalk runtime (only what this program uses) ----\n{}\n// ---- end of runtime ----\n", parts.join("\n"))
    }
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Tail {
    None,
    Return,
    Method,
    DoIt,
}

fn balanced_parens(s: &str) -> bool {
    let mut d = 0i32;
    for c in s.chars() {
        match c {
            '(' => d += 1,
            ')' => {
                d -= 1;
                if d < 0 {
                    return false;
                }
            }
            _ => {}
        }
    }
    d == 0
}

fn one_based(i: &E) -> String {
    match i.code.parse::<i64>() {
        Ok(n) => (n - 1).to_string(),
        Err(_) => match i.code.strip_suffix(" + 1") {
            Some(rest) if i.prec == P_ADD => rest.to_string(),
            _ => format!("{} - 1", i.at(P_ADD)),
        },
    }
}

/// Does a `^nil` appear anywhere in these statements (including inside blocks)?
fn returns_nil(stmts: &[Stmt]) -> bool {
    fn in_expr(e: &Expr) -> bool {
        match e {
            Expr::Block(b) => returns_nil(&b.body),
            Expr::Send { recv, args, .. } => in_expr(recv) || args.iter().any(in_expr),
            Expr::Cascade { recv, msgs } => in_expr(recv) || msgs.iter().any(|(_, a, _)| a.iter().any(in_expr)),
            Expr::Assign(_, v, _) => in_expr(v),
            _ => false,
        }
    }
    stmts.iter().any(|s| match s {
        Stmt::Return(Expr::Lit(Lit::Nil, _), _) => true,
        Stmt::Return(e, _) | Stmt::Expr(e) => in_expr(e),
    })
}

fn should_print(e: &Expr) -> bool {
    match e {
        Expr::Assign(..) => false,
        Expr::Send { recv, sel, .. } => {
            !matches!(&**recv, Expr::Var(n, _) if n == "Transcript") && !matches!(sel.as_str(), "printNl" | "displayNl" | "halt" | "inspect")
        }
        _ => true,
    }
}

fn wrap_nlr(lines: Vec<String>, method: bool) -> Vec<String> {
    let mut out = vec!["const $nlr = new NonLocalReturn();".to_string(), "try {".into()];
    out.extend(indent(lines));
    out.push("} catch (e) {".into());
    if method {
        out.push("  if (e === $nlr) return $nlr.value;".into());
    } else {
        out.push("  if (e === $nlr) console.log($printString($nlr.value)); else throw e;".into());
        out.push("}".into());
        return out;
    }
    out.push("  throw e;".into());
    out.push("}".into());
    out
}

fn error_result(src: &str, msg: &str, pos: usize) -> Translation {
    let pos = pos.min(src.len());
    let before = &src[..pos];
    let line = before.matches('\n').count() + 1;
    let column = before.rsplit('\n').next().map(|l| l.chars().count()).unwrap_or(0) + 1;
    Translation { ok: false, error: Some(msg.to_string()), line, column, ..Default::default() }
}

fn doc_comment(text: &str, pad: &str) -> String {
    let lines: Vec<&str> = text.trim().lines().map(|l| l.trim()).collect();
    if lines.len() == 1 {
        return format!("{}/** {} */\n", pad, lines[0].replace("*/", "* /"));
    }
    let mut s = format!("{}/**\n", pad);
    for l in lines {
        s.push_str(&format!("{} * {}\n", pad, l.replace("*/", "* /")).replace(" * \n", " *\n"));
    }
    s.push_str(&format!("{} */\n", pad));
    s
}

/// The first comment after a method's header.
fn method_comment(src: &str, header_end: usize) -> Option<String> {
    let rest = src.get(header_end..)?.trim_start();
    let rest = rest.strip_prefix('"')?;
    let end = rest.find('"')?;
    let c = rest[..end].trim();
    if c.is_empty() { None } else { Some(c.to_string()) }
}

fn sym_or_str(e: &Expr) -> Option<String> {
    match e {
        Expr::Lit(Lit::Sym(s), _) | Expr::Lit(Lit::Str(s), _) => Some(s.clone()),
        _ => None,
    }
}

/// `Delta define: #Foo as: (Class subclassOf: 'Bar' instanceVariables: '...')`, optionally inside
/// `Generic forAll: 'E' body: (...)`, or the classic `Bar subclass: #Foo instanceVariableNames: '...' ...`.
fn definition(e: &Expr) -> Option<ClassDef> {
    if let Expr::Send { recv, sel, args, .. } = e {
        if sel == "define:as:" && matches!(&**recv, Expr::Var(n, _) if n == "Delta") {
            let name = sym_or_str(&args[0])?;
            let mut generics = vec![];
            let mut body = &args[1];
            if let Expr::Send { sel, args: ga, .. } = body {
                if sel == "forAll:body:" {
                    generics = sym_or_str(&ga[0])?.split(|c: char| c.is_whitespace() || c == ',').filter(|s| !s.is_empty()).map(|s| s.to_string()).collect();
                    body = &ga[1];
                }
            }
            if let Expr::Send { sel, args: ca, .. } = body {
                if sel.starts_with("subclassOf:") {
                    let sup = sym_or_str(&ca[0])?;
                    let parts: Vec<&str> = sel.split(':').filter(|p| !p.is_empty()).collect();
                    let mut ivars = vec![];
                    let mut class_vars = vec![];
                    for (i, p) in parts.iter().enumerate().skip(1) {
                        let text = sym_or_str(&ca[i]).unwrap_or_default();
                        match *p {
                            "instanceVariables" => ivars = parse_var_decls(&text).unwrap_or_default(),
                            "classVariables" => class_vars = parse_var_decls(&text).unwrap_or_default(),
                            _ => {}
                        }
                    }
                    return Some(ClassDef { name, superclass: Some(sup), generics, ivars, class_vars, ..Default::default() });
                }
            }
        }
        if sel.starts_with("subclass:") {
            let sup = match &**recv {
                Expr::Var(n, _) => n.clone(),
                Expr::Lit(Lit::Nil, _) => "Object".into(),
                _ => return None,
            };
            let name = sym_or_str(&args[0])?;
            let parts: Vec<&str> = sel.split(':').filter(|p| !p.is_empty()).collect();
            let mut ivars = vec![];
            let mut class_vars = vec![];
            for (i, p) in parts.iter().enumerate().skip(1) {
                let text = sym_or_str(&args[i]).unwrap_or_default();
                match *p {
                    "instanceVariableNames" => ivars = parse_var_decls(&text).unwrap_or_default(),
                    "classVariableNames" => class_vars = parse_var_decls(&text).unwrap_or_default(),
                    _ => {}
                }
            }
            return Some(ClassDef { name, superclass: Some(sup), ivars, class_vars, ..Default::default() });
        }
    }
    None
}

fn mirror_class(e: &Expr) -> Option<String> {
    match e {
        Expr::Send { recv, sel, args, .. } if sel == "mirrorFor:" && matches!(&**recv, Expr::Var(n, _) if n == "Delta") => sym_or_str(&args[0]),
        Expr::Var(n, _) if n.chars().next().map_or(false, |c| c.is_uppercase()) => Some(n.clone()),
        _ => None,
    }
}

/// `(Delta mirrorFor: #Foo) [classSide] methodsFor: '...'` or `Foo [class] methodsFor: '...'`.
fn method_reader(e: &Expr) -> Option<(String, bool)> {
    if let Expr::Send { recv, sel, .. } = e {
        if sel == "methodsFor:" {
            if let Expr::Send { recv: inner, sel: s2, args, .. } = &**recv {
                if s2 == "classSide" || s2 == "class" {
                    return mirror_class(inner).map(|c| (c, true));
                }
                if s2 == "mirrorFor:" {
                    return sym_or_str(&args[0]).map(|c| (c, false));
                }
            }
            return mirror_class(recv).map(|c| (c, false));
        }
    }
    None
}

fn comment_of(e: &Expr) -> Option<(String, String)> {
    if let Expr::Send { recv, sel, args, .. } = e {
        if sel == "comment:" {
            return Some((mirror_class(recv)?, sym_or_str(&args[0])?));
        }
    }
    None
}

fn is_mirror_noise(e: &Expr) -> bool {
    matches!(e, Expr::Send { recv, sel, .. } if (sel == "group:" || sel == "category:" || sel == "revision:") && mirror_class(recv).is_some())
}

// ------------------------------------------------------------------ runtime helpers
//
// (name, dependencies, TypeScript source). Emitted in this order, only when used.

const HELPERS: &[(&str, &[&str], &str)] = &[
    (
        "nlr",
        &["printString"],
        r#"/** Carries a `^` from inside a block out of the enclosing method. */
class NonLocalReturn {
  value: any;
  return(v: any): never { this.value = v; throw this; }
}"#,
    ),
    (
        "exceptions",
        &[],
        r#"class ZeroDivide extends Error {}
class MessageNotUnderstood extends Error {}
class Warning extends Error {}
class NotFound extends Error {}
class SubscriptOutOfBounds extends Error {}"#,
    ),
    (
        "WriteStream",
        &["printString"],
        r#"/** Smalltalk's WriteStream on a String. */
class WriteStream {
  text: string;
  constructor(text: string = '') { this.text = text; }
  static on(_s?: unknown) { return new WriteStream(); }
  static with(s: string) { return new WriteStream(s); }
  nextPutAll(s: unknown) { this.text += String(s); return this; }
  nextPut(c: unknown) { this.text += String(c); return this; }
  print(x: unknown) { this.text += $printString(x); return this; }
  display(x: unknown) { this.text += $displayString(x); return this; }
  cr() { this.text += '\n'; return this; }
  tab() { this.text += '\t'; return this; }
  space() { this.text += ' '; return this; }
  contents() { return this.text; }
}"#,
    ),
    (
        "Random",
        &[],
        r#"class Random {
  next() { return Math.random(); }
  nextInt(n: number) { return Math.floor(Math.random() * n) + 1; }
  betweenAnd(a: number, b: number) { return a + Math.floor(Math.random() * (b - a + 1)); }
}"#,
    ),
    (
        "Transcript",
        &["printString"],
        r#"/** The Smalltalk Transcript: writes to standard output (Node) or the console. */
const Transcript = {
  buffer: '',
  write(s: string) {
    const out = (globalThis as any).process?.stdout;
    if (out?.write) { out.write(s); return this; }
    this.buffer += s;
    const nl = this.buffer.lastIndexOf('\n');
    if (nl >= 0) { console.log(this.buffer.slice(0, nl)); this.buffer = this.buffer.slice(nl + 1); }
    return this;
  },
  show(x: unknown) { return this.write($displayString(x)); },
  nextPutAll(x: unknown) { return this.write($displayString(x)); },
  print(x: unknown) { return this.write($printString(x)); },
  display(x: unknown) { return this.write($displayString(x)); },
  showCr(x: unknown) { return this.write($displayString(x) + '\n'); },
  cr() { return this.write('\n'); },
  tab() { return this.write('\t'); },
  space() { return this.write(' '); },
  clear() { return this; },
  flush() { if (this.buffer) { console.log(this.buffer); this.buffer = ''; } return this; },
};"#,
    ),
    (
        "printString",
        &["WriteStream"],
        r#"function $article(name: string): string {
  return (/^[AEIOU]/.test(name) ? 'an ' : 'a ') + name;
}
/** What Smalltalk's printString shows for a value. */
function $printString(x: any): string {
  if (x === null || x === undefined) return 'nil';
  if (typeof x === 'string') return "'" + x.replaceAll("'", "''") + "'";
  if (typeof x === 'number' || typeof x === 'bigint' || typeof x === 'boolean') return String(x);
  if (typeof x === 'function') return 'a Block';
  if (Array.isArray(x)) return '#(' + x.map($printString).join(' ') + ')';
  if (x instanceof Set) return 'a Set(' + [...x].map($printString).join(' ') + ')';
  if (x instanceof Map) return 'a Dictionary(' + [...x].map(([k, v]) => $printString(k) + '->' + $printString(v)).join(' ') + ')';
  if (typeof x.printOn === 'function') { const s = new WriteStream(); x.printOn(s); return s.contents(); }
  if (typeof x.printString === 'function') return x.printString();
  if (x instanceof Error) return $article(x.constructor.name) + (x.message ? ': ' + x.message : '');
  return $article(x.constructor?.name ?? 'Object');
}
/** displayString: strings without quotes, everything else as printString. */
function $displayString(x: any): string {
  return typeof x === 'string' ? x : $printString(x);
}"#,
    ),
    ("error", &[], "function $error(message: string): never { throw new Error(message); }"),
    (
        "raise",
        &[],
        r#"function $raise(e: any, message?: string): never {
  if (message !== undefined) e.message = message;
  throw e;
}"#,
    ),
    ("mod", &[], "/** Smalltalk's \\\\ : the modulo rounds toward negative infinity. */\nfunction $mod(a: number, b: number): number { return ((a % b) + b) % b; }"),
    (
        "range",
        &[],
        r#"/** An Interval (a to: b by: step) as an array. */
function $range(from: number, to: number, step = 1): number[] {
  const out: number[] = [];
  if (step > 0) for (let i = from; i <= to; i += step) out.push(i);
  else for (let i = from; i >= to; i += step) out.push(i);
  return out;
}"#,
    ),
    (
        "div",
        &["exceptions"],
        "/** Division that raises ZeroDivide, as Smalltalk does. */\nfunction $div(a: number, b: number): number {\n  if (b === 0) throw new ZeroDivide('division by zero');\n  return a / b;\n}",
    ),
    ("between", &[], "function $between(x: number, lo: number, hi: number): boolean { return x >= lo && x <= hi; }"),
    (
        "size",
        &[],
        r#"function $size(c: any): number {
  if (typeof c === 'string' || Array.isArray(c)) return c.length;
  if (c instanceof Set || c instanceof Map) return c.size;
  return typeof c?.size === 'function' ? c.size() : 0;
}"#,
    ),
    (
        "isEmpty",
        &["size"],
        "function $isEmpty(c: any): boolean { return typeof c?.isEmpty === 'function' ? c.isEmpty() : $size(c) === 0; }",
    ),
    (
        "at",
        &[],
        r#"/** Smalltalk's 1-based at: for arrays and strings, keys for dictionaries. */
function $at(c: any, i: any): any {
  if (c instanceof Map) return c.get(i);
  if (typeof c === 'string' || Array.isArray(c)) {
    if (i < 1 || i > c.length) throw new RangeError('index ' + i + ' is out of bounds');
    return c[i - 1];
  }
  return c.at(i);
}"#,
    ),
    (
        "atPut",
        &[],
        r#"function $atPut(c: any, i: any, v: any): any {
  if (c instanceof Map) { c.set(i, v); return v; }
  if (Array.isArray(c)) {
    if (i < 1 || i > c.length) throw new RangeError('index ' + i + ' is out of bounds');
    c[i - 1] = v; return v;
  }
  return c.atPut(i, v);
}"#,
    ),
    (
        "add",
        &[],
        r#"function $add(c: any, v: any): any {
  if (Array.isArray(c)) c.push(v); else if (c instanceof Set) c.add(v); else c.add(v);
  return v;
}"#,
    ),
    (
        "remove",
        &[],
        r#"function $remove(c: any, v: any): any {
  if (c instanceof Set) { c.delete(v); return v; }
  const i = c.indexOf(v);
  if (i < 0) throw new Error('object not found');
  c.splice(i, 1);
  return v;
}"#,
    ),
    (
        "includes",
        &[],
        "function $includes(c: any, v: any): boolean { return c instanceof Set || c instanceof Map ? c.has(v) : c.includes(v); }",
    ),
    (
        "reversed",
        &[],
        "function $reversed(c: any): any { return typeof c === 'string' ? [...c].reverse().join('') : [...c].reverse(); }",
    ),
    (
        "sorted",
        &[],
        r#"/** asSortedCollection: numbers numerically, everything else by < */
function $sorted<T>(c: Iterable<T>): T[] {
  return $sort([...c]);
}
/** sort: in place, by < */
function $sort<T>(c: T[]): T[] {
  return c.sort((a: any, b: any) => (a < b ? -1 : a > b ? 1 : 0));
}"#,
    ),
    (
        "occurrences",
        &[],
        "function $occurrencesOf(c: any, v: any): number { return [...c].filter((x) => x === v).length; }",
    ),
    (
        "factorial",
        &[],
        r#"/** n factorial, exact: a BigInt once it no longer fits a number. */
function $factorial(n: number): number | bigint {
  let r = 1n;
  for (let i = 2n; i <= BigInt(n); i++) r *= i;
  return r <= BigInt(Number.MAX_SAFE_INTEGER) ? Number(r) : r;
}"#,
    ),
    (
        "isPrime",
        &[],
        r#"function $isPrime(n: number): boolean {
  if (n < 2) return false;
  for (let i = 2; i * i <= n; i++) if (n % i === 0) return false;
  return true;
}"#,
    ),
    (
        "gcd",
        &[],
        r#"function $gcd(a: number, b: number): number { a = Math.abs(a); b = Math.abs(b); while (b) [a, b] = [b, a % b]; return a; }
function $lcm(a: number, b: number): number { return a === 0 || b === 0 ? 0 : Math.abs(a * b) / $gcd(a, b); }"#,
    ),
    (
        "time",
        &[],
        "function $millisecondsToRun(f: () => unknown): number { const t = performance.now(); f(); return Math.round(performance.now() - t); }",
    ),
    (
        "prim",
        &[],
        "function $primitive(name: string, _args: unknown[]): never { throw new Error('the VM primitive ' + name + ' is not available in TypeScript'); }",
    ),
];

#[cfg(test)]
mod tests {
    use super::*;

    fn ts(src: &str) -> String {
        let t = translate(src);
        assert!(t.ok, "translation failed: {:?}", t.error);
        t.code
    }

    #[test]
    fn arithmetic_left_to_right() {
        let c = ts("3 + 4 * 2");
        assert!(c.contains("(3 + 4) * 2"), "{}", c);
    }

    #[test]
    fn typed_block_and_collect() {
        let c = ts("(1 to: 10) collect: [:i <Int> | i * i]");
        assert!(c.contains("$range(1, 10).map((i: number) => i * i)"), "{}", c);
    }

    #[test]
    fn control_flow_statements() {
        let c = ts("| s <Int> | s := 0. 1 to: 10 do: [:i | i even ifTrue: [s := s + i]]. s");
        assert!(c.contains("for (let i = 1; i <= 10; i++) {"), "{}", c);
        assert!(c.contains("if (i % 2 === 0) {"), "{}", c);
    }

    #[test]
    fn classes() {
        let src = include_str!("../lib/demos.dlt");
        let t = translate(src);
        assert!(t.ok, "{:?}", t.error);
        assert!(t.code.contains("export class BankAccount {"), "{}", t.code);
        assert!(t.code.contains("deposit(amount: number): this {"), "{}", t.code);
        assert!(t.code.contains("static owner(name: string): BankAccount {"), "{}", t.code);
    }

    #[test]
    fn syntax_error_position() {
        let t = translate("3 + + 4");
        assert!(!t.ok);
        assert_eq!(t.line, 1);
    }
}
