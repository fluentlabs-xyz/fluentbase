#![no_std]
#![allow(dead_code)]

extern crate alloc;
extern crate core;
extern crate fluentbase_sdk;
extern crate paste;

use fluentbase_codec::Encoder;
use fluentbase_sdk::LowLevelAPI;
pub use fluentbase_types::ExitCode;

pub mod account;
pub mod account_types;
mod contract_entry;
pub mod evm;
pub mod fluent_host;
mod utils;
