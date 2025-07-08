#![allow(soft_unstable)]

extern crate alloc;
extern crate core;

#[cfg(test)]
mod bridge;
#[cfg(test)]
mod builtins;
#[cfg(test)]
mod constructor;
#[cfg(test)]
mod deployer;
#[cfg(test)]
mod eip2935;
#[cfg(test)]
mod erc20;
#[cfg(test)]
mod evm;
#[cfg(test)]
mod gas;
#[cfg(test)]
mod multicall;
#[cfg(test)]
mod nitro;
#[cfg(test)]
mod router;
#[cfg(test)]
mod stateless;
#[cfg(all(test, feature = "enable-svm"))]
pub mod svm_loader_v4;
#[cfg(test)]
mod update_account;
#[cfg(test)]
mod wasm;

pub const EXAMPLE_ABI_SOLIDITY: &[u8] = fluentbase_examples_abi_solidity::WASM_BYTECODE;
pub const EXAMPLE_CHECKMATE: &[u8] = fluentbase_examples_checkmate::WASM_BYTECODE;
pub const EXAMPLE_CLIENT_SOLIDITY: &[u8] = fluentbase_examples_client_solidity::WASM_BYTECODE;
pub const EXAMPLE_CONSTRUCTOR_PARAMS: &[u8] = fluentbase_examples_constructor_params::WASM_BYTECODE;
pub const EXAMPLE_ERC20: &[u8] = fluentbase_examples_erc20::WASM_BYTECODE;
pub const EXAMPLE_GREETING: &[u8] = fluentbase_examples_greeting::WASM_BYTECODE;
pub const EXAMPLE_JSON: &[u8] = fluentbase_examples_json::WASM_BYTECODE;
pub const EXAMPLE_KECCAK256: &[u8] = fluentbase_examples_keccak::WASM_BYTECODE;
pub const EXAMPLE_PANIC: &[u8] = fluentbase_examples_panic::WASM_BYTECODE;
pub const EXAMPLE_ROUTER_SOLIDITY: &[u8] = fluentbase_examples_router_solidity::WASM_BYTECODE;
pub const EXAMPLE_RWASM: &[u8] = fluentbase_examples_rwasm::WASM_BYTECODE;
pub const EXAMPLE_SECP256K1: &[u8] = fluentbase_examples_secp256k1::WASM_BYTECODE;
pub const EXAMPLE_SIMPLE_STORAGE: &[u8] = fluentbase_examples_simple_storage::WASM_BYTECODE;
pub const EXAMPLE_STORAGE: &[u8] = fluentbase_examples_storage::WASM_BYTECODE;
pub const EXAMPLE_TINY_KECCAK256: &[u8] = fluentbase_examples_tiny_keccak::WASM_BYTECODE;
