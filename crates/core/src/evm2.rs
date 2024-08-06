use crate::{debug_log, fluentbase_sdk::NativeAPI, helpers::exit_code_from_evm_error};
use alloc::boxed::Box;
use core::mem::take;
use fluentbase_sdk::{Address, Bytes, B256, U256};
use fluentbase_types::SovereignAPI;
use revm_interpreter::{
    analysis::to_analysed,
    as_usize_saturated,
    opcode::make_instruction_table,
    CallInputs,
    CallOutcome,
    Contract,
    CreateInputs,
    CreateOutcome,
    Gas,
    Host,
    InstructionResult,
    Interpreter,
    InterpreterAction,
    InterpreterResult,
    LoadAccountResult,
    SStoreResult,
    SelfDestructResult,
    SharedMemory,
};
use revm_primitives::{
    AnalysisKind,
    BlockEnv,
    Bytecode,
    CancunSpec,
    CfgEnv,
    Env,
    Log,
    TransactTo,
    TxEnv,
    BLOCK_HASH_HISTORY,
};

struct EvmBytecodeExecutor<'a, SDK> {
    sdk: &'a mut SDK,
    env: Env,
}

impl<'a, SDK: SovereignAPI> Host for EvmBytecodeExecutor<'a, SDK> {
    fn env(&self) -> &Env {
        &self.env
    }

    fn env_mut(&mut self) -> &mut Env {
        &mut self.env
    }

    fn load_account(&mut self, address: Address) -> Option<LoadAccountResult> {
        let (account, is_cold) = self.sdk.account(&address);
        // Some((is_cold, account.is_not_empty()))
        Some(LoadAccountResult {
            is_cold,
            is_empty: account.is_empty(),
        })
    }

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

    fn balance(&mut self, address: Address) -> Option<(U256, bool)> {
        let (account, is_cold) = self.sdk.account(&address);
        Some((account.balance, is_cold))
    }

    fn code(&mut self, address: Address) -> Option<(Bytes, bool)> {
        let (account, is_cold) = self.sdk.account(&address);
        Some((
            self.sdk
                .preimage(&account.source_code_hash)
                .map(Bytes::copy_from_slice)
                .unwrap_or_default(),
            is_cold,
        ))
    }

    fn code_hash(&mut self, address: Address) -> Option<(B256, bool)> {
        let (account, is_cold) = self.sdk.account(&address);
        if !account.is_not_empty() {
            return Some((B256::ZERO, is_cold));
        }
        Some((account.source_code_hash, is_cold))
    }

    fn sload(&mut self, address: Address, index: U256) -> Option<(U256, bool)> {
        let (value, is_cold) = self.sdk.storage(address, index);
        debug_log!(
            self.sdk,
            "ecl(sload): address={}, index={}, value={}",
            address,
            hex::encode(index.to_be_bytes::<32>().as_slice()),
            hex::encode(value.to_be_bytes::<32>().as_slice()),
        );
        Some((value, is_cold))
    }

    fn sstore(&mut self, address: Address, index: U256, value: U256) -> Option<SStoreResult> {
        debug_log!(
            self.sdk,
            "ecl(sstore): address={}, index={}, value={}",
            address,
            hex::encode(index.to_be_bytes::<32>().as_slice()),
            hex::encode(value.to_be_bytes::<32>().as_slice()),
        );
        let (original_value, _) = self.sdk.committed_storage(address, index);
        let (present_value, is_cold) = self.sdk.storage(address, index);
        self.sdk.write_storage(address, index, value);
        return Some(SStoreResult {
            original_value,
            present_value,
            new_value: value,
            is_cold,
        });
    }

    fn tload(&mut self, address: Address, index: U256) -> U256 {
        // self.transient_storage
        //     .get(&(address, index))
        //     .copied()
        //     .unwrap_or_default()
        // self.sdk.unwrap().transient_storage(address, index)
        todo!("not supported yet")
    }

    fn tstore(&mut self, address: Address, index: U256, value: U256) {
        // self.transient_storage.insert((address, index), value);
        // self.sdk
        //     .unwrap()
        //     .write_transient_storage(address, index, value)
        todo!("not supported yet")
    }

    fn log(&mut self, mut log: Log) {
        self.sdk
            .write_log(log.address, take(&mut log.data.data), log.data.topics());
    }

    fn selfdestruct(&mut self, address: Address, target: Address) -> Option<SelfDestructResult> {
        todo!("not supported yet");
        // let [had_value, target_exists, is_cold, previously_destroyed] =
        //     self.sdk.as_mut().unwrap().self_destruct(address, target);
        // Some(SelfDestructResult {
        //     had_value,
        //     target_exists,
        //     is_cold,
        //     previously_destroyed,
        // })
    }
}

impl<'a, SDK: SovereignAPI> EvmBytecodeExecutor<'a, SDK> {
    pub fn new(sdk: &'a mut SDK) -> Self {
        Self {
            env: Env {
                cfg: {
                    let mut cfg_env = CfgEnv::default();
                    cfg_env.chain_id = sdk.block_context().chain_id;
                    cfg_env.perf_analyse_created_bytecodes = AnalysisKind::Raw;
                    cfg_env
                },
                block: BlockEnv {
                    number: U256::from(sdk.block_context().number),
                    coinbase: sdk.block_context().coinbase,
                    timestamp: U256::from(sdk.block_context().timestamp),
                    gas_limit: U256::from(sdk.block_context().gas_limit),
                    basefee: sdk.block_context().base_fee,
                    difficulty: sdk.block_context().difficulty,
                    prevrandao: Some(sdk.block_context().prev_randao),
                    blob_excess_gas_and_price: None,
                },
                tx: TxEnv {
                    caller: sdk.tx_context().origin,
                    gas_limit: sdk.tx_context().gas_limit,
                    gas_price: sdk.tx_context().gas_price,
                    transact_to: TransactTo::Call(Address::ZERO), // will do nothing
                    value: sdk.tx_context().value,
                    data: Default::default(), // not used because we already pass all validations
                    nonce: Some(sdk.tx_context().nonce),
                    chain_id: None, // no checks
                    access_list: Default::default(),
                    gas_priority_fee: Default::default(),
                    blob_hashes: Default::default(),
                    max_fee_per_blob_gas: Default::default(),
                    #[cfg(feature = "optimism")]
                    optimism: Default::default(),
                },
            },
            sdk,
        }
    }

    fn code_hash_storage_key(&self, address: &Address) -> U256 {
        let mut buffer64: [u8; 64] = [0u8; 64];
        buffer64[44..64].copy_from_slice(address.as_slice());
        let code_hash_key = self.sdk.native_sdk().keccak256(&buffer64);
        U256::from_be_bytes(code_hash_key.0)
    }

    fn exec_evm_create(&mut self, inputs: Box<CreateInputs>, depth: u32) -> CreateOutcome {
        let contract = Contract::new(
            Bytes::new(),
            to_analysed(Bytecode::new_raw(inputs.init_code)),
            None,
            Address::ZERO,
            inputs.caller,
            inputs.value,
        );
        let interpreter_result = self.exec_evm_bytecode(contract, inputs.gas_limit, false, depth);

        let address = if interpreter_result.is_ok() {
            assert_eq!(
                interpreter_result.output.len(),
                20,
                "create/create2 output result doesn't equal to 20 bytes"
            );
            Some(Address::from_slice(interpreter_result.output.as_ref()))
        } else {
            None
        };

        CreateOutcome {
            result: interpreter_result,
            address,
        }
    }

    fn load_evm_bytecode(&mut self, address: &Address) -> (Bytecode, B256) {
        let contract_address = self.sdk.contract_context().map(|v| v.address).unwrap();

        // get EVM bytecode hash from the current address
        let evm_code_hash: B256 = {
            let code_hash_storage_key = self.code_hash_storage_key(address);
            let (value, _) = self.sdk.storage(contract_address, code_hash_storage_key);
            value.to_be_bytes::<32>().into()
        };

        // load EVM bytecode from preimage storage
        let evm_bytecode = self
            .sdk
            .preimage(&evm_code_hash)
            .map(Bytes::copy_from_slice)
            .unwrap_or_default();

        // make sure bytecode is analyzed (required by interpreter)
        let evm_bytecode = to_analysed(Bytecode::new_raw(evm_bytecode));
        (evm_bytecode, evm_code_hash)
    }

    fn exec_evm_call(&mut self, mut inputs: Box<CallInputs>, depth: u32) -> CallOutcome {
        let return_memory_offset = inputs.return_memory_offset.clone();

        let (bytecode, code_hash) = self.load_evm_bytecode(&inputs.bytecode_address);

        let contract = Contract::new(
            inputs.input,
            bytecode,
            Some(code_hash),
            inputs.target_address,
            inputs.caller,
            // here we take transfer value, because for DELEGATECALL it's not apparent
            inputs.value.transfer().unwrap_or_default(),
        );
        let interpreter_result =
            self.exec_evm_bytecode(contract, inputs.gas_limit, inputs.is_static, depth);

        CallOutcome {
            result: interpreter_result,
            memory_offset: return_memory_offset,
        }
    }

    fn exec_evm_bytecode(
        &mut self,
        contract: Contract,
        gas_limit: u64,
        is_static: bool,
        depth: u32,
    ) -> InterpreterResult {
        debug_log!(
            self.sdk,
            "ecl(exec_evm_bytecode): executing EVM contract={}, caller={}, gas_limit={} bytecode={} input={} depth={}",
            &contract.target_address,
            &contract.caller,
            gas_limit,
            hex::encode(contract.bytecode.original_byte_slice()),
            hex::encode(&contract.input),
            depth,
        );
        if depth >= 1024 {
            debug_log!(self.sdk, "depth limit reached: {}", depth);
        }
        let contract_address = contract.target_address;

        let instruction_table = make_instruction_table::<Self, CancunSpec>();

        let mut interpreter = Interpreter::new(contract, gas_limit, is_static);
        let mut shared_memory = SharedMemory::new();

        loop {
            // run EVM bytecode to produce next action
            let next_action = interpreter.run(shared_memory, &instruction_table, self);

            // take memory and cr from interpreter and host back (return later)
            shared_memory = interpreter.take_memory();

            match next_action {
                InterpreterAction::Call { inputs } => {
                    debug_log!(
                        self.sdk,
                        "ecl(exec_evm_bytecode): nested call={:?} code={} caller={} callee={} gas={} prev_address={} value={} apparent_value={}",
                        inputs.scheme,
                        &inputs.bytecode_address,
                        &inputs.caller,
                        &inputs.target_address,
                        inputs.gas_limit,
                        contract_address,
                        hex::encode(inputs.value.transfer().unwrap_or_default().to_be_bytes::<32>()),
                        hex::encode(inputs.value.apparent().unwrap_or_default().to_be_bytes::<32>()),
                    );
                    let call_outcome = self.exec_evm_call(inputs, depth + 1);
                    interpreter.insert_call_outcome(&mut shared_memory, call_outcome);
                }
                InterpreterAction::Create { inputs } => {
                    debug_log!(
                        self.sdk,
                        "ecl(exec_evm_bytecode): nested create caller={}, value={}",
                        inputs.caller,
                        hex::encode(inputs.value.to_be_bytes::<32>())
                    );
                    let create_outcome = self.exec_evm_create(inputs, depth + 1);
                    interpreter.insert_create_outcome(create_outcome);
                }
                InterpreterAction::Return { result } => {
                    debug_log!(
                        self.sdk,
                        "ecl(exec_evm_bytecode): return result={:?}, message={} gas_spent={}",
                        result.result,
                        hex::encode(result.output.as_ref()),
                        result.gas.spent(),
                    );
                    return result;
                }
                InterpreterAction::None => unreachable!("not supported EVM interpreter state"),
                InterpreterAction::EOFCreate { .. } => {
                    unreachable!("not supported EVM interpreter state: EOF")
                }
            }
        }
    }

    pub fn exec(&mut self) -> InterpreterResult {
        // load bytecode, if its empty, then exit
        let (bytecode_address, gas) = self
            .sdk
            .contract_context()
            .map(|v| (v.bytecode_address, Gas::new(v.gas_limit)))
            .unwrap();
        let (bytecode, code_hash) = self.load_evm_bytecode(&bytecode_address);
        if bytecode.is_empty() {
            return InterpreterResult::new(InstructionResult::Return, Bytes::new(), gas);
        }

        let contract_context = self.sdk.contract_context().unwrap();

        let contract = Contract::new(
            contract_context.input.clone(),
            bytecode,
            Some(code_hash),
            contract_context.address,
            contract_context.caller,
            contract_context.value,
        );
        let result = self.exec_evm_bytecode(contract, contract_context.gas_limit, false, 0);

        // if matches!(result.result, return_ok!()) {
        //     self.sdk.commit();
        // } else {
        //     self.sdk.rollback(checkpoint);
        // }

        let exit_code = exit_code_from_evm_error(result.result);

        debug_log!(
            self.sdk,
            "ecl(_evm_call): return exit_code={} gas_remaining={} spent={} gas_refund={}",
            exit_code,
            result.gas.remaining(),
            result.gas.spent(),
            result.gas.refunded()
        );
        result
    }
}

pub fn exec_evm_bytecode<SDK: SovereignAPI>(sdk: &mut SDK) -> InterpreterResult {
    EvmBytecodeExecutor::new(sdk).exec()
}
