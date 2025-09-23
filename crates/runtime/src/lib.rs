#![allow(dead_code, unreachable_patterns, unused_macros)]
#![warn(unused_crate_dependencies)]
extern crate core;

mod context;
#[cfg(feature = "std")]
mod context_wrapper;
mod factory;
#[cfg(all(feature = "wasmtime", feature = "inter-process-lock"))]
mod inter_process_lock;
mod runtime;
pub mod syscall_handler;
#[cfg(test)]
mod tests;

pub use context::*;
#[cfg(feature = "std")]
pub use context_wrapper::*;
pub use runtime::*;
