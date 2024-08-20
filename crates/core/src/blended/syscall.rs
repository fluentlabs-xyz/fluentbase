use crate::{blended::BlendedRuntime, helpers::exit_code_from_evm_error, types::NextAction};
use alloc::{boxed::Box, vec::Vec};
use fluentbase_sdk::{
    Address,
    Bytes,
    ContractContext,
    ExitCode,
    SovereignAPI,
    SyscallInvocationParams,
    B256,
    STATE_MAIN,
    U256,
};
use revm_interpreter::{
    gas,
    gas::{sstore_cost, COLD_SLOAD_COST, WARM_STORAGE_READ_COST},
    CallInputs,
    CallScheme,
    CallValue,
    CreateInputs,
    SelfDestructResult,
};
use revm_primitives::{CreateScheme, SpecId};

impl<'a, SDK: SovereignAPI> BlendedRuntime<'a, SDK> {
    pub(crate) fn syscall_storage_read(
        &mut self,
        context: &ContractContext,
        params: SyscallInvocationParams,
    ) -> NextAction {
        // make sure input is 32 bytes len, and we have enough gas to pay for the call
        if params.input.len() != 32 {
            return NextAction::from_exit_code(params.fuel_limit, ExitCode::MalformedSyscallParams);
        }

        // read value from storage
        let (value, is_cold) = self.sdk.storage(
            &context.address,
            &U256::from_le_slice(params.input.as_ref()),
        );

        let gas_cost = if is_cold {
            COLD_SLOAD_COST
        } else {
            WARM_STORAGE_READ_COST
        };

        // return value as bytes with success exit code
        NextAction::ExecutionResult(
            value.to_le_bytes::<32>().into(),
            gas_cost,
            ExitCode::Ok.into_i32(),
        )
    }

    pub(crate) fn syscall_storage_write(
        &mut self,
        context: &ContractContext,
        params: SyscallInvocationParams,
    ) -> NextAction {
        // make sure input is 32 bytes len, and we have enough gas to pay for the call
        if params.input.len() != 64 {
            return NextAction::from_exit_code(params.fuel_limit, ExitCode::MalformedSyscallParams);
        }

        let slot = U256::from_le_slice(&params.input[0..32]);
        let value = U256::from_le_slice(&params.input[32..64]);

        let (original_value, _) = self.sdk.committed_storage(&context.address, &slot);
        let (present_value, is_cold) = self.sdk.storage(&context.address, &slot);
        self.sdk.write_storage(context.address, slot, value);

        let gas_cost = match sstore_cost(
            SpecId::CANCUN,
            original_value,
            present_value,
            value,
            params.fuel_limit,
            is_cold,
        ) {
            Some(gas_cost) => {
                if params.fuel_limit < gas_cost {
                    return NextAction::from_exit_code(params.fuel_limit, ExitCode::OutOfGas);
                }
                gas_cost
            }
            None => return NextAction::from_exit_code(params.fuel_limit, ExitCode::OutOfGas),
        };

        let _is_cold = self.sdk.write_storage(context.address, slot, value);
        NextAction::ExecutionResult(Bytes::default(), gas_cost, ExitCode::Ok.into_i32())
    }

    pub(crate) fn syscall_call(
        &mut self,
        context: &ContractContext,
        params: SyscallInvocationParams,
        call_depth: u32,
        is_static: bool,
    ) -> NextAction {
        // make sure we have enough bytes inside input params, where:
        // - 20 bytes for the target address
        // - 32 bytes for the call value
        if params.input.len() < 20 + 32 {
            return NextAction::from_exit_code(params.fuel_limit, ExitCode::MalformedSyscallParams);
        }

        let target_address = Address::from_slice(&params.input[0..20]);
        let value = U256::from_le_slice(&params.input[20..52]);
        let contract_input = Bytes::copy_from_slice(&params.input[52..]);

        // execute a nested call to another binary
        let (output, gas, exit_code) = self.call_inner(
            Box::new(CallInputs {
                input: contract_input,
                return_memory_offset: Default::default(),
                gas_limit: params.fuel_limit,
                bytecode_address: target_address,
                target_address,
                caller: context.address,
                value: CallValue::Transfer(value),
                scheme: CallScheme::Call,
                is_static,
                is_eof: false,
            }),
            STATE_MAIN,
            call_depth + 1,
        );

        NextAction::ExecutionResult(output, params.fuel_limit - gas.remaining(), exit_code)
    }

    pub(crate) fn syscall_delegate_call(
        &mut self,
        context: &ContractContext,
        params: SyscallInvocationParams,
        call_depth: u32,
    ) -> NextAction {
        // make sure we have enough bytes inside input params, where:
        // - 20 bytes for the target address
        if params.input.len() < 20 {
            return NextAction::from_exit_code(params.fuel_limit, ExitCode::MalformedSyscallParams);
        }

        let target_address = Address::from_slice(&params.input[0..20]);
        let contract_input = Bytes::copy_from_slice(&params.input[20..]);

        // execute a nested call to another binary
        let (output, gas, exit_code) = self.call_inner(
            Box::new(CallInputs {
                input: contract_input,
                return_memory_offset: Default::default(),
                gas_limit: params.fuel_limit,
                bytecode_address: target_address,
                target_address: context.address,
                caller: context.caller,
                value: CallValue::Apparent(context.value),
                scheme: CallScheme::DelegateCall,
                is_static: false,
                is_eof: false,
            }),
            params.state,
            call_depth + 1,
        );

        NextAction::ExecutionResult(output, gas.remaining(), exit_code)
    }

    pub(crate) fn syscall_emit_log(
        &mut self,
        context: &ContractContext,
        params: SyscallInvocationParams,
    ) -> NextAction {
        // make sure input is 32 bytes len, and we have enough gas to pay for the call
        if params.input.len() < 1 {
            return NextAction::from_exit_code(params.fuel_limit, ExitCode::MalformedSyscallParams);
        }

        // read topics from input
        let topics_len = params.input[0] as usize;
        if topics_len > 4 {
            return NextAction::from_exit_code(params.fuel_limit, ExitCode::MalformedSyscallParams);
        }
        let mut topics = Vec::new();
        if params.input.len() < 1 + topics_len * B256::len_bytes() {
            return NextAction::from_exit_code(params.fuel_limit, ExitCode::MalformedSyscallParams);
        }
        for i in 0..topics_len {
            let topic = &params.input
                [(1 + i * B256::len_bytes())..(1 + i * B256::len_bytes() + B256::len_bytes())];
            topics.push(B256::from_slice(topic));
        }

        // all remaining bytes are data
        let data = Bytes::copy_from_slice(&params.input[(1 + topics_len * B256::len_bytes())..]);

        // make sure we have enough gas to cover this operation
        let Some(gas_cost) = gas::log_cost(topics_len as u8, data.len() as u64) else {
            return NextAction::from_exit_code(params.fuel_limit, ExitCode::OutOfGas);
        };

        // write new log into the journal
        self.sdk.write_log(context.address, data, topics);

        NextAction::ExecutionResult(Bytes::default(), gas_cost, ExitCode::Ok.into_i32())
    }

    pub(crate) fn syscall_destroy_account(
        &mut self,
        context: &ContractContext,
        params: SyscallInvocationParams,
    ) -> NextAction {
        // make sure input is 32 bytes len, and we have enough gas to pay for the call
        if params.input.len() != 20 {
            return NextAction::from_exit_code(params.fuel_limit, ExitCode::MalformedSyscallParams);
        }

        let result = self
            .sdk
            .destroy_account(&context.address, &Address::from_slice(&params.input[0..20]));

        let gas_cost = gas::selfdestruct_cost(
            SpecId::CANCUN,
            SelfDestructResult {
                had_value: result.had_value,
                target_exists: result.target_exists,
                is_cold: result.is_cold,
                previously_destroyed: result.previously_destroyed,
            },
        );

        // return value as bytes with success exit code
        NextAction::ExecutionResult(Bytes::default(), gas_cost, ExitCode::Ok.into_i32())
    }

    pub(crate) fn syscall_create(
        &mut self,
        context: &ContractContext,
        params: SyscallInvocationParams,
        call_depth: u32,
    ) -> NextAction {
        // make sure we have enough bytes inside input params, where:
        // - 1 byte for salt option flag
        // - 32 bytes for salt
        if params.input.len() < 1 + 32 {
            return NextAction::from_exit_code(params.fuel_limit, ExitCode::MalformedSyscallParams);
        }

        let is_create2 = params.input[0] != 0;
        let create_scheme = if is_create2 {
            CreateScheme::Create2 {
                salt: U256::from_le_slice(&params.input[1..33]),
            }
        } else {
            CreateScheme::Create
        };

        let init_code = Bytes::copy_from_slice(&params.input[33..]);

        // execute a nested call to another binary
        let call_outcome = self.create_inner(
            Box::new(CreateInputs {
                caller: context.address,
                scheme: create_scheme,
                // actually, "apparent" value is not possible for CREATE
                value: context.value,
                init_code,
                gas_limit: params.fuel_limit,
            }),
            call_depth + 1,
        );

        NextAction::ExecutionResult(
            call_outcome.result.output,
            call_outcome.result.gas.remaining(),
            exit_code_from_evm_error(call_outcome.result.result).into_i32(),
        )
    }
}
