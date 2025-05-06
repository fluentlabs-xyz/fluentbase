#![allow(soft_unstable)]
#![feature(test)]

extern crate alloc;
extern crate core;
use fluentbase_types::include_wasm;

#[cfg(test)]
mod bench;
#[cfg(test)]
mod bridge;
#[cfg(test)]
mod builtins;
#[cfg(test)]
mod constructor;
#[cfg(test)]
mod deployer;
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
#[cfg(test)]
mod utils;
#[cfg(test)]
mod wasm;

pub const _EXAMPLE_ABI_SOLIDITY: &[u8] = include_wasm!("fluentbase-examples-abi-solidity");
pub const EXAMPLE_CHECKMATE: &[u8] = include_wasm!("fluentbase-examples-checkmate");
pub const EXAMPLE_CLIENT_SOLIDITY: &[u8] = include_wasm!("fluentbase-examples-client-solidity");
pub const EXAMPLE_CONSTRUCTOR_PARAMS: &[u8] =
    include_wasm!("fluentbase-examples-constructor-params");
pub const EXAMPLE_ERC20: &[u8] = include_wasm!("fluentbase-examples-erc20");
pub const EXAMPLE_GREETING: &[u8] = include_wasm!("fluentbase-examples-greeting");
pub const EXAMPLE_JSON: &[u8] = include_wasm!("fluentbase-examples-json");
pub const EXAMPLE_TINY_KECCAK256: &[u8] = include_wasm!("fluentbase-examples-tiny-keccak");
pub const EXAMPLE_KECCAK256: &[u8] = include_wasm!("fluentbase-examples-keccak");
pub const EXAMPLE_PANIC: &[u8] = include_wasm!("fluentbase-examples-panic");
pub const EXAMPLE_ROUTER_SOLIDITY: &[u8] = include_wasm!("fluentbase-examples-router-solidity");
pub const EXAMPLE_RWASM: &[u8] = include_wasm!("fluentbase-examples-rwasm");
pub const EXAMPLE_SECP256K1: &[u8] = include_wasm!("fluentbase-examples-secp256k1");
pub const EXAMPLE_SIMPLE_STORAGE: &[u8] = include_wasm!("fluentbase-examples-simple-storage");
pub const _EXAMPLE_STORAGE: &[u8] = include_wasm!("fluentbase-examples-storage");
