#![allow(dead_code, unreachable_patterns, unused_macros)]
#![warn(unused_crate_dependencies)]

pub mod instruction;
mod macros;
mod runtime;
mod storage;
pub mod types;

pub use runtime::*;
pub use storage::*;

#[cfg(test)]
mod tests;
