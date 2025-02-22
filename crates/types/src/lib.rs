#![cfg_attr(not(feature = "std"), no_std)]
#![allow(dead_code)]
#![warn(unused_crate_dependencies)]

extern crate alloc;
extern crate core;

mod allocator;
mod context;
mod contracts;
mod exit_code;
mod linker;
mod rwasm;
mod sdk;
mod sys_func_idx;
mod types;
mod utils;

pub use allocator::*;
pub use alloy_primitives::{address, b256, bloom, bytes, fixed_bytes, Address, Bytes, B256, U256};
pub use byteorder;
pub use context::*;
pub use contracts::*;
pub use exit_code::*;
pub use hashbrown::{hash_map, hash_set, HashMap, HashSet};
pub use linker::*;
pub use rwasm::*;
pub use sdk::*;
pub use sys_func_idx::SysFuncIdx;
pub use types::*;
pub use utils::*;

pub const KECCAK_EMPTY: B256 =
    b256!("c5d2460186f7233c927e7db2dcc703c0e500b653ca82273b7bfad8045d85a470");
pub const POSEIDON_EMPTY: F254 =
    b256!("2098f5fb9e239eab3ceac3f27b81e481dc3124d55ffed523a839ee8446b64864");

pub type F254 = B256;

/// keccak256 of "Transfer(address,address,uint256)" that notifies
/// about native transfer of eth
pub const NATIVE_TRANSFER_KECCAK: B256 =
    b256!("ddf252ad1be2c89b69c2b068fc378daa952ba7f163c4a11628f55a4df523b3ef");
pub const NATIVE_TRANSFER_ADDRESS: Address = address!("0000000000000000000000000000000000000000");

pub const STATE_MAIN: u32 = 0;
pub const STATE_DEPLOY: u32 = 1;

/// A chain id for Fluent Developer Preview, where value hex is equal to 0x5201 where:
/// - 0x52 - is ASCII of R
/// - 0x01 - is a version of developer preview
pub const DEVELOPER_PREVIEW_CHAIN_ID: u64 = 20993;

pub const FUEL_DENOM_RATE: u64 = 1000;
