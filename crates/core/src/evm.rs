use crate::{
    debug_log,
    fluentbase_sdk::NativeAPI,
    helpers::{evm_error_from_exit_code, wasm2rwasm},
};
use alloc::{boxed::Box, string::ToString};
use core::mem::take;
use fluentbase_sdk::{syscall::execute_rwasm_smart_contract, Address, Bytes, B256, U256};
use fluentbase_types::{
    env_from_context,
    Account,
    AccountStatus,
    BytecodeType,
    ExitCode,
    Fuel,
    SovereignAPI,
    STATE_DEPLOY,
    STATE_MAIN,
};
use revm_interpreter::{
    analysis::to_analysed,
    as_usize_saturated,
    gas,
    opcode::make_instruction_table,
    return_ok,
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
    Bytecode,
    CancunSpec,
    CreateScheme,
    Env,
    Log,
    BLOCK_HASH_HISTORY,
    MAX_CALL_STACK_LIMIT,
    MAX_CODE_SIZE,
    MAX_INITCODE_SIZE,
    WASM_MAX_CODE_SIZE,
};

pub struct EvmRuntime<'a, SDK> {
    sdk: &'a mut SDK,
    env: Env,
}

impl<'a, SDK: SovereignAPI> Host for EvmRuntime<'a, SDK> {
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
        Some((
            self.sdk
                .preimage(&account.source_code_hash)
                .unwrap_or_default(),
            is_cold,
        ))
        // let (account, is_cold) = self.sdk.account(&address);
        // if account.is_empty() {
        //     return Some((Bytes::new(), is_cold));
        // }
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
        // let (account, is_cold) = self.sdk.account(&address);
        // if account.is_empty() {
        //     return Some((B256::ZERO, is_cold));
        // }
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
        return Some(SStoreResult {
            original_value,
            present_value,
            new_value: value,
            is_cold,
        });
    }

    fn tload(&mut self, address: Address, index: U256) -> U256 {
        self.sdk.transient_storage(address, index)
    }

    fn tstore(&mut self, address: Address, index: U256, value: U256) {
        self.sdk.write_transient_storage(address, index, value)
    }

    fn log(&mut self, mut log: Log) {
        self.sdk
            .write_log(log.address, take(&mut log.data.data), log.data.topics());
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

impl<'a, SDK: SovereignAPI> EvmRuntime<'a, SDK> {
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
        return (bytecode, account.source_code_hash);

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
        //
        // // store EVM bytecode inside preimage storage
        // self.sdk
        //     .write_preimage(*address, code_hash, bytecode.original_bytes());
    }

    fn exec_rwasm_bytecode(
        &mut self,
        caller: &Address,
        account: &Account,
        input: &[u8],
        gas: Gas,
        state: u32,
    ) -> InterpreterResult {
        debug_log!(
            self.sdk,
            "ecl(exec_rwasm_bytecode): executing rWASM contract={}, caller={}, gas={} input={}",
            &account.address,
            &caller,
            gas.remaining(),
            hex::encode(&input),
        );
        let mut fuel = Fuel::from(gas.remaining());
        let (output, exit_code) =
            execute_rwasm_smart_contract(self.sdk, account, &mut fuel, input, state);
        InterpreterResult {
            result: evm_error_from_exit_code(exit_code),
            output,
            gas: Gas::new(fuel.remaining()),
        }
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
        if depth >= MAX_CALL_STACK_LIMIT {
            debug_log!(self.sdk, "depth limit reached: {}", depth);
        }

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
                    let mut call_outcome = self.call_inner(inputs, depth + 1);
                    call_outcome.memory_offset = return_memory_offset;
                    interpreter.insert_call_outcome(&mut shared_memory, call_outcome);
                }
                InterpreterAction::Create { inputs } => {
                    debug_log!(
                        self.sdk,
                        "ecl(exec_evm_bytecode): nested create caller={}, value={}",
                        inputs.caller,
                        hex::encode(inputs.value.to_be_bytes::<32>())
                    );
                    let create_outcome = self.create_inner(inputs, depth + 1);
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

    pub fn deploy_wasm_contract(&mut self, contract: Contract, mut gas: Gas) -> InterpreterResult {
        let return_error = |gas: Gas, exit_code: ExitCode| -> InterpreterResult {
            InterpreterResult::new(evm_error_from_exit_code(exit_code), Bytes::new(), gas)
        };

        // translate WASM to rWASM
        let rwasm_bytecode = match wasm2rwasm(contract.bytecode.original_byte_slice()) {
            Ok(rwasm_bytecode) => rwasm_bytecode,
            Err(exit_code) => {
                return return_error(gas, exit_code);
            }
        };

        // record gas for each created byte
        let gas_for_code = rwasm_bytecode.len() as u64 * gas::CODEDEPOSIT;
        if !gas.record_cost(gas_for_code) {
            return return_error(gas, ExitCode::OutOfGas);
        }

        // write callee changes to a database (lets keep rWASM part empty for now since universal
        // loader is not ready yet)
        let (mut contract_account, _) = self.sdk.account(&contract.target_address);
        contract_account.update_bytecode(self.sdk, Bytes::new(), None, rwasm_bytecode.into(), None);

        // execute rWASM deploy function
        self.exec_rwasm_bytecode(&contract.caller, &contract_account, &[], gas, STATE_DEPLOY)
    }

    pub fn deploy_evm_contract(
        &mut self,
        contract: Contract,
        gas: Gas,
        depth: u32,
    ) -> InterpreterResult {
        let return_error = |gas: Gas, exit_code: InstructionResult| -> InterpreterResult {
            InterpreterResult::new(exit_code, Bytes::new(), gas)
        };
        let target_address = contract.target_address;

        // execute EVM constructor bytecode to produce new resulting EVM bytecode
        let mut result = self.exec_evm_bytecode(contract, gas, false, depth);

        // if there is an address, then we have new EVM bytecode inside output
        if !matches!(result.result, return_ok!()) {
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
        contract_account.update_bytecode(self.sdk, result.output.clone(), None, Bytes::new(), None);

        // if there is an address, then we have new EVM bytecode inside output
        self.store_evm_bytecode(
            &contract_account.address,
            contract_account.source_code_hash,
            Bytecode::new_raw(result.output.clone()),
        );

        result
    }

    fn create_inner(&mut self, inputs: Box<CreateInputs>, depth: u32) -> CreateOutcome {
        let return_error = |gas: Gas, exit_code: ExitCode| -> CreateOutcome {
            CreateOutcome::new(
                InterpreterResult::new(evm_error_from_exit_code(exit_code), Bytes::new(), gas),
                None,
            )
        };
        let gas = Gas::new(inputs.gas_limit);
        debug_log!(
            self.sdk,
            "ecl(_evm_create): start. gas_limit {}",
            inputs.gas_limit
        );

        // determine bytecode type
        let bytecode_type = BytecodeType::from_slice(&inputs.init_code);

        // load deployer and contract accounts
        let (mut caller_account, _) = self.sdk.account(&inputs.caller);
        if caller_account.balance < inputs.value {
            return return_error(gas, ExitCode::InsufficientBalance);
        }

        // call depth check
        if depth > MAX_CALL_STACK_LIMIT {
            return return_error(gas, ExitCode::CallDepthOverflow);
        }

        // check init max code size for EIP-3860
        match bytecode_type {
            BytecodeType::EVM => {
                if inputs.init_code.len() > MAX_INITCODE_SIZE {
                    return return_error(gas, ExitCode::ContractSizeLimit);
                }
            }
            BytecodeType::WASM => {
                if inputs.init_code.len() > WASM_MAX_CODE_SIZE {
                    return return_error(gas, ExitCode::ContractSizeLimit);
                }
            }
        }

        // calc source code hash
        let source_code_hash = self.sdk.native_sdk().keccak256(inputs.init_code.as_ref());

        // create an account
        let salt_hash = match inputs.scheme {
            CreateScheme::Create2 { salt } => Some((salt, source_code_hash)),
            CreateScheme::Create => None,
        };
        let (contract_account, checkpoint) = match Account::create_account_checkpoint(
            self.sdk,
            &mut caller_account,
            inputs.value,
            salt_hash,
        ) {
            Ok(result) => result,
            Err(exit_code) => return return_error(gas, exit_code),
        };

        debug_log!(
            self.sdk,
            "ecl(_evm_create): creating account={} balance={}",
            contract_account.address,
            hex::encode(contract_account.balance.to_be_bytes::<32>())
        );

        let contract = Contract {
            input: Bytes::new(),
            bytecode: Bytecode::new_raw(inputs.init_code),
            hash: Some(source_code_hash),
            target_address: contract_account.address,
            caller: inputs.caller,
            call_value: inputs.value,
        };

        let result = match bytecode_type {
            BytecodeType::EVM => self.deploy_evm_contract(contract, gas, depth),
            BytecodeType::WASM => self.deploy_wasm_contract(contract, gas),
        };

        debug_log!(
            self.sdk,
            "ecl(_evm_create): return: Ok: callee_account.address: {}",
            contract_account.address
        );

        // commit all changes made
        if result.result.is_ok() {
            self.sdk.commit();
        } else {
            self.sdk.rollback(checkpoint);
        }

        CreateOutcome::new(result, Some(contract_account.address))
    }

    pub fn create(&mut self, create_inputs: Box<CreateInputs>) -> CreateOutcome {
        self.create_inner(create_inputs, 0)
    }

    fn call_inner(&mut self, inputs: Box<CallInputs>, depth: u32) -> CallOutcome {
        let return_error = |gas: Gas, exit_code: ExitCode| -> CallOutcome {
            CallOutcome::new(
                InterpreterResult::new(evm_error_from_exit_code(exit_code), Bytes::new(), gas),
                Default::default(),
            )
        };
        let gas = Gas::new(inputs.gas_limit);
        debug_log!(
            self.sdk,
            "ecl(_evm_call): start. gas_limit {}",
            gas.remaining()
        );

        // call depth check
        if depth > MAX_CALL_STACK_LIMIT {
            return return_error(gas, ExitCode::CallDepthOverflow);
        }

        // read caller and callee
        let (mut caller_account, _) = self.sdk.account(&inputs.caller);
        let (mut callee_account, _) = self.sdk.account(&inputs.target_address);

        // create a new checkpoint position in the journal
        let checkpoint = self.sdk.checkpoint();

        // transfer funds from caller to callee
        if let Some(value) = inputs.value.transfer() {
            debug_log!(
                self.sdk,
                "ecm(_evm_call): transfer from={} to={} value={}",
                caller_account.address,
                callee_account.address,
                hex::encode(value.to_be_bytes::<32>())
            );
        }

        if caller_account.address != callee_account.address {
            let value = inputs.transfer_value().unwrap_or_default();
            // do transfer from caller to callee
            match self
                .sdk
                .transfer(&mut caller_account, &mut callee_account, value)
            {
                Err(exit_code) => return return_error(gas, exit_code),
                Ok(_) => {}
            }
            // write current account state before doing nested calls
            self.sdk
                .write_account(caller_account.clone(), AccountStatus::Modified);
            self.sdk
                .write_account(callee_account.clone(), AccountStatus::Modified);
        } else {
            let value = inputs.transfer_value().unwrap_or_default();
            // what if self-transfer amount exceeds our balance?
            if value > caller_account.balance {
                return return_error(gas, ExitCode::InsufficientBalance);
            }
            // write only one account's state since caller equals callee
            self.sdk
                .write_account(caller_account.clone(), AccountStatus::Modified);
        }

        // check is it precompile
        if let Some(result) =
            self.sdk
                .precompile(&inputs.bytecode_address, &inputs.input, gas.remaining())
        {
            // calculate total gas consumed by precompile call
            let mut gas = Gas::new(gas.remaining());
            if !gas.record_cost(gas.remaining() - result.gas_remaining) {
                return return_error(gas, ExitCode::OutOfGas);
            };
            gas.record_refund(result.gas_refund);
            // if exit code is no successful, then rollback changes, otherwise commit them
            if result.exit_code.is_ok() {
                self.sdk.commit();
            } else {
                self.sdk.rollback(checkpoint);
            }
            // map precompile execution result into EVM interpreter result
            return CallOutcome::new(
                InterpreterResult::new(
                    evm_error_from_exit_code(result.exit_code),
                    result.output,
                    gas,
                ),
                Default::default(),
            );
        }

        let (bytecode_account, _) = self.sdk.account(&inputs.bytecode_address);
        let result = if bytecode_account.source_code_size > 0 {
            // take right bytecode depending on context params
            let (evm_bytecode, code_hash) = self.load_evm_bytecode(&inputs.bytecode_address);
            debug_log!(
                self.sdk,
                "ecl(_evm_call): source_bytecode: {}",
                hex::encode(evm_bytecode.original_byte_slice())
            );

            // if bytecode is empty, then commit result and return empty buffer
            if evm_bytecode.is_empty() {
                self.sdk.commit();
                debug_log!(self.sdk, "ecl(_evm_call): empty bytecode exit_code=Ok");
                return return_error(gas, ExitCode::Ok);
            }

            // initiate contract instance and pass it to interpreter for and EVM transition
            let call_value = inputs.call_value();
            let contract = Contract {
                input: inputs.input,
                hash: Some(code_hash),
                bytecode: evm_bytecode,
                // we don't take contract callee, because callee refers to address with bytecode
                target_address: inputs.target_address,
                // inside the contract context, we pass "apparent" value that can be different to
                // transfer value (it can happen for DELEGATECALL or CALLCODE opcodes)
                call_value,
                caller: caller_account.address,
            };
            self.exec_evm_bytecode(contract, gas, inputs.is_static, depth)
        } else {
            let (account, _) = self.sdk.account(&inputs.target_address);
            self.exec_rwasm_bytecode(
                &inputs.caller,
                &account,
                inputs.input.as_ref(),
                gas,
                STATE_MAIN,
            )
        };

        if matches!(result.result, return_ok!()) {
            self.sdk.commit();
        } else {
            self.sdk.rollback(checkpoint);
        }

        debug_log!(
            self.sdk,
            "ecl(_evm_call): return exit_code={:?} gas_remaining={} spent={} gas_refund={}",
            result.result,
            result.gas.remaining(),
            result.gas.spent(),
            result.gas.refunded()
        );

        CallOutcome::new(result, Default::default())
    }

    pub fn call(&mut self, inputs: Box<CallInputs>) -> CallOutcome {
        self.call_inner(inputs, 0)
    }
}
