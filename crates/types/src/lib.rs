#![cfg_attr(not(feature = "std"), no_std)]
#![allow(dead_code, unreachable_patterns, unused_macros)]

pub use consts::*;
pub use journal::*;
pub use linker::*;
pub use types::*;

extern crate alloc;
extern crate core;

mod consts;
mod journal;
mod linker;
mod types;

pub use alloy_primitives::{address, b256, bloom, bytes, fixed_bytes, Address, Bytes, B256, U256};

pub const KECCAK_EMPTY: B256 =
    b256!("c5d2460186f7233c927e7db2dcc703c0e500b653ca82273b7bfad8045d85a470");
pub const POSEIDON_EMPTY: F254 =
    b256!("2098f5fb9e239eab3ceac3f27b81e481dc3124d55ffed523a839ee8446b64864");

pub type F254 = B256;
