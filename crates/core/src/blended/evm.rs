use crate::{
    blended::{util::create_rwasm_proxy_bytecode, BlendedRuntime},
    helpers::evm_error_from_exit_code,
};
use alloc::boxed::Box;
use core::mem::take;
use fluentbase_sdk::{
    Address,
    Bytes,
    ContractContext,
    ExitCode,
    SovereignAPI,
    B256,
    STATE_DEPLOY,
    STATE_MAIN,
    U256,
};
use revm_interpreter::{
    analysis::to_analysed,
    as_usize_saturated,
    gas,
    opcode::make_instruction_table,
    CallOutcome,
    Contract,
    CreateInputs,
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
use revm_primitives::{Bytecode, CancunSpec, Env, Log, BLOCK_HASH_HISTORY, MAX_CODE_SIZE};

impl<'a, SDK: SovereignAPI> Host for BlendedRuntime<'a, SDK> {
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
            .preimage(&address, &account.source_code_hash)
            .unwrap_or_default();
        Some((evm_bytecode, is_cold))
    }

    fn code_hash(&mut self, address: Address) -> Option<(B256, bool)> {
        let (account, is_cold) = self.sdk.account(&address);
        if account.is_empty() {
            return Some((B256::ZERO, is_cold));
        }
        Some((account.source_code_hash, is_cold))
    }

    fn sload(&mut self, address: Address, index: U256) -> Option<(U256, bool)> {
        let (value, is_cold) = self.sdk.storage(&address, &index);
        Some((value, is_cold))
    }

    fn sstore(&mut self, address: Address, index: U256, new_value: U256) -> Option<SStoreResult> {
        let (original_value, _) = self.sdk.committed_storage(&address, &index);
        let (present_value, is_cold) = self.sdk.storage(&address, &index);
        self.sdk.write_storage(address, index, new_value);
        Some(SStoreResult {
            original_value,
            present_value,
            new_value,
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

impl<'a, SDK: SovereignAPI> BlendedRuntime<'a, SDK> {
    pub fn load_evm_bytecode(&self, address: &Address) -> (Bytecode, B256) {
        let (account, _) = self.sdk.account(address);
        let bytecode = self
            .sdk
            .preimage(address, &account.source_code_hash)
            .unwrap_or_default();
        let bytecode = Bytecode::new_raw(bytecode);
        (bytecode, account.source_code_hash)
    }

    pub fn store_evm_bytecode(&mut self, address: &Address, code_hash: B256, bytecode: Bytecode) {
        self.sdk
            .write_preimage(*address, code_hash, bytecode.original_bytes());
    }

    pub fn exec_evm_bytecode(
        &mut self,
        mut contract: Contract,
        gas: Gas,
        is_static: bool,
        depth: u32,
    ) -> InterpreterResult {
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
                    let return_memory_offset = inputs.return_memory_offset.clone();
                    let (output, gas, exit_code) = self.call_inner(inputs, STATE_MAIN, depth + 1);
                    let result = InterpreterResult::new(
                        evm_error_from_exit_code(ExitCode::from(exit_code)),
                        output,
                        gas,
                    );
                    let call_outcome = CallOutcome::new(result, return_memory_offset);
                    interpreter.insert_call_outcome(&mut shared_memory, call_outcome);
                }
                InterpreterAction::Create { inputs } => {
                    let create_outcome = self.create_inner(inputs, depth + 1);
                    interpreter.insert_create_outcome(create_outcome);
                }
                InterpreterAction::Return { result } => {
                    return result;
                }
                InterpreterAction::None => {
                    unreachable!("not supported EVM interpreter state: None")
                }
                InterpreterAction::EOFCreate { .. } => {
                    unreachable!("not supported EVM interpreter state: EOF")
                }
            }
        }
    }

    pub fn deploy_evm_contract(
        &mut self,
        target_address: Address,
        inputs: Box<CreateInputs>,
        gas: Gas,
        call_depth: u32,
    ) -> InterpreterResult {
        let return_error = |gas: Gas, result: InstructionResult| -> InterpreterResult {
            InterpreterResult::new(result, Bytes::new(), gas)
        };

        let contract = Contract {
            input: Bytes::default(),
            bytecode: to_analysed(Bytecode::new_raw(inputs.init_code)),
            hash: None,
            target_address,
            caller: inputs.caller,
            call_value: inputs.value,
        };
        // execute EVM constructor bytecode to produce new resulting EVM bytecode
        let mut result = self.exec_evm_bytecode(contract, gas, false, call_depth);
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

    pub fn deploy_evm_contract_proxy(
        &mut self,
        target_address: Address,
        inputs: Box<CreateInputs>,
        mut gas: Gas,
        call_depth: u32,
    ) -> InterpreterResult {
        // let return_error = |gas: Gas, result: InstructionResult| -> InterpreterResult {
        //     InterpreterResult::new(result, Bytes::new(), gas)
        // };

        let rwasm_bytecode = create_rwasm_proxy_bytecode(&target_address);

        // write callee changes to a database (lets keep rWASM part empty for now since universal
        // loader is not ready yet)
        let (mut contract_account, _) = self.sdk.account(&target_address);
        contract_account.update_bytecode(self.sdk, Bytes::default(), None, rwasm_bytecode, None);

        let context = ContractContext {
            address: target_address,
            bytecode_address: target_address,
            caller: inputs.caller,
            is_static: false,
            value: inputs.value,
        };
        let (output, exit_code) = self.exec_rwasm_bytecode(
            context,
            &contract_account,
            inputs.init_code.as_ref(),
            &mut gas,
            STATE_DEPLOY,
            call_depth,
        );

        // if bytecode starts with 0xEF or exceeds MAX_CODE_SIZE then return the corresponding error
        // if !result.output.is_empty() && result.output.first() == Some(&0xEF) {
        //     return return_error(gas, InstructionResult::CreateContractStartingWithEF);
        // } else if result.output.len() > MAX_CODE_SIZE {
        //     return return_error(gas, InstructionResult::CreateContractSizeLimit);
        // }

        // record gas for each created byte
        // let gas_for_code = result.output.len() as u64 * gas::CODEDEPOSIT;
        // if !result.gas.record_cost(gas_for_code) {
        //     return return_error(gas, InstructionResult::OutOfGas);
        // }

        InterpreterResult {
            result: evm_error_from_exit_code(ExitCode::from(exit_code)),
            output,
            gas,
        }
    }
}
