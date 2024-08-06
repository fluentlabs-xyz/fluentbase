#![no_std]
#![allow(dead_code)]

extern crate alloc;
extern crate core;
extern crate fluentbase_sdk;
extern crate paste;

pub use fluentbase_types::ExitCode;
pub mod evm;

pub mod evm2;
pub mod fluent_host;
pub mod helpers;
pub mod loader;
pub mod svm;
pub mod wasm;
