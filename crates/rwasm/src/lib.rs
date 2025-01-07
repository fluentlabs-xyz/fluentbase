#![cfg_attr(not(feature = "std"), no_std)]
#![allow(dead_code)]
#![warn(unused_crate_dependencies)]

mod executor;
mod handler;
mod opcodes;
#[cfg(test)]
mod tests;
mod utils;

extern crate alloc;
extern crate core;

pub use executor::*;
pub use handler::*;
