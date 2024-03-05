#![no_std]
#![allow(dead_code)]

extern crate alloc;
extern crate core;
extern crate paste;

pub use crate::api::{CoreInput, CreateMethodInput};
use fluentbase_codec::Encoder;
use fluentbase_sdk::LowLevelAPI;
pub use fluentbase_types::ExitCode;

pub mod account;
pub mod account_types;
pub mod api;
mod entry;
pub mod evm;
pub mod fluent_host;
#[cfg(test)]
mod testing_utils;
mod utils;
