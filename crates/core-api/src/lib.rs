#![cfg_attr(not(test), no_std)]
#![allow(dead_code)]

extern crate alloc;
extern crate core;
extern crate fluentbase_sdk;

pub use fluentbase_types::ExitCode;

pub mod api;
pub mod bindings;
#[cfg(test)]
mod tests;
