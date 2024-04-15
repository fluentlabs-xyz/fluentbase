use crate::gas::Gas;
use crate::interpreter::{BytecodeType, CallOutcome, CreateOutcome, InterpreterResult};
use crate::{
    builder::{EvmBuilder, HandlerStage, SetGenericStage},
    db::{Database, DatabaseCommit, EmptyDB},
    handler::Handler,
    primitives::{
        specification::SpecId, Address, BlockEnv, CfgEnv, EVMError, EVMResult, EnvWithHandlerCfg,
        ExecutionResult, HandlerCfg, ResultAndState, TransactTo, TxEnv, B256, U256,
    },
    Context, ContextWithHandlerCfg, EvmContext, FrameResult,
};
use core::cell::RefCell;
use core::fmt;
use fluentbase_codec::Encoder;
use fluentbase_core::consts::{ECL_CONTRACT_ADDRESS, WCL_CONTRACT_ADDRESS};
use fluentbase_core::{
    Account, AccountCheckpoint, JZKT_ACCOUNT_COMPRESSION_FLAGS, JZKT_ACCOUNT_FIELDS_COUNT,
    JZKT_ACCOUNT_RWASM_CODE_HASH_FIELD, JZKT_ACCOUNT_SOURCE_CODE_HASH_FIELD,
    JZKT_STORAGE_COMPRESSION_FLAGS, JZKT_STORAGE_FIELDS_COUNT,
};
use fluentbase_core_api::api::CoreInput;
use fluentbase_core_api::bindings::{
    EvmCreate2MethodInput, EvmCreateMethodInput, WasmCreate2MethodInput, WasmCreateMethodInput,
    EVM_CREATE2_METHOD_ID, EVM_CREATE_METHOD_ID, WASM_CREATE2_METHOD_ID, WASM_CREATE_METHOD_ID,
};
use fluentbase_sdk::evm::ContractInput;
use fluentbase_sdk::{LowLevelAPI, LowLevelSDK};
use fluentbase_types::{
    address, Bytes, ExitCode, IJournaledTrie, JournalCheckpoint, JournalEvent, JournalLog,
    STATE_MAIN,
};
use revm_primitives::{hex, Bytecode, CreateScheme, Log, LogData};
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

        // load precompiles
        let precompiles = pre_exec.load_precompiles();
        ctx.evm.set_precompiles(precompiles);

        // load access list and beneficiary if needed.
        pre_exec.load_accounts(ctx)?;

        // deduce caller balance with its limit.
        pre_exec.deduct_caller(ctx)?;

        let mut caller_account = ctx.evm.load_jzkt_account(ctx.evm.env.tx.caller)?;

        let gas_limit = ctx.evm.env.tx.gas_limit - initial_gas_spend;

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
                    self.context
                        .evm
                        .load_jzkt_account(ECL_CONTRACT_ADDRESS)
                        .expect("failed to load ECL"),
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
                    self.context
                        .evm
                        .load_jzkt_account(WCL_CONTRACT_ADDRESS)
                        .expect("failed to load WCL"),
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
        if self.context.evm.journaled_state.depth as u64 > CALL_STACK_LIMIT {
            return return_result(ExitCode::CallDepthOverflow, gas);
        }

        let checkpoint = Account::checkpoint();

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

        let (output_buffer, exit_code) = self.exec_rwasm_binary(
            checkpoint,
            &mut gas,
            caller_account,
            callee_account,
            input,
            value,
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

    #[cfg(feature = "std")]
    fn exec_rwasm_binary(
        &mut self,
        checkpoint: AccountCheckpoint,
        gas: &mut Gas,
        caller: &mut Account,
        callee: &mut Account,
        input: Bytes,
        value: U256,
    ) -> (Bytes, ExitCode) {
        use fluentbase_runtime::{Runtime, RuntimeContext};
        let input = self
            .input_from_env(checkpoint, gas, caller, callee, input, value)
            .encode_to_vec(0);
        let jzkt = JournalDbWrapper {
            ctx: RefCell::new(&mut self.context.evm),
        };
        let rwasm_bytecode = jzkt.preimage(&callee.rwasm_code_hash.0);
        let ctx = RuntimeContext::new(rwasm_bytecode)
            .with_input(input)
            .with_fuel_limit(gas.remaining())
            .with_jzkt(jzkt)
            .with_catch_trap(true)
            .with_state(STATE_MAIN);
        let import_linker = Runtime::new_sovereign_linker();
        let mut runtime = match Runtime::new(ctx, import_linker) {
            Ok(runtime) => runtime,
            Err(_) => return (Bytes::default(), ExitCode::CompilationError),
        };
        let result = match runtime.call() {
            Ok(result) => result,
            Err(_) => return (Bytes::default(), ExitCode::TransactError),
        };
        {
            println!("executed rWASM binary:");
            println!(" - caller: 0x{}", hex::encode(caller.address));
            println!(" - callee: 0x{}", hex::encode(callee.address));
            println!(" - source hash: 0x{}", hex::encode(callee.source_code_hash));
            println!(" - source size: {}", callee.source_code_size);
            println!(" - rwasm hash: 0x{}", hex::encode(callee.rwasm_code_hash));
            println!(" - rwasm size: {}", callee.rwasm_code_size);
            println!(" - value: 0x{}", hex::encode(&value.to_be_bytes::<32>()));
            println!(" - fuel consumed: {}", result.fuel_consumed);
            println!(" - exit code: {}", result.exit_code);
            println!(
                " - output message: {}",
                core::str::from_utf8(&result.output)
                    .map(|value| value.to_string().replace("\n", " "))
                    .unwrap_or_else(|_| format!("0x{}", hex::encode(&result.output)))
            );
            println!(
                " - last opcode: {:?}",
                runtime.store().tracer().logs.last().unwrap().opcode
            );
            println!(" - opcode used: {}", runtime.store().tracer().logs.len());
            // for log in runtime.store().tracer().logs.iter() {
            //     match log.opcode {
            //         Instruction::Call(index) => println!("{:?}", SysFuncIdx::from(index.to_u32())),
            //         _ => {}
            //     }
            // }
        }
        gas.record_cost(result.fuel_consumed);
        (Bytes::from(result.output.clone()), result.exit_code.into())
    }

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
const EVM_STORAGE_ADDRESS: Address = address!("fabefeab43f96e51d7ace194b9abd33305bb6bfb");

impl<'a, DB: Database> IJournaledTrie for JournalDbWrapper<'a, DB> {
    fn checkpoint(&self) -> JournalCheckpoint {
        let mut ctx = self.ctx.borrow_mut();
        let (a, b) = ctx.journaled_state.checkpoint().into();
        JournalCheckpoint::from((a, b))
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
            ctx.sload(EVM_STORAGE_ADDRESS, U256::from_be_bytes(*key))
                .ok()
                .map(|(value, is_cold)| {
                    (
                        vec![value.to_be_bytes::<32>()],
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
            let jzkt_account = Account::new_from_fields(&address, value.as_slice());
            account.info.balance = jzkt_account.balance;
            account.info.nonce = jzkt_account.nonce;
            account.info.code_hash = jzkt_account.source_code_hash;
            account.info.rwasm_code_hash = jzkt_account.rwasm_code_hash;
        } else if value.len() == JZKT_STORAGE_FIELDS_COUNT as usize {
            ctx.sstore(
                EVM_STORAGE_ADDRESS,
                U256::from_be_bytes(*key),
                U256::from_be_bytes(*value.get(0).unwrap()),
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
        ctx.journaled_state.log(Log {
            address,
            data: LogData::new_unchecked(topics, data),
        });
    }

    fn commit(&self) -> Result<([u8; 32], Vec<JournalLog>), ExitCode> {
        // TODO: "commit is not supported"
        Err(ExitCode::NotSupportedCall)
    }

    fn rollback(&self, checkpoint: JournalCheckpoint) {
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
