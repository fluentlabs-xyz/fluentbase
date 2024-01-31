use crate::{
    primitives::{Address, Bytecode, Bytes, HashMap, B256, KECCAK_EMPTY, U256},
    translator::{
        host::Host,
        inner_models::{CreateInputs, SelfDestructResult},
        instruction_result::InstructionResult,
    },
};
use alloc::vec::Vec;

/// A dummy [Host] implementation.
#[derive(Debug, PartialEq)]
pub struct HostImpl {
    pub transient_storage: HashMap<U256, U256>,
}

impl HostImpl {
    /// Create a new dummy host with the given [`Env`].
    #[inline]
    pub fn new() -> Self {
        Self {
            transient_storage: HashMap::new(),
        }
    }
}

impl Host for HostImpl {
    #[inline]
    fn load_account(&mut self, _address: Address) -> Option<(bool, bool)> {
        Some((true, true))
    }

    #[inline]
    fn block_hash(&mut self, _number: U256) -> Option<B256> {
        Some(B256::ZERO)
    }

    #[inline]
    fn balance(&mut self, _address: Address) -> Option<(U256, bool)> {
        Some((U256::ZERO, false))
    }

    #[inline]
    fn code(&mut self, _address: Address) -> Option<(Bytecode, bool)> {
        Some((Bytecode::default(), false))
    }

    #[inline]
    fn code_hash(&mut self, __address: Address) -> Option<(B256, bool)> {
        Some((KECCAK_EMPTY, false))
    }

    #[inline]
    fn sload(&mut self, __address: Address, index: U256) -> Option<(U256, bool)> {
        None
    }

    #[inline]
    fn sstore(
        &mut self,
        _address: Address,
        index: U256,
        value: U256,
    ) -> Option<(U256, U256, U256, bool)> {
        Some((U256::ZERO, U256::ZERO, U256::ZERO, false))
    }

    #[inline]
    fn tload(&mut self, _address: Address, index: U256) -> U256 {
        self.transient_storage
            .get(&index)
            .copied()
            .unwrap_or_default()
    }

    #[inline]
    fn tstore(&mut self, _address: Address, index: U256, value: U256) {
        self.transient_storage.insert(index, value);
    }

    #[inline]
    fn log(&mut self, address: Address, topics: Vec<B256>, data: Bytes) {}

    #[inline]
    fn call(&mut self) -> (InstructionResult, Bytes) {
        // #[cfg(test)]
        panic!("not supported");
    }

    #[inline]
    fn create(
        &mut self,
        _inputs: &mut CreateInputs,
    ) -> (InstructionResult, Option<Address>, Bytes) {
        // #[cfg(test)]
        panic!("not supported")
    }

    #[inline]
    fn selfdestruct(&mut self, _address: Address, _target: Address) -> Option<SelfDestructResult> {
        // #[cfg(test)]
        panic!("not supported")
    }
}
