#![cfg_attr(not(feature = "std"), no_std)]
#![allow(dead_code)]

pub mod bytecode;
pub mod evm;
pub mod gas;
pub mod macros;
pub mod memory;
pub mod result;
pub mod stack;
pub mod utils;

pub use evm::EVM;

extern crate alloc;
extern crate core;

/// Number of block hashes that EVM can access in the past (pre-Prague).
pub const BLOCK_HASH_HISTORY: u64 = 256;
