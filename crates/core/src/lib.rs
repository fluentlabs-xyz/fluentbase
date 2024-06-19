#![no_std]
#![allow(dead_code)]

extern crate alloc;
extern crate core;
extern crate fluentbase_sdk;
extern crate paste;

pub use fluentbase_types::ExitCode;
#[cfg(feature = "revm-rwasm")]
extern crate revm_interpreter_fluent as revm_interpreter;

pub mod evm;

pub mod fluent_host;
pub mod helpers;
pub mod loader;
pub mod svm;
pub mod wasm;
