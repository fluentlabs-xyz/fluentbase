#![allow(dead_code, unreachable_patterns, unused_macros)]
#![warn(unused_crate_dependencies)]
extern crate core;

mod context;
#[cfg(feature = "std")]
mod context_wrapper;
mod executor;
mod factory;
mod global_executor;
#[cfg(all(feature = "wasmtime", feature = "inter-process-lock"))]
mod inter_process_lock;
mod local_executor;
mod runtime;
pub mod syscall_handler;
#[cfg(test)]
mod tests;
#[cfg(feature = "wasmtime")]
mod wasmtime;

pub use context::*;
#[cfg(feature = "std")]
pub use context_wrapper::*;
pub use executor::{default_runtime_executor, RuntimeExecutor};
pub use runtime::*;
