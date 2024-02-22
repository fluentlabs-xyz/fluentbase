#![no_std]
#![allow(dead_code)]

extern crate alloc;
extern crate core;

pub mod account;
mod evm;
#[cfg(test)]
mod testing_utils;
mod utils;
