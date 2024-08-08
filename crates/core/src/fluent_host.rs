use crate::debug_log;
use core::mem::take;
use fluentbase_sdk::{Bytes, SovereignAPI};
use fluentbase_types::{env_from_context, NativeAPI};
use revm_interpreter::{
    as_usize_saturated,
    primitives::{Address, Env, Log, B256, U256},
    Host,
    LoadAccountResult,
    SStoreResult,
    SelfDestructResult,
};
use revm_primitives::BLOCK_HASH_HISTORY;

pub struct FluentHost<'sdk, SDK: SovereignAPI> {
    pub env: Env,
    pub sdk: Option<&'sdk mut SDK>,
}

impl<'sdk, SDK: SovereignAPI> FluentHost<'sdk, SDK> {
    pub fn new(sdk: &'sdk mut SDK) -> Self {
        Self {
            env: env_from_context(sdk.block_context(), sdk.tx_context()),
            sdk: Some(sdk),
        }
    }

    pub fn take_sdk(&mut self) -> &mut SDK {
        self.sdk.take().unwrap()
    }
}

impl<'sdk, SDK: SovereignAPI> Host for FluentHost<'sdk, SDK> {
    fn env(&self) -> &Env {
        &self.env
    }

    fn env_mut(&mut self) -> &mut Env {
        &mut self.env
    }

    #[inline]
    fn load_account(&mut self, address: Address) -> Option<LoadAccountResult> {
        let (account, is_cold) = self.sdk.as_ref().unwrap().account(&address);
        // Some((is_cold, account.is_not_empty()))
        Some(LoadAccountResult {
            is_cold,
            is_empty: account.is_empty(),
        })
    }

    #[inline]
    fn block_hash(&mut self, number: U256) -> Option<B256> {
        let block_number = as_usize_saturated!(self.env().block.number);
        let requested_number = as_usize_saturated!(number);
        let Some(diff) = block_number.checked_sub(requested_number) else {
            return Some(B256::ZERO);
        };
        if diff > 0 && diff <= BLOCK_HASH_HISTORY {
            todo!("implement block hash history")
        } else {
            Some(B256::ZERO)
        }
    }

    #[inline]
    fn balance(&mut self, address: Address) -> Option<(U256, bool)> {
        let (account, is_cold) = self.sdk.as_ref().unwrap().account(&address);
        Some((account.balance, is_cold))
    }

    #[inline]
    fn code(&mut self, address: Address) -> Option<(Bytes, bool)> {
        let sdk = self.sdk.as_ref().unwrap();
        let (account, is_cold) = sdk.account(&address);
        Some((
            sdk.preimage(&account.source_code_hash).unwrap_or_default(),
            is_cold,
        ))
    }

    #[inline]
    fn code_hash(&mut self, address: Address) -> Option<(B256, bool)> {
        let (account, is_cold) = self.sdk.as_ref().unwrap().account(&address);
        if !account.is_not_empty() {
            return Some((B256::ZERO, is_cold));
        }
        Some((account.source_code_hash, is_cold))
    }

    #[inline]
    fn sload(&mut self, address: Address, index: U256) -> Option<(U256, bool)> {
        let (value, is_cold) = self.sdk.as_ref().unwrap().storage(&address, &index);
        debug_log!(
            self.sdk.as_ref().unwrap(),
            "ecl(sload): address={}, index={}, value={}",
            address,
            hex::encode(index.to_be_bytes::<32>().as_slice()),
            hex::encode(value.to_be_bytes::<32>().as_slice()),
        );
        Some((value, is_cold))
    }

    #[inline]
    fn sstore(&mut self, address: Address, index: U256, value: U256) -> Option<SStoreResult> {
        let sdk = self.sdk.as_mut().unwrap();
        debug_log!(
            sdk,
            "ecl(sstore): address={}, index={}, value={}",
            address,
            hex::encode(index.to_be_bytes::<32>().as_slice()),
            hex::encode(value.to_be_bytes::<32>().as_slice()),
        );
        let (original_value, _) = sdk.committed_storage(&address, &index);
        let (present_value, is_cold) = sdk.storage(&address, &index);
        sdk.write_storage(address, index, value);
        return Some(SStoreResult {
            original_value,
            present_value,
            new_value: value,
            is_cold,
        });
    }

    #[inline]
    fn tload(&mut self, address: Address, index: U256) -> U256 {
        self.sdk.as_mut().unwrap().transient_storage(address, index)
    }

    #[inline]
    fn tstore(&mut self, address: Address, index: U256, value: U256) {
        self.sdk
            .as_mut()
            .unwrap()
            .write_transient_storage(address, index, value)
    }

    #[inline]
    fn log(&mut self, mut log: Log) {
        self.sdk.as_mut().unwrap().write_log(
            log.address,
            take(&mut log.data.data),
            log.data.topics(),
        );
    }

    #[inline]
    fn selfdestruct(&mut self, address: Address, target: Address) -> Option<SelfDestructResult> {
        let result = self
            .sdk
            .as_mut()
            .unwrap()
            .destroy_account(&address, &target);
        Some(SelfDestructResult {
            had_value: result.had_value,
            target_exists: result.target_exists,
            is_cold: result.is_cold,
            previously_destroyed: result.previously_destroyed,
        })
    }
}
