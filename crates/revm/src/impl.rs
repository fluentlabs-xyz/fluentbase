use crate::{
    gas::Gas,
    handler::Handler,
    types::{BytecodeType, CallCreateResult},
    EVMData,
};
use alloc::rc::Rc;
use core::marker::PhantomData;
use fluentbase_codec::Encoder;
use fluentbase_core::{
    consts::{ECL_CONTRACT_ADDRESS, WCL_CONTRACT_ADDRESS},
    Account, AccountCheckpoint,
};
use fluentbase_core_api::{
    api::CoreInput,
    bindings::{
        EvmCreate2MethodInput, EvmCreateMethodInput, WasmCreate2MethodInput, WasmCreateMethodInput,
        EVM_CREATE2_METHOD_ID, EVM_CREATE_METHOD_ID, WASM_CREATE2_METHOD_ID, WASM_CREATE_METHOD_ID,
    },
};
use fluentbase_sdk::{evm::ContractInput, LowLevelAPI, LowLevelSDK};
use fluentbase_types::{Address, Bytes, ExitCode, STATE_DEPLOY, STATE_MAIN, U256};
use revm_primitives::{
    CreateScheme, EVMError, EVMResult, Env, Output, Spec, SpecId::*, TransactTo,
};

/// EVM call stack limit.
pub const CALL_STACK_LIMIT: u64 = 1024;

pub struct EVMImpl<'a, GSPEC: Spec> {
    pub data: EVMData<'a>,
    pub handler: Handler,
    depth: u64,
    _pd: PhantomData<GSPEC>,
}

/// EVM transaction interface.
#[auto_impl::auto_impl(&mut, Box)]
pub trait Transact<DBError> {
    /// Run checks that could make transaction fail before call/create.
    fn preverify_transaction(&mut self) -> Result<(), EVMError<DBError>>;

    /// Skip pre-verification steps and execute the transaction.
    fn transact_preverified(&mut self) -> EVMResult<DBError>;

    /// Execute transaction by running pre-verification steps and then transaction itself.
    fn transact(&mut self) -> EVMResult<DBError>;
}

impl<'a, GSPEC: Spec + 'static> Transact<ExitCode> for EVMImpl<'a, GSPEC> {
    #[inline]
    fn preverify_transaction(&mut self) -> Result<(), EVMError<ExitCode>> {
        self.preverify_transaction_inner()
    }

    #[inline]
    fn transact_preverified(&mut self) -> EVMResult<ExitCode> {
        let output = self.transact_preverified_inner();
        self.handler.end(&mut self.data, output)
    }

    #[inline]
    fn transact(&mut self) -> EVMResult<ExitCode> {
        let output = self
            .preverify_transaction_inner()
            .and_then(|()| self.transact_preverified_inner());
        self.handler.end(&mut self.data, output)
    }
}

impl<'a, GSPEC: Spec + 'static> EVMImpl<'a, GSPEC> {
    pub fn new(env: &'a mut Env) -> Self {
        Self {
            data: EVMData { env },
            handler: Handler::mainnet::<GSPEC>(),
            _pd: PhantomData {},
            depth: 0,
        }
    }

    /// Pre verify transaction.
    pub fn preverify_transaction_inner(&mut self) -> Result<(), EVMError<ExitCode>> {
        // Important: validate block before tx.
        self.data.env.validate_block_env::<GSPEC>()?;
        self.data.env.validate_tx::<GSPEC>()?;

        // load acc
        // TODO: "warmup and verify tx caller?"
        // let tx_caller = env.tx.caller;
        // let (caller_account, _) = self
        //     .data
        //     .journaled_state
        //     .load_account(tx_caller, self.data.db)
        //     .map_err(EVMError::Database)?;
        //
        // self.data
        //     .env
        //     .validate_tx_against_state(caller_account)
        //     .map_err(Into::into)
        Ok(())
    }

    /// Transact preverified transaction.
    pub fn transact_preverified_inner(&mut self) -> EVMResult<ExitCode> {
        let env = &self.data.env;
        let tx_caller = env.tx.caller;
        let tx_value = env.tx.value;
        let tx_data = env.tx.data.clone();
        let tx_gas_limit = env.tx.gas_limit;
        let block_coinbase = env.block.coinbase;

        // load coinbase
        // EIP-3651: Warm COINBASE. Starts the `COINBASE` address warm
        if GSPEC::enabled(SHANGHAI) {
            // TODO: "warmup coinbase"
        }
        // TODO: "warmup access list"

        let mut caller_account = Account::new_from_jzkt(&tx_caller);

        // Subtract gas costs from the caller's account.
        // We need to saturate the gas cost to prevent underflow in case that
        // `disable_balance_check` is enabled.
        let mut gas_cost =
            U256::from(tx_gas_limit).saturating_mul(self.data.env.effective_gas_price());

        // EIP-4844
        if GSPEC::enabled(CANCUN) {
            let data_fee = self.data.env.calc_data_fee().expect("already checked");
            gas_cost = gas_cost.saturating_add(data_fee);
        }
        caller_account.sub_balance_saturating(gas_cost);

        let transact_gas_limit = tx_gas_limit;

        // call inner handling of call/create
        let (call_result, ret_gas, output) = match self.data.env.tx.transact_to {
            TransactTo::Call(address) => {
                caller_account.inc_nonce()?;
                let mut callee_account = Account::new_from_jzkt(&address);
                let result = self.call_inner(
                    &mut caller_account,
                    &mut callee_account,
                    tx_value,
                    tx_data,
                    transact_gas_limit,
                );
                (result.result, result.gas, Output::Call(result.return_value))
            }
            TransactTo::Create(scheme) => {
                let salt = match scheme {
                    CreateScheme::Create2 { salt } => Some(salt),
                    CreateScheme::Create => None,
                };
                let result = self.create_inner(
                    &mut caller_account,
                    tx_value,
                    tx_data,
                    transact_gas_limit,
                    salt,
                );
                (
                    result.result,
                    result.gas,
                    Output::Create(result.return_value, result.created_address),
                )
            }
        };

        let handler = &self.handler;
        let data = &mut self.data;

        // handle output of call/create calls.
        let mut gas = handler.call_return(data.env, call_result, ret_gas);

        // set refund. Refund amount depends on hardfork.
        gas.set_refund(handler.calculate_gas_refund(data.env, &gas) as i64);

        // Reimburse the caller
        let effective_gas_price = data.env.effective_gas_price();
        caller_account.add_balance_saturating(
            effective_gas_price * U256::from(gas.remaining() + gas.refunded() as u64),
        );

        // Reward beneficiary
        if !data.env.cfg.is_beneficiary_reward_disabled() {
            let mut coinbase_account = Account::new_from_jzkt(&block_coinbase);
            let effective_gas_price = data.env.effective_gas_price();
            // EIP-1559 discard basefee for coinbase transfer. Basefee amount of gas is discarded.
            let coinbase_gas_price = if GSPEC::enabled(LONDON) {
                effective_gas_price.saturating_sub(data.env.block.basefee)
            } else {
                effective_gas_price
            };
            coinbase_account.add_balance_saturating(
                coinbase_gas_price * U256::from(gas.spend() - gas.refunded() as u64),
            );
        }

        // main return
        handler.main_return(data, call_result, output, &gas)
    }

    /// EVM create opcode for both initial crate and CREATE and CREATE2 opcodes.
    fn create_inner(
        &mut self,
        caller_account: &mut Account,
        value: U256,
        input: Bytes,
        gas_limit: u64,
        salt: Option<U256>,
    ) -> CallCreateResult {
        let mut gas = Gas::new(gas_limit);
        if self.depth > CALL_STACK_LIMIT {
            return CallCreateResult::from_error(ExitCode::CallDepthOverflow, gas);
        } else if caller_account.balance < value {
            return CallCreateResult::from_error(ExitCode::InsufficientBalance, gas);
        }

        let checkpoint = LowLevelSDK::jzkt_checkpoint();

        let (mut middleware_account, core_input) = match BytecodeType::from_slice(input.as_ref()) {
            BytecodeType::EVM => {
                let method_id = match salt {
                    Some(_) => EVM_CREATE2_METHOD_ID,
                    None => EVM_CREATE_METHOD_ID,
                };
                let method_data = match salt {
                    Some(salt) => EvmCreate2MethodInput {
                        value32: value.to_be_bytes(),
                        salt32: salt.to_be_bytes(),
                        code: input.to_vec(),
                        gas_limit: gas.remaining() as u32,
                    }
                    .encode_to_vec(0),
                    None => EvmCreateMethodInput {
                        value32: value.to_be_bytes(),
                        code: input.to_vec(),
                        gas_limit: gas.remaining() as u32,
                    }
                    .encode_to_vec(0),
                };
                let input = CoreInput {
                    method_id,
                    method_data,
                };
                (
                    Account::new_from_jzkt(&ECL_CONTRACT_ADDRESS),
                    input.encode_to_vec(0),
                )
            }
            BytecodeType::WASM => {
                let method_id = match salt {
                    Some(_) => WASM_CREATE2_METHOD_ID,
                    None => WASM_CREATE_METHOD_ID,
                };
                let method_data = match salt {
                    Some(salt) => WasmCreate2MethodInput {
                        value32: value.to_be_bytes(),
                        salt32: salt.to_be_bytes(),
                        code: input.to_vec(),
                        gas_limit: gas.remaining() as u32,
                    }
                    .encode_to_vec(0),
                    None => WasmCreateMethodInput {
                        value32: value.to_be_bytes(),
                        code: input.to_vec(),
                        gas_limit: gas.remaining() as u32,
                    }
                    .encode_to_vec(0),
                };
                let input = CoreInput {
                    method_id,
                    method_data,
                };
                (
                    Account::new_from_jzkt(&WCL_CONTRACT_ADDRESS),
                    input.encode_to_vec(0),
                )
            }
        };

        let (output_buffer, exit_code) = self.exec_rwasm_binary(
            checkpoint,
            &mut gas,
            caller_account,
            &mut middleware_account,
            core_input.into(),
            value,
            STATE_DEPLOY,
        );

        let created_address = if exit_code == ExitCode::Ok.into_i32() {
            assert_eq!(
                output_buffer.len(),
                20,
                "output buffer is not 20 bytes after create/create2"
            );
            Some(Address::from_slice(output_buffer.as_ref()))
        } else {
            LowLevelSDK::jzkt_rollback(checkpoint);
            None
        };

        CallCreateResult {
            result: exit_code,
            created_address,
            gas,
            return_value: Bytes::new(),
        }
    }

    /// Main contract call of the EVM.
    fn call_inner(
        &mut self,
        caller_account: &mut Account,
        callee_account: &mut Account,
        value: U256,
        input: Bytes,
        gas_limit: u64,
    ) -> CallCreateResult {
        let mut gas = Gas::new(gas_limit);

        // check call stack limit
        if self.depth > CALL_STACK_LIMIT {
            return CallCreateResult::from_error(ExitCode::CallDepthOverflow, gas);
        }

        let checkpoint = Account::checkpoint();

        let (output_buffer, exit_code) = self.exec_rwasm_binary(
            checkpoint,
            &mut gas,
            caller_account,
            callee_account,
            input,
            value,
            STATE_MAIN,
        );

        let ret = CallCreateResult {
            result: exit_code,
            created_address: None,
            gas,
            return_value: output_buffer.into(),
        };

        // revert changes or not
        if exit_code != ExitCode::Ok.into_i32() {
            Account::rollback(checkpoint);
        }

        ret
    }

    fn input_from_env(
        &self,
        checkpoint: AccountCheckpoint,
        gas: &Gas,
        caller: &mut Account,
        callee: &mut Account,
        input: Bytes,
        value: U256,
    ) -> ContractInput {
        ContractInput {
            journal_checkpoint: checkpoint,
            env_chain_id: self.data.env.cfg.chain_id,
            contract_gas_limit: gas.remaining(),
            contract_address: callee.address,
            contract_caller: caller.address,
            contract_input: input,
            contract_value: value,
            contract_is_static: false,
            block_coinbase: self.data.env.block.coinbase,
            block_timestamp: self.data.env.block.timestamp.as_limbs()[0],
            block_number: self.data.env.block.number.as_limbs()[0],
            block_difficulty: self.data.env.block.difficulty.as_limbs()[0],
            block_gas_limit: self.data.env.block.gas_limit.as_limbs()[0],
            block_base_fee: self.data.env.block.basefee,
            tx_gas_price: self.data.env.tx.gas_price,
            tx_gas_priority_fee: self.data.env.tx.gas_priority_fee,
            tx_caller: self.data.env.tx.caller,
        }
    }

    fn exec_rwasm_binary(
        &mut self,
        checkpoint: AccountCheckpoint,
        gas: &mut Gas,
        caller: &mut Account,
        callee: &mut Account,
        input: Bytes,
        value: U256,
        state: u32,
    ) -> (Bytes, i32) {
        let input = self
            .input_from_env(checkpoint, gas, caller, callee, input, value)
            .encode_to_vec(0);

        let mut gas_limit_ref = gas.remaining() as u32;
        let gas_limit_ref = &mut gas_limit_ref as *mut u32;
        let exit_code = LowLevelSDK::sys_exec_hash(
            callee.rwasm_bytecode_hash.as_ptr(),
            input.as_ptr(),
            input.len() as u32,
            core::ptr::null_mut(),
            0,
            gas_limit_ref,
            state,
        );
        let gas_used = gas.remaining() - unsafe { *gas_limit_ref } as u64;
        gas.record_cost(gas_used);

        let output_size = LowLevelSDK::sys_output_size();
        let mut output_buffer = vec![0u8; output_size as usize];
        LowLevelSDK::sys_read_output(output_buffer.as_mut_ptr(), 0, output_size);

        (output_buffer.into(), exit_code)
    }
}
