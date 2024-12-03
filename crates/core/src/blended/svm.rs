use crate::{
    blended::{util::create_rwasm_proxy_bytecode, BlendedRuntime},
    helpers::{evm_error_from_exit_code, exit_code_from_evm_error},
};
use alloc::boxed::Box;
use core::mem::take;
use fluentbase_sdk::{
    Account,
    Address,
    Bytes,
    ContractContext,
    ExitCode,
    SovereignAPI,
    B256,
    PRECOMPILE_SVM,
    STATE_DEPLOY,
    STATE_MAIN,
};
use revm_interpreter::{
    analysis::to_analysed,
    gas,
    opcode::make_instruction_table,
    CallOutcome,
    Contract,
    CreateInputs,
    Gas,
    InstructionResult,
    Interpreter,
    InterpreterAction,
    InterpreterResult,
    SharedMemory,
};
use revm_primitives::{Bytecode, CancunSpec, MAX_CODE_SIZE};

impl<SDK: SovereignAPI> BlendedRuntime<SDK> {
    pub fn load_svm_bytecode(&self, address: &Address) -> (Bytecode, B256) {
        // let address = Address::left_padding_from(&address[12..32]);
        let (account, _) = self.sdk.account(&address);
        let bytecode = self
            .sdk
            .preimage(address, &account.code_hash)
            .unwrap_or_default();
        let bytecode = Bytecode::new_raw(bytecode);
        (bytecode, account.code_hash)
    }

    pub fn store_svm_bytecode(&mut self, address: &Address, code_hash: B256, bytecode: Bytecode) {
        // let address = Address::left_padding_from(&address[12..32]);
        self.sdk
            .write_preimage(*address, code_hash, bytecode.original_bytes());
    }

    pub fn exec_svm_bytecode(
        &mut self,
        context: ContractContext,
        _bytecode_account: &Account,
        input: Bytes,
        gas: &mut Gas,
        _state: u32,
        call_depth: u32,
    ) -> (Bytes, i32) {
        // take right bytecode depending on context params
        let (svm_bytecode, code_hash) = self.load_svm_bytecode(&context.bytecode_address);

        // if bytecode is empty, then commit result and return empty buffer
        if svm_bytecode.is_empty() {
            return (Bytes::default(), ExitCode::Ok.into_i32());
        }

        // initiate contract instance and pass it to interpreter for and SVM transition
        let contract = Contract {
            input,
            hash: Some(code_hash),
            bytecode: svm_bytecode,
            // we don't take contract callee, because callee refers to address with bytecode
            target_address: context.address,
            // inside the contract context, we pass "apparent" value that can be different to
            // transfer value (it can happen for DELEGATECALL or CALLCODE opcodes)
            call_value: context.value,
            caller: context.caller,
            bytecode_address: None,
        };
        let result = self.exec_contract(contract, take(gas), context.is_static, call_depth);
        *gas = result.gas;
        (
            result.output,
            exit_code_from_evm_error(result.result).into_i32(),
        )
    }

    pub fn exec_contract(
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
                    unreachable!("not supported SVM interpreter state: None")
                }
                InterpreterAction::EOFCreate { .. } => {
                    unreachable!("not supported SVM interpreter state: EOF")
                }
            }
        }
    }

    pub fn deploy_svm_contract(
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
            bytecode_address: None,
            caller: inputs.caller,
            call_value: inputs.value,
        };
        // execute EVM constructor bytecode to produce new resulting EVM bytecode
        let mut result = self.exec_contract(contract, gas, false, call_depth);
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
        let svm_bytecode = Bytecode::new_raw(result.output.clone());
        let code_hash = svm_bytecode.hash_slow();
        contract_account.update_bytecode(
            &mut self.sdk,
            svm_bytecode.original_bytes(),
            Some(code_hash),
        );

        // if there is an address, then we have new SVM bytecode inside output
        self.store_svm_bytecode(&target_address, code_hash, svm_bytecode);

        result
    }

    pub fn deploy_svm_contract_proxy(
        &mut self,
        target_address: Address,
        inputs: Box<CreateInputs>,
        mut gas: Gas,
        call_depth: u32,
    ) -> InterpreterResult {
        let rwasm_bytecode = create_rwasm_proxy_bytecode(PRECOMPILE_SVM);

        // write callee changes to a database (lets keep rWASM part empty for now since universal
        // loader is not ready yet)
        let (mut contract_account, _) = self.sdk.account(&target_address);
        contract_account.update_bytecode(&mut self.sdk, rwasm_bytecode, None);

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
            inputs.init_code,
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
