use crate::types::Gas;
use crate::types::{BytecodeType, CallOutcome, CreateOutcome, InterpreterResult};
use crate::{
    builder::{EvmBuilder, HandlerStage, SetGenericStage},
    db::{Database, DatabaseCommit, EmptyDB},
    handler::Handler,
    primitives::{
        specification::SpecId, Address, BlockEnv, CfgEnv, EVMError, EVMResult, Env,
        EnvWithHandlerCfg, ExecutionResult, HandlerCfg, ResultAndState, TransactTo, TxEnv, U256,
    },
    Context, ContextWithHandlerCfg, FrameResult,
};
use core::fmt;
use fluentbase_codec::Encoder;
use fluentbase_core::consts::{ECL_CONTRACT_ADDRESS, WCL_CONTRACT_ADDRESS};
use fluentbase_core::{Account, AccountCheckpoint};
use fluentbase_core_api::api::CoreInput;
use fluentbase_core_api::bindings::{
    EvmCreate2MethodInput, EvmCreateMethodInput, WasmCreate2MethodInput, WasmCreateMethodInput,
    EVM_CREATE2_METHOD_ID, EVM_CREATE_METHOD_ID, WASM_CREATE2_METHOD_ID, WASM_CREATE_METHOD_ID,
};
use fluentbase_sdk::evm::{Bytes, ContractInput};
use fluentbase_sdk::{LowLevelAPI, LowLevelSDK};
use fluentbase_types::{ExitCode, STATE_DEPLOY, STATE_MAIN};
use revm_primitives::{CreateScheme, State};

/// EVM call stack limit.
pub const CALL_STACK_LIMIT: u64 = 1024;

/// EVM instance containing both internal EVM context and external context
/// and the handler that dictates the logic of EVM (or hardfork specification).
pub struct Evm<'a, EXT, DB: Database> {
    /// Context of execution, containing both EVM and external context.
    pub context: Context<EXT, DB>,
    /// Handler of EVM that contains all the logic. Handler contains specification id
    /// and it different depending on the specified fork.
    pub handler: Handler<'a, EXT, DB>,
}

impl<EXT, DB> fmt::Debug for Evm<'_, EXT, DB>
where
    EXT: fmt::Debug,
    DB: Database + fmt::Debug,
    DB::Error: fmt::Debug,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Evm")
            .field("evm context", &self.context.evm)
            .finish_non_exhaustive()
    }
}

impl<EXT, DB: Database + DatabaseCommit> Evm<'_, EXT, DB> {
    /// Commit the changes to the database.
    pub fn transact_commit(&mut self) -> Result<ExecutionResult, EVMError<DB::Error>> {
        let ResultAndState { result, state } = self.transact()?;
        self.context.evm.db.commit(state);
        Ok(result)
    }
}

impl<'a> Evm<'a, (), EmptyDB> {
    /// Returns evm builder with empty database and empty external context.
    pub fn builder() -> EvmBuilder<'a, SetGenericStage, (), EmptyDB> {
        EvmBuilder::default()
    }
}

impl<'a, EXT, DB: Database> Evm<'a, EXT, DB> {
    /// Create new EVM.
    pub fn new(mut context: Context<EXT, DB>, handler: Handler<'a, EXT, DB>) -> Evm<'a, EXT, DB> {
        context.evm.inner.spec_id = handler.cfg.spec_id;
        Evm { context, handler }
    }

    /// Allow for evm setting to be modified by feeding current evm
    /// into the builder for modifications.
    pub fn modify(self) -> EvmBuilder<'a, HandlerStage, EXT, DB> {
        EvmBuilder::new(self)
    }
}

impl<EXT, DB: Database> Evm<'_, EXT, DB> {
    /// Returns specification (hardfork) that the EVM is instanced with.
    ///
    /// SpecId depends on the handler.
    pub fn spec_id(&self) -> SpecId {
        self.handler.cfg.spec_id
    }

    pub fn env(&self) -> &Env {
        &self.context.evm.env
    }

    /// Pre verify transaction by checking Environment, initial gas spend and if caller
    /// has enough balance to pay for the gas.
    #[inline]
    pub fn preverify_transaction(&mut self) -> Result<(), EVMError<DB::Error>> {
        self.handler.validation().env(&self.context.evm.env)?;
        self.handler
            .validation()
            .initial_tx_gas(&self.context.evm.env)?;
        self.handler
            .validation()
            .tx_against_state(&mut self.context)?;
        Ok(())
    }

    /// Transact pre-verified transaction
    ///
    /// This function will not validate the transaction.
    #[inline]
    pub fn transact_preverified(&mut self) -> EVMResult<DB::Error> {
        let initial_gas_spend = self
            .handler
            .validation()
            .initial_tx_gas(&self.context.evm.env)?;
        let output = self.transact_preverified_inner(initial_gas_spend);
        self.handler.post_execution().end(&mut self.context, output)
    }

    /// Returns the reference of handler configuration
    #[inline]
    pub fn handler_cfg(&self) -> &HandlerCfg {
        &self.handler.cfg
    }

    /// Returns the reference of Env configuration
    #[inline]
    pub fn cfg(&self) -> &CfgEnv {
        &self.context.evm.env.cfg
    }

    /// Returns the mutable reference of Env configuration
    #[inline]
    pub fn cfg_mut(&mut self) -> &mut CfgEnv {
        &mut self.context.evm.env.cfg
    }

    /// Returns the reference of transaction
    #[inline]
    pub fn tx(&self) -> &TxEnv {
        &self.context.evm.env.tx
    }

    /// Returns the mutable reference of transaction
    #[inline]
    pub fn tx_mut(&mut self) -> &mut TxEnv {
        &mut self.context.evm.env.tx
    }

    /// Returns the reference of database
    #[inline]
    pub fn db(&self) -> &DB {
        &self.context.evm.db
    }

    /// Returns the mutable reference of database
    #[inline]
    pub fn db_mut(&mut self) -> &mut DB {
        &mut self.context.evm.db
    }

    /// Returns the reference of block
    #[inline]
    pub fn block(&self) -> &BlockEnv {
        &self.context.evm.env.block
    }

    /// Returns the mutable reference of block
    #[inline]
    pub fn block_mut(&mut self) -> &mut BlockEnv {
        &mut self.context.evm.env.block
    }

    /// Transact transaction
    ///
    /// This function will validate the transaction.
    #[inline]
    pub fn transact(&mut self) -> EVMResult<DB::Error> {
        self.handler.validation().env(&self.context.evm.env)?;
        let initial_gas_spend = self
            .handler
            .validation()
            .initial_tx_gas(&self.context.evm.env)?;
        self.handler
            .validation()
            .tx_against_state(&mut self.context)?;

        let output = self.transact_preverified_inner(initial_gas_spend);
        self.handler.post_execution().end(&mut self.context, output)
    }

    /// Modify spec id, this will create new EVM that matches this spec id.
    pub fn modify_spec_id(&mut self, spec_id: SpecId) {
        self.handler.modify_spec_id(spec_id);
    }

    /// Returns internal database and external struct.
    #[inline]
    pub fn into_context(self) -> Context<EXT, DB> {
        self.context
    }

    /// Returns database and [`EnvWithHandlerCfg`].
    #[inline]
    pub fn into_db_and_env_with_handler_cfg(self) -> (DB, EnvWithHandlerCfg) {
        (
            self.context.evm.inner.db,
            EnvWithHandlerCfg {
                env: self.context.evm.inner.env,
                handler_cfg: self.handler.cfg,
            },
        )
    }

    /// Returns [Context] and [HandlerCfg].
    #[inline]
    pub fn into_context_with_handler_cfg(self) -> ContextWithHandlerCfg<EXT, DB> {
        ContextWithHandlerCfg::new(self.context, self.handler.cfg)
    }

    /// Transact pre-verified transaction.
    fn transact_preverified_inner(&mut self, initial_gas_spend: u64) -> EVMResult<DB::Error> {
        // TODO: "yes, we create empty jzkt here only for devnet purposes"
        let jzkt = LowLevelSDK::with_default_jzkt();

        let ctx = &mut self.context;
        let pre_exec = self.handler.pre_execution();

        // load access list and beneficiary if needed.
        pre_exec.load_accounts(ctx)?;

        // deduce caller balance with its limit.
        pre_exec.deduct_caller(ctx)?;

        let mut caller_account = Account::new_from_jzkt(&ctx.evm.env.tx.caller);

        let gas_limit = ctx.evm.env.tx.gas_limit - initial_gas_spend;

        // call inner handling of call/create
        let mut frame_result = match ctx.evm.env.tx.transact_to {
            TransactTo::Call(address) => {
                caller_account.inc_nonce().unwrap();
                let mut callee_account = Account::new_from_jzkt(&address);
                let value = ctx.evm.env.tx.value;
                let data = ctx.evm.env.tx.data.clone();
                let result = self.call_inner(
                    &mut caller_account,
                    &mut callee_account,
                    value,
                    data,
                    gas_limit,
                );
                FrameResult::Call(result)
            }
            TransactTo::Create(scheme) => {
                let salt = match scheme {
                    CreateScheme::Create2 { salt } => Some(salt),
                    CreateScheme::Create => None,
                };
                let value = ctx.evm.env.tx.value;
                let data = ctx.evm.env.tx.data.clone();
                let result = self.create_inner(&mut caller_account, value, data, gas_limit, salt);
                FrameResult::Create(result)
            }
        };

        let ctx = &mut self.context;

        // handle output of call/create calls.
        self.handler
            .execution()
            .last_frame_return(ctx, &mut frame_result)?;

        let post_exec = self.handler.post_execution();
        // Reimburse the caller
        post_exec.reimburse_caller(ctx, frame_result.gas())?;
        // Reward beneficiary
        post_exec.reward_beneficiary(ctx, frame_result.gas())?;
        // Returns output of transaction.
        match post_exec.output(ctx, frame_result) {
            Ok(mut result) => {
                let mut state: State = Default::default();
                for event in jzkt.borrow().journal() {
                    let address = Address::from_slice(&event.key()[12..]);
                    if !event.is_removed() {
                        let fields = event.preimage().unwrap().0;
                        state.insert(
                            address,
                            revm_primitives::Account {
                                info: Account::new_from_fields(&address, &fields).into(),
                                storage: Default::default(),
                                status: Default::default(),
                            },
                        );
                    } else {
                        state.remove(&address);
                    }
                }
                result.state = state;
                Ok(result)
            }
            Err(err) => Err(err),
        }
    }

    /// EVM create opcode for both initial crate and CREATE and CREATE2 opcodes.
    fn create_inner(
        &mut self,
        caller_account: &mut Account,
        value: U256,
        input: Bytes,
        gas_limit: u64,
        salt: Option<U256>,
    ) -> CreateOutcome {
        let return_result = |instruction_result: ExitCode, gas: Gas| CreateOutcome {
            result: InterpreterResult {
                result: instruction_result,
                output: Default::default(),
                gas,
            },
            address: None,
        };

        let mut gas = Gas::new(gas_limit);

        if self.context.evm.depth > CALL_STACK_LIMIT {
            return return_result(ExitCode::CallDepthOverflow, gas);
        } else if caller_account.balance < value {
            return return_result(ExitCode::InsufficientBalance, gas);
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

        let created_address = if exit_code == ExitCode::Ok {
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

        CreateOutcome {
            result: InterpreterResult {
                result: exit_code,
                output: Bytes::new(),
                gas,
            },
            address: created_address,
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
    ) -> CallOutcome {
        let return_result = |instruction_result: ExitCode, gas: Gas| CallOutcome {
            result: InterpreterResult {
                result: instruction_result,
                output: Default::default(),
                gas,
            },
            memory_offset: Default::default(),
        };

        let mut gas = Gas::new(gas_limit);

        // check call stack limit
        if self.context.evm.depth > CALL_STACK_LIMIT {
            return return_result(ExitCode::CallDepthOverflow, gas);
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

        let ret = CallOutcome {
            result: InterpreterResult {
                result: exit_code,
                output: output_buffer.into(),
                gas,
            },
            memory_offset: Default::default(),
        };

        // revert changes or not
        if exit_code != ExitCode::Ok {
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
            env_chain_id: self.context.evm.env.cfg.chain_id,
            contract_gas_limit: gas.remaining(),
            contract_address: callee.address,
            contract_caller: caller.address,
            contract_input: input,
            contract_value: value,
            contract_is_static: false,
            block_coinbase: self.context.evm.env.block.coinbase,
            block_timestamp: self.context.evm.env.block.timestamp.as_limbs()[0],
            block_number: self.context.evm.env.block.number.as_limbs()[0],
            block_difficulty: self.context.evm.env.block.difficulty.as_limbs()[0],
            block_gas_limit: self.context.evm.env.block.gas_limit.as_limbs()[0],
            block_base_fee: self.context.evm.env.block.basefee,
            tx_gas_price: self.context.evm.env.tx.gas_price,
            tx_gas_priority_fee: self.context.evm.env.tx.gas_priority_fee,
            tx_caller: self.context.evm.env.tx.caller,
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
    ) -> (Bytes, ExitCode) {
        let input = self
            .input_from_env(checkpoint, gas, caller, callee, input, value)
            .encode_to_vec(0);

        let mut gas_limit_ref = gas.remaining() as u32;
        let gas_limit_ref = &mut gas_limit_ref as *mut u32;
        let exit_code = LowLevelSDK::sys_exec_hash(
            callee.rwasm_code_hash.as_ptr(),
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

        let exit_code = match exit_code {
            0 => ExitCode::Ok,
            _ => ExitCode::ExecutionHalted,
        };

        (output_buffer.into(), exit_code)
    }
}
