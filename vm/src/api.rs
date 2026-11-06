//! The environment's programming interface, shared by the command line and the web page.
//! Everything returns JSON text so the browser side stays simple.

use crate::interp::{method_label, Event, Interp};
use crate::natives::{definition, file_out};
use crate::object::*;
use crate::typecheck::{self, Problem};
use serde_json::{json, Value as J};
use std::rc::Rc;

pub const LIBRARY: &[(&str, &str)] = &[
    ("kernel.dlt", include_str!("../lib/kernel.dlt")),
    ("numbers.dlt", include_str!("../lib/numbers.dlt")),
    ("collections.dlt", include_str!("../lib/collections.dlt")),
    ("strings.dlt", include_str!("../lib/strings.dlt")),
    ("exceptions.dlt", include_str!("../lib/exceptions.dlt")),
    ("system.dlt", include_str!("../lib/system.dlt")),
    ("demos.dlt", include_str!("../lib/demos.dlt")),
];

pub struct Vm {
    pub it: Interp,
    pub boot_errors: Vec<String>,
}

fn problems_json(ps: &[Problem]) -> J {
    J::Array(
        ps.iter()
            .map(|p| json!({"class": p.class, "meta": p.meta, "selector": p.selector, "line": p.line, "message": p.message}))
            .collect(),
    )
}

impl Default for Vm {
    fn default() -> Self {
        Vm::new()
    }
}

impl Vm {
    pub fn new() -> Vm {
        let mut it = Interp::new();
        let mut boot_errors = Vec::new();
        for (name, src) in LIBRARY {
            let rep = it.file_in(src);
            for e in rep.errors {
                boot_errors.push(format!("{}: {}", name, e));
            }
        }
        it.transcript_buf.clear();
        it.events.clear();
        it.record_changes = true;
        Vm { it, boot_errors }
    }

    pub fn boot_errors(&self) -> String {
        json!(self.boot_errors).to_string()
    }

    pub fn take_transcript(&mut self) -> String {
        std::mem::take(&mut self.it.transcript_buf)
    }

    fn events_json(&mut self) -> J {
        let evs = std::mem::take(&mut self.it.events);
        J::Array(
            evs.into_iter()
                .map(|e| match e {
                    Event::Inspect(id) => json!({"type": "inspect", "id": id}),
                    Event::Canvas(id) => {
                        let c = &self.it.canvases[id];
                        json!({"type": "canvas", "id": id, "width": c.width, "height": c.height, "title": c.title})
                    }
                    Event::ClearTranscript => json!({"type": "clear"}),
                    Event::Browse(n) => json!({"type": "browse", "name": n}),
                })
                .collect(),
        )
    }

    /// Runs a do-it. With `print`, the result's printString comes back as `value`.
    pub fn eval(&mut self, src: &str, workspace: &str, print: bool) -> String {
        let t0 = Interp::now_ms();
        let ws = if workspace.is_empty() { None } else { Some(workspace) };
        let r = self.it.evaluate(src, ws, Value::Nil);
        let ms = Interp::now_ms() - t0;
        let out = match r {
            Ok(v) => {
                let value = if print { self.it.print_string(&v) } else { String::new() };
                let (c, m) = self.it.behavior_of(&v);
                let cname = if m { format!("{} class", c.name()) } else { c.name() };
                self.it.inspected.push(v);
                let id = self.it.inspected.len() - 1;
                json!({"ok": true, "value": value, "class": cname, "id": id, "ms": ms})
            }
            Err(e) => {
                let pos = e.stack.first().and_then(|s| s.strip_prefix('@')).and_then(|p| p.parse::<usize>().ok());
                let stack: Vec<String> = e.stack.into_iter().filter(|s| !s.starts_with('@')).collect();
                json!({"ok": false, "error": e.message, "errorClass": e.class_name, "stack": stack, "pos": pos, "ms": ms})
            }
        };
        let mut o = out;
        o["events"] = self.events_json();
        o["transcript"] = J::String(self.take_transcript());
        o.to_string()
    }

    pub fn file_in(&mut self, text: &str) -> String {
        let rep = self.it.file_in(text);
        let mut o = json!({"ok": rep.errors.is_empty(), "methods": rep.methods, "doits": rep.doits, "errors": rep.errors});
        o["events"] = self.events_json();
        o["transcript"] = J::String(self.take_transcript());
        o.to_string()
    }

    fn classes_sorted(&self) -> Vec<Rc<Class>> {
        let mut v: Vec<Rc<Class>> = self
            .it
            .globals
            .iter()
            .filter_map(|(k, v)| match v {
                Value::Class(c) if c.name() == *k => Some(c.clone()),
                _ => None,
            })
            .collect();
        v.sort_by_key(|c| c.name());
        v
    }

    pub fn classes(&self) -> String {
        J::Array(
            self.classes_sorted()
                .iter()
                .map(|c| {
                    json!({
                        "name": c.name(),
                        "superclass": c.superclass().map(|s| s.name()),
                        "group": c.group.borrow().clone(),
                        "methods": c.methods.borrow().len() + c.class_methods.borrow().len(),
                        "generic": !c.generics.borrow().is_empty(),
                    })
                })
                .collect(),
        )
        .to_string()
    }

    fn methods_json(c: &Rc<Class>, meta: bool) -> J {
        let mut ms: Vec<Rc<Method>> = c.method_dict(meta).borrow().values().cloned().collect();
        ms.sort_by(|a, b| a.selector.cmp(&b.selector));
        J::Array(
            ms.iter()
                .map(|m| {
                    json!({
                        "selector": &*m.selector,
                        "category": m.category,
                        "signature": m.signature,
                        "native": matches!(m.body, MethodBody::Native(_)),
                    })
                })
                .collect(),
        )
    }

    pub fn class_info(&self, name: &str) -> String {
        let c = match self.it.class_named(name) {
            Some(c) => c,
            None => return json!({"ok": false, "error": format!("no class named {}", name)}).to_string(),
        };
        let decl = |vs: &[crate::ast::Var]| -> J {
            J::Array(vs.iter().map(|v| json!({"name": v.name, "type": v.ty.as_ref().map(|t| t.to_string())})).collect())
        };
        let subclasses: Vec<String> = self
            .classes_sorted()
            .iter()
            .filter(|k| k.superclass().map(|s| Rc::ptr_eq(&s, &c)).unwrap_or(false))
            .map(|k| k.name())
            .collect();
        let mut chain = Vec::new();
        let mut s = c.superclass();
        while let Some(k) = s {
            chain.push(k.name());
            s = k.superclass();
        }
        json!({
            "ok": true,
            "name": c.name(),
            "superclass": c.superclass().map(|s| s.name()),
            "superText": c.super_text.borrow().clone(),
            "superclasses": chain,
            "subclasses": subclasses,
            "generics": decl(&c.generics.borrow()),
            "ivars": decl(&c.ivars.borrow()),
            "classIvars": decl(&c.class_ivars.borrow()),
            "classVars": c.class_vars.borrow().iter().map(|(n, _)| n.clone()).collect::<Vec<_>>(),
            "comment": c.comment.borrow().clone(),
            "group": c.group.borrow().clone(),
            "definition": definition(&c),
            "instanceMethods": Self::methods_json(&c, false),
            "classMethods": Self::methods_json(&c, true),
        })
        .to_string()
    }

    pub fn method(&self, class: &str, meta: bool, sel: &str) -> String {
        let c = match self.it.class_named(class) {
            Some(c) => c,
            None => return json!({"ok": false}).to_string(),
        };
        let m = c.method_dict(meta).borrow().get(sel).cloned();
        match m {
            Some(m) => json!({"ok": true, "source": m.source, "category": m.category, "signature": m.signature,
                              "native": matches!(m.body, MethodBody::Native(_))})
            .to_string(),
            None => json!({"ok": false}).to_string(),
        }
    }

    pub fn compile(&mut self, class: &str, meta: bool, category: &str, source: &str) -> String {
        let c = match self.it.class_named(class) {
            Some(c) => c,
            None => return json!({"ok": false, "error": format!("no class named {}", class)}).to_string(),
        };
        match self.it.compile_method(&c, meta, category, source) {
            Ok(m) => {
                let problems = typecheck::check_method(&mut self.it, &m);
                json!({"ok": true, "selector": &*m.selector, "category": m.category, "problems": problems_json(&problems)})
                    .to_string()
            }
            Err((e, pos)) => json!({"ok": false, "error": e, "pos": pos}).to_string(),
        }
    }

    pub fn remove_method(&mut self, class: &str, meta: bool, sel: &str) -> String {
        let target = if meta { format!("{} class", class) } else { class.to_string() };
        self.eval(&format!("{} removeSelector: #'{}'", target, sel.replace('\'', "''")), "", false)
    }

    pub fn set_comment(&mut self, class: &str, comment: &str) -> String {
        self.eval(&format!("(Delta mirrorFor: #{}) comment: '{}'", class, comment.replace('\'', "''")), "", false)
    }

    pub fn typecheck_class(&mut self, class: &str) -> String {
        match self.it.class_named(class) {
            Some(c) => {
                let ps = typecheck::check_class(&mut self.it, &c);
                json!({"ok": true, "problems": problems_json(&ps), "classes": 1}).to_string()
            }
            None => json!({"ok": false, "error": "no such class"}).to_string(),
        }
    }

    pub fn typecheck_all(&mut self) -> String {
        let cs = self.classes_sorted();
        let mut all = Vec::new();
        for c in &cs {
            all.extend(typecheck::check_class(&mut self.it, c));
        }
        json!({"ok": true, "problems": problems_json(&all), "classes": cs.len()}).to_string()
    }

    pub fn typecheck_doit(&mut self, src: &str) -> String {
        match typecheck::check_doit(&mut self.it, src) {
            Ok(ps) => json!({"ok": true, "problems": problems_json(&ps)}).to_string(),
            Err(e) => json!({"ok": false, "error": e}).to_string(),
        }
    }

    fn all_methods(&self) -> Vec<Rc<Method>> {
        let mut v = Vec::new();
        for c in self.classes_sorted() {
            for meta in [false, true] {
                v.extend(c.method_dict(meta).borrow().values().cloned());
            }
        }
        v
    }

    pub fn implementors(&self, sel: &str) -> String {
        let mut v: Vec<J> = self
            .all_methods()
            .iter()
            .filter(|m| &*m.selector == sel)
            .map(|m| json!({"class": m.class.name(), "meta": m.meta, "selector": &*m.selector, "label": method_label(m)}))
            .collect();
        v.sort_by_key(|j| j["label"].as_str().unwrap_or("").to_string());
        J::Array(v).to_string()
    }

    pub fn senders(&self, sel: &str) -> String {
        let words: Vec<&str> = sel.split_inclusive(':').collect();
        let mut v: Vec<J> = self
            .all_methods()
            .iter()
            .filter(|m| matches!(m.body, MethodBody::Code(_)))
            .filter(|m| {
                let body = &m.source[m.source.find('\n').unwrap_or(0)..];
                if words.len() <= 1 {
                    contains_word(body, sel)
                } else {
                    words.iter().all(|w| body.contains(w))
                }
            })
            .map(|m| json!({"class": m.class.name(), "meta": m.meta, "selector": &*m.selector, "label": method_label(m)}))
            .collect();
        v.sort_by_key(|j| j["label"].as_str().unwrap_or("").to_string());
        J::Array(v).to_string()
    }

    pub fn file_out(&self, class: &str) -> String {
        match self.it.class_named(class) {
            Some(c) => file_out(&c),
            None => String::new(),
        }
    }

    pub fn take_changes(&mut self) -> String {
        std::mem::take(&mut self.it.changes).concat()
    }

    pub fn inspect(&mut self, id: usize) -> String {
        let v = match self.it.inspected.get(id) {
            Some(v) => v.clone(),
            None => return json!({"ok": false}).to_string(),
        };
        let title = self.it.print_string(&v);
        let (c, m) = self.it.behavior_of(&v);
        let cname = if m { format!("{} class", c.name()) } else { c.name() };
        let mut fields: Vec<(String, Value)> = Vec::new();
        match &v {
            Value::Obj(o) => {
                let cls = o.class.borrow().clone();
                for (i, iv) in cls.all_ivars().iter().enumerate() {
                    fields.push((iv.name.clone(), o.get(i)));
                }
                if let Some(ix) = &o.indexed {
                    for (i, x) in ix.borrow().iter().take(300).enumerate() {
                        fields.push((format!("{}", i + 1), x.clone()));
                    }
                }
            }
            Value::Class(k) => {
                if let Some(s) = k.superclass() {
                    fields.push(("superclass".into(), Value::Class(s)));
                }
                for (n, _) in k.class_vars.borrow().iter() {
                    fields.push((n.clone(), k.class_var_values.borrow().get(n).cloned().unwrap_or(Value::Nil)));
                }
                for iv in k.all_class_ivars() {
                    fields.push((iv.name.clone(), k.class_ivar_values.borrow().get(&iv.name).cloned().unwrap_or(Value::Nil)));
                }
            }
            Value::Str(s) => {
                fields.push(("size".into(), Value::Int(s.borrow().len() as i64)));
                for (i, ch) in s.borrow().iter().take(300).enumerate() {
                    fields.push((format!("{}", i + 1), Value::Char(*ch)));
                }
            }
            Value::Block(b) => fields.push(("numArgs".into(), Value::Int(b.code.nparams as i64))),
            _ => {}
        }
        let mut fj = Vec::new();
        for (n, x) in fields {
            let mut p = self.it.print_string(&x);
            if p.chars().count() > 200 {
                p = p.chars().take(200).collect::<String>() + "...";
            }
            self.it.inspected.push(x);
            fj.push(json!({"name": n, "value": p, "id": self.it.inspected.len() - 1}));
        }
        self.it.transcript_buf.clear();
        json!({"ok": true, "title": title, "class": cname, "fields": fj}).to_string()
    }

    pub fn canvas_pixels(&self, id: usize) -> Vec<u8> {
        match self.it.canvases.get(id) {
            Some(c) => {
                let mut out = Vec::with_capacity(c.pixels.len() * 4);
                for p in &c.pixels {
                    out.push((p >> 16) as u8);
                    out.push((p >> 8) as u8);
                    out.push(*p as u8);
                    out.push(255);
                }
                out
            }
            None => vec![],
        }
    }

    pub fn globals_list(&self) -> String {
        let mut ks: Vec<&String> = self.it.globals.keys().collect();
        ks.sort();
        json!(ks).to_string()
    }
}

fn contains_word(hay: &str, w: &str) -> bool {
    let mut start = 0;
    while let Some(i) = hay[start..].find(w) {
        let a = start + i;
        let b = a + w.len();
        let before = hay[..a].chars().last();
        let after = hay[b..].chars().next();
        let is_id = |c: Option<char>| c.map(|c| c.is_alphanumeric() || c == '_' || c == ':').unwrap_or(false);
        if !is_id(before) && !is_id(after) {
            return true;
        }
        start = b;
    }
    false
}
