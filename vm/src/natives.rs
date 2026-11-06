//! Methods the VM implements directly: class definition (`Delta define:as:`), mirrors,
//! method readers for file-in, and reflection on classes.

use crate::interp::{Interp, R};
use crate::object::*;
use crate::parser::parse_var_decls;
use crate::prims::string_of;
use std::rc::Rc;

fn install_one(it: &mut Interp, cls: &Rc<Class>, meta: bool, sel: &str, f: NativeFn) {
    let s = it.intern(sel);
    let m = Rc::new(Method {
        selector: s.clone(),
        class: cls.clone(),
        meta,
        category: "primitives".into(),
        source: format!("{}\n    \"implemented inside the virtual machine\"", sel),
        signature: sel.to_string(),
        ast: None,
        body: MethodBody::Native(f),
    });
    cls.method_dict(meta).borrow_mut().insert(s, m);
}

pub fn install(it: &mut Interp) {
    let generic = Class::new("Generic", Some(it.core.object.clone()), Format::Plain);
    *generic.group.borrow_mut() = "kernel".into();
    it.globals.insert("Generic".into(), Value::Class(generic.clone()));
    let sys = it.core.system.clone();
    let cls = it.core.class.clone();
    let decl = it.core.class_decl.clone();
    let mirror = it.core.mirror.clone();
    let beh = it.core.behavior.clone();

    install_one(it, &sys, false, "define:as:", sys_define);
    install_one(it, &sys, false, "mirrorFor:", sys_mirror_for);
    install_one(it, &sys, false, "at:", |it, _, a| {
        let k = string_of(&a[0]).unwrap_or_default();
        match it.globals.get(&k) {
            Some(v) => Ok(v.clone()),
            None => it.signal_error("Error", &format!("key not found: {}", k)),
        }
    });
    install_one(it, &sys, false, "at:put:", |it, _, a| {
        let k = string_of(&a[0]).unwrap_or_default();
        it.globals.insert(k, a[1].clone());
        Ok(a[1].clone())
    });
    install_one(it, &sys, false, "at:ifAbsent:", |it, _, a| {
        let k = string_of(&a[0]).unwrap_or_default();
        match it.globals.get(&k) {
            Some(v) => Ok(v.clone()),
            None => it.call_flexible(&a[1], vec![]),
        }
    });
    install_one(it, &sys, false, "includesKey:", |it, _, a| {
        let k = string_of(&a[0]).unwrap_or_default();
        Ok(Value::Bool(it.globals.contains_key(&k)))
    });
    install_one(it, &sys, false, "removeKey:", |it, _, a| {
        let k = string_of(&a[0]).unwrap_or_default();
        Ok(it.globals.remove(&k).unwrap_or(Value::Nil))
    });
    install_one(it, &sys, false, "classNames", |it, _, _| {
        let mut ks: Vec<String> = it.globals.iter().filter(|(_, v)| matches!(v, Value::Class(_))).map(|(k, _)| k.clone()).collect();
        ks.sort();
        let v = ks.iter().map(|k| Value::Sym(it.intern(k))).collect();
        Ok(it.array(v))
    });

    for sel in [
        "subclassOf:instanceVariables:",
        "subclassOf:instanceVariables:classInstanceVariables:",
        "subclassOf:instanceVariables:classVariables:",
        "subclassOf:instanceVariables:classVariables:classInstanceVariables:",
    ] {
        install_one(it, &cls, true, sel, |it, _, a| {
            let d = it.new_object(&it.core.class_decl.clone());
            if let Value::Obj(o) = &d {
                o.set(0, a[0].clone());
                o.set(1, a.get(1).cloned().unwrap_or(Value::str("")));
                o.set(2, Value::str(""));
                o.set(3, Value::str(""));
                o.set(4, Value::str(""));
                if a.len() == 3 {
                    o.set(3, a[2].clone());
                }
                if a.len() == 4 {
                    o.set(2, a[2].clone());
                    o.set(3, a[3].clone());
                }
            }
            Ok(d)
        });
    }
    // ClassDeclaration slots: superclass, ivars, classVars, classIvars, generics
    decl.ivars.borrow_mut().extend(parse_var_decls("superclass ivars classVars classIvars generics").unwrap_or_default());
    // Class sentinel objects: the subclassOf: entry point above uses the shape below.
    install_one(it, &decl, false, "classVariables:", |_, r, a| {
        if let Value::Obj(o) = &r {
            o.set(2, a[0].clone());
        }
        Ok(r.clone())
    });
    install_one(it, &decl, false, "classInstanceVariables:", |_, r, a| {
        if let Value::Obj(o) = &r {
            o.set(3, a[0].clone());
        }
        Ok(r.clone())
    });
    for sel in ["abstract", "protocols:", "isPrimaryInvocation:"] {
        install_one(it, &decl, false, sel, |_, r, _| Ok(r.clone()));
    }
    install_one(it, &generic, true, "forAll:body:", |_, _, a| {
        if let Value::Obj(o) = &a[1] {
            o.set(4, a[0].clone());
        }
        Ok(a[1].clone())
    });

    mirror.ivars.borrow_mut().extend(parse_var_decls("reflectee isMeta").unwrap_or_default());
    it.core.method_reader.ivars.borrow_mut().extend(parse_var_decls("reflectee isMeta category").unwrap_or_default());
    install_one(it, &mirror, false, "methodsFor:", |it, r, a| {
        let (c, m) = mirror_target(&r);
        Ok(method_reader(it, c, m, a[0].clone()))
    });
    install_one(it, &mirror, false, "classSide", |it, r, _| {
        let (c, _) = mirror_target(&r);
        Ok(new_mirror(it, c, true))
    });
    install_one(it, &mirror, false, "instanceSide", |it, r, _| {
        let (c, _) = mirror_target(&r);
        Ok(new_mirror(it, c, false))
    });
    install_one(it, &mirror, false, "comment:", |it, r, a| {
        if let (Some(c), _) = mirror_target(&r) {
            *c.comment.borrow_mut() = string_of(&a[0]).unwrap_or_default();
            if it.record_changes {
                it.changes.push(format!("(Delta mirrorFor: #{}) comment: {} !\n", c.name(), quote(&c.comment.borrow())));
            }
        }
        Ok(r.clone())
    });
    install_one(it, &mirror, false, "comment", |_, r, _| match mirror_target(&r) {
        (Some(c), _) => Ok(Value::str(&c.comment.borrow())),
        _ => Ok(Value::Nil),
    });
    for sel in ["group:", "category:"] {
        install_one(it, &mirror, false, sel, |_, r, a| {
            if let (Some(c), _) = mirror_target(&r) {
                *c.group.borrow_mut() = string_of(&a[0]).unwrap_or_default();
            }
            Ok(r.clone())
        });
    }
    install_one(it, &mirror, false, "revision:", |_, r, _| Ok(r.clone()));
    install_one(it, &mirror, false, "reflectee", |_, r, _| match mirror_target(&r) {
        (Some(c), false) => Ok(Value::Class(c)),
        (Some(c), true) => Ok(Value::Meta(c)),
        _ => Ok(Value::Nil),
    });

    // ---- reflection on classes (instance side of Behavior: classes and metaclasses)
    install_one(it, &beh, false, "name", |_, r, _| Ok(match side(&r) {
        Some((c, false)) => Value::str(&c.name()),
        Some((c, true)) => Value::str(&format!("{} class", c.name())),
        None => Value::Nil,
    }));
    install_one(it, &beh, false, "superclass", |it, r, _| Ok(match side(&r) {
        Some((c, false)) => c.superclass().map(Value::Class).unwrap_or(Value::Nil),
        Some((c, true)) => c.superclass().map(Value::Meta).unwrap_or(Value::Class(it.core.class.clone())),
        None => Value::Nil,
    }));
    install_one(it, &beh, false, "subclasses", |it, r, _| {
        let (c, meta) = match side(&r) {
            Some(x) => x,
            None => return Ok(it.array(vec![])),
        };
        let mut subs: Vec<Rc<Class>> = it
            .globals
            .values()
            .filter_map(|v| match v {
                Value::Class(k) if k.superclass().map(|s| Rc::ptr_eq(&s, &c)).unwrap_or(false) => Some(k.clone()),
                _ => None,
            })
            .collect();
        subs.sort_by_key(|k| k.name());
        let v = subs.into_iter().map(|k| if meta { Value::Meta(k) } else { Value::Class(k) }).collect();
        Ok(it.array(v))
    });
    install_one(it, &beh, false, "selectors", |it, r, _| {
        let (c, meta) = match side(&r) {
            Some(x) => x,
            None => return Ok(it.array(vec![])),
        };
        let mut ks: Vec<Sym> = c.method_dict(meta).borrow().keys().cloned().collect();
        ks.sort();
        let v = ks.into_iter().map(Value::Sym).collect();
        Ok(it.array(v))
    });
    install_one(it, &beh, false, "includesSelector:", |_, r, a| {
        let s = string_of(&a[0]).unwrap_or_default();
        Ok(Value::Bool(match side(&r) {
            Some((c, meta)) => c.method_dict(meta).borrow().contains_key(s.as_str()),
            None => false,
        }))
    });
    install_one(it, &beh, false, "canUnderstand:", |it, r, a| {
        let s = string_of(&a[0]).unwrap_or_default();
        let s = it.intern(&s);
        Ok(Value::Bool(match side(&r) {
            Some((c, meta)) => it.lookup_from(Some(c), meta, &s).is_some(),
            None => false,
        }))
    });
    install_one(it, &beh, false, "sourceCodeAt:", |_, r, a| {
        let s = string_of(&a[0]).unwrap_or_default();
        Ok(match side(&r) {
            Some((c, meta)) => c.method_dict(meta).borrow().get(s.as_str()).map(|m| Value::str(&m.source)).unwrap_or(Value::Nil),
            None => Value::Nil,
        })
    });
    install_one(it, &beh, false, "removeSelector:", |it, r, a| {
        let s = string_of(&a[0]).unwrap_or_default();
        if let Some((c, meta)) = side(&r) {
            c.method_dict(meta).borrow_mut().remove(s.as_str());
            it.flush_cache();
            if it.record_changes {
                let target = if meta { format!("{} class", c.name()) } else { c.name() };
                it.changes.push(format!("{} removeSelector: #{} !\n", target, s));
            }
        }
        Ok(r.clone())
    });
    install_one(it, &beh, false, "instanceVariableNames", |it, r, _| {
        let v = match side(&r) {
            Some((c, false)) => c.ivars.borrow().iter().map(|v| Value::str(&v.name)).collect(),
            Some((c, true)) => c.class_ivars.borrow().iter().map(|v| Value::str(&v.name)).collect(),
            None => vec![],
        };
        Ok(it.array(v))
    });
    install_one(it, &beh, false, "allInstVarNames", |it, r, _| {
        let v = match side(&r) {
            Some((c, false)) => c.all_ivars().iter().map(|v| Value::str(&v.name)).collect(),
            Some((c, true)) => c.all_class_ivars().iter().map(|v| Value::str(&v.name)).collect(),
            None => vec![],
        };
        Ok(it.array(v))
    });
    install_one(it, &beh, false, "classVariableNames", |it, r, _| {
        let v = match side(&r) {
            Some((c, _)) => c.class_vars.borrow().iter().map(|(n, _)| Value::str(n)).collect(),
            None => vec![],
        };
        Ok(it.array(v))
    });
    install_one(it, &beh, false, "comment", |_, r, _| Ok(match side(&r) {
        Some((c, _)) => Value::str(&c.comment.borrow()),
        None => Value::Nil,
    }));
    install_one(it, &beh, false, "comment:", |it, r, a| {
        if let Some((c, _)) = side(&r) {
            *c.comment.borrow_mut() = string_of(&a[0]).unwrap_or_default();
            if it.record_changes {
                it.changes.push(format!("(Delta mirrorFor: #{}) comment: {} !\n", c.name(), quote(&c.comment.borrow())));
            }
        }
        Ok(r.clone())
    });
    install_one(it, &beh, false, "category", |_, r, _| Ok(match side(&r) {
        Some((c, _)) => Value::str(&c.group.borrow()),
        None => Value::Nil,
    }));
    install_one(it, &beh, false, "isMeta", |_, r, _| Ok(Value::Bool(matches!(r, Value::Meta(_)))));
    install_one(it, &beh, false, "instanceClass", |_, r, _| Ok(match r {
        Value::Meta(c) => Value::Class(c.clone()),
        other => other.clone(),
    }));
    install_one(it, &beh, false, "theNonMetaClass", |_, r, _| Ok(match r {
        Value::Meta(c) => Value::Class(c.clone()),
        other => other.clone(),
    }));
    install_one(it, &beh, false, "inheritsFrom:", |_, r, a| Ok(Value::Bool(match (side(&r), side(&a[0])) {
        (Some((c, m1)), Some((d, m2))) => m1 == m2 && !Rc::ptr_eq(&c, &d) && c.inherits_from(&d),
        _ => false,
    })));
    install_one(it, &beh, false, "includesBehavior:", |_, r, a| Ok(Value::Bool(match (side(&r), side(&a[0])) {
        (Some((c, m1)), Some((d, m2))) => m1 == m2 && c.inherits_from(&d),
        (Some((_, true)), None) => false,
        _ => false,
    })));
    install_one(it, &beh, false, "compile:", |it, r, a| compile_native(it, r, &a[0], ""));
    install_one(it, &beh, false, "compile:classified:", |it, r, a| {
        let cat = string_of(&a[1]).unwrap_or_default();
        compile_native(it, r, &a[0], &cat)
    });
    install_one(it, &beh, false, "methodsFor:", |it, r, a| Ok(match side(&r) {
        Some((c, m)) => method_reader(it, Some(c), m, a[0].clone()),
        None => Value::Nil,
    }));
    install_one(it, &beh, false, "definition", |_, r, _| Ok(match side(&r) {
        Some((c, _)) => Value::str(&definition(&c)),
        None => Value::Nil,
    }));
    install_one(it, &beh, false, "fileOutString", |_, r, _| Ok(match side(&r) {
        Some((c, _)) => Value::str(&file_out(&c)),
        None => Value::Nil,
    }));
    for sel in [
        "subclass:",
        "subclass:instanceVariableNames:",
        "subclass:instanceVariableNames:classVariableNames:category:",
        "subclass:instanceVariableNames:classVariableNames:package:",
        "subclass:instanceVariableNames:classVariableNames:poolDictionaries:category:",
    ] {
        install_one(it, &beh, false, sel, |it, r, a| {
            let sup = match side(&r) {
                Some((c, false)) => c,
                _ => return it.signal_error("Error", "subclass: must be sent to a class"),
            };
            let name = string_of(&a[0]).unwrap_or_default();
            let ivars = a.get(1).and_then(string_of).unwrap_or_default();
            let cvars = a.get(2).and_then(string_of).unwrap_or_default();
            let cat = a.last().filter(|_| a.len() >= 4).and_then(string_of).unwrap_or_else(|| "user".into());
            define_class(it, &name, &sup.name(), &ivars, &cvars, "", "", &cat)
        });
    }
}

fn quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "''").replace('!', "!!"))
}

fn side<V: std::borrow::Borrow<Value>>(v: V) -> Option<(Rc<Class>, bool)> {
    match v.borrow() {
        Value::Class(c) => Some((c.clone(), false)),
        Value::Meta(c) => Some((c.clone(), true)),
        _ => None,
    }
}

fn mirror_target<V: std::borrow::Borrow<Value>>(r: V) -> (Option<Rc<Class>>, bool) {
    if let Value::Obj(o) = r.borrow() {
        let c = match o.get(0) {
            Value::Class(c) => Some(c),
            _ => None,
        };
        return (c, matches!(o.get(1), Value::Bool(true)));
    }
    (None, false)
}

fn new_mirror(it: &Interp, c: Option<Rc<Class>>, meta: bool) -> Value {
    let m = it.new_object(&it.core.mirror.clone());
    if let Value::Obj(o) = &m {
        o.set(0, c.map(Value::Class).unwrap_or(Value::Nil));
        o.set(1, Value::Bool(meta));
    }
    m
}

fn method_reader(it: &Interp, c: Option<Rc<Class>>, meta: bool, cat: Value) -> Value {
    let m = it.new_object(&it.core.method_reader.clone());
    if let Value::Obj(o) = &m {
        o.set(0, c.map(Value::Class).unwrap_or(Value::Nil));
        o.set(1, Value::Bool(meta));
        o.set(2, match cat {
            Value::Sym(s) => Value::str(&s),
            other => other,
        });
    }
    m
}

fn compile_native(it: &mut Interp, r: Value, src: &Value, cat: &str) -> R {
    let (c, meta) = match side(&r) {
        Some(x) => x,
        None => return Ok(Value::Nil),
    };
    let src = string_of(src).unwrap_or_default();
    match it.compile_method(&c, meta, cat, &src) {
        Ok(m) => Ok(Value::Sym(m.selector.clone())),
        Err((e, _)) => it.signal_error("Error", &format!("compile error: {}", e)),
    }
}

fn sys_mirror_for(it: &mut Interp, _: Value, a: Vec<Value>) -> R {
    let name = string_of(&a[0]).unwrap_or_default();
    match it.class_named(&name) {
        Some(c) => Ok(new_mirror(it, Some(c), false)),
        None => it.signal_error("Error", &format!("no class named {}", name)),
    }
}

fn sys_define(it: &mut Interp, _: Value, a: Vec<Value>) -> R {
    let name = string_of(&a[0]).unwrap_or_default();
    let d = match &a[1] {
        Value::Obj(o) if Rc::ptr_eq(&o.class.borrow(), &it.core.class_decl) => o.clone(),
        _ => return it.signal_error("Error", "define:as: expects a class declaration"),
    };
    let f = |i: usize| string_of(&d.get(i)).unwrap_or_default();
    define_class(it, &name, &f(0), &f(1), &f(2), &f(3), &f(4), "")
}

/// Creates or updates a class from its textual declaration.
#[allow(clippy::too_many_arguments)]
pub fn define_class(
    it: &mut Interp,
    name: &str,
    super_text: &str,
    ivars: &str,
    cvars: &str,
    civars: &str,
    generics: &str,
    group: &str,
) -> R {
    if name.is_empty() || !name.chars().next().unwrap().is_uppercase() {
        return it.signal_error("Error", &format!("class names must start with a capital letter: '{}'", name));
    }
    let super_text = super_text.trim();
    let base = super_text.split('[').next().unwrap_or("").trim().to_string();
    let sup = if base == "nil" || base.is_empty() {
        None
    } else {
        match it.class_named(&base) {
            Some(s) => Some(s),
            None => return it.signal_error("Error", &format!("superclass {} is not defined", base)),
        }
    };
    let ivar_decls = match parse_var_decls(ivars) {
        Ok(v) => v,
        Err(e) => return it.signal_error("Error", &format!("instance variables: {}", e.msg)),
    };
    let cvar_decls = parse_var_decls(cvars).unwrap_or_default();
    let civar_decls = parse_var_decls(civars).unwrap_or_default();
    let gen_decls = parse_var_decls(generics).unwrap_or_default();
    let existing = it.class_named(name);
    let cls = match existing {
        Some(c) => {
            if let Some(s) = &sup {
                if s.inherits_from(&c) {
                    return it.signal_error("Error", "a class cannot inherit from itself");
                }
            }
            let old_names: Vec<String> = c.all_ivars().into_iter().map(|v| v.name).collect();
            let old_cnames: Vec<String> = c.all_class_ivars().into_iter().map(|v| v.name).collect();
            let old_super = c.superclass();
            *c.superclass.borrow_mut() = sup.clone();
            *c.ivars.borrow_mut() = ivar_decls;
            *c.class_ivars.borrow_mut() = civar_decls;
            let changed = old_names != c.all_ivars().into_iter().map(|v| v.name).collect::<Vec<_>>()
                || old_cnames != c.all_class_ivars().into_iter().map(|v| v.name).collect::<Vec<_>>()
                || match (&old_super, &sup) {
                    (Some(a), Some(b)) => !Rc::ptr_eq(a, b),
                    (None, None) => false,
                    _ => true,
                };
            c.class_vars.borrow_mut().clear();
            c.class_vars.borrow_mut().extend(cvar_decls.into_iter().map(|v| (v.name, v.ty)));
            if c.format.get() == Format::Plain {
                if let Some(s) = &sup {
                    if s.format.get() == Format::Indexed {
                        c.format.set(Format::Indexed);
                    }
                }
            }
            it.flush_cache();
            if changed {
                for e in it.recompile_tree(&c) {
                    it.transcript_write(&format!("recompile: {}\n", e));
                }
            }
            c
        }
        None => {
            let fmt = match &sup {
                Some(s) if s.format.get() == Format::Indexed => Format::Indexed,
                _ => Format::Plain,
            };
            let c = Class::new(name, sup.clone(), fmt);
            *c.ivars.borrow_mut() = ivar_decls;
            *c.class_ivars.borrow_mut() = civar_decls;
            c.class_vars.borrow_mut().extend(cvar_decls.into_iter().map(|v| (v.name, v.ty)));
            it.globals.insert(name.to_string(), Value::Class(c.clone()));
            c
        }
    };
    *cls.super_text.borrow_mut() = if super_text.is_empty() { "nil".into() } else { super_text.to_string() };
    *cls.generics.borrow_mut() = gen_decls;
    if !group.is_empty() {
        *cls.group.borrow_mut() = group.to_string();
    } else if cls.group.borrow().is_empty() {
        *cls.group.borrow_mut() = "user".into();
    }
    if it.record_changes {
        it.changes.push(format!("{} !\n(Delta mirrorFor: #{}) group: {} !\n", definition(&cls), name, quote(&cls.group.borrow())));
    }
    Ok(Value::Class(cls))
}

fn decl_list(vs: &[crate::ast::Var]) -> String {
    vs.iter()
        .map(|v| match &v.ty {
            Some(t) => format!("{} <{}>", v.name, t),
            None => v.name.clone(),
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// The class definition in Strongtalk's file-out form.
pub fn definition(c: &Rc<Class>) -> String {
    let ivars = decl_list(&c.ivars.borrow());
    let mut s = format!("(Class subclassOf: '{}' instanceVariables: '{}')", c.super_text.borrow(), ivars);
    let cvars: Vec<String> = c.class_vars.borrow().iter().map(|(n, _)| n.clone()).collect();
    let civars = decl_list(&c.class_ivars.borrow());
    let mut extras = Vec::new();
    if !cvars.is_empty() {
        extras.push(format!("classVariables: '{}'", cvars.join(" ")));
    }
    if !civars.is_empty() {
        extras.push(format!("classInstanceVariables: '{}'", civars));
    }
    if !extras.is_empty() {
        s = format!("{}\n\t{}", s, extras.join(";\n\t"));
    }
    let gens = decl_list(&c.generics.borrow());
    if !gens.is_empty() {
        s = format!("Generic forAll: '{}' body: ({})", gens, s);
    }
    format!("Delta define: #{} as: (\n{})", c.name(), s)
}

/// A whole class in chunk format, readable by file-in.
pub fn file_out(c: &Rc<Class>) -> String {
    let mut out = String::new();
    out.push_str(&definition(c));
    out.push_str(" !\n\n");
    out.push_str(&format!("(Delta mirrorFor: #{}) group: {} !\n\n", c.name(), quote(&c.group.borrow())));
    if !c.comment.borrow().is_empty() {
        out.push_str(&format!("(Delta mirrorFor: #{}) comment: {} !\n\n", c.name(), quote(&c.comment.borrow())));
    }
    for meta in [true, false] {
        let mut ms: Vec<Rc<Method>> = c
            .method_dict(meta)
            .borrow()
            .values()
            .filter(|m| matches!(m.body, MethodBody::Code(_)))
            .cloned()
            .collect();
        ms.sort_by(|a, b| (a.category.clone(), a.selector.to_string()).cmp(&(b.category.clone(), b.selector.to_string())));
        let mut cat: Option<String> = None;
        for m in ms {
            if cat.as_deref() != Some(&m.category) {
                if cat.is_some() {
                    out.push_str("! !\n\n");
                }
                out.push_str(&format!(
                    "! (Delta mirrorFor: #{}){} methodsFor: {} !\n\n",
                    c.name(),
                    if meta { " classSide" } else { "" },
                    quote(&m.category)
                ));
                cat = Some(m.category.clone());
            }
            out.push_str(&m.source.replace('!', "!!"));
            out.push_str("\n!\n\n");
        }
        if cat.is_some() {
            out.push_str("! !\n\n");
        }
    }
    out
}
