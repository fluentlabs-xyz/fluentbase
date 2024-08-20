use crate::{
    blended::BlendedRuntime,
    helpers::{evm_error_from_exit_code, wasm2rwasm},
};
use alloc::boxed::Box;
use fluentbase_sdk::{Address, Bytes, ContractContext, ExitCode, SovereignAPI, STATE_DEPLOY};
use revm_interpreter::{gas, CreateInputs, Gas, InterpreterResult};

impl<'a, SDK: SovereignAPI> BlendedRuntime<'a, SDK> {
    pub fn deploy_wasm_contract(
        &mut self,
        target_address: Address,
        inputs: Box<CreateInputs>,
        mut gas: Gas,
        call_depth: u32,
    ) -> InterpreterResult {
        let return_error = |gas: Gas, exit_code: ExitCode| -> InterpreterResult {
            InterpreterResult::new(evm_error_from_exit_code(exit_code), Bytes::new(), gas)
        };

        // translate WASM to rWASM
        let rwasm_bytecode = match wasm2rwasm(inputs.init_code.as_ref()) {
            Ok(rwasm_bytecode) => rwasm_bytecode,
            Err(exit_code) => {
                return return_error(gas, exit_code);
            }
        };

        // // record gas for each created byte
        let gas_for_code = rwasm_bytecode.len() as u64 * gas::CODEDEPOSIT;
        if !gas.record_cost(gas_for_code) {
            return return_error(gas, ExitCode::OutOfGas);
        }

        // write callee changes to a database (lets keep rWASM part empty for now since universal
        // loader is not ready yet)
        let (mut contract_account, _) = self.sdk.account(&target_address);
        contract_account.update_bytecode(self.sdk, Bytes::new(), None, rwasm_bytecode.into(), None);

        // execute rWASM deploy function
        let context = ContractContext {
            address: target_address,
            caller: inputs.caller,
            value: inputs.value,
        };
        let (output, exit_code) = self.exec_rwasm_bytecode(
            context,
            &contract_account,
            &[],
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
}
