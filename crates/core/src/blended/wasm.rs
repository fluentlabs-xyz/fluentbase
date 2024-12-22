use crate::{
    blended::BlendedRuntime,
    helpers::{evm_error_from_exit_code, wasm2rwasm},
    types::{Frame, NextAction},
};
use alloc::{boxed::Box, vec, vec::Vec};
use core::mem::take;
use fluentbase_sdk::{
    debug_log,
    Account,
    Address,
    Bytes,
    ContractContext,
    ExitCode,
    SovereignAPI,
    SyscallInvocationParams,
    STATE_DEPLOY,
};
use revm_interpreter::{gas, CreateInputs, Gas, InterpreterResult};

impl<SDK: SovereignAPI> BlendedRuntime<SDK> {
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
        contract_account.update_bytecode(&mut self.sdk, rwasm_bytecode.into(), None);

        // execute rWASM deploy function
        let context = ContractContext {
            address: target_address,
            bytecode_address: target_address,
            caller: inputs.caller,
            is_static: false,
            value: inputs.value,
        };
        let (output, exit_code) = self.exec_rwasm_bytecode(
            context,
            contract_account,
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

    pub(crate) fn exec_rwasm_bytecode(
        &mut self,
        context: ContractContext,
        bytecode_account: Account,
        input: Bytes,
        gas: &mut Gas,
        state: u32,
        call_depth: u32,
    ) -> (Bytes, i32) {
        let mut call_stack: Vec<Frame> = vec![Frame::Execute {
            params: SyscallInvocationParams {
                code_hash: bytecode_account.code_hash,
                input,
                fuel_limit: gas.remaining(),
                state,
            },
            call_id: 0,
        }];

        let mut stack_frame = call_stack.last_mut().unwrap();
        println!("Stack frame: {:?}", stack_frame);
        let (output, fuel_used, exit_code) = loop {
            let next_action = match stack_frame {
                Frame::Execute {
                    params,
                    ref call_id,
                } => {
                    if *call_id > 0 {
                        self.process_syscall(&context, take(params), call_depth)
                    } else {
                        self.process_exec(&context, take(params))
                    }
                }
                Frame::Resume {
                    ref call_id,
                    ref output,
                    ref exit_code,
                    ref gas_used,
                } => self.process_resume(*call_id, output.as_ref(), *exit_code, *gas_used),
            };
            debug_log!("Next action: {:?}", next_action);
            match next_action {
                NextAction::ExecutionResult {
                    exit_code,
                    output: return_data,
                    gas_used,
                } => {
                    let _resumable_frame = call_stack.pop().unwrap();
                    if call_stack.is_empty() {
                        break (return_data, gas_used, exit_code);
                    }
                    // execution result can happen only after resume
                    match call_stack.last_mut().unwrap() {
                        Frame::Resume {
                            output: ref mut return_data_result,
                            exit_code: ref mut exit_code_result,
                            gas_used: ref mut fuel_used_result,
                            ..
                        } => {
                            *return_data_result = return_data;
                            *exit_code_result = exit_code;
                            *fuel_used_result += gas_used;
                        }
                        _ => unreachable!(),
                    }
                    stack_frame = call_stack.last_mut().unwrap();
                }
                NextAction::NestedCall {
                    call_id,
                    params,
                    gas_used,
                } => {
                    let last_frame = call_stack.last_mut().unwrap();
                    match last_frame {
                        Frame::Execute { .. } => {
                            *last_frame = Frame::Resume {
                                call_id,
                                output: Bytes::default(),
                                exit_code: 0,
                                gas_used,
                            };
                        }
                        Frame::Resume {
                            call_id: ref mut call_id_result,
                            gas_used: ref mut gas_used_result,
                            ..
                        } => {
                            *call_id_result = call_id;
                            println!("Gas used: {:?} {}", gas_used, gas.spent());
                            *gas_used_result = gas_used;
                        }
                    }
                    call_stack.push(Frame::Execute { params, call_id });
                    stack_frame = call_stack.last_mut().unwrap();
                }
            }
        };
        println!("Final Gas used: {:?} {}", fuel_used, gas.spent());
        if !gas.record_cost(fuel_used) {
            return (Bytes::default(), ExitCode::OutOfGas.into_i32());
        }

        (output, exit_code)
    }
}
