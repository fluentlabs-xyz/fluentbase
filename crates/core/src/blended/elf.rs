use crate::{blended::BlendedRuntime, debug_log, helpers::evm_error_from_exit_code};
use alloc::boxed::Box;
use fluentbase_sdk::{
    Account,
    Address,
    Bytes,
    ContractContext,
    ExitCode,
    SovereignAPI,
    STATE_DEPLOY,
};
use revm_interpreter::{gas, CreateInputs, Gas, InterpreterResult};
use sp1_core_executor::{Executor, Program};

impl<'a, SDK: SovereignAPI> BlendedRuntime<'a, SDK> {
    pub fn deploy_elf_contract(
        &mut self,
        target_address: Address,
        inputs: Box<CreateInputs>,
        mut gas: Gas,
        call_depth: u32,
    ) -> InterpreterResult {
        let return_error = |gas: Gas, exit_code: ExitCode| -> InterpreterResult {
            InterpreterResult::new(evm_error_from_exit_code(exit_code), Bytes::new(), gas)
        };

        // TODO(dmitry123): "validate gas/fuel checks"

        // // record gas for each created byte
        let gas_for_code = inputs.init_code.len() as u64 * gas::CODEDEPOSIT;
        if !gas.record_cost(gas_for_code) {
            return return_error(gas, ExitCode::OutOfGas);
        }

        // write callee changes to a database (lets keep rWASM part empty for now since universal
        // loader is not ready yet)
        let (mut contract_account, _) = self.sdk.account(&target_address);
        contract_account.update_bytecode(self.sdk, inputs.init_code, None);

        // execute rWASM deploy function
        let context = ContractContext {
            address: target_address,
            bytecode_address: target_address,
            caller: inputs.caller,
            is_static: false,
            value: inputs.value,
        };
        let (output, exit_code) = self.exec_elf_bytecode(
            context,
            &contract_account,
            Bytes::default(),
            &mut gas,
            STATE_DEPLOY,
            call_depth,
        );

        InterpreterResult {
            result: evm_error_from_exit_code(ExitCode::from(exit_code)),
            output,
            gas,
        }
    }

    pub(crate) fn exec_elf_bytecode(
        &mut self,
        context: ContractContext,
        bytecode_account: &Account,
        input: Bytes,
        gas: &mut Gas,
        state: u32,
        call_depth: u32,
    ) -> (Bytes, i32) {
        let elf_binary = self
            .sdk
            .preimage(&bytecode_account.address, &bytecode_account.code_hash)
            .unwrap_or_default();
        let program = match Program::from(elf_binary.as_ref()) {
            Ok(program) => program,
            Err(err) => {
                debug_log!("sp1 parse error: {}", err);
                return (Bytes::default(), ExitCode::CompilationError.into_i32());
            }
        };
        let mut executor = Executor::new(program, Default::default());
        match executor.run() {
            Ok(_) => {}
            Err(err) => {
                debug_log!("sp1 execution error: {}", err);
                return (Bytes::default(), ExitCode::Panic.into_i32());
            }
        }
        unreachable!("")
    }
}
