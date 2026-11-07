//! The browser binding: a `Strongtalk` object the web page drives (from a Web Worker).

use crate::api::Vm;
use wasm_bindgen::prelude::*;

/// Translates Strongtalk (a file-in or a do-it) to TypeScript. Answers JSON:
/// `{ok, code, warnings, error, line, column, classes}`.
#[wasm_bindgen(js_name = translateToTypeScript)]
pub fn translate_to_typescript(src: &str) -> String {
    crate::typescript::translate(src).to_json()
}

#[wasm_bindgen]
pub struct Strongtalk {
    vm: Vm,
}

#[wasm_bindgen]
impl Strongtalk {
    #[wasm_bindgen(constructor)]
    pub fn new() -> Strongtalk {
        let mut vm = Vm::new();
        vm.it.max_run_depth = 150;
        Strongtalk { vm }
    }

    /// Transcript output is passed to `f` as it is written, so long runs show progress.
    #[wasm_bindgen(js_name = setTranscript)]
    pub fn set_transcript(&mut self, f: js_sys::Function) {
        self.vm.it.transcript_sink = Some(Box::new(move |s: &str| {
            let _ = f.call1(&JsValue::NULL, &JsValue::from_str(s));
        }));
    }

    /// Smalltalk calls nest on the heap, but exception handlers, ensure: blocks and callbacks
    /// from primitives nest on the browser's small machine stack: this caps how deep those go.
    #[wasm_bindgen(js_name = setMaxNesting)]
    pub fn set_max_nesting(&mut self, n: usize) {
        self.vm.it.max_run_depth = n.max(40);
    }

    #[wasm_bindgen(js_name = bootErrors)]
    pub fn boot_errors(&self) -> String {
        self.vm.boot_errors()
    }
    pub fn eval(&mut self, src: &str, workspace: &str, print: bool) -> String {
        self.vm.eval(src, workspace, print)
    }
    #[wasm_bindgen(js_name = fileIn)]
    pub fn file_in(&mut self, text: &str) -> String {
        self.vm.file_in(text)
    }
    pub fn classes(&self) -> String {
        self.vm.classes()
    }
    #[wasm_bindgen(js_name = classInfo)]
    pub fn class_info(&self, name: &str) -> String {
        self.vm.class_info(name)
    }
    pub fn method(&self, class: &str, meta: bool, sel: &str) -> String {
        self.vm.method(class, meta, sel)
    }
    pub fn compile(&mut self, class: &str, meta: bool, category: &str, source: &str) -> String {
        self.vm.compile(class, meta, category, source)
    }
    #[wasm_bindgen(js_name = removeMethod)]
    pub fn remove_method(&mut self, class: &str, meta: bool, sel: &str) -> String {
        self.vm.remove_method(class, meta, sel)
    }
    #[wasm_bindgen(js_name = setComment)]
    pub fn set_comment(&mut self, class: &str, comment: &str) -> String {
        self.vm.set_comment(class, comment)
    }
    #[wasm_bindgen(js_name = typecheckClass)]
    pub fn typecheck_class(&mut self, class: &str) -> String {
        self.vm.typecheck_class(class)
    }
    #[wasm_bindgen(js_name = typecheckAll)]
    pub fn typecheck_all(&mut self) -> String {
        self.vm.typecheck_all()
    }
    #[wasm_bindgen(js_name = typecheckDoit)]
    pub fn typecheck_doit(&mut self, src: &str) -> String {
        self.vm.typecheck_doit(src)
    }
    pub fn senders(&self, sel: &str) -> String {
        self.vm.senders(sel)
    }
    pub fn implementors(&self, sel: &str) -> String {
        self.vm.implementors(sel)
    }
    #[wasm_bindgen(js_name = fileOut)]
    pub fn file_out(&self, class: &str) -> String {
        self.vm.file_out(class)
    }
    #[wasm_bindgen(js_name = takeChanges)]
    pub fn take_changes(&mut self) -> String {
        self.vm.take_changes()
    }
    pub fn inspect(&mut self, id: usize) -> String {
        self.vm.inspect(id)
    }
    #[wasm_bindgen(js_name = canvasPixels)]
    pub fn canvas_pixels(&self, id: usize) -> Vec<u8> {
        self.vm.canvas_pixels(id)
    }
    #[wasm_bindgen(js_name = setRecordChanges)]
    pub fn set_record_changes(&mut self, on: bool) {
        self.vm.it.record_changes = on;
    }
}

impl Default for Strongtalk {
    fn default() -> Self {
        Strongtalk::new()
    }
}
