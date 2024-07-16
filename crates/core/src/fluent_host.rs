use crate::debug_log;
use fluentbase_sdk::{Bytes, ContextReader, SovereignAPI};
use revm_interpreter::{
    as_usize_saturated,
    primitives::{
        Address,
        AnalysisKind,
        BlockEnv,
        CfgEnv,
        Env,
        Log,
        TransactTo,
        TxEnv,
        B256,
        U256,
    },
    Host,
    LoadAccountResult,
    SStoreResult,
    SelfDestructResult,
};
use revm_primitives::{TransientStorage, BLOCK_HASH_HISTORY};

pub struct FluentHost<'ctx, 'sdk, CTX: ContextReader, SDK: SovereignAPI> {
    pub(crate) env: Env,
    pub(crate) ctx: Option<&'ctx CTX>,
    pub(crate) sdk: Option<&'sdk SDK>,
    // transient storage
    transient_storage: TransientStorage,
}

impl<'ctx, 'sdk, CTX: ContextReader, SDK: SovereignAPI> FluentHost<'ctx, 'sdk, CTX, SDK> {
    pub fn new(ctx: &'ctx CTX, sdk: &'sdk SDK) -> Self {
        Self {
            env: Env {
                cfg: {
                    let mut cfg_env = CfgEnv::default();
                    cfg_env.chain_id = ctx.block_chain_id();
                    cfg_env.perf_analyse_created_bytecodes = AnalysisKind::Raw;
                    cfg_env
                },
                block: BlockEnv {
                    number: U256::from(ctx.block_number()),
                    coinbase: ctx.block_coinbase(),
                    timestamp: U256::from(ctx.block_timestamp()),
                    gas_limit: U256::from(ctx.block_gas_limit()),
                    basefee: ctx.block_base_fee(),
                    difficulty: U256::from(ctx.block_difficulty()),
                    prevrandao: Some(ctx.block_prevrandao()),
                    blob_excess_gas_and_price: None,
                },
                tx: TxEnv {
                    caller: ctx.tx_caller(),
                    gas_limit: ctx.tx_gas_limit(),
                    gas_price: ctx.tx_gas_price(),
                    transact_to: TransactTo::Call(Address::ZERO), // will do nothing
                    value: ctx.contract_value(),
                    data: Default::default(), // not used because we already pass all validations
                    nonce: Some(ctx.tx_nonce()),
                    chain_id: None, // no checks
                    access_list: ctx.tx_access_list(),
                    gas_priority_fee: ctx.tx_gas_priority_fee(),
                    blob_hashes: ctx.tx_blob_hashes(),
                    max_fee_per_blob_gas: ctx.tx_max_fee_per_blob_gas(),
                    #[cfg(feature = "optimism")]
                    optimism: Default::default(),
                },
            },
            ctx: Some(ctx),
            sdk: Some(sdk),
            transient_storage: TransientStorage::new(),
        }
    }
}

impl<'ctx, 'sdk, CTX: ContextReader, SDK: SovereignAPI> Host for FluentHost<'ctx, 'sdk, CTX, SDK> {
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
            Some(self.sdk.as_ref().unwrap().block_hash(number))
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
        Some((sdk.preimage(&account.source_code_hash), is_cold))
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
        let (value, is_cold) = self.sdk.as_ref().unwrap().storage(&address, &index, false);
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
        let sdk = self.sdk.unwrap();
        debug_log!(
            sdk,
            "ecl(sstore): address={}, index={}, value={}",
            address,
            hex::encode(index.to_be_bytes::<32>().as_slice()),
            hex::encode(value.to_be_bytes::<32>().as_slice()),
        );
        let (original_value, _) = sdk.storage(&address, &index, true);
        let (present_value, is_cold) = sdk.storage(&address, &index, false);
        sdk.write_storage(&address, &index, &value);
        return Some(SStoreResult {
            original_value,
            present_value,
            new_value: value,
            is_cold,
        });
    }

    #[inline]
    fn tload(&mut self, address: Address, index: U256) -> U256 {
        // self.transient_storage
        //     .get(&(address, index))
        //     .copied()
        //     .unwrap_or_default()
        self.sdk.unwrap().transient_storage(address, index)
    }

    #[inline]
    fn tstore(&mut self, address: Address, index: U256, value: U256) {
        // self.transient_storage.insert((address, index), value);
        self.sdk
            .unwrap()
            .write_transient_storage(address, index, value)
    }

    #[inline]
    fn log(&mut self, mut log: Log) {
        self.sdk
            .unwrap()
            .write_log(&log.address, &log.data.data, log.data.topics());
    }

    #[inline]
    fn selfdestruct(&mut self, address: Address, target: Address) -> Option<SelfDestructResult> {
        let [had_value, target_exists, is_cold, previously_destroyed] =
            self.sdk.unwrap().self_destruct(address, target);
        Some(SelfDestructResult {
            had_value,
            target_exists,
            is_cold,
            previously_destroyed,
        })
    }
}
