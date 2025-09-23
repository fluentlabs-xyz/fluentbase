#![cfg_attr(not(feature = "std"), no_std)]
#![allow(dead_code)]
#![warn(unused_crate_dependencies)]

extern crate alloc;
extern crate core;

pub extern crate rwasm as rwasm_core;

mod address;
mod block_fuel;
mod bytecode;
mod context;
mod curves;
pub mod evm;
mod exit_code;
pub mod genesis;
pub mod hashes;
pub mod helpers;
mod import_linker;
mod native_api;
mod preimage;
mod rwasm;
mod sdk;
mod sys_func_idx;
mod syscall;

pub use address::*;
pub use alloy_primitives::*;
pub use block_fuel::*;
pub use bytecode::*;
pub use byteorder;
pub use context::*;
pub use curves::*;
pub use exit_code::*;
pub use genesis::*;
pub use hashbrown::{hash_map, hash_set, HashMap, HashSet};
pub use import_linker::*;
pub use native_api::*;
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

pub const SYSTEM_ADDRESS: Address = address!("0xfffffffffffffffffffffffffffffffffffffffe");

pub const STATE_MAIN: u32 = 0;
pub const STATE_DEPLOY: u32 = 1;

pub const CALL_DEPTH_ROOT: u32 = 0;

/// A chain id for Fluent Developer Preview, where value hex is equal to 0x5201 where:
/// - 0x52 - is ASCII of R
/// - 0x01 - is a version of developer preview
pub const DEVELOPER_PREVIEW_CHAIN_ID: u64 = 10993;

/// A relation between fuel and gas,
/// according to our benchmarks, average WebAssembly instruction is ~1000 faster than average EVM
/// instruction.
///
/// The value can be changed in the future.
pub const FUEL_DENOM_RATE: u64 = 1000;

/// A max rWasm call stack limit
pub const CALL_STACK_LIMIT: u32 = 1024;

pub fn is_delegated_runtime_address(address: &Address) -> bool {
    address == &PRECOMPILE_EVM_RUNTIME
        || address == &PRECOMPILE_SVM_RUNTIME
        || address == &PRECOMPILE_UNIVERSAL_TOKEN_RUNTIME
        || address == &PRECOMPILE_WASM_RUNTIME
}

/// WASM max code size
///
/// This value is temporary for testing purposes, requires recalculation.
/// The limit is equal to 2Mb.
pub const WASM_MAX_CODE_SIZE: usize = revm_primitives::wasm::WASM_MAX_CODE_SIZE;
pub const SVM_MAX_CODE_SIZE: usize = revm_primitives::wasm::SVM_MAX_CODE_SIZE;

/// WebAssembly magic bytes
///
/// These values are equal to \0ASM
pub const WASM_MAGIC_BYTES: [u8; 4] = revm_primitives::wasm::WASM_MAGIC_BYTES;
/// Solana magic bytes
pub const SVM_ELF_MAGIC_BYTES: [u8; 4] = revm_primitives::wasm::SVM_ELF_MAGIC_BYTES;
/// ERC20 magic bytes: as char codes for "ERC" and the number 0x20
pub const ERC20_MAGIC_BYTES: [u8; 4] = revm_primitives::wasm::ERC20_MAGIC_BYTES;

/// EIP-170: Contract code size limit
///
/// By default, the limit is `0x6000` (~25kb)
pub const EVM_MAX_CODE_SIZE: usize = revm_primitives::eip170::MAX_CODE_SIZE;

/// EIP-3860: Limit and meter initcode
///
/// Limit of maximum initcode size is `2 * WASM_MAX_CODE_SIZE`.
pub const EVM_MAX_INITCODE_SIZE: usize = 2 * EVM_MAX_CODE_SIZE;

pub const EIP7702_SIG_LEN: usize = 2;
/// rWASM binary format signature:
/// - 0xef 0x00 - EIP-3540 compatible prefix
/// - 0x52 - rWASM version number (equal to 'R')
pub const EIP7702_SIG: [u8; EIP7702_SIG_LEN] = [0xef, 0x01];

pub const WASM_SIG_LEN: usize = 4;
/// WebAssembly signature (\00ASM)
pub const WASM_SIG: [u8; WASM_SIG_LEN] = [0x00, 0x61, 0x73, 0x6d];

pub const RWASM_SIG_LEN: usize = 2;
/// rWASM binary format signature:
/// - 0xef 0x00 - EIP-3540 compatible prefix
/// - 0x52 - rWASM version number (equal to 'R')
pub const RWASM_SIG: [u8; RWASM_SIG_LEN] = [0xef, 0x52];

#[macro_export]
macro_rules! bn254_add_common_impl {
    ($p: ident, $q: ident, $action_p_eq_q: block, $action_rest: block) => {
        if *$p == [0u8; 64] {
            if *$q != [0u8; 64] {
                *$p = *$q;
            }
            return;
        } else if *$q == [0u8; 64] {
            return;
        } else if *$p == *$q {
            $action_p_eq_q
        } else {
            $action_rest
        }
    };
}
