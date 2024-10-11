#[cfg(feature = "elf")]
mod elf;
mod evm;
mod syscall;
mod util;
mod wasm;

use crate::{debug_log, helpers::evm_error_from_exit_code, types::NextAction};
use alloc::boxed::Box;
use fluentbase_sdk::{
    codec::Encoder,
    env_from_context,
    Account,
    AccountStatus,
    BytecodeType,
    Bytes,
    ContractContext,
    ExitCode,
    NativeAPI,
    SharedContextInputV1,
    SovereignAPI,
    SyscallInvocationParams,
    STATE_MAIN,
};
use revm_interpreter::{
    CallInputs,
    CallOutcome,
    CreateInputs,
    CreateOutcome,
    Gas,
    InterpreterResult,
};
use revm_primitives::{
    CreateScheme,
    Env,
    MAX_CALL_STACK_LIMIT,
    MAX_INITCODE_SIZE,
    WASM_MAX_CODE_SIZE,
};
pub use util::{create_rwasm_proxy_bytecode, ENABLE_EVM_PROXY_CONTRACT};

pub struct BlendedRuntime<'a, SDK> {
    sdk: &'a mut SDK,
    env: Env,
}

impl<'a, SDK: SovereignAPI> BlendedRuntime<'a, SDK> {
    pub fn new(sdk: &'a mut SDK) -> Self {
        Self {
            env: env_from_context(sdk.block_context(), sdk.tx_context()),
            sdk,
        }
    }

    fn process_exec_params(&mut self, exit_code: i32, gas_used: u64) -> NextAction {
        // if the exit code is non-positive (stands for termination), then execution is finished
        if exit_code <= 0 {
            let return_data = self.sdk.native_sdk().return_data();
            return NextAction::ExecutionResult {
                exit_code,
                output: return_data,
                gas_used,
            };
        }

        // otherwise, exit code is a "call_id" that identifies saved context
        let call_id = exit_code as u32;

        // try to parse execution params, if it's not possible then return an error
        let exec_params = self.sdk.native_sdk().return_data();
        let Some(params) = SyscallInvocationParams::from_slice(exec_params.as_ref()) else {
            unreachable!("can't decode invocation params");
        };

        NextAction::NestedCall {
            call_id,
            params,
            gas_used,
        }
    }

    fn process_exec(
        &mut self,
        contract_context: &ContractContext,
        params: SyscallInvocationParams,
    ) -> NextAction {
        // warmup bytecode before execution,
        // it's a technical limitation we're having right now,
        // planning to solve it in the future
        #[cfg(feature = "std")]
        if !fluentbase_runtime::Runtime::is_warm_bytecode(&params.code_hash) {
            let bytecode = self
                .sdk
                .preimage(&contract_context.bytecode_address, &params.code_hash)
                .unwrap_or_default();
            fluentbase_runtime::Runtime::warmup_bytecode(params.code_hash, bytecode);
        }

        let mut context_input = SharedContextInputV1 {
            block: self.sdk.block_context().clone(),
            tx: self.sdk.tx_context().clone(),
            contract: contract_context.clone(),
        }
        .encode_to_vec(0);
        context_input.extend_from_slice(params.input.as_ref());

        // #[cfg(feature = "std")]
        // if contract_context.bytecode_address == PRECOMPILE_EVM {
        //     let runtime_ctx = RuntimeContext::new_with_hash(params.code_hash)
        //         .with_input(context_input)
        //         .with_fuel(params.fuel_limit)
        //         .with_state(params.state)
        //         .with_depth(call_depth)
        //         .with_jzkt(Box::new(DefaultEmptyRuntimeDatabase::default()));
        //     let native_sdk = RuntimeContextWrapper::new(runtime_ctx);
        //     let sdk = SharedContextImpl::parse_from_input(native_sdk);
        //     let mut evm_loader2 = EvmLoaderEntrypoint::new(sdk);
        //     let exit_code = if params.state == STATE_DEPLOY {
        //         evm_loader2.deploy_inner()
        //     } else {
        //         evm_loader2.main_inner()
        //     };
        //     return self.process_exec_params(exit_code.into_i32(), 0);
        // }

        // execute smart contract
        let (fuel_consumed, exit_code) = self.sdk.native_sdk().exec(
            &params.code_hash,
            &context_input,
            params.fuel_limit,
            params.state,
        );

        self.process_exec_params(exit_code, fuel_consumed)
    }

    fn process_resume(
        &mut self,
        call_id: u32,
        return_data: &[u8],
        exit_code: i32,
        fuel_used: u64,
    ) -> NextAction {
        let (fuel_spent, exit_code) =
            self.sdk
                .native_sdk()
                .resume(call_id, return_data, exit_code, fuel_used);
        debug_log!(
            "process_resume: call_id={}, fuel_spent={} exit_code={}",
            call_id,
            fuel_spent,
            exit_code
        );
        self.process_exec_params(exit_code, fuel_spent)
    }

    fn exec_bytecode(
        &mut self,
        context: ContractContext,
        bytecode_account: &Account,
        input: Bytes,
        gas: &mut Gas,
        state: u32,
        call_depth: u32,
    ) -> (Bytes, i32) {
        let bytecode = self
            .sdk
            .preimage(&bytecode_account.address, &bytecode_account.code_hash)
            .unwrap_or_default();
        let bytecode_type = BytecodeType::from_slice(bytecode.as_ref());
        match bytecode_type {
            BytecodeType::EVM => {
                self.exec_evm_bytecode(context, bytecode_account, input, gas, state, call_depth)
            }
            BytecodeType::WASM => {
                self.exec_rwasm_bytecode(context, bytecode_account, input, gas, state, call_depth)
            }
            #[cfg(feature = "elf")]
            BytecodeType::ELF => {
                self.exec_elf_bytecode(context, bytecode_account, input, gas, state, call_depth)
            }
            _ => unreachable!("not supported bytecode type"),
        }
    }

    fn create_inner(&mut self, inputs: Box<CreateInputs>, call_depth: u32) -> CreateOutcome {
        let return_error = |gas: Gas, exit_code: ExitCode| -> CreateOutcome {
            CreateOutcome::new(
                InterpreterResult::new(evm_error_from_exit_code(exit_code), Bytes::new(), gas),
                None,
            )
        };
        let gas = Gas::new(inputs.gas_limit);

        // determine bytecode type
        let bytecode_type = BytecodeType::from_slice(&inputs.init_code);

        // load deployer and contract accounts
        let (mut caller_account, _) = self.sdk.account(&inputs.caller);
        if caller_account.balance < inputs.value {
            return return_error(gas, ExitCode::InsufficientBalance);
        }

        // call depth check
        if call_depth > MAX_CALL_STACK_LIMIT {
            return return_error(gas, ExitCode::CallDepthOverflow);
        }

        // check init max code size for EIP-3860
        if inputs.init_code.len()
            > match bytecode_type {
                BytecodeType::EVM => MAX_INITCODE_SIZE,
                _ => WASM_MAX_CODE_SIZE,
            }
        {
            return return_error(gas, ExitCode::ContractSizeLimit);
        }

        // calc source code hash
        let source_code_hash = SDK::keccak256(inputs.init_code.as_ref());

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

        let result = match bytecode_type {
            BytecodeType::EVM => {
                if ENABLE_EVM_PROXY_CONTRACT {
                    self.deploy_evm_contract_proxy(
                        contract_account.address,
                        inputs,
                        gas,
                        call_depth,
                    )
                } else {
                    self.deploy_evm_contract(contract_account.address, inputs, gas, call_depth)
                }
            }
            BytecodeType::WASM => {
                self.deploy_wasm_contract(contract_account.address, inputs, gas, call_depth)
            }
            #[cfg(feature = "elf")]
            BytecodeType::ELF => {
                self.deploy_elf_contract(contract_account.address, inputs, gas, call_depth)
            }
            #[cfg(not(feature = "elf"))]
            _ => unreachable!("not supported bytecode type"),
        };

        // commit all changes made
        if result.result.is_ok() {
            self.sdk.commit();
        } else {
            self.sdk.rollback(checkpoint);
        }

        CreateOutcome::new(result, Some(contract_account.address))
    }

    fn call_inner(
        &mut self,
        inputs: Box<CallInputs>,
        state: u32,
        call_depth: u32,
    ) -> (Bytes, Gas, i32) {
        let return_error = |gas: Gas, exit_code: ExitCode| -> (Bytes, Gas, i32) {
            (Bytes::default(), gas, exit_code.into_i32())
        };
        let mut gas = Gas::new(inputs.gas_limit);

        // call depth check
        if call_depth > MAX_CALL_STACK_LIMIT {
            return return_error(gas, ExitCode::CallDepthOverflow);
        }

        // read caller and callee
        let (mut caller_account, _) = self.sdk.account(&inputs.caller);
        let (mut callee_account, _) = self.sdk.account(&inputs.target_address);

        // create a new checkpoint position in the journal
        let checkpoint = self.sdk.checkpoint();

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
            // calculate total gas consumed by precompiled call
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
            return (result.output, gas, result.exit_code.into_i32());
        }

        let (bytecode_account, _) = self.sdk.account(&inputs.bytecode_address);
        let contract_context = ContractContext {
            address: inputs.target_address,
            bytecode_address: bytecode_account.address,
            caller: inputs.caller,
            is_static: inputs.is_static,
            value: inputs.value.get(),
        };
        let (output, exit_code) = self.exec_bytecode(
            contract_context,
            &bytecode_account,
            inputs.input,
            &mut gas,
            state,
            call_depth,
        );

        if ExitCode::from(exit_code).is_ok() {
            self.sdk.commit();
        } else {
            self.sdk.rollback(checkpoint);
        }

        (output, gas, exit_code)
    }

    pub fn create(&mut self, create_inputs: Box<CreateInputs>) -> CreateOutcome {
        self.create_inner(create_inputs, 0)
    }

    pub fn call(&mut self, inputs: Box<CallInputs>) -> CallOutcome {
        let (output, gas, exit_code) = self.call_inner(inputs, STATE_MAIN, 0);
        let result = InterpreterResult {
            result: evm_error_from_exit_code(ExitCode::from(exit_code)),
            output,
            gas,
        };
        CallOutcome::new(result, Default::default())
    }
}
