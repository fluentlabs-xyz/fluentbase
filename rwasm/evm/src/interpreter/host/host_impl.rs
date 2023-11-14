use crate::interpreter::host::Host;
use crate::interpreter::inner_models::{CreateInputs, SelfDestructResult};
use crate::interpreter::instruction_result::InstructionResult;
use crate::primitives::{Address, B256, KECCAK_EMPTY};
use crate::primitives::{Bytecode, Bytes, HashMap, U256};
use alloc::vec::Vec;
use fluentbase_rwasm::rwasm::InstructionSet;

/// A dummy [Host] implementation.
#[derive(Debug, PartialEq)]
pub struct HostImpl<'a> {
    // pub env: Env,
    // pub storage: HashMap<U256, U256>,
    pub transient_storage: HashMap<U256, U256>,
    // pub log: Vec<Log>,
    instruction_set: &'a mut InstructionSet,
}

impl<'a> HostImpl<'a> {
    /// Create a new dummy host with the given [`Env`].
    #[inline]
    pub fn new(/*env: Env*/ instruction_set: &'a mut InstructionSet) -> Self {
        Self {
            // env,
            instruction_set,
            transient_storage: HashMap::new(),
        }
    }
}

impl<'a> Host for HostImpl<'a> {
    #[inline]
    fn instruction_set(&mut self) -> &mut InstructionSet {
        &mut self.instruction_set
    }

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
        // match self.storage.entry(index) {
        //     Entry::Occupied(entry) => Some((*entry.get(), false)),
        //     Entry::Vacant(entry) => {
        //         entry.insert(U256::ZERO);
        //         Some((U256::ZERO, true))
        //     }
        // }
        None
    }

    #[inline]
    fn sstore(
        &mut self,
        _address: Address,
        index: U256,
        value: U256,
    ) -> Option<(U256, U256, U256, bool)> {
        // let (present, is_cold) = match self.storage.entry(index) {
        //     Entry::Occupied(mut entry) => (entry.insert(value), false),
        //     Entry::Vacant(entry) => {
        //         entry.insert(value);
        //         (U256::ZERO, true)
        //     }
        // };

        // Some((U256::ZERO, present, value, is_cold))
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
    fn log(&mut self, address: Address, topics: Vec<B256>, data: Bytes) {
        // self.log.push(Log {
        //     address,
        //     topics,
        //     data,
        // })
    }

    #[inline]
    fn selfdestruct(&mut self, _address: Address, _target: Address) -> Option<SelfDestructResult> {
        panic!("Selfdestruct is not supported")
    }

    #[inline]
    fn create(
        &mut self,
        _inputs: &mut CreateInputs,
        // _shared_memory: &mut SharedMemory,
    ) -> (InstructionResult, Option<Address> /*, Gas*/, Bytes) {
        panic!("Create is not supported for this host")
    }

    #[inline]
    fn call(
        &mut self,
        // _input: &mut CallInputs,
        // _shared_memory: &mut SharedMemory,
    ) -> (InstructionResult /*, Gas*/, Bytes) {
        panic!("Call is not supported")
    }
}
