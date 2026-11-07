//! Strongtalk: a typed Smalltalk environment, re-implemented in Rust.
//! The same engine runs as a command-line program and, compiled to WebAssembly, in the browser.

pub mod api;
pub mod ast;
pub mod codegen;
pub mod compiler;
pub mod interp;
pub mod lexer;
pub mod natives;
pub mod object;
pub mod parser;
pub mod typescript;
pub mod prims;
pub mod typecheck;
pub mod vm;
#[cfg(target_arch = "wasm32")]
pub mod wasm;

pub use api::Vm;
