#![no_std]
#![allow(dead_code)]

extern crate alloc;
extern crate core;
extern crate fluentbase_sdk;
extern crate paste;

pub use fluentbase_types::ExitCode;
pub mod evm;

pub mod fluent_host;
pub mod fvm;
pub mod helpers;
pub mod helpers_fvm;
pub mod loader;
pub mod svm;
pub mod wasm;

pub use fluentbase_types::consts::DEVNET_CHAIN_ID;
