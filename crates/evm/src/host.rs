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
use revm_context_interface::cfg::GasParams;
use revm_interpreter::{Host, SStoreResult, SelfDestructResult};
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

    fn blob_gasprice(&self) -> U256 {
        // TODO(dmitry123): Why block base fee works here and tests pass if blob price equals to base price?
        //  Check test (cargo:test://evm_e2e::short_tests::good_coverage_tests::opc4_adiff_places)
        //  P.S: We don't support blobs in Fluent yet, so need to check how it can affect the system.
        self.sdk.context().block_base_fee()
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
}
