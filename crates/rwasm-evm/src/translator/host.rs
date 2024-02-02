pub mod host_impl;

use crate::{
    primitives::{Address, Bytecode, Bytes, B256, U256},
    translator::{
        inner_models::{CreateInputs, SelfDestructResult},
        instruction_result::InstructionResult,
    },
};
use alloc::vec::Vec;

/// EVM context host.
pub trait Host {
    /// Load an account.
    ///
    /// Returns (is_cold, is_new_account)
    fn load_account(&mut self, address: Address) -> Option<(bool, bool)>;

    /// Get the block hash of the given block `number`.
    fn block_hash(&mut self, number: U256) -> Option<B256>;

    /// Get balance of `address` and if the account is cold.
    fn balance(&mut self, address: Address) -> Option<(U256, bool)>;

    /// Get code of `address` and if the account is cold.
    fn code(&mut self, address: Address) -> Option<(Bytecode, bool)>;

    /// Get code hash of `address` and if the account is cold.
    fn code_hash(&mut self, address: Address) -> Option<(B256, bool)>;

    /// Get storage value of `address` at `index` and if the account is cold.
    fn sload(&mut self, address: Address, index: U256) -> Option<(U256, bool)>;

    /// Set storage value of account address at index.
    ///
    /// Returns (original, present, new, is_cold).
    fn sstore(
        &mut self,
        address: Address,
        index: U256,
        value: U256,
    ) -> Option<(U256, U256, U256, bool)>;

    /// Get the transient storage value of `address` at `index`.
    fn tload(&mut self, address: Address, index: U256) -> U256;

    /// Set the transient storage value of `address` at `index`.
    fn tstore(&mut self, address: Address, index: U256, value: U256);

    /// Emit a log owned by `address` with given `topics` and `data`.
    fn log(&mut self, address: Address, topics: Vec<B256>, data: Bytes);

    /// Invoke a call operation.
    fn call(
        &mut self,
        // input: &mut CallInputs,
        // shared_memory: &mut SharedMemory,
    ) -> (InstructionResult, Bytes);

    /// Invoke a create operation.
    fn create(
        &mut self,
        inputs: &mut CreateInputs,
        // shared_memory: &mut SharedMemory,
    ) -> (InstructionResult, Option<Address>, Bytes);

    /// Mark `address` to be deleted, with funds transferred to `target`.
    fn selfdestruct(&mut self, address: Address, target: Address) -> Option<SelfDestructResult>;
}
