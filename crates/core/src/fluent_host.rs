use crate::helpers::debug_log;
use crate::{
    account::Account,
    account_types::MAX_BYTECODE_SIZE,
    evm::{sload::_evm_sload, sstore::_evm_sstore},
};
use alloc::{format, vec};
use core::cell::Cell;
use core::marker::PhantomData;
use fluentbase_sdk::{ContextReader, LowLevelAPI, LowLevelSDK};
use fluentbase_types::Bytes32;
use revm_interpreter::{
    primitives::{
        Address, AnalysisKind, BlockEnv, Bytecode, Bytes, CfgEnv, Env, Log, TransactTo, TxEnv,
        B256, U256,
    },
    Host, SStoreResult, SelfDestructResult,
};
use revm_primitives::RWASM_MAX_CODE_SIZE;

pub struct FluentHost<'cr, CR: ContextReader> {
    pub(crate) env: Env,
    pub(crate) cr: Option<&'cr CR>,
}

impl<'cr, CR: ContextReader> FluentHost<'cr, CR> {
    pub fn new(cr: &'cr CR) -> Self {
        Self {
            env: Env {
                cfg: {
                    let mut cfg_env = CfgEnv::default();
                    cfg_env.chain_id = cr.block_chain_id();
                    cfg_env.perf_analyse_created_bytecodes = AnalysisKind::Raw;
                    cfg_env.limit_contract_code_size = Some(RWASM_MAX_CODE_SIZE);
                    cfg_env
                },
                block: BlockEnv {
                    number: U256::from(cr.block_number()),
                    coinbase: cr.block_coinbase(),
                    timestamp: U256::from(cr.block_timestamp()),
                    gas_limit: U256::from(cr.block_gas_limit()),
                    basefee: cr.block_base_fee(),
                    difficulty: U256::from(cr.block_difficulty()),
                    prevrandao: None,
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
        }
    }
}

impl<'cr, CR: ContextReader> Host for FluentHost<'cr, CR> {
    fn env(&self) -> &Env {
        &self.env
    }

    fn env_mut(&mut self) -> &mut Env {
        &mut self.env
    }

    #[inline]
    fn load_account(&mut self, _address: Address) -> Option<(bool, bool)> {
        // TODO(dmitry123): "fix `is_cold` and `is_new` calculation"
        Some((true, true))
    }

    #[inline]
    fn block_hash(&mut self, _number: U256) -> Option<B256> {
        // TODO(dmitry123): "not supported yet"
        Some(B256::ZERO)
    }

    #[inline]
    fn balance(&mut self, address: Address) -> Option<(U256, bool)> {
        let account = Account::new_from_jzkt(address);
        Some((account.balance, true))
    }

    #[inline]
    fn code(&mut self, address: Address) -> Option<(Bytecode, bool)> {
        // TODO optimize using separate methods
        let account = Account::new_from_jzkt(address);
        let bytecode_bytes = Bytes::copy_from_slice(account.load_source_bytecode().as_ref());

        Some((Bytecode::new_raw(bytecode_bytes), false))
    }

    #[inline]
    fn code_hash(&mut self, address: Address) -> Option<(B256, bool)> {
        let account = Account::new_from_jzkt(address);
        Some((account.source_code_hash, false))
    }

    #[inline]
    fn sload(&mut self, address: Address, index: U256) -> Option<(U256, bool)> {
        let mut slot_value32 = Bytes32::default();
        let is_cold = _evm_sload::<CR>(
            &address,
            index.as_le_slice().as_ptr(),
            slot_value32.as_mut_ptr(),
        )
        .ok()?;
        debug_log(&format!(
            "ecl(sload): address={}, index={}, value={}",
            address,
            hex::encode(index.to_be_bytes::<32>().as_slice()),
            hex::encode(
                U256::from_le_bytes(slot_value32)
                    .to_be_bytes::<32>()
                    .as_slice()
            ),
        ));
        Some((U256::from_le_bytes(slot_value32), is_cold))
    }

    #[inline]
    fn sstore(&mut self, address: Address, index: U256, value: U256) -> Option<SStoreResult> {
        debug_log(&format!(
            "ecl(sstore): address={}, index={}, value={}",
            address,
            hex::encode(index.to_be_bytes::<32>().as_slice()),
            hex::encode(value.to_be_bytes::<32>().as_slice()),
        ));
        let mut previous = U256::default();
        _evm_sload::<CR>(&address, index.as_le_slice().as_ptr(), unsafe {
            previous.as_le_slice_mut().as_mut_ptr()
        })
        .ok()?;
        let is_cold = _evm_sstore::<CR>(
            &address,
            index.as_le_slice().as_ptr(),
            value.as_le_slice().as_ptr(),
        )
        .ok()?;
        return Some(SStoreResult {
            original_value: previous,
            present_value: previous,
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
    fn log(&mut self, log: Log) {
        LowLevelSDK::jzkt_emit_log(
            log.address.as_ptr(),
            // we can do such cast because B256 has transparent repr
            log.topics().as_ptr() as *const [u8; 32],
            log.topics().len() as u32 * 32,
            log.data.data.0.as_ptr(),
            log.data.data.0.len() as u32,
        );
    }

    #[inline]
    fn selfdestruct(&mut self, _address: Address, _target: Address) -> Option<SelfDestructResult> {
        panic!("SELFDESTRUCT opcode is not supported")
    }
}
