//! Bridge between revm Host trait and the external SDK host.
//!
//! We do not execute Host methods directly; host-bound opcodes are routed
//! via interruptions. The unreachable!() bodies here document that path.

use crate::utils::evm_gas_params;
use core::ops::{Deref, DerefMut};
use fluentbase_sdk::{Address, Bytes, ContextReader, Log, SystemAPI, B256, U256};
use revm_context::{
    host::LoadError,
    journaled_state::{AccountInfoLoad, AccountLoad, StateLoad},
};
use revm_context_interface::{cfg::GasParams, Host};
use revm_interpreter::{SStoreResult, SelfDestructResult};
use revm_primitives::{StorageKey, StorageValue};

/// Helper trait to access the underlying SDK from opcode handlers.
pub(crate) trait HostWrapper: Host {
    fn sdk_mut(&mut self) -> &mut impl SystemAPI;
}

/// Wrapper that implements revm::Host for our SDK, but actual effects
/// are performed through the interruption protocol.
pub struct HostWrapperImpl<'a, SDK: SystemAPI> {
    sdk: &'a mut SDK,
}

impl<'a, SDK: SystemAPI> Deref for HostWrapperImpl<'a, SDK> {
    type Target = SDK;

    fn deref(&self) -> &Self::Target {
        self.sdk
    }
}
impl<'a, SDK: SystemAPI> DerefMut for HostWrapperImpl<'a, SDK> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.sdk
    }
}

impl<'a, SDK: SystemAPI> HostWrapperImpl<'a, SDK> {
    pub fn wrap(sdk: &'a mut SDK) -> Self {
        Self { sdk }
    }
}

impl<'a, SDK: SystemAPI> HostWrapper for HostWrapperImpl<'a, SDK> {
    fn sdk_mut(&mut self) -> &mut impl SystemAPI {
        self.sdk
    }
}

impl<'a, SDK: SystemAPI> Host for HostWrapperImpl<'a, SDK> {
    fn basefee(&self) -> U256 {
        self.sdk.context().block_base_fee()
    }

    /// Always zero: Fluent has no blob transactions.
    ///
    /// See [`Self::blob_hash`] for why this is intended rather than a missing context field.
    fn blob_gasprice(&self) -> U256 {
        U256::ZERO
    }

    fn gas_limit(&self) -> U256 {
        U256::from(self.sdk.context().block_gas_limit())
    }

    fn difficulty(&self) -> U256 {
        self.sdk.context().block_difficulty()
    }

    fn prevrandao(&self) -> Option<U256> {
        Some(self.sdk.context().block_prev_randao().into())
    }

    fn block_number(&self) -> U256 {
        U256::from(self.sdk.context().block_number())
    }

    fn timestamp(&self) -> U256 {
        U256::from(self.sdk.context().block_timestamp())
    }

    fn beneficiary(&self) -> Address {
        self.sdk.context().block_coinbase()
    }

    fn chain_id(&self) -> U256 {
        U256::from(self.sdk.context().block_chain_id())
    }

    fn effective_gas_price(&self) -> U256 {
        self.sdk.context().tx_gas_price()
    }

    fn caller(&self) -> Address {
        self.sdk.context().tx_origin()
    }

    /// Always zero: Fluent has no blob transactions.
    ///
    /// EIP-4844 is not supported by the chain — blocks carry no `excess_blob_gas` / `blob_gas_used`
    /// and the blob schedule is empty (`crates/genesis/build.rs`), so a type-3 transaction can
    /// never be included. Every transaction therefore has an empty versioned-hash list, and
    /// canonical EVM semantics for `BLOBHASH` with an out-of-range index are exactly this: push
    /// zero. `BLOBBASEFEE` ([`Self::blob_gasprice`]) is zero for the same reason.
    ///
    /// This is why the shared context carries no blob fields (see [`fluentbase_sdk::TxContextV1`]):
    /// there is no value to plumb through. If Fluent ever gains blob transactions, both methods and
    /// the context must be extended together.
    fn blob_hash(&self, _number: usize) -> Option<U256> {
        Some(U256::ZERO)
    }

    fn max_initcode_size(&self) -> usize {
        unreachable!()
    }

    fn gas_params(&self) -> &GasParams {
        evm_gas_params()
    }

    fn block_hash(&mut self, _number: u64) -> Option<B256> {
        unreachable!()
    }

    fn selfdestruct(
        &mut self,
        _address: Address,
        _target: Address,
        _skip_cold_load: bool,
    ) -> Result<StateLoad<SelfDestructResult>, LoadError> {
        unreachable!()
    }

    fn log(&mut self, _log: Log) {
        unreachable!()
    }

    fn sstore_skip_cold_load(
        &mut self,
        _address: Address,
        _key: StorageKey,
        _value: StorageValue,
        _skip_cold_load: bool,
    ) -> Result<StateLoad<SStoreResult>, LoadError> {
        unreachable!()
    }

    fn sstore(
        &mut self,
        _address: Address,
        _key: StorageKey,
        _value: StorageValue,
    ) -> Option<StateLoad<SStoreResult>> {
        unreachable!()
    }

    fn sload_skip_cold_load(
        &mut self,
        _address: Address,
        _key: StorageKey,
        _skip_cold_load: bool,
    ) -> Result<StateLoad<StorageValue>, LoadError> {
        unreachable!()
    }

    fn sload(&mut self, _address: Address, _key: StorageKey) -> Option<StateLoad<StorageValue>> {
        unreachable!()
    }

    fn tstore(&mut self, _address: Address, _key: StorageKey, _value: StorageValue) {
        unreachable!()
    }

    fn tload(&mut self, _address: Address, _key: StorageKey) -> StorageValue {
        unreachable!()
    }

    fn load_account_info_skip_cold_load(
        &mut self,
        _address: Address,
        _load_code: bool,
        _skip_cold_load: bool,
    ) -> Result<AccountInfoLoad<'_>, LoadError> {
        unreachable!()
    }

    fn balance(&mut self, _address: Address) -> Option<StateLoad<U256>> {
        unreachable!()
    }

    fn load_account_delegated(&mut self, _address: Address) -> Option<StateLoad<AccountLoad>> {
        unreachable!()
    }

    fn load_account_code(&mut self, _address: Address) -> Option<StateLoad<Bytes>> {
        unreachable!()
    }

    fn load_account_code_hash(&mut self, _address: Address) -> Option<StateLoad<B256>> {
        unreachable!()
    }

    fn is_amsterdam_eip8037_enabled(&self) -> bool {
        false
    }

    fn slot_num(&self) -> U256 {
        U256::ZERO
    }
}
