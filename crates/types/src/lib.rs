#![cfg_attr(not(feature = "std"), no_std)]
#![allow(dead_code)]
#![warn(unused_crate_dependencies)]

extern crate alloc;
extern crate core;

use primitives as _;

mod address;
mod bytecode_type;
mod context;
pub mod evm;
mod exit_code;
mod fuel_procedures;
pub mod genesis;
mod linker;
pub mod native_api;
mod preimage;
mod rwasm;
mod sdk;
mod sys_func_idx;
mod syscall;

pub use address::*;
pub use alloy_primitives::*;
pub use bytecode_type::*;
pub use byteorder;
pub use context::*;
pub use exit_code::*;
pub use fuel_procedures::*;
pub use genesis::*;
pub use hashbrown::{hash_map, hash_set, HashMap, HashSet};
pub use linker::*;
pub use preimage::*;
pub use rwasm::*;
pub use sdk::*;
pub use sys_func_idx::SysFuncIdx;
pub use syscall::*;

pub const KECCAK_EMPTY: B256 =
    b256!("c5d2460186f7233c927e7db2dcc703c0e500b653ca82273b7bfad8045d85a470");
pub const POSEIDON_EMPTY: B256 =
    b256!("2098f5fb9e239eab3ceac3f27b81e481dc3124d55ffed523a839ee8446b64864");

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

/// A relation between fuel and gas,
/// according to our benchmarks, average WebAssembly instruction is ~1000 faster than average EVM
/// instruction
pub const FUEL_DENOM_RATE: u64 = 1000;

/// A max rWasm call stack limit
pub const CALL_STACK_LIMIT: u32 = 1024;

/// EVM code hash slot: `hash=keccak256("_evm_code_hash")`
pub const PROTECTED_STORAGE_SLOT_0: B256 =
    b256!("575bdaed2313333f49ce8fccd329e40d2042d950450ea7045276ef8f6b18113b");
pub const PROTECTED_STORAGE_SLOT_1: B256 =
    b256!("575bdaed2313333f49ce8fccd329e40d2042d950450ea7045276ef8f6b18113c");

pub fn is_protected_storage_slot<I: Into<B256>>(slot: I) -> bool {
    let slot: B256 = slot.into();
    slot == PROTECTED_STORAGE_SLOT_0 || slot == PROTECTED_STORAGE_SLOT_1
}

/// rWASM max code size
///
/// This value is temporary for testing purposes, requires recalculation.
/// The limit is equal to 2Mb.
pub const WASM_MAX_CODE_SIZE: usize = 0x200000;

/// WebAssembly magic bytes
///
/// These values are equal to \0ASM
pub const WASM_MAGIC_BYTES: [u8; 4] = [0x00, 0x61, 0x73, 0x6d];

/// EIP-170: Contract code size limit
///
/// By default, the limit is `0x6000` (~25kb)
pub const EVM_MAX_CODE_SIZE: usize = 0x6000;

/// EIP-3860: Limit and meter initcode
///
/// Limit of maximum initcode size is `2 * WASM_MAX_CODE_SIZE`.
pub const EVM_MAX_INITCODE_SIZE: usize = 2 * EVM_MAX_CODE_SIZE;
