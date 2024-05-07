use crate::types::{bytecode_type_from_account, SStoreResult, SelfDestructResult};
use crate::{
    builder::{EvmBuilder, HandlerStage, SetGenericStage},
    db::{Database, DatabaseCommit, EmptyDB},
    gas::Gas,
    handler::Handler,
    interpreter::{CallOutcome, CreateOutcome, InterpreterResult},
    primitives::{
        specification::SpecId, Address, BlockEnv, CfgEnv, EVMError, EVMResult, EnvWithHandlerCfg,
        ExecutionResult, HandlerCfg, ResultAndState, TransactTo, TxEnv, B256, U256,
    },
    Context, ContextWithHandlerCfg, EvmContext, FrameResult, JournalCheckpoint, JournalEntry,
};
use core::{cell::RefCell, fmt, str::from_utf8};
use fluentbase_codec::{BufferDecoder, Encoder};
use fluentbase_core::evm::call::_evm_call;
use fluentbase_core::evm::sload::_evm_sload;
use fluentbase_core::evm::sstore::_evm_sstore;
use fluentbase_core::fluent_host::FluentHost;
use fluentbase_core::helpers::calc_storage_key;
use fluentbase_core::wasm::call::_wasm_call;
use fluentbase_core::{
    consts::{ECL_CONTRACT_ADDRESS, WCL_CONTRACT_ADDRESS},
    evm::create::_evm_create,
    wasm::create::_wasm_create,
};
use fluentbase_sdk::{
    Account, AccountCheckpoint, AccountManager, ContractInput, CoreInput, EvmCallMethodInput,
    EvmCallMethodOutput, EvmCreateMethodInput, EvmCreateMethodOutput, LowLevelSDK,
    WasmCallMethodInput, WasmCreateMethodInput, EVM_CALL_METHOD_ID, EVM_CREATE_METHOD_ID,
    JZKT_ACCOUNT_COMPRESSION_FLAGS, JZKT_ACCOUNT_FIELDS_COUNT, JZKT_ACCOUNT_RWASM_CODE_HASH_FIELD,
    JZKT_ACCOUNT_SOURCE_CODE_HASH_FIELD, JZKT_STORAGE_COMPRESSION_FLAGS, JZKT_STORAGE_FIELDS_COUNT,
    WASM_CALL_METHOD_ID, WASM_CREATE_METHOD_ID,
};
use fluentbase_types::{
    address, BytecodeType, Bytes, Bytes32, ExitCode, IJournaledTrie, JournalEvent, JournalLog,
    NATIVE_TRANSFER_ADDRESS, NATIVE_TRANSFER_KECCAK, POSEIDON_EMPTY, STATE_MAIN,
};
use revm_primitives::{hex, Bytecode, CreateScheme, Env, Log, LogData};
use std::vec::Vec;

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
    ExitCode: fmt::Debug,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Evm")
            .field("evm context", &self.context.evm)
            .finish_non_exhaustive()
    }
}

impl<EXT, DB: Database + DatabaseCommit> Evm<'_, EXT, DB> {
    /// Commit the changes to the database.
    pub fn transact_commit(&mut self) -> Result<ExecutionResult, EVMError<ExitCode>> {
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
        context.evm.journaled_state.set_spec_id(handler.cfg.spec_id);
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

    /// Pre verify transaction by checking Environment, initial gas spend and if caller
    /// has enough balance to pay for the gas.
    #[inline]
    pub fn preverify_transaction(&mut self) -> Result<(), EVMError<ExitCode>> {
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
    pub fn transact_preverified(&mut self) -> EVMResult<ExitCode> {
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
    pub fn transact(&mut self) -> EVMResult<ExitCode> {
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
    fn transact_preverified_inner(&mut self, initial_gas_spend: u64) -> EVMResult<ExitCode> {
        let ctx = &mut self.context;
        let pre_exec = self.handler.pre_execution();

        // load access list and beneficiary if needed.
        pre_exec.load_accounts(ctx)?;

        // load precompiles
        let precompiles = pre_exec.load_precompiles();
        ctx.evm.set_precompiles(precompiles);

        // deduce caller balance with its limit.
        pre_exec.deduct_caller(ctx)?;

        let gas_limit = ctx.evm.env.tx.gas_limit - initial_gas_spend;

        let mut caller_account = ctx.evm.load_jzkt_account(ctx.evm.env.tx.caller)?;

        // Load EVM storage account
        let (evm_storage, _) = ctx.evm.load_account(EVM_STORAGE_ADDRESS)?;
        evm_storage.info.nonce = 1;
        ctx.evm.touch(&EVM_STORAGE_ADDRESS);

        // call inner handling of call/create
        let mut frame_result = match ctx.evm.env.tx.transact_to {
            TransactTo::Call(address) => {
                let mut callee_account = ctx.evm.load_jzkt_account(address)?;
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
        post_exec.output(ctx, frame_result)
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

        if self.context.evm.journaled_state.depth as u64 > CALL_STACK_LIMIT {
            return return_result(ExitCode::CallDepthOverflow, gas);
        } else if caller_account.balance < value {
            return return_result(ExitCode::InsufficientBalance, gas);
        }

        let (_middleware_account, method_id, method_data) =
            match BytecodeType::from_slice(input.as_ref()) {
                BytecodeType::EVM => (
                    ECL_CONTRACT_ADDRESS,
                    EVM_CREATE_METHOD_ID,
                    EvmCreateMethodInput {
                        bytecode: input,
                        value,
                        gas_limit: gas.remaining(),
                        salt,
                    },
                ),
                BytecodeType::WASM => (
                    WCL_CONTRACT_ADDRESS,
                    WASM_CREATE_METHOD_ID,
                    WasmCreateMethodInput {
                        bytecode: input,
                        value,
                        gas_limit: gas.remaining(),
                        salt,
                    },
                ),
            };

        let contract_input = self.input_from_env(
            &mut gas,
            caller_account,
            Address::ZERO,
            Default::default(),
            value,
        );
        let am = JournalDbWrapper {
            ctx: RefCell::new(&mut self.context.evm),
        };
        let create_output = match method_id {
            EVM_CREATE_METHOD_ID => _evm_create(&contract_input, &am, method_data),
            WASM_CREATE_METHOD_ID => _wasm_create(&contract_input, &am, method_data),
            _ => unreachable!(),
        };

        // let (output_buffer, exit_code) = self.exec_rwasm_binary(
        //     &mut gas,
        //     caller_account,
        //     &mut middleware_account,
        //     None,
        //     core_input.into(),
        //     value,
        // );

        // let create_output = if exit_code == ExitCode::Ok {
        //     let mut buffer_decoder = BufferDecoder::new(output_buffer.as_ref());
        //     let mut create_output = EvmCreateMethodOutput::default();
        //     EvmCreateMethodOutput::decode_body(&mut buffer_decoder, 0, &mut create_output);
        //     create_output
        // } else {
        //     EvmCreateMethodOutput::from_exit_code(exit_code).with_gas(gas.remaining())
        // };

        // let created_address = if exit_code == ExitCode::Ok {
        //     if output_buffer.len() != 20 {
        //         return return_result(ExitCode::CreateError, gas);
        //     }
        //     assert_eq!(
        //         output_buffer.len(),
        //         20,
        //         "output buffer is not 20 bytes after create/create2"
        //     );
        //     Some(Address::from_slice(output_buffer.as_ref()))
        // } else {
        //     None
        // };

        CreateOutcome {
            result: InterpreterResult {
                result: ExitCode::from(create_output.exit_code),
                output: Bytes::new(),
                gas: Gas::new(create_output.gas),
            },
            address: create_output.address,
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
        let mut gas = Gas::new(gas_limit);

        // Touch address. For "EIP-158 State Clear", this will erase empty accounts.
        if value == U256::ZERO {
            self.context
                .evm
                .load_account(callee_account.address)
                .expect("failed to load");
            self.context
                .evm
                .journaled_state
                .touch(&callee_account.address);
        }

        self.context.evm.touch(&caller_account.address);
        self.context.evm.touch(&callee_account.address);

        let (callee_bytecode, _) = self
            .context
            .evm
            .load_account(callee_account.address)
            .unwrap();

        let (_middleware_account, method_id, method_data) =
            match bytecode_type_from_account(&callee_bytecode.info) {
                BytecodeType::EVM => (
                    ECL_CONTRACT_ADDRESS,
                    EVM_CALL_METHOD_ID,
                    EvmCallMethodInput {
                        callee: callee_account.address,
                        value,
                        input,
                        gas_limit: gas.remaining(),
                    },
                ),
                BytecodeType::WASM => (
                    WCL_CONTRACT_ADDRESS,
                    WASM_CALL_METHOD_ID,
                    WasmCallMethodInput {
                        callee: callee_account.address,
                        value,
                        input,
                        gas_limit: gas.remaining(),
                    },
                ),
            };

        let contract_input = self.input_from_env(
            &mut gas,
            caller_account,
            callee_account.address,
            Default::default(),
            value,
        );
        let am = JournalDbWrapper {
            ctx: RefCell::new(&mut self.context.evm),
        };
        let call_output = match method_id {
            EVM_CALL_METHOD_ID => _evm_call(&contract_input, &am, method_data),
            WASM_CALL_METHOD_ID => _wasm_call(&contract_input, &am, method_data),
            _ => unreachable!(),
        };

        // let core_input = CoreInput {
        //     method_id,
        //     method_data,
        // }
        // .encode_to_vec(0);
        //
        // let (output_buffer, exit_code) = self.exec_rwasm_binary(
        //     &mut gas,
        //     caller_account,
        //     &mut middleware_account,
        //     Some(callee_account.address),
        //     core_input.into(),
        //     value,
        // );
        //
        // let call_output = if exit_code == ExitCode::Ok {
        //     let mut buffer_decoder = BufferDecoder::new(output_buffer.as_ref());
        //     let mut call_output = EvmCallMethodOutput::default();
        //     EvmCallMethodOutput::decode_body(&mut buffer_decoder, 0, &mut call_output);
        //     call_output
        // } else {
        //     EvmCallMethodOutput::from_exit_code(exit_code).with_gas(gas.remaining())
        // };

        {
            println!("executed ECL call:");
            println!(" - caller: 0x{}", hex::encode(caller_account.address));
            println!(" - callee: 0x{}", hex::encode(callee_account.address));
            println!(" - value: 0x{}", hex::encode(&value.to_be_bytes::<32>()));
            println!(
                " - fuel consumed: {}",
                gas.remaining() as i64 - call_output.gas as i64
            );
            println!(" - exit code: {}", call_output.exit_code);
            if call_output.output.iter().all(|c| c.is_ascii()) {
                println!(
                    " - output message: {}",
                    from_utf8(&call_output.output).unwrap()
                );
            } else {
                println!(
                    " - output message: {}",
                    format!("0x{}", hex::encode(&call_output.output))
                );
            }
        }

        CallOutcome {
            result: InterpreterResult {
                result: ExitCode::from(call_output.exit_code),
                output: call_output.output,
                gas: Gas::new(call_output.gas),
            },
            memory_offset: Default::default(),
        }
    }

    fn input_from_env(
        &self,
        gas: &Gas,
        caller: &mut Account,
        callee_address: Address,
        input: Bytes,
        value: U256,
    ) -> ContractInput {
        ContractInput {
            journal_checkpoint: 0,
            contract_gas_limit: gas.remaining(),
            contract_address: callee_address,
            contract_caller: caller.address,
            contract_input: input,
            contract_value: value,
            contract_is_static: false,
            block_chain_id: self.context.evm.env.cfg.chain_id,
            block_coinbase: self.context.evm.env.block.coinbase,
            block_timestamp: self.context.evm.env.block.timestamp.as_limbs()[0],
            block_number: self.context.evm.env.block.number.as_limbs()[0],
            block_difficulty: self.context.evm.env.block.difficulty.as_limbs()[0],
            block_gas_limit: self.context.evm.env.block.gas_limit.as_limbs()[0],
            block_base_fee: self.context.evm.env.block.basefee,
            tx_gas_limit: self.context.evm.env.tx.gas_limit,
            tx_nonce: self.context.evm.env.tx.nonce.unwrap_or_default(),
            tx_gas_price: self.context.evm.env.tx.gas_price,
            tx_gas_priority_fee: self.context.evm.env.tx.gas_priority_fee,
            tx_caller: self.context.evm.env.tx.caller,
            tx_access_list: self.context.evm.env.tx.access_list.clone(),
        }
    }

    // #[cfg(feature = "std")]
    // fn exec_rwasm_binary(
    //     &mut self,
    //     gas: &mut Gas,
    //     caller: &mut Account,
    //     middleware: &mut Account,
    //     callee_address: Option<Address>,
    //     input: Bytes,
    //     value: U256,
    // ) -> (Bytes, ExitCode) {
    //     use fluentbase_runtime::{Runtime, RuntimeContext};
    //     let input = self
    //         .input_from_env(
    //             gas,
    //             caller,
    //             callee_address.unwrap_or(Address::ZERO),
    //             input,
    //             value,
    //         )
    //         .encode_to_vec(0);
    //     let jzkt = JournalDbWrapper {
    //         ctx: RefCell::new(&mut self.context.evm),
    //     };
    //     let rwasm_bytecode = jzkt.preimage(&middleware.rwasm_code_hash.0);
    //     if rwasm_bytecode.is_empty() {
    //         return (Bytes::default(), ExitCode::Ok);
    //     }
    //     let ctx = RuntimeContext::new(rwasm_bytecode)
    //         .with_input(input)
    //         .with_fuel_limit(gas.remaining())
    //         .with_jzkt(jzkt)
    //         .with_state(STATE_MAIN);
    //     let mut runtime = Runtime::new(ctx);
    //     let result = match runtime.call() {
    //         Ok(result) => result,
    //         Err(err) => {
    //             let exit_code = Runtime::catch_trap(&err);
    //             println!("execution failed with err: {:?}", err);
    //             return (Bytes::default(), ExitCode::from(exit_code));
    //         }
    //     };
    //     gas.record_cost(result.fuel_consumed);
    //     (Bytes::from(result.output.clone()), result.exit_code.into())
    // }

    #[cfg(not(feature = "std"))]
    fn exec_rwasm_binary(
        &mut self,
        checkpoint: AccountCheckpoint,
        gas: &mut Gas,
        caller: &mut Account,
        callee: &mut Account,
        input: Bytes,
        value: U256,
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

struct JournalDbWrapper<'a, DB: Database> {
    ctx: RefCell<&'a mut EvmContext<DB>>,
}

/// A special account for storing EVM storage trie `keccak256("evm_storage_trie")[12..32]`
pub const EVM_STORAGE_ADDRESS: Address = address!("fabefeab43f96e51d7ace194b9abd33305bb6bfb");

impl<'a, DB: Database> IJournaledTrie for JournalDbWrapper<'a, DB> {
    fn checkpoint(&self) -> fluentbase_types::JournalCheckpoint {
        let mut ctx = self.ctx.borrow_mut();
        let (a, b) = ctx.journaled_state.checkpoint().into();
        fluentbase_types::JournalCheckpoint::from((a, b))
    }

    fn get(&self, key: &[u8; 32]) -> Option<(Vec<[u8; 32]>, u32, bool)> {
        let mut ctx = self.ctx.borrow_mut();
        // if first 12 bytes are empty then its account load otherwise storage
        if key[..12] == [0u8; 12] {
            let address = Address::from_slice(&key[12..]);
            let (account, _) = ctx
                .load_account_with_code(address)
                .expect("can't load account");
            let account = Account::from(account.info.clone());
            Some((
                account.get_fields().to_vec(),
                JZKT_ACCOUNT_COMPRESSION_FLAGS,
                false,
            ))
        } else {
            ctx.sload(EVM_STORAGE_ADDRESS, U256::from_le_bytes(*key))
                .ok()
                .map(|(value, is_cold)| {
                    // println!(
                    //     "reading storage value: slot={}, value={}",
                    //     hex::encode(U256::from_le_bytes(*key).to_be_bytes::<32>()),
                    //     hex::encode(value.to_be_bytes::<32>())
                    // );
                    (
                        vec![value.to_le_bytes::<32>()],
                        JZKT_STORAGE_COMPRESSION_FLAGS,
                        is_cold,
                    )
                })
        }
    }

    fn update(&self, key: &[u8; 32], value: &Vec<[u8; 32]>, _flags: u32) {
        let mut ctx = self.ctx.borrow_mut();
        if value.len() == JZKT_ACCOUNT_FIELDS_COUNT as usize {
            let address = Address::from_slice(&key[12..]);
            let (account, _) = ctx.load_account_with_code(address).expect("database error");
            account.mark_touch();
            let jzkt_account = Account::new_from_fields(address, value.as_slice());
            account.info.balance = jzkt_account.balance;
            account.info.nonce = jzkt_account.nonce;
            account.info.code_hash = jzkt_account.source_code_hash;
            account.info.rwasm_code_hash = jzkt_account.rwasm_code_hash;
        } else if value.len() == JZKT_STORAGE_FIELDS_COUNT as usize {
            // println!(
            //     "writing storage value: slot={}, value={}",
            //     hex::encode(U256::from_le_bytes(*key).to_be_bytes::<32>()),
            //     hex::encode(U256::from_le_bytes(*value.get(0).unwrap()).to_be_bytes::<32>())
            // );
            ctx.sstore(
                EVM_STORAGE_ADDRESS,
                U256::from_le_bytes(*key),
                U256::from_le_bytes(*value.get(0).unwrap()),
            )
            .expect("failed to update storage slot");
        } else {
            panic!("not supported field count: {}", value.len())
        }
    }

    fn remove(&self, _key: &[u8; 32]) {
        // TODO: "account removal is not supported"
    }

    fn compute_root(&self) -> [u8; 32] {
        // TODO: "root is not supported"
        [0u8; 32]
    }

    fn emit_log(&self, address: Address, topics: Vec<B256>, data: Bytes) {
        let mut ctx = self.ctx.borrow_mut();
        if address == NATIVE_TRANSFER_ADDRESS && topics[0] == NATIVE_TRANSFER_KECCAK {
            assert_eq!(topics.len(), 4, "topics count mismatched");
            let from = Address::from_slice(&topics[1][12..]);
            let to = Address::from_slice(&topics[2][12..]);
            let balance = U256::from_be_slice(&topics[3][..]);
            ctx.journaled_state
                .journal
                .last_mut()
                .unwrap()
                .push(JournalEntry::BalanceTransfer { from, to, balance });
        }
        ctx.journaled_state.log(Log {
            address,
            data: LogData::new_unchecked(topics, data),
        });
    }

    fn commit(&self) -> Result<([u8; 32], Vec<JournalLog>), ExitCode> {
        let mut ctx = self.ctx.borrow_mut();
        ctx.journaled_state.checkpoint_commit();
        Ok(([0u8; 32], vec![]))
    }

    fn rollback(&self, checkpoint: fluentbase_types::JournalCheckpoint) {
        let mut ctx = self.ctx.borrow_mut();
        ctx.journaled_state
            .checkpoint_revert((checkpoint.0, checkpoint.1).into());
    }

    fn update_preimage(&self, key: &[u8; 32], field: u32, preimage: &[u8]) -> bool {
        let mut ctx = self.ctx.borrow_mut();
        let address = Address::from_slice(&key[12..]);
        if field == JZKT_ACCOUNT_SOURCE_CODE_HASH_FIELD {
            ctx.journaled_state.set_code(
                address,
                Bytecode::new_raw(Bytes::copy_from_slice(preimage)),
                None,
            );
        } else if field == JZKT_ACCOUNT_RWASM_CODE_HASH_FIELD {
            ctx.journaled_state.set_rwasm_code(
                address,
                Bytecode::new_raw(Bytes::copy_from_slice(preimage)),
                None,
            );
        }
        true
    }

    fn preimage(&self, hash: &[u8; 32]) -> Vec<u8> {
        let mut ctx = self.ctx.borrow_mut();
        let bytecode = ctx
            .code_by_hash(B256::from(hash))
            .expect("failed to get bytecode by hash");
        bytecode.to_vec()
    }

    fn preimage_size(&self, hash: &[u8; 32]) -> u32 {
        self.ctx
            .borrow_mut()
            .db
            .code_by_hash(B256::from(hash))
            .map(|b| b.bytecode.len() as u32)
            .unwrap_or_default()
    }

    fn journal(&self) -> Vec<JournalEvent> {
        // TODO: "journal is not supported here"
        vec![]
    }
}

impl<'a, DB: Database> AccountManager for JournalDbWrapper<'a, DB> {
    fn checkpoint(&self) -> AccountCheckpoint {
        let mut ctx = self.ctx.borrow_mut();
        let (a, b) = ctx.journaled_state.checkpoint().into();
        fluentbase_types::JournalCheckpoint::from((a, b)).to_u64()
    }

    fn commit(&self) {
        let mut ctx = self.ctx.borrow_mut();
        ctx.journaled_state.checkpoint_commit();
    }

    fn rollback(&self, checkpoint: AccountCheckpoint) {
        let checkpoint = fluentbase_types::JournalCheckpoint::from_u64(checkpoint);
        let mut ctx = self.ctx.borrow_mut();
        ctx.journaled_state
            .checkpoint_revert((checkpoint.0, checkpoint.1).into());
    }

    fn account(&self, address: Address) -> (Account, bool) {
        let mut ctx = self.ctx.borrow_mut();
        let (account, is_cold) = ctx.load_account(address).expect("database error");
        let mut account = Account::from(account.info.clone());
        account.address = address;
        (account, is_cold)
    }

    fn write_account(&self, account: &Account) {
        let mut ctx = self.ctx.borrow_mut();
        let (db_account, _) = ctx
            .load_account_with_code(account.address)
            .expect("database error");
        db_account.info.balance = account.balance;
        db_account.info.nonce = account.nonce;
        db_account.info.code_hash = account.source_code_hash;
        db_account.info.rwasm_code_hash = account.rwasm_code_hash;
        db_account.mark_touch();
    }

    fn preimage_size(&self, hash: &[u8; 32]) -> u32 {
        self.ctx
            .borrow_mut()
            .db
            .code_by_hash(B256::from(hash))
            .map(|b| b.bytecode.len() as u32)
            .unwrap_or_default()
    }

    fn preimage(&self, hash: &[u8; 32]) -> Bytes {
        let mut ctx = self.ctx.borrow_mut();
        ctx.code_by_hash(B256::from(hash))
            .expect("failed to get bytecode by hash")
    }

    fn update_preimage(&self, key: &[u8; 32], field: u32, preimage: &[u8]) {
        let mut ctx = self.ctx.borrow_mut();
        let address = Address::from_slice(&key[12..]);
        if field == JZKT_ACCOUNT_SOURCE_CODE_HASH_FIELD {
            ctx.journaled_state.set_code(
                address,
                Bytecode::new_raw(Bytes::copy_from_slice(preimage)),
                None,
            );
        } else if field == JZKT_ACCOUNT_RWASM_CODE_HASH_FIELD {
            ctx.journaled_state.set_rwasm_code(
                address,
                Bytecode::new_raw(Bytes::copy_from_slice(preimage)),
                None,
            );
        }
    }

    fn storage(&self, _address: Address, slot: U256) -> (U256, bool) {
        let mut ctx = self.ctx.borrow_mut();
        ctx.sload(EVM_STORAGE_ADDRESS, slot)
            .expect("failed to read storage slot")
    }

    fn write_storage(&self, address: Address, slot: U256, value: U256) -> bool {
        let mut ctx = self.ctx.borrow_mut();
        let storage_key = calc_storage_key(&address, slot.as_le_slice().as_ptr());
        let result = ctx
            .sstore(EVM_STORAGE_ADDRESS, U256::from_le_bytes(storage_key), value)
            .expect("failed to update storage slot");
        result.is_cold
    }

    fn log(&self, address: Address, data: Bytes, topics: &[B256]) {
        self.emit_log(address, topics.into(), data)
    }

    fn exec_hash(
        &self,
        hash32_offset: *const u8,
        input: &[u8],
        fuel_offset: *mut u32,
        state: u32,
    ) -> (Bytes, i32) {
        use fluentbase_runtime::{Runtime, RuntimeContext};
        let hash32: [u8; 32] = unsafe { &*core::ptr::slice_from_raw_parts(hash32_offset, 32) }
            .try_into()
            .unwrap();
        let rwasm_bytecode = AccountManager::preimage(self, &hash32);
        if rwasm_bytecode.is_empty() {
            return (Bytes::default(), ExitCode::Ok.into_i32());
        }
        let mut ctx = self.ctx.borrow_mut();
        let jzkt = JournalDbWrapper {
            ctx: RefCell::new(&mut ctx),
        };
        let ctx = RuntimeContext::new(rwasm_bytecode)
            .with_input(input.into())
            .with_fuel_limit(unsafe { *fuel_offset } as u64)
            .with_jzkt(jzkt)
            .with_state(state);
        let mut runtime = Runtime::new(ctx);
        let result = match runtime.call() {
            Ok(result) => result,
            Err(err) => {
                let exit_code = Runtime::catch_trap(&err);
                println!("execution failed with err: {:?}", err);
                return (Bytes::default(), exit_code);
            }
        };
        unsafe {
            *fuel_offset -= result.fuel_consumed as u32;
        }
        (Bytes::from(result.output.clone()), result.exit_code.into())
    }
}
