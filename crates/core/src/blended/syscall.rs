use crate::{blended::BlendedRuntime, helpers::exit_code_from_evm_error, types::NextAction};
use alloc::{boxed::Box, vec::Vec};
use byteorder::{ByteOrder, LittleEndian};
use fluentbase_sdk::{
    Address,
    Bytes,
    ContractContext,
    ExitCode,
    NativeAPI,
    SovereignAPI,
    SyscallInvocationParams,
    B256,
    STATE_MAIN,
    U256,
};
use revm_interpreter::{gas, CallInputs, CallScheme, CallValue, CreateInputs, SelfDestructResult};
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
            gas::COLD_SLOAD_COST
        } else {
            gas::WARM_STORAGE_READ_COST
        };

        // make sure we have enough gas for this op
        if params.fuel_limit < gas_cost {
            return NextAction::from_exit_code(params.fuel_limit, ExitCode::OutOfGas);
        }

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

        let gas_cost = match gas::sstore_cost(
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

        // make sure we have enough gas for this op
        if params.fuel_limit < gas_cost {
            return NextAction::from_exit_code(params.fuel_limit, ExitCode::OutOfGas);
        }

        let _is_cold = self.sdk.write_storage(context.address, slot, value);
        NextAction::ExecutionResult(Bytes::default(), gas_cost, ExitCode::Ok.into_i32())
    }

    pub(crate) fn syscall_call(
        &mut self,
        context: &ContractContext,
        params: SyscallInvocationParams,
        call_depth: u32,
    ) -> NextAction {
        // make sure we have enough bytes inside input params, where:
        // - 20 bytes for the target address
        // - 32 bytes for the call value
        if params.input.len() < 20 + 32 {
            return NextAction::from_exit_code(params.fuel_limit, ExitCode::MalformedSyscallParams);
        }

        let target_address = Address::from_slice(&params.input[0..20]);
        let value = U256::from_le_slice(&params.input[20..52]);
        let contract_input = params.input.slice(52..);

        let (account, is_cold) = self.sdk.account(&target_address);

        // make sure we have enough gas for the call
        let call_gas_cost = gas::call_cost(
            SpecId::CANCUN,
            !value.is_zero(),
            is_cold,
            account.is_empty(),
        );
        if params.fuel_limit < call_gas_cost {
            return NextAction::from_exit_code(params.fuel_limit, ExitCode::OutOfGas);
        }

        // add call stipend if there is a value to be transferred.
        let mut gas_limit = params.fuel_limit - call_gas_cost;
        if !value.is_zero() {
            gas_limit = gas_limit.saturating_add(gas::CALL_STIPEND);
        }

        // execute a nested call to another binary
        let (output, gas, exit_code) = self.call_inner(
            Box::new(CallInputs {
                input: contract_input,
                return_memory_offset: Default::default(),
                gas_limit,
                bytecode_address: target_address,
                target_address,
                caller: context.address,
                value: CallValue::Transfer(value),
                scheme: CallScheme::Call,
                is_static: false,
                is_eof: false,
            }),
            STATE_MAIN,
            call_depth + 1,
        );

        NextAction::ExecutionResult(output, call_gas_cost + gas.spent(), exit_code)
    }

    pub(crate) fn syscall_static_call(
        &mut self,
        context: &ContractContext,
        params: SyscallInvocationParams,
        call_depth: u32,
    ) -> NextAction {
        // make sure we have enough bytes inside input params, where:
        // - 20 bytes for the target address
        // - 32 bytes for the call value
        if params.input.len() < 20 {
            return NextAction::from_exit_code(params.fuel_limit, ExitCode::MalformedSyscallParams);
        }

        let target_address = Address::from_slice(&params.input[0..20]);
        let contract_input = params.input.slice(20..);

        let (_, is_cold) = self.sdk.account(&target_address);

        // make sure we have enough gas for the call
        let call_gas_cost = gas::call_cost(SpecId::CANCUN, false, is_cold, false);
        if params.fuel_limit < call_gas_cost {
            return NextAction::from_exit_code(params.fuel_limit, ExitCode::OutOfGas);
        }

        // execute a nested call to another binary
        let (output, gas, exit_code) = self.call_inner(
            Box::new(CallInputs {
                input: contract_input,
                return_memory_offset: Default::default(),
                gas_limit: params.fuel_limit - call_gas_cost,
                bytecode_address: target_address,
                target_address,
                caller: context.address,
                value: CallValue::Transfer(U256::ZERO),
                scheme: CallScheme::StaticCall,
                is_static: true,
                is_eof: false,
            }),
            STATE_MAIN,
            call_depth + 1,
        );

        NextAction::ExecutionResult(output, call_gas_cost + gas.spent(), exit_code)
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
        let contract_input = params.input.slice(20..);

        let (_, is_cold) = self.sdk.account(&target_address);

        // make sure we have enough gas for the call
        let call_gas_cost = gas::call_cost(SpecId::CANCUN, false, is_cold, false);
        if params.fuel_limit < call_gas_cost {
            return NextAction::from_exit_code(params.fuel_limit, ExitCode::OutOfGas);
        }

        // execute a nested call to another binary
        let (output, gas, exit_code) = self.call_inner(
            Box::new(CallInputs {
                input: contract_input,
                return_memory_offset: Default::default(),
                gas_limit: params.fuel_limit - call_gas_cost,
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

        NextAction::ExecutionResult(output, call_gas_cost + gas.spent(), exit_code)
    }

    pub(crate) fn syscall_call_code(
        &mut self,
        context: &ContractContext,
        params: SyscallInvocationParams,
        call_depth: u32,
    ) -> NextAction {
        // make sure we have enough bytes inside input params, where:
        // - 20 bytes for the target address
        if params.input.len() < 20 + 32 {
            return NextAction::from_exit_code(params.fuel_limit, ExitCode::MalformedSyscallParams);
        }

        let target_address = Address::from_slice(&params.input[0..20]);
        let value = U256::from_le_slice(&params.input[20..52]);
        let contract_input = params.input.slice(52..);

        let (_, is_cold) = self.sdk.account(&target_address);

        // make sure we have enough gas for the call
        let call_gas_cost = gas::call_cost(SpecId::CANCUN, !value.is_zero(), is_cold, false);
        if params.fuel_limit < call_gas_cost {
            return NextAction::from_exit_code(params.fuel_limit, ExitCode::OutOfGas);
        }

        // add call stipend if there is a value to be transferred.
        let mut gas_limit = params.fuel_limit - call_gas_cost;
        if !value.is_zero() {
            gas_limit = gas_limit.saturating_add(gas::CALL_STIPEND);
        }

        // execute a nested call to another binary
        let (output, gas, exit_code) = self.call_inner(
            Box::new(CallInputs {
                input: contract_input,
                return_memory_offset: Default::default(),
                gas_limit,
                bytecode_address: target_address,
                target_address: context.address,
                caller: context.address,
                value: CallValue::Transfer(value),
                scheme: CallScheme::CallCode,
                is_static: false,
                is_eof: false,
            }),
            params.state,
            call_depth + 1,
        );

        NextAction::ExecutionResult(output, call_gas_cost + gas.spent(), exit_code)
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

        let init_code = params.input.slice(33..);

        let is_create2 = params.input[0] != 0;
        let (create_scheme, create_gas_cost) = if is_create2 {
            let Some(gas_cost) = gas::create2_cost(init_code.len() as u64) else {
                return NextAction::from_exit_code(params.fuel_limit, ExitCode::OutOfGas);
            };
            (
                CreateScheme::Create2 {
                    salt: U256::from_le_slice(&params.input[1..33]),
                },
                gas_cost,
            )
        } else {
            (CreateScheme::Create, gas::CREATE)
        };

        let gas_limit = params.fuel_limit - params.fuel_limit / 64 - create_gas_cost;

        // execute a nested call to another binary
        let mut call_outcome = self.create_inner(
            Box::new(CreateInputs {
                caller: context.address,
                scheme: create_scheme,
                value: context.value,
                init_code,
                gas_limit,
            }),
            call_depth + 1,
        );

        call_outcome.result.output = if call_outcome.result.is_ok() {
            Bytes::copy_from_slice(call_outcome.address.unwrap().as_slice())
        } else {
            call_outcome.result.output
        };

        let exit_code = exit_code_from_evm_error(call_outcome.result.result).into_i32();
        NextAction::ExecutionResult(
            call_outcome.result.output,
            create_gas_cost + call_outcome.result.gas.spent(),
            exit_code,
        )
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
        let data = params.input.slice((1 + topics_len * B256::len_bytes())..);

        // make sure we have enough gas to cover this operation
        let Some(gas_cost) = gas::log_cost(topics_len as u8, data.len() as u64) else {
            return NextAction::from_exit_code(params.fuel_limit, ExitCode::OutOfGas);
        };
        if params.fuel_limit < gas_cost {
            return NextAction::from_exit_code(params.fuel_limit, ExitCode::OutOfGas);
        }

        // write new log into the journal
        self.sdk.write_log(context.address, data, topics);

        NextAction::ExecutionResult(Bytes::default(), gas_cost, ExitCode::Ok.into_i32())
    }

    pub(crate) fn syscall_destroy_account(
        &mut self,
        context: &ContractContext,
        params: SyscallInvocationParams,
    ) -> NextAction {
        // make sure input is 20 bytes len, and we have enough gas to pay for the call
        if params.input.len() != 20 {
            return NextAction::from_exit_code(params.fuel_limit, ExitCode::MalformedSyscallParams);
        }

        let result = self
            .sdk
            .destroy_account(&context.address, &Address::from_slice(&params.input[0..20]));

        // make sure we have enough gas for this op
        let gas_cost = gas::selfdestruct_cost(
            SpecId::CANCUN,
            SelfDestructResult {
                had_value: result.had_value,
                target_exists: result.target_exists,
                is_cold: result.is_cold,
                previously_destroyed: result.previously_destroyed,
            },
        );
        if params.fuel_limit < gas_cost {
            return NextAction::from_exit_code(params.fuel_limit, ExitCode::OutOfGas);
        }

        // return value as bytes with success exit code
        NextAction::ExecutionResult(Bytes::default(), gas_cost, ExitCode::Ok.into_i32())
    }

    pub(crate) fn syscall_balance(
        &mut self,
        context: &ContractContext,
        params: SyscallInvocationParams,
    ) -> NextAction {
        // make sure input is 32 bytes len, and we have enough gas to pay for the call
        if params.input.len() != 20 {
            return NextAction::from_exit_code(params.fuel_limit, ExitCode::MalformedSyscallParams);
        }

        let address = &Address::from_slice(&params.input[0..20]);
        let (result, is_cold) = self.sdk.account(&address);

        // make sure we have enough gas for this op
        let gas_cost = if &context.address == address {
            gas::LOW
        } else if is_cold {
            gas::COLD_ACCOUNT_ACCESS_COST
        } else {
            gas::WARM_STORAGE_READ_COST
        };
        if params.fuel_limit < gas_cost {
            return NextAction::from_exit_code(params.fuel_limit, ExitCode::OutOfGas);
        }

        // return value as bytes with success exit code
        NextAction::ExecutionResult(
            Bytes::from(result.balance.to_le_bytes::<32>()),
            gas_cost,
            ExitCode::Ok.into_i32(),
        )
    }

    pub(crate) fn syscall_write_preimage(
        &mut self,
        context: &ContractContext,
        params: SyscallInvocationParams,
    ) -> NextAction {
        let preimage = params.input;
        let preimage_hash = self.sdk.native_sdk().keccak256(preimage.as_ref());

        self.sdk
            .write_preimage(context.address, preimage_hash, preimage);

        // return value as bytes with success exit code
        NextAction::ExecutionResult(preimage_hash.0.into(), 0, ExitCode::Ok.into_i32())
    }

    pub(crate) fn syscall_preimage_copy(
        &mut self,
        context: &ContractContext,
        params: SyscallInvocationParams,
    ) -> NextAction {
        // make sure we have at least 32 bytes
        if params.input.len() != 32 {
            return NextAction::from_exit_code(params.fuel_limit, ExitCode::MalformedSyscallParams);
        }
        let preimage_hash = B256::from_slice(&params.input[0..32]);
        let preimage = self.sdk.preimage(&context.address, &preimage_hash);

        // return value as bytes with success exit code
        NextAction::ExecutionResult(preimage.unwrap_or_default(), 0, ExitCode::Ok.into_i32())
    }

    pub(crate) fn syscall_preimage_size(
        &mut self,
        context: &ContractContext,
        params: SyscallInvocationParams,
    ) -> NextAction {
        // make sure we have at least 32 bytes
        if params.input.len() != 32 {
            return NextAction::from_exit_code(params.fuel_limit, ExitCode::MalformedSyscallParams);
        }
        let preimage_hash = B256::from_slice(&params.input[0..32]);
        let gas_cost = gas::warm_cold_cost(false);
        let preimage_size = self.sdk.preimage_size(&context.address, &preimage_hash);

        let mut buffer = [0u8; 4];
        LittleEndian::write_u32(&mut buffer, preimage_size);

        // return value as bytes with success exit code
        NextAction::ExecutionResult(buffer.into(), gas_cost, ExitCode::Ok.into_i32())
    }

    pub(crate) fn syscall_ext_storage_read(
        &mut self,
        _context: &ContractContext,
        params: SyscallInvocationParams,
    ) -> NextAction {
        // make sure input is 32 bytes len, and we have enough gas to pay for the call
        if params.input.len() != 20 + 32 {
            return NextAction::from_exit_code(params.fuel_limit, ExitCode::MalformedSyscallParams);
        }

        let ext_address = Address::from_slice(&params.input[0..20]);
        let slot = U256::from_le_slice(params.input.as_ref());

        // read value from storage
        let (value, is_cold) = self.sdk.storage(&ext_address, &slot);

        let gas_cost = if is_cold {
            gas::COLD_SLOAD_COST
        } else {
            gas::WARM_STORAGE_READ_COST
        };

        // make sure we have enough gas for this op
        if params.fuel_limit < gas_cost {
            return NextAction::from_exit_code(params.fuel_limit, ExitCode::OutOfGas);
        }

        // return value as bytes with success exit code
        NextAction::ExecutionResult(
            value.to_le_bytes::<32>().into(),
            gas_cost,
            ExitCode::Ok.into_i32(),
        )
    }
}
