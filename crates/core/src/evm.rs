use crate::{debug_log, helpers::evm_error_from_exit_code};
use core::mem::take;
use fluentbase_sdk::{
    codec::Encoder,
    env_from_context,
    Address,
    Bytes,
    ExitCode,
    NativeAPI,
    SovereignAPI,
    SyscallAPI,
    B256,
    U256,
};
use revm_interpreter::{
    analysis::to_analysed,
    as_usize_saturated,
    gas,
    opcode::make_instruction_table,
    CallOutcome,
    CallScheme,
    Contract,
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
    Bytecode,
    CancunSpec,
    CreateScheme,
    Env,
    Log,
    BLOCK_HASH_HISTORY,
    MAX_CALL_STACK_LIMIT,
    MAX_CODE_SIZE,
};

pub struct EvmLoader<'a, SDK> {
    pub(crate) sdk: &'a mut SDK,
    pub(crate) env: Env,
}

impl<'a, SDK: SovereignAPI> Host for EvmLoader<'a, SDK> {
    fn env(&self) -> &Env {
        &self.env
    }

    fn env_mut(&mut self) -> &mut Env {
        &mut self.env
    }

    fn load_account(&mut self, address: Address) -> Option<LoadAccountResult> {
        let (account, is_cold) = self.sdk.account(&address);
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
        if account.is_empty() {
            return Some((Bytes::new(), is_cold));
        }
        let evm_bytecode = self
            .sdk
            .preimage(&account.source_code_hash)
            .unwrap_or_default();
        Some((evm_bytecode, is_cold))
        // let code_hash_storage_key = self.code_hash_storage_key(&address);
        // let (value, _) = self
        //     .sdk
        //     .storage(&PRECOMPILE_EVM_LOADER, &code_hash_storage_key);
        // let evm_code_hash = B256::from(value.to_le_bytes());
        // Some((
        //     self.sdk.preimage(&evm_code_hash).unwrap_or_default(),
        //     is_cold,
        // ))
    }

    fn code_hash(&mut self, address: Address) -> Option<(B256, bool)> {
        let (account, is_cold) = self.sdk.account(&address);
        if account.is_empty() {
            return Some((B256::ZERO, is_cold));
        }
        Some((account.source_code_hash, is_cold))
        // let code_hash_storage_key = self.code_hash_storage_key(&address);
        // let (value, _) = self
        //     .sdk
        //     .storage(&PRECOMPILE_EVM_LOADER, &code_hash_storage_key);
        // let evm_code_hash = B256::from(value.to_le_bytes());
        // Some((evm_code_hash, is_cold))
    }

    fn sload(&mut self, address: Address, index: U256) -> Option<(U256, bool)> {
        let (value, is_cold) = self.sdk.storage(&address, &index);
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
        let (original_value, _) = self.sdk.committed_storage(&address, &index);
        let (present_value, is_cold) = self.sdk.storage(&address, &index);
        self.sdk.write_storage(address, index, value);
        Some(SStoreResult {
            original_value,
            present_value,
            new_value: value,
            is_cold,
        })
    }

    fn tload(&mut self, address: Address, index: U256) -> U256 {
        self.sdk.transient_storage(address, index)
    }

    fn tstore(&mut self, address: Address, index: U256, value: U256) {
        self.sdk.write_transient_storage(address, index, value)
    }

    fn log(&mut self, mut log: Log) {
        self.sdk.write_log(
            log.address,
            take(&mut log.data.data),
            log.data.topics().to_vec(),
        );
    }

    fn selfdestruct(&mut self, address: Address, target: Address) -> Option<SelfDestructResult> {
        let result = self.sdk.destroy_account(&address, &target);
        // we must remove EVM bytecode from our storage to match EVM standards,
        // because after calling SELFDESTRUCT bytecode, and it's hash must be empty
        // self.store_evm_bytecode(&address, Bytecode::new());
        Some(SelfDestructResult {
            had_value: result.had_value,
            target_exists: result.target_exists,
            is_cold: result.is_cold,
            previously_destroyed: result.previously_destroyed,
        })
    }
}

impl<'a, SDK: SovereignAPI> EvmLoader<'a, SDK> {
    pub fn new(sdk: &'a mut SDK) -> Self {
        Self {
            env: env_from_context(sdk.block_context(), sdk.tx_context()),
            sdk,
        }
    }

    fn code_hash_storage_key(&self, address: &Address) -> U256 {
        let mut buffer64: [u8; 64] = [0u8; 64];
        buffer64[0..32].copy_from_slice(U256::ZERO.as_le_slice());
        buffer64[44..64].copy_from_slice(address.as_slice());
        let code_hash_key = self.sdk.native_sdk().keccak256(&buffer64);
        U256::from_be_bytes(code_hash_key.0)
    }

    pub fn load_evm_bytecode(&self, address: &Address) -> (Bytecode, B256) {
        let (account, _) = self.sdk.account(address);
        let bytecode = self
            .sdk
            .preimage(&account.source_code_hash)
            .unwrap_or_default();
        let bytecode = Bytecode::new_raw(bytecode);
        (bytecode, account.source_code_hash)

        // // get EVM bytecode hash from the current address
        // let evm_code_hash: B256 = {
        //     let code_hash_storage_key = self.code_hash_storage_key(address);
        //     let (value, _) = self
        //         .sdk
        //         .storage(&PRECOMPILE_EVM_LOADER, &code_hash_storage_key);
        //     B256::from(value.to_le_bytes())
        // };
        //
        // // load EVM bytecode from preimage storage
        // let evm_bytecode = self.sdk.preimage(&evm_code_hash).unwrap_or_default();
        //
        // // make sure bytecode is analyzed (required by interpreter)
        // let evm_bytecode = Bytecode::new_raw(evm_bytecode);
        // (evm_bytecode, evm_code_hash)
    }

    pub fn store_evm_bytecode(&mut self, address: &Address, code_hash: B256, bytecode: Bytecode) {
        self.sdk
            .write_preimage(*address, code_hash, bytecode.original_bytes());
        // // write bytecode hash to the storage
        // let code_hash = bytecode.hash_slow();
        // let code_hash_storage_key = self.code_hash_storage_key(address);
        // self.sdk.write_storage(
        //     PRECOMPILE_EVM_LOADER,
        //     code_hash_storage_key,
        //     U256::from_le_bytes(code_hash.0),
        // );
        // // store EVM bytecode inside preimage storage
        // self.sdk
        //     .write_preimage(*address, code_hash, bytecode.original_bytes());
    }

    pub fn exec_evm_bytecode(
        &mut self,
        mut contract: Contract,
        gas: Gas,
        is_static: bool,
        depth: u32,
    ) -> InterpreterResult {
        debug_log!(
            self.sdk,
            "ecl(exec_evm_bytecode): executing EVM contract={}, caller={}, gas={} bytecode={} input={} depth={}",
            &contract.target_address,
            &contract.caller,
            gas.remaining(),
            hex::encode(contract.bytecode.original_byte_slice()),
            hex::encode(&contract.input),
            depth,
        );

        // make sure bytecode is analyzed
        contract.bytecode = to_analysed(contract.bytecode);

        let instruction_table = make_instruction_table::<Self, CancunSpec>();

        let mut interpreter = Interpreter::new(contract, gas.remaining(), is_static);
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
                        interpreter.contract.target_address,
                        inputs.value.transfer().unwrap_or_default().to_string(),
                        inputs.value.apparent().unwrap_or_default().to_string(),
                    );
                    let return_memory_offset = inputs.return_memory_offset.clone();

                    let (output, exit_code) = match inputs.scheme {
                        CallScheme::Call => self.sdk.native_sdk().syscall_call(
                            inputs.gas_limit,
                            inputs.target_address,
                            inputs.value.transfer().unwrap_or_default(),
                            inputs.input.as_ref(),
                        ),
                        CallScheme::CallCode => unreachable!(),
                        CallScheme::DelegateCall => self.sdk.native_sdk().syscall_delegate_call(
                            inputs.gas_limit,
                            inputs.target_address,
                            inputs.input.as_ref(),
                        ),
                        CallScheme::StaticCall => self.sdk.native_sdk().syscall_static_call(
                            inputs.gas_limit,
                            inputs.target_address,
                            inputs.value.transfer().unwrap_or_default(),
                            inputs.input.as_ref(),
                        ),
                    };

                    let result = InterpreterResult::new(
                        evm_error_from_exit_code(ExitCode::from(exit_code)),
                        output,
                        gas,
                    );
                    let call_outcome = CallOutcome::new(result, return_memory_offset);
                    interpreter.insert_call_outcome(&mut shared_memory, call_outcome);
                }
                InterpreterAction::Create { inputs } => {
                    debug_log!(
                        self.sdk,
                        "ecl(exec_evm_bytecode): nested create caller={}, value={}",
                        inputs.caller,
                        hex::encode(inputs.value.to_be_bytes::<32>())
                    );

                    let create_outcome = match self.sdk.native_sdk().syscall_create(
                        inputs.gas_limit,
                        match inputs.scheme {
                            CreateScheme::Create => None,
                            CreateScheme::Create2 { salt } => Some(salt),
                        },
                        &inputs.value,
                        inputs.init_code.as_ref(),
                    ) {
                        Ok(created_address) => {
                            let result = InterpreterResult::new(
                                InstructionResult::Return,
                                Bytes::default(),
                                gas,
                            );
                            CreateOutcome::new(result, Some(created_address))
                        }
                        Err(exit_code) => {
                            let result = InterpreterResult::new(
                                evm_error_from_exit_code(ExitCode::from(exit_code)),
                                Bytes::default(),
                                gas,
                            );
                            CreateOutcome::new(result, None)
                        }
                    };

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

    pub fn call(
        &mut self,
        caller: Address,
        target_address: Address,
        call_value: U256,
        input: Bytes,
        gas_limit: u64,
    ) -> InterpreterResult {
        let (evm_bytecode, _code_hash) = self.load_evm_bytecode(&target_address);
        let contract = Contract {
            input,
            bytecode: to_analysed(evm_bytecode),
            hash: None,
            target_address,
            caller,
            call_value,
        };
        self.exec_evm_bytecode(contract, Gas::new(gas_limit), false, 0)
    }

    pub fn deploy(
        &mut self,
        caller: Address,
        target_address: Address,
        init_code: Bytes,
        call_value: U256,
        gas_limit: u64,
    ) -> InterpreterResult {
        let return_error = |gas: Gas, result: InstructionResult| -> InterpreterResult {
            InterpreterResult::new(result, Bytes::new(), gas)
        };
        let gas = Gas::new(gas_limit);
        let contract = Contract {
            input: Default::default(),
            bytecode: to_analysed(Bytecode::new_raw(init_code)),
            hash: None,
            target_address,
            caller,
            call_value,
        };

        // execute EVM constructor bytecode to produce new resulting EVM bytecode
        let mut result = self.exec_evm_bytecode(contract, gas, false, 0);
        if !result.result.is_ok() {
            return result;
        }

        // if bytecode starts with 0xEF or exceeds MAX_CODE_SIZE then return the corresponding error
        if !result.output.is_empty() && result.output.first() == Some(&0xEF) {
            return return_error(gas, InstructionResult::CreateContractStartingWithEF);
        } else if result.output.len() > MAX_CODE_SIZE {
            return return_error(gas, InstructionResult::CreateContractSizeLimit);
        }

        // record gas for each created byte
        let gas_for_code = result.output.len() as u64 * gas::CODEDEPOSIT;
        if !result.gas.record_cost(gas_for_code) {
            return return_error(gas, InstructionResult::OutOfGas);
        }

        // write callee changes to a database (lets keep rWASM part empty for now since universal
        // loader is not ready yet)
        let (mut contract_account, _) = self.sdk.account(&target_address);
        let evm_bytecode = Bytecode::new_raw(result.output.clone());
        let code_hash = evm_bytecode.hash_slow();
        contract_account.update_bytecode(
            self.sdk,
            evm_bytecode.original_bytes(),
            Some(code_hash),
            Bytes::default(),
            None,
        );

        // if there is an address, then we have new EVM bytecode inside output
        self.store_evm_bytecode(&target_address, code_hash, evm_bytecode);

        result
    }
}
