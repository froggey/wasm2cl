//! wasm2cl library crate: transpiles wasm binaries to Common Lisp.

pub mod emit;
pub mod expr;
pub mod expressionify;
pub mod module;

pub fn symbolicate(s: &str) -> String {
    format!("|{s}|")
}
