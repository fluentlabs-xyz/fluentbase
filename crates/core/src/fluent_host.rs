use crate::{
    account::Account,
    evm::{sload::_evm_sload, sstore::_evm_sstore},
};
use alloc::vec::Vec;
use fluentbase_sdk::{Bytes32, LowLevelAPI, LowLevelSDK};
use fluentbase_types::ExitCode;
use hashbrown::HashMap;
use revm_interpreter::{
    primitives::{Address, Bytecode, Bytes, Env, Log, B256, U256},
    Host,
    SelfDestructResult,
};

/// A dummy [Host] implementation.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FluentHost {
    env: Env,
    storage: HashMap<U256, U256>,
    transient_storage: HashMap<U256, U256>,
    log: Vec<Log>,
}

impl FluentHost {
    /// Create a new dummy host with the given [`Env`].
    #[inline]
    pub fn new(env: Env) -> Self {
        Self {
            env,
            ..Default::default()
        }
    }

    /// Clears the storage and logs of the dummy host.
    #[inline]
    pub fn clear(&mut self) {
        self.storage.clear();
        self.log.clear();
    }
}

impl Host for FluentHost {
    fn env(&self) -> &Env {
        // TODO check who calls this method and what params it requires (and initialize those params
        // from context) TODO probably must be lazily generated/created
        &self.env
    }

    fn env_mut(&mut self) -> &mut Env {
        todo!()
    }

    #[inline]
    fn load_account(&mut self, _address: Address) -> Option<(bool, bool)> {
        Some((true, true))
    }

    #[inline]
    fn block_hash(&mut self, _number: U256) -> Option<B256> {
        // TODO not supported yet
        Some(B256::ZERO)
    }

    #[inline]
    fn balance(&mut self, address: Address) -> Option<(U256, bool)> {
        let account = Account::new(&fluentbase_types::Address::new(address.into_array()));

        Some((account.balance, false))
    }

    #[inline]
    fn code(&mut self, address: Address) -> Option<(Bytecode, bool)> {
        let account = Account::new(&fluentbase_types::Address::new(address.into_array()));
        let bytecode_bytes = Bytes::copy_from_slice(account.load_source_bytecode().as_ref());

        Some((Bytecode::new_raw(bytecode_bytes), false))
    }

    #[inline]
    fn code_hash(&mut self, address: Address) -> Option<(B256, bool)> {
        let account = Account::new(&fluentbase_types::Address::new(address.into_array()));
        let code_hash = B256::from_slice(account.source_code_hash.as_slice());

        Some((code_hash, false))
    }

    #[inline]
    fn sload(&mut self, address: Address, index: U256) -> Option<(U256, bool)> // ... -> (present, is_cold)
    {
        let mut slot_value32 = Bytes32::default();
        let mut is_cold: u32 = 0;
        let exit_code = _evm_sload(
            address.as_ptr(),
            index.as_le_slice().as_ptr(),
            slot_value32.as_mut_ptr(),
            &mut is_cold,
        );
        if exit_code != ExitCode::Ok {
            return None;
        }

        Some((U256::from_be_bytes(slot_value32), is_cold != 0))
    }

    #[inline]
    fn sstore(
        &mut self,
        address: Address,
        index: U256,
        value: U256,
    ) -> Option<(U256, U256, U256, bool)> // ... -> (previous_or_original_value, present, new, is_cold)
    {
        let mut previous_or_original_value = U256::default();
        let mut present = U256::default();
        let mut new_value = U256::default();
        let mut is_cold: u32 = 0;
        let sload_exit_code = _evm_sstore(
            address.as_ptr(),
            index.as_le_slice().as_ptr(),
            value.as_le_slice().as_ptr(),
            unsafe { previous_or_original_value.as_le_slice_mut().as_mut_ptr() },
            unsafe { present.as_le_slice_mut().as_mut_ptr() },
            unsafe { new_value.as_le_slice_mut().as_mut_ptr() },
            is_cold as *mut u32,
        );
        if sload_exit_code == ExitCode::Ok {
            return Some((previous_or_original_value, present, new_value, is_cold != 0));
        }
        // if let Some((present, is_cold)) = self.sload(address, index) {
        //     let mut slot_value32 = Bytes32::default();
        //     let _slot_value32_load_res =
        //         LowLevelSDK::jzkt_load(index.as_le_slice().as_ptr(), slot_value32.as_mut_ptr());
        //     // new value is same as present, we don't need to do anything
        //     if present == value {
        //         return Some((U256::from_be_bytes(slot_value32), present, value, is_cold));
        //     }
        //
        //     // insert value into present state.
        //     LowLevelSDK::jzkt_store(index.as_le_slice().as_ptr(), value.as_le_slice().as_ptr());
        //     // Ok((slot.previous_or_original_value, present, new, is_cold))
        //     return Some((U256::from_be_bytes(slot_value32), present, value, is_cold));
        // }
        return None;
    }

    #[inline]
    fn tload(&mut self, _address: Address, index: U256) -> U256 {
        panic!("tload not supported")
    }

    #[inline]
    fn tstore(&mut self, _address: Address, index: U256, value: U256) {
        panic!("tstore not supported")
    }

    #[inline]
    fn log(&mut self, log: Log) {
        let address_word = log.address.into_word();
        let data = log.data.data.0.clone();
        let topics: Vec<[u8; 32]> = log.topics().iter().copied().map(|v| v.0).collect();
        LowLevelSDK::jzkt_emit_log(
            address_word.as_ptr(),
            topics.as_ptr(),
            topics.len() as u32,
            data.as_ptr(),
            data.len() as u32,
        );
    }

    #[inline]
    fn selfdestruct(&mut self, _address: Address, _target: Address) -> Option<SelfDestructResult> {
        panic!("selfdestruct is not supported")
    }
}
