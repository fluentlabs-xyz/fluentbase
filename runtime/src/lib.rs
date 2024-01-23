#![allow(dead_code, unreachable_patterns, unused_macros)]
#![cfg_attr(not(test), warn(unused_crate_dependencies))]

pub mod instruction;
mod macros;
mod runtime;
pub use runtime::*;
mod storage;
#[cfg(test)]
mod tests;
pub mod types;
