use fluentbase_sdk::{AccountManager, Address, Bytes, ContextReader, U256};

pub mod address;
pub mod balance;
#[cfg(feature = "ecl")]
pub mod call;
pub mod calldatacopy;
pub mod calldataload;
pub mod calldatasize;
pub mod codecopy;
pub mod codehash;
pub mod codesize;
#[cfg(feature = "ecl")]
pub mod create;
pub mod extcodecopy;
pub mod extcodehash;
pub mod extcodesize;
mod r#impl;
pub mod log0;
pub mod log1;
pub mod log2;
pub mod log3;
pub mod log4;
pub mod r#return;
pub mod selfbalance;
pub mod sload;
pub mod sstore;

pub trait EVM {
    fn address(&self) -> Address;
    fn balance(&self, address: Address) -> U256;
    fn call(&self, callee: Address, value: U256, input: Bytes, gas: u64);
    fn sload(&self, index: U256) -> U256;
    fn sstore(&self, index: U256, value: U256);
}

pub struct EvmImpl<'a, CR: ContextReader, AM: AccountManager> {
    cr: &'a CR,
    am: &'a AM,
}
