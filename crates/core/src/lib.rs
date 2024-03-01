#![no_std]
#![allow(dead_code)]

extern crate alloc;
extern crate core;

pub use fluentbase_types::ExitCode;

pub mod account;
pub mod account_types;
pub mod evm;
pub mod fluent_host;
#[cfg(test)]
mod testing_utils;
mod utils;
