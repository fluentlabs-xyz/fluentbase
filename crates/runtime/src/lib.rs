#![allow(dead_code, unreachable_patterns, unused_macros)]
#![warn(unused_crate_dependencies)]
extern crate core;

pub mod instruction;

mod context;
mod runtime;
mod storage;
#[cfg(test)]
mod tests;
mod utils;
mod wasmtime;

pub use context::*;
pub use runtime::*;
pub use storage::*;
