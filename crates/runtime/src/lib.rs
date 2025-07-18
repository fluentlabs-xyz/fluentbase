#![allow(dead_code, unreachable_patterns, unused_macros)]
#![warn(unused_crate_dependencies)]
#![feature(file_lock)]
extern crate core;

pub mod instruction;

mod context;
#[cfg(feature = "std")]
mod context_wrapper;
mod inter_process_lock;
mod runtime;
mod storage;
#[cfg(test)]
mod tests;
mod utils;

pub use context::*;
#[cfg(feature = "std")]
pub use context_wrapper::*;
pub use runtime::*;
pub use storage::*;
