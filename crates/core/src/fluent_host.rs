use crate::debug_log;
use alloc::vec;
use core::mem::take;
use fluentbase_sdk::{AccountManager, ContextReader, LowLevelAPI};
use revm_interpreter::{
    primitives::{
        Address, AnalysisKind, BlockEnv, Bytecode, CfgEnv, Env, Log, TransactTo, TxEnv, B256, U256,
    },
    Host, SStoreResult, SelfDestructResult,
};
use revm_primitives::RWASM_MAX_CODE_SIZE;

pub struct FluentHost<'cr, 'am, CR: ContextReader, AM: AccountManager> {
    pub(crate) env: Env,
    pub(crate) cr: Option<&'cr CR>,
    pub(crate) am: Option<&'am AM>,
}

impl<'cr, 'am, CR: ContextReader, AM: AccountManager> FluentHost<'cr, 'am, CR, AM> {
    pub fn new(cr: &'cr CR, am: &'am AM) -> Self {
        Self {
            env: Env {
                cfg: {
                    let mut cfg_env = CfgEnv::default();
                    cfg_env.chain_id = cr.block_chain_id();
                    cfg_env.perf_analyse_created_bytecodes = AnalysisKind::Raw;
                    cfg_env
                },
                block: BlockEnv {
                    number: U256::from(cr.block_number()),
                    coinbase: cr.block_coinbase(),
                    timestamp: U256::from(cr.block_timestamp()),
                    gas_limit: U256::from(cr.block_gas_limit()),
                    basefee: cr.block_base_fee(),
                    difficulty: U256::from(cr.block_difficulty()),
                    prevrandao: Some(B256::from(U256::from(cr.block_difficulty()))),
                    blob_excess_gas_and_price: None,
                },
                tx: TxEnv {
                    caller: cr.tx_caller(),
                    gas_limit: cr.tx_gas_limit(),
                    gas_price: cr.tx_gas_price(),
                    transact_to: TransactTo::Call(Address::ZERO), // will do nothing
                    value: cr.contract_value(),
                    data: cr.contract_input(),
                    nonce: Some(cr.tx_nonce()),
                    chain_id: None, // no checks
                    access_list: cr.tx_access_list(),
                    gas_priority_fee: cr.tx_gas_priority_fee(),
                    blob_hashes: vec![],
                    max_fee_per_blob_gas: None,
                    #[cfg(feature = "optimism")]
                    optimism: Default::default(),
                },
            },
            cr: Some(cr),
            am: Some(am),
        }
    }
}

impl<'cr, 'am, CR: ContextReader, AM: AccountManager> Host for FluentHost<'cr, 'am, CR, AM> {
    fn env(&self) -> &Env {
        &self.env
    }

    fn env_mut(&mut self) -> &mut Env {
        &mut self.env
    }

    #[inline]
    fn load_account(&mut self, address: Address) -> Option<(bool, bool)> {
        let (account, is_cold) = self.am.unwrap().account(address);
        Some((is_cold, account.is_not_empty()))
    }

    #[inline]
    fn block_hash(&mut self, number: U256) -> Option<B256> {
        let block_hash = self.am.unwrap().block_hash(number);
        Some(block_hash)
    }

    #[inline]
    fn balance(&mut self, address: Address) -> Option<(U256, bool)> {
        let (account, is_cold) = self.am.unwrap().account(address);
        Some((account.balance, is_cold))
    }

    #[inline]
    fn code(&mut self, address: Address) -> Option<(Bytecode, bool)> {
        let (account, is_cold) = self.am.unwrap().account(address);
        let bytecode = self.am.unwrap().preimage(&account.source_code_hash);
        Some((Bytecode::new_raw(bytecode), is_cold))
    }

    #[inline]
    fn code_hash(&mut self, address: Address) -> Option<(B256, bool)> {
        let (account, is_cold) = self.am.unwrap().account(address);
        Some((account.source_code_hash, is_cold))
    }

    #[inline]
    fn sload(&mut self, address: Address, index: U256) -> Option<(U256, bool)> {
        let (value, is_cold) = self.am.unwrap().storage(address, index, false);
        debug_log!(
            "ecl(sload): address={}, index={}, value={}",
            address,
            hex::encode(index.to_be_bytes::<32>().as_slice()),
            hex::encode(value.to_be_bytes::<32>().as_slice()),
        );
        Some((value, is_cold))
    }

    #[inline]
    fn sstore(&mut self, address: Address, index: U256, value: U256) -> Option<SStoreResult> {
        debug_log!(
            "ecl(sstore): address={}, index={}, value={}",
            address,
            hex::encode(index.to_be_bytes::<32>().as_slice()),
            hex::encode(value.to_be_bytes::<32>().as_slice()),
        );
        let (original_value, _) = self.am.unwrap().storage(address, index, true);
        let (present_value, is_cold) = self.am.unwrap().storage(address, index, false);
        self.am.unwrap().write_storage(address, index, value);
        return Some(SStoreResult {
            original_value,
            present_value,
            new_value: value,
            is_cold,
        });
    }

    #[inline]
    fn tload(&mut self, _address: Address, _index: U256) -> U256 {
        panic!("TLOAD opcode is not supported")
    }

    #[inline]
    fn tstore(&mut self, _address: Address, _index: U256, _value: U256) {
        panic!("TSTORE opcode is not supported")
    }

    #[inline]
    fn log(&mut self, mut log: Log) {
        self.am
            .unwrap()
            .log(log.address, take(&mut log.data.data), log.data.topics());
    }

    #[inline]
    fn selfdestruct(&mut self, address: Address, target: Address) -> Option<SelfDestructResult> {
        let [had_value, target_exists, is_cold, previously_destroyed] =
            self.am.unwrap().self_destruct(address, target);
        Some(SelfDestructResult {
            had_value,
            target_exists,
            is_cold,
            previously_destroyed,
        })
    }
}
