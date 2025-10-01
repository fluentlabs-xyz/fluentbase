#![allow(dead_code, unreachable_patterns, unused_macros)]
#![warn(unused_crate_dependencies)]
extern crate core;

mod context;
#[cfg(feature = "std")]
mod context_wrapper;
mod executor;
#[cfg(feature = "global-executor")]
mod global_executor;
#[cfg(all(feature = "wasmtime", feature = "inter-process-lock"))]
mod inter_process_lock;
mod math;
mod module_factory;
mod runtime;
pub mod syscall_handler;
mod utils;

pub use context::*;
#[cfg(feature = "std")]
pub use context_wrapper::*;
pub use executor::{default_runtime_executor, RuntimeExecutor};
