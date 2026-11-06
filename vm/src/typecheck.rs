//! Strongtalk's optional type checker, simplified: annotated variables, arguments and
//! return types are checked; untyped code is dynamic and never reported.
//!
//! Subtyping follows inheritance (the original also allowed structural protocol matches).

use crate::ast::*;
use crate::interp::Interp;
use crate::object::*;
use crate::parser::Parser;
use std::collections::HashMap;
use std::rc::Rc;

#[derive(Clone)]
pub enum Ty {
    Dyn,
    Bottom,
    Class(Rc<Class>, bool),
    Union(Vec<Ty>),
    Block(usize, Box<Ty>),
}

pub struct Problem {
    pub class: String,
    pub meta: bool,
    pub selector: String,
    pub line: usize,
    pub message: String,
}

pub fn alias(name: &str) -> &str {
    match name {
        "Int" => "Integer",
        "Str" => "String",
        "Sym" => "Symbol",
        "Bool" => "Boolean",
        "Char" => "Character",
        "Num" => "Number",
        "Cltn" => "Collection",
        "SeqCltn" => "SequenceableCollection",
        "OrdCltn" => "OrderedCollection",
        "Dict" => "Dictionary",
        "Fun" => "BlockClosure",
        "Nil" => "UndefinedObject",
        other => other,
    }
}

impl Ty {
    pub fn show(&self) -> String {
        match self {
            Ty::Dyn => "<dynamic>".into(),
            Ty::Bottom => "UndefinedObject".into(),
            Ty::Class(c, false) => c.name(),
            Ty::Class(c, true) => format!("{} class", c.name()),
            Ty::Union(v) => v.iter().map(|t| t.show()).collect::<Vec<_>>().join(" | "),
            Ty::Block(n, r) => {
                let mut s = String::from("[");
                for _ in 0..*n {
                    s.push_str("Object, ");
                }
                s.push_str(&format!("^{}]", r.show()));
                s
            }
        }
    }
}

struct Env {
    scopes: Vec<HashMap<String, Ty>>,
}

pub struct Checker<'a> {
    it: &'a mut Interp,
    class: Rc<Class>,
    meta: bool,
    generics: Vec<String>,
    ivars: HashMap<String, Ty>,
    ret: Option<Ty>,
    src: String,
    sel: String,
    pub problems: Vec<Problem>,
}

impl<'a> Checker<'a> {
    fn report(&mut self, pos: usize, msg: String) {
        let line = self.src[..pos.min(self.src.len())].matches('\n').count() + 1;
        self.problems.push(Problem { class: self.class.name(), meta: self.meta, selector: self.sel.clone(), line, message: msg });
    }

    fn class_named(&self, n: &str) -> Option<Rc<Class>> {
        self.it.class_named(alias(n))
    }

    fn resolve(&mut self, t: &TypeExpr, pos: usize) -> Ty {
        match t {
            TypeExpr::Name(n) => {
                if n == "Self" {
                    return Ty::Class(self.class.clone(), self.meta);
                }
                if n == "Instance" {
                    return Ty::Class(self.class.clone(), false);
                }
                if n == "Object" {
                    return Ty::Class(self.it.core.object.clone(), false);
                }
                if self.generics.iter().any(|g| g == n) {
                    return Ty::Dyn;
                }
                // single capital letters and the like are type variables of generic methods
                if n.len() <= 2 && n.chars().all(|c| c.is_ascii_uppercase()) {
                    return Ty::Dyn;
                }
                match self.class_named(n) {
                    Some(c) => Ty::Class(c, false),
                    None => {
                        self.report(pos, format!("unknown type '{}'", n));
                        Ty::Dyn
                    }
                }
            }
            TypeExpr::Generic(n, args) => {
                for a in args {
                    self.resolve(a, pos);
                }
                self.resolve(&TypeExpr::Name(n.clone()), pos)
            }
            TypeExpr::Union(ts) => {
                let v: Vec<Ty> = ts.iter().map(|t| self.resolve(t, pos)).collect();
                if v.iter().any(|t| matches!(t, Ty::Dyn)) {
                    Ty::Dyn
                } else {
                    Ty::Union(v)
                }
            }
            TypeExpr::Block(args, r) => {
                for a in args {
                    self.resolve(a, pos);
                }
                let rt = match r {
                    Some(r) => self.resolve(r, pos),
                    None => Ty::Dyn,
                };
                Ty::Block(args.len(), Box::new(rt))
            }
            TypeExpr::Meta(inner) => match self.resolve(inner, pos) {
                Ty::Class(c, false) => Ty::Class(c, true),
                other => other,
            },
        }
    }

    /// Whether a value of type `a` may be used where `b` is expected.
    fn compatible(&self, a: &Ty, b: &Ty) -> bool {
        match (a, b) {
            (Ty::Dyn, _) | (_, Ty::Dyn) | (Ty::Bottom, _) => true,
            (Ty::Union(xs), _) => xs.iter().all(|x| self.compatible(x, b)),
            (_, Ty::Union(ys)) => ys.iter().any(|y| self.compatible(a, y)),
            (_, Ty::Class(o, false)) if Rc::ptr_eq(o, &self.it.core.object) => true,
            (Ty::Class(_, true), Ty::Class(d, false)) => {
                Rc::ptr_eq(d, &self.it.core.class) || Rc::ptr_eq(d, &self.it.core.behavior)
            }
            (Ty::Class(c, m1), Ty::Class(d, m2)) => m1 == m2 && c.inherits_from(d),
            (Ty::Block(..), Ty::Block(..)) => true,
            (Ty::Block(..), Ty::Class(d, false)) => self.it.core.block.inherits_from(d),
            (Ty::Class(c, false), Ty::Block(..)) => c.inherits_from(&self.it.core.block),
            _ => false,
        }
    }

    fn lit_ty(&self, l: &Lit) -> Ty {
        let c = &self.it.core;
        match l {
            Lit::Nil => Ty::Bottom,
            Lit::True => Ty::Class(c.true_.clone(), false),
            Lit::False => Ty::Class(c.false_.clone(), false),
            Lit::Int(i) if (SMI_MIN..=SMI_MAX).contains(i) => Ty::Class(c.small_int.clone(), false),
            Lit::Int(_) | Lit::Big(_) => Ty::Class(c.large_int.clone(), false),
            Lit::Float(_) => Ty::Class(c.float.clone(), false),
            Lit::Char(_) => Ty::Class(c.character.clone(), false),
            Lit::Str(_) => Ty::Class(c.string.clone(), false),
            Lit::Sym(_) => Ty::Class(c.symbol.clone(), false),
            Lit::Array(_) => Ty::Class(c.array.clone(), false),
            Lit::Bytes(_) => Ty::Class(c.byte_array.clone(), false),
        }
    }

    fn var_ty(&mut self, env: &Env, name: &str, pos: usize) -> Ty {
        if name == "self" {
            return Ty::Class(self.class.clone(), self.meta);
        }
        if name == "super" {
            return match self.class.superclass() {
                Some(s) => Ty::Class(s, self.meta),
                None => Ty::Dyn,
            };
        }
        for sc in env.scopes.iter().rev() {
            if let Some(t) = sc.get(name) {
                return t.clone();
            }
        }
        if let Some(t) = self.ivars.get(name) {
            return t.clone();
        }
        if let Some(c) = self.it.class_named(name) {
            return Ty::Class(c, true);
        }
        let _ = pos;
        Ty::Dyn
    }

    fn method_for(&mut self, c: &Rc<Class>, meta: bool, sel: &str) -> Option<Rc<Method>> {
        let s = self.it.intern(sel);
        self.it.lookup_from(Some(c.clone()), meta, &s)
    }

    /// The declared result of `sel` sent to a receiver of type `rt`.
    fn send_ty(&mut self, rt: &Ty, sel: &str, args: &[Ty], pos: usize) -> Ty {
        match rt {
            Ty::Dyn | Ty::Bottom => Ty::Dyn,
            Ty::Union(alts) => {
                let mut results = Vec::new();
                for a in alts.clone() {
                    results.push(self.send_ty(&a, sel, args, pos));
                }
                Ty::Dyn
            }
            Ty::Block(n, r) => {
                if sel.starts_with("value") && sel.matches(':').count() == *n {
                    (**r).clone()
                } else {
                    let b = self.it.core.block.clone();
                    self.send_ty(&Ty::Class(b, false), sel, args, pos)
                }
            }
            Ty::Class(c, meta) => {
                let (c, meta) = (c.clone(), *meta);
                if let Some(t) = self.arith_rule(&c, meta, sel, args) {
                    return t;
                }
                let is_object = !meta && Rc::ptr_eq(&c, &self.it.core.object);
                let m = match self.method_for(&c, meta, sel) {
                    Some(m) => m,
                    None if is_object => return Ty::Dyn,
                    None => {
                        let dnu_custom = self
                            .method_for(&c, meta, "doesNotUnderstand:")
                            .map(|m| !Rc::ptr_eq(&m.class, &self.it.core.object))
                            .unwrap_or(false);
                        if !dnu_custom {
                            let who = if meta { format!("{} class", c.name()) } else { c.name() };
                            self.report(pos, format!("{} does not understand #{}", who, sel));
                        }
                        return Ty::Dyn;
                    }
                };
                let ast = match &m.ast {
                    Some(a) => a.clone(),
                    None => return Ty::Dyn,
                };
                // check arguments against the declared parameter types, in the callee's context
                let saved = (self.class.clone(), self.meta, std::mem::take(&mut self.generics));
                self.class = m.class.clone();
                self.meta = m.meta;
                self.generics = m.class.generics.borrow().iter().map(|v| v.name.clone()).collect();
                let quiet = self.problems.len();
                let mut params = Vec::new();
                for p in &ast.params {
                    params.push(match &p.ty {
                        Some(t) => self.resolve(t, 0),
                        None => Ty::Dyn,
                    });
                }
                let ret = match &ast.ret {
                    Some(TypeExpr::Name(n)) if n == "Self" => Some(Ty::Class(c.clone(), meta)),
                    Some(TypeExpr::Name(n)) if n == "Instance" => Some(Ty::Class(c.clone(), false)),
                    Some(t) => Some(self.resolve(t, 0)),
                    None => None,
                };
                self.problems.truncate(quiet);
                self.class = saved.0;
                self.meta = saved.1;
                self.generics = saved.2;
                for (i, (a, p)) in args.iter().zip(params.iter()).enumerate() {
                    if !self.compatible(a, p) {
                        self.report(
                            pos,
                            format!("argument {} of #{}: {} is not a {}", i + 1, sel, a.show(), p.show()),
                        );
                    }
                }
                match ret {
                    // <Object> results carry no information: treat them as dynamic
                    Some(Ty::Class(o, false)) if Rc::ptr_eq(&o, &self.it.core.object) => Ty::Dyn,
                    Some(t) => t,
                    None => Ty::Dyn,
                }
            }
        }
    }

    /// Strongtalk declared arithmetic with inference clauses (Int + Int is an Int);
    /// these rules play that part.
    fn arith_rule(&self, c: &Rc<Class>, meta: bool, sel: &str, args: &[Ty]) -> Option<Ty> {
        if meta {
            return None;
        }
        let core = &self.it.core;
        let is = |t: &Ty, k: &Rc<Class>| matches!(t, Ty::Class(x, false) if x.inherits_from(k));
        let me = Ty::Class(c.clone(), false);
        let int = Ty::Class(core.integer.clone(), false);
        let float = Ty::Class(core.float.clone(), false);
        let boolean = Ty::Class(core.boolean.clone(), false);
        if !c.inherits_from(&core.number) {
            return None;
        }
        let int_recv = is(&me, &core.integer);
        if args.iter().any(|a| matches!(a, Ty::Dyn)) && sel.chars().next().map(|ch| !ch.is_alphabetic()).unwrap_or(false) {
            return Some(Ty::Dyn);
        }
        match (sel, args.len()) {
            ("+" | "-" | "*" | "//" | r"\\" | "rem:" | "quo:" | "gcd:" | "lcm:" | "max:" | "min:", 1) => {
                let a = &args[0];
                if int_recv && is(a, &core.integer) {
                    Some(int)
                } else if (is(&me, &core.float) && is(a, &core.number)) || (is(&me, &core.number) && is(a, &core.float)) {
                    Some(if matches!(sel, "//" | "quo:") { int } else { float })
                } else {
                    None
                }
            }
            ("bitAnd:" | "bitOr:" | "bitXor:" | "bitShift:" | "<<" | ">>", 1) if int_recv => Some(int),
            ("<" | ">" | "<=" | ">=" | "=" | "~=", 1) if matches!(args[0], Ty::Class(..)) && is(&args[0], &core.number) => Some(boolean),
            ("negated" | "abs" | "squared", 0) => Some(me),
            _ => None,
        }
    }

    fn expr(&mut self, env: &mut Env, e: &Expr) -> Ty {
        match e {
            Expr::Lit(l, _) => self.lit_ty(l),
            Expr::Var(n, p) => self.var_ty(env, n, *p),
            Expr::Assign(n, v, p) => {
                let vt = self.expr(env, v);
                let target = self.var_ty(env, n, *p);
                if !self.compatible(&vt, &target) {
                    self.report(*p, format!("assigning {} to '{}', which is declared {}", vt.show(), n, target.show()));
                }
                vt
            }
            Expr::Send { recv, sel, args, pos } => {
                let rt = self.expr(env, recv);
                let at: Vec<Ty> = args.iter().map(|a| self.expr(env, a)).collect();
                self.send_ty(&rt, sel, &at, *pos)
            }
            Expr::Cascade { recv, msgs } => {
                let rt = self.expr(env, recv);
                let mut last = Ty::Dyn;
                for (sel, args, pos) in msgs {
                    let at: Vec<Ty> = args.iter().map(|a| self.expr(env, a)).collect();
                    last = self.send_ty(&rt, sel, &at, *pos);
                }
                last
            }
            Expr::Block(b) => {
                let mut sc = HashMap::new();
                for p in b.params.iter().chain(b.temps.iter()) {
                    let t = match &p.ty {
                        Some(t) => self.resolve(t, p.pos),
                        None => Ty::Dyn,
                    };
                    sc.insert(p.name.clone(), t);
                }
                env.scopes.push(sc);
                let mut last = Ty::Bottom;
                for s in &b.body {
                    last = self.stmt(env, s);
                }
                env.scopes.pop();
                if b.body.is_empty() {
                    last = Ty::Bottom;
                }
                Ty::Block(b.params.len(), Box::new(last))
            }
            Expr::Brace(items, _) => {
                for i in items {
                    self.expr(env, i);
                }
                Ty::Class(self.it.core.array.clone(), false)
            }
            Expr::Guaranteed(t, e, p) => {
                self.expr(env, e);
                self.resolve(t, *p)
            }
            Expr::Prim { recv, args, .. } => {
                if let Some(r) = recv {
                    self.expr(env, r);
                }
                for a in args {
                    self.expr(env, a);
                }
                Ty::Dyn
            }
        }
    }

    fn stmt(&mut self, env: &mut Env, s: &Stmt) -> Ty {
        match s {
            Stmt::Expr(e) => self.expr(env, e),
            Stmt::Return(e, p) => {
                let t = self.expr(env, e);
                if let Some(r) = self.ret.clone() {
                    if !self.compatible(&t, &r) {
                        self.report(*p, format!("returns {} but the method is declared to return {}", t.show(), r.show()));
                    }
                }
                Ty::Bottom
            }
        }
    }
}

pub fn check_method(it: &mut Interp, m: &Rc<Method>) -> Vec<Problem> {
    let ast = match &m.ast {
        Some(a) => a.clone(),
        None => return vec![],
    };
    let class = m.class.clone();
    let generics: Vec<String> = class.generics.borrow().iter().map(|v| v.name.clone()).collect();
    let mut ck = Checker {
        it,
        class: class.clone(),
        meta: m.meta,
        generics,
        ivars: HashMap::new(),
        ret: None,
        src: m.source.clone(),
        sel: m.selector.to_string(),
        problems: vec![],
    };
    let decls = if m.meta { class.all_class_ivars() } else { class.all_ivars() };
    for v in decls {
        let t = match &v.ty {
            Some(t) => ck.resolve(t, 0),
            None => Ty::Dyn,
        };
        ck.ivars.insert(v.name.clone(), t);
    }
    // instance variables are resolved against the class that declares them; problems with
    // their declarations are reported by check_class, not for every method
    ck.problems.clear();
    let mut env = Env { scopes: vec![HashMap::new()] };
    for p in ast.params.iter().chain(ast.temps.iter()) {
        let t = match &p.ty {
            Some(t) => ck.resolve(t, p.pos),
            None => Ty::Dyn,
        };
        env.scopes[0].insert(p.name.clone(), t);
    }
    ck.ret = ast.ret.as_ref().map(|t| ck.resolve(t, ast.header_end));
    for s in &ast.body {
        ck.stmt(&mut env, s);
    }
    if !matches!(ast.body.last(), Some(Stmt::Return(..))) {
        if let Some(r) = ck.ret.clone() {
            let selfty = Ty::Class(class.clone(), m.meta);
            if !ck.compatible(&selfty, &r) {
                ck.report(ast.header_end, format!("returns self ({}) at the end but is declared to return {}", selfty.show(), r.show()));
            }
        }
    }
    ck.problems
}

pub fn check_class(it: &mut Interp, c: &Rc<Class>) -> Vec<Problem> {
    let mut out = Vec::new();
    // declarations of instance variables
    let dummy = Rc::new(Method {
        selector: it.intern("<definition>"),
        class: c.clone(),
        meta: false,
        category: String::new(),
        source: String::new(),
        signature: String::new(),
        ast: None,
        body: MethodBody::Native(|_, r, _| Ok(r)),
    });
    {
        let generics: Vec<String> = c.generics.borrow().iter().map(|v| v.name.clone()).collect();
        let mut ck = Checker {
            it,
            class: c.clone(),
            meta: false,
            generics,
            ivars: HashMap::new(),
            ret: None,
            src: String::new(),
            sel: "definition".into(),
            problems: vec![],
        };
        let decls: Vec<Var> = c.ivars.borrow().iter().chain(c.class_ivars.borrow().iter()).cloned().collect();
        for v in decls {
            if let Some(t) = &v.ty {
                ck.resolve(t, 0);
            }
        }
        out.extend(ck.problems);
    }
    let _ = dummy;
    for meta in [false, true] {
        let mut ms: Vec<Rc<Method>> = c.method_dict(meta).borrow().values().cloned().collect();
        ms.sort_by_key(|m| m.selector.to_string());
        for m in ms {
            out.extend(check_method(it, &m));
        }
    }
    out
}

/// Type-checks a do-it without running it.
pub fn check_doit(it: &mut Interp, src: &str) -> Result<Vec<Problem>, String> {
    let mut p = Parser::new(src).map_err(|e| e.msg)?;
    let (temps, body) = p.parse_doit().map_err(|e| e.msg)?;
    let ast = MethodAst { selector: "DoIt".into(), params: vec![], ret: None, temps, body, header_end: 0 };
    let m = Rc::new(Method {
        selector: it.intern("DoIt"),
        class: it.core.undefined.clone(),
        meta: false,
        category: String::new(),
        source: src.to_string(),
        signature: "DoIt".into(),
        ast: Some(Rc::new(ast)),
        body: MethodBody::Native(|_, r, _| Ok(r)),
    });
    Ok(check_method(it, &m))
}
