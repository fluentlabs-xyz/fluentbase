use crate::{
    account::Account,
    account_types::MAX_CODE_SIZE,
    evm::{sload::_evm_sload, sstore::_evm_sstore},
};
use alloc::{vec, vec::Vec};
use fluentbase_sdk::{evm::ExecutionContext, Bytes32, LowLevelAPI, LowLevelSDK};
use fluentbase_types::ExitCode;
use revm_interpreter::{
    primitives::{
        Address,
        AnalysisKind,
        BlockEnv,
        Bytecode,
        Bytes,
        CfgEnv,
        Env,
        Log,
        TransactTo,
        TxEnv,
        B256,
        U256,
    },
    Host,
    SelfDestructResult,
};

#[derive(Debug)]
pub struct FluentHost {
    env: Env,
    need_to_init_env: bool,
}

impl Default for FluentHost {
    fn default() -> Self {
        let env = Default::default();
        Self {
            env,
            need_to_init_env: true,
            // _phantom: Default::default(),
        }
    }
}

impl FluentHost {
    // #[inline]
    // pub fn new(env: Env) -> Self {
    //     Self {
    //         env: Rc::new(RefCell::new(Some(&env))),
    //         ..Default::default()
    //     }
    // }

    #[inline]
    pub fn clear(&mut self) {}

    fn env_from_context() -> Env {
        let mut cfg_env = CfgEnv::default();
        cfg_env.chain_id = ExecutionContext::env_chain_id();
        cfg_env.perf_analyse_created_bytecodes = AnalysisKind::Raw; // do not analyze
        cfg_env.limit_contract_code_size = Some(MAX_CODE_SIZE as usize); // do not analyze
        Env {
            cfg: cfg_env,
            block: BlockEnv {
                number: U256::from_be_slice(
                    ExecutionContext::block_number().to_be_bytes().as_slice(),
                ),
                coinbase: Address::from_slice(ExecutionContext::block_coinbase().as_ref()),
                timestamp: U256::from_be_slice(
                    ExecutionContext::block_timestamp().to_be_bytes().as_slice(),
                ),
                gas_limit: U256::from_be_slice(
                    ExecutionContext::block_gas_limit().to_be_bytes().as_slice(),
                ),
                basefee: ExecutionContext::block_base_fee(),
                difficulty: U256::from_be_slice(
                    ExecutionContext::block_difficulty()
                        .to_be_bytes()
                        .as_slice(),
                ),
                prevrandao: None,
                blob_excess_gas_and_price: None,
            },
            tx: TxEnv {
                caller: Address::from_slice(ExecutionContext::tx_caller().as_ref()),
                gas_limit: Default::default(),
                gas_price: ExecutionContext::tx_gas_price(),
                transact_to: TransactTo::Call(Address::ZERO), // will do nothing
                value: ExecutionContext::contract_value(),
                data: Default::default(), // no data?
                nonce: None,              // no checks
                chain_id: None,           // no checks
                access_list: vec![],
                gas_priority_fee: None,
                blob_hashes: vec![],
                max_fee_per_blob_gas: None,
            },
        }
    }

    fn init_from_context(env: &mut Env) {
        *env = Self::env_from_context();
    }
}

impl Host for FluentHost {
    fn env(&self) -> &Env {
        if self.need_to_init_env {
            #[allow(mutable_transmutes)]
            let self_mut: &mut FluentHost = unsafe { core::mem::transmute(&self) };
            Self::init_from_context(&mut self_mut.env);
            self_mut.need_to_init_env = false;
        }
        &self.env
    }

    fn env_mut(&mut self) -> &mut Env {
        if self.need_to_init_env {
            Self::init_from_context(&mut self.env);
            self.need_to_init_env = false;
        }
        &mut self.env
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
        let account = Account::new_from_jzkt(&fluentbase_types::Address::new(address.into_array()));

        Some((account.balance, false))
    }

    #[inline]
    fn code(&mut self, address: Address) -> Option<(Bytecode, bool)> {
        // TODO optimize using separate methods
        let account = Account::new_from_jzkt(&fluentbase_types::Address::new(address.into_array()));
        let bytecode_bytes = Bytes::copy_from_slice(account.load_source_bytecode().as_ref());

        Some((Bytecode::new_raw(bytecode_bytes), false))
    }

    #[inline]
    fn code_hash(&mut self, address: Address) -> Option<(B256, bool)> {
        // TODO optimize using separate methods
        let account = Account::new_from_jzkt(&fluentbase_types::Address::new(address.into_array()));
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
