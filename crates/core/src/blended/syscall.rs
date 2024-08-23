use crate::{blended::BlendedRuntime, helpers::exit_code_from_evm_error, types::NextAction};
use alloc::{boxed::Box, vec::Vec};
use alloy_rlp::Encodable;
use byteorder::{ByteOrder, LittleEndian};
use fluentbase_sdk::{
    Address,
    Bytes,
    ContractContext,
    ExitCode,
    Fuel,
    NativeAPI,
    SovereignAPI,
    SyscallInvocationParams,
    B256,
    PRECOMPILE_EVM,
    STATE_MAIN,
    U256,
};
use revm_interpreter::{
    gas,
    CallInputs,
    CallScheme,
    CallValue,
    CreateInputs,
    Gas,
    SelfDestructResult,
};
use revm_primitives::{CreateScheme, SpecId, MAX_INITCODE_SIZE};

fn is_gas_free_call(context: &ContractContext) -> bool {
    context.bytecode_address == PRECOMPILE_EVM
}

impl<'a, SDK: SovereignAPI> BlendedRuntime<'a, SDK> {
    pub(crate) fn syscall_storage_read(
        &mut self,
        context: &ContractContext,
        gas: &mut Gas,
        params: SyscallInvocationParams,
    ) -> NextAction {
        let is_gas_free = is_gas_free_call(context);

        // make sure input is 32 bytes len, and we have enough gas to pay for the call
        if params.input.len() != 32 {
            return NextAction::from_exit_code(params.fuel_limit, ExitCode::MalformedSyscallParams);
        }

        // read value from storage
        let (value, is_cold) = self.sdk.storage(
            &context.address,
            &U256::from_le_slice(params.input.as_ref()),
        );

        let gas_cost = if !is_gas_free {
            let gas_cost = if is_cold {
                gas::COLD_SLOAD_COST
            } else {
                gas::WARM_STORAGE_READ_COST
            };
            // make sure we have enough gas for this op
            if !gas.record_cost(gas_cost) {
                return NextAction::from_exit_code(params.fuel_limit, ExitCode::OutOfGas);
            }
            gas_cost
        } else {
            0
        };

        // return value as bytes with success exit code
        NextAction::ExecutionResult(
            value.to_le_bytes::<32>().into(),
            gas_cost.into(),
            ExitCode::Ok.into_i32(),
        )
    }

    pub(crate) fn syscall_storage_write(
        &mut self,
        context: &ContractContext,
        gas: &mut Gas,
        params: SyscallInvocationParams,
    ) -> NextAction {
        let is_gas_free = is_gas_free_call(context);

        // make sure input is 32 bytes len, and we have enough gas to pay for the call
        if params.input.len() != 64 {
            return NextAction::from_exit_code(params.fuel_limit, ExitCode::MalformedSyscallParams);
        }

        let slot = U256::from_le_slice(&params.input[0..32]);
        let value = U256::from_le_slice(&params.input[32..64]);

        let (original_value, _) = self.sdk.committed_storage(&context.address, &slot);
        let (present_value, is_cold) = self.sdk.storage(&context.address, &slot);
        self.sdk.write_storage(context.address, slot, value);

        let gas_cost = if !is_gas_free {
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
            if !gas.record_cost(gas_cost) {
                return NextAction::from_exit_code(params.fuel_limit, ExitCode::OutOfGas);
            }
            gas_cost
        } else {
            0
        };

        let _is_cold = self.sdk.write_storage(context.address, slot, value);
        NextAction::ExecutionResult(Bytes::default(), gas_cost.into(), ExitCode::Ok.into_i32())
    }

    pub(crate) fn syscall_call(
        &mut self,
        context: &ContractContext,
        gas: &mut Gas,
        params: SyscallInvocationParams,
        call_depth: u32,
    ) -> NextAction {
        let is_gas_free = is_gas_free_call(context);

        // make sure we have enough bytes inside input params, where:
        // - 20 bytes for the target address
        // - 32 bytes for the call value
        if params.input.len() < 20 + 32 {
            return NextAction::from_exit_code(params.fuel_limit, ExitCode::MalformedSyscallParams);
        }

        let target_address = Address::from_slice(&params.input[0..20]);
        let value = U256::from_le_slice(&params.input[20..52]);
        let contract_input = params.input.slice(52..);

        let has_transfer = !value.is_zero();
        if context.is_static && has_transfer {
            return NextAction::from_exit_code(params.fuel_limit, ExitCode::WriteProtection);
        }

        let (account, is_cold) = self.sdk.account(&target_address);

        let gas_limit = if !is_gas_free {
            // make sure we have enough gas for the call
            let call_cost = gas::call_cost(
                SpecId::CANCUN,
                !value.is_zero(),
                is_cold,
                account.is_empty(),
            );
            if !gas.record_cost(call_cost) {
                return NextAction::from_exit_code(params.fuel_limit, ExitCode::OutOfGas);
            }
            let mut gas_limit = gas.remaining();
            gas_limit = core::cmp::min(gas_limit - gas_limit / 64, params.fuel_limit);
            if !gas.record_cost(gas_limit) {
                return NextAction::from_exit_code(params.fuel_limit, ExitCode::OutOfGas);
            }
            // add call stipend if there is a value to be transferred.
            if has_transfer {
                gas_limit = gas_limit.saturating_add(gas::CALL_STIPEND);
            }
            gas_limit
        } else {
            params.fuel_limit
        };

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

        let gas_cost = Fuel::new(gas_limit)
            .with_spent(gas.spent())
            .with_refund(gas.refunded());

        NextAction::ExecutionResult(output, gas_cost, exit_code)
    }

    pub(crate) fn syscall_static_call(
        &mut self,
        context: &ContractContext,
        gas: &mut Gas,
        params: SyscallInvocationParams,
        call_depth: u32,
    ) -> NextAction {
        let is_gas_free = is_gas_free_call(context);

        // make sure we have enough bytes inside input params, where:
        if params.input.len() < 20 {
            return NextAction::from_exit_code(params.fuel_limit, ExitCode::MalformedSyscallParams);
        }

        let target_address = Address::from_slice(&params.input[0..20]);
        let contract_input = params.input.slice(20..);

        let (_, is_cold) = self.sdk.account(&target_address);

        let gas_limit = if !is_gas_free {
            // make sure we have enough gas for the call
            let call_cost = gas::call_cost(SpecId::CANCUN, false, is_cold, false);
            if !gas.record_cost(call_cost) {
                return NextAction::from_exit_code(params.fuel_limit, ExitCode::OutOfGas);
            }
            let mut gas_limit = gas.remaining();
            gas_limit = core::cmp::min(gas_limit - gas_limit / 64, params.fuel_limit);
            if !gas.record_cost(gas_limit) {
                return NextAction::from_exit_code(params.fuel_limit, ExitCode::OutOfGas);
            }
            gas_limit
        } else {
            params.fuel_limit
        };

        // execute a nested call to another binary
        let (output, gas, exit_code) = self.call_inner(
            Box::new(CallInputs {
                input: contract_input,
                return_memory_offset: Default::default(),
                gas_limit,
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

        let gas_cost = Fuel::new(gas_limit)
            .with_spent(gas.spent())
            .with_refund(gas.refunded());

        NextAction::ExecutionResult(output, gas_cost, exit_code)
    }

    pub(crate) fn syscall_delegate_call(
        &mut self,
        context: &ContractContext,
        gas: &mut Gas,
        params: SyscallInvocationParams,
        call_depth: u32,
    ) -> NextAction {
        let is_gas_free = is_gas_free_call(context);

        // make sure we have enough bytes inside input params, where:
        // - 20 bytes for the target address
        if params.input.len() < 20 {
            return NextAction::from_exit_code(params.fuel_limit, ExitCode::MalformedSyscallParams);
        }

        let target_address = Address::from_slice(&params.input[0..20]);
        let contract_input = params.input.slice(20..);

        let (_, is_cold) = self.sdk.account(&target_address);

        let gas_limit = if !is_gas_free {
            // make sure we have enough gas for the call
            let call_cost = gas::call_cost(SpecId::CANCUN, false, is_cold, false);
            if !gas.record_cost(call_cost) {
                return NextAction::from_exit_code(params.fuel_limit, ExitCode::OutOfGas);
            }
            let mut gas_limit = gas.remaining();
            gas_limit = core::cmp::min(gas_limit - gas_limit / 64, params.fuel_limit);
            if !gas.record_cost(gas_limit) {
                return NextAction::from_exit_code(params.fuel_limit, ExitCode::OutOfGas);
            }
            gas_limit
        } else {
            params.fuel_limit
        };

        // execute a nested call to another binary
        let (output, gas, exit_code) = self.call_inner(
            Box::new(CallInputs {
                input: contract_input,
                return_memory_offset: Default::default(),
                gas_limit,
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

        let gas_cost = Fuel::new(gas_limit)
            .with_spent(gas.spent())
            .with_refund(gas.refunded());

        NextAction::ExecutionResult(output, gas_cost, exit_code)
    }

    pub(crate) fn syscall_call_code(
        &mut self,
        context: &ContractContext,
        gas: &mut Gas,
        params: SyscallInvocationParams,
        call_depth: u32,
    ) -> NextAction {
        let is_gas_free = is_gas_free_call(context);

        // make sure we have enough bytes inside input params, where:
        // - 20 bytes for the target address
        // - 32 bytes for the transfer value
        if params.input.len() < 20 + 32 {
            return NextAction::from_exit_code(params.fuel_limit, ExitCode::MalformedSyscallParams);
        }

        let target_address = Address::from_slice(&params.input[0..20]);
        let value = U256::from_le_slice(&params.input[20..52]);
        let contract_input = params.input.slice(52..);

        let (_, is_cold) = self.sdk.account(&target_address);

        let gas_limit = if !is_gas_free {
            // make sure we have enough gas for the call
            let call_cost = gas::call_cost(SpecId::CANCUN, !value.is_zero(), is_cold, false);
            if !gas.record_cost(call_cost) {
                return NextAction::from_exit_code(params.fuel_limit, ExitCode::OutOfGas);
            }
            let mut gas_limit = gas.remaining();
            gas_limit = core::cmp::min(gas_limit - gas_limit / 64, params.fuel_limit);
            if !gas.record_cost(gas_limit) {
                return NextAction::from_exit_code(params.fuel_limit, ExitCode::OutOfGas);
            }
            // add call stipend if there is a value to be transferred.
            if !value.is_zero() {
                gas_limit = gas_limit.saturating_add(gas::CALL_STIPEND);
            }
            gas_limit
        } else {
            params.fuel_limit
        };

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

        let gas_cost = Fuel::new(gas_limit)
            .with_spent(gas.spent())
            .with_refund(gas.refunded());

        NextAction::ExecutionResult(output, gas_cost, exit_code)
    }

    pub(crate) fn syscall_create<const IS_CREATE2: bool>(
        &mut self,
        context: &ContractContext,
        gas: &mut Gas,
        params: SyscallInvocationParams,
        call_depth: u32,
    ) -> NextAction {
        let is_gas_free = is_gas_free_call(context);

        // make sure we have enough bytes inside input params
        let (scheme, value, init_code) = if IS_CREATE2 {
            if params.input.len() < 32 + 32 {
                return NextAction::from_exit_code(
                    params.fuel_limit,
                    ExitCode::MalformedSyscallParams,
                );
            }
            let value = U256::from_le_slice(&params.input[0..32]);
            let salt = U256::from_le_slice(&params.input[32..64]);
            let init_code = params.input.slice(64..);
            (CreateScheme::Create2 { salt }, value, init_code)
        } else {
            if params.input.len() < 32 {
                return NextAction::from_exit_code(
                    params.fuel_limit,
                    ExitCode::MalformedSyscallParams,
                );
            }
            let value = U256::from_le_slice(&params.input[0..32]);
            let init_code = params.input.slice(32..);
            (CreateScheme::Create, value, init_code)
        };

        let gas_limit = if !is_gas_free {
            // make sure we don't exceed max possible init code
            if init_code.len() > 0 {
                let max_initcode_size = self
                    .env
                    .cfg
                    .limit_contract_code_size
                    .map(|limit| limit.saturating_mul(2))
                    .unwrap_or(MAX_INITCODE_SIZE);
                if init_code.len() > max_initcode_size {
                    return NextAction::from_exit_code(
                        params.fuel_limit,
                        ExitCode::ContractSizeLimit,
                    );
                }
                if !gas.record_cost(gas::initcode_cost(init_code.length() as u64)) {
                    return NextAction::from_exit_code(params.fuel_limit, ExitCode::OutOfGas);
                }
            }
            // calc gas cost for CREATE/CREATE2 opcode call
            if IS_CREATE2 {
                let Some(gas_cost) = gas::create2_cost(init_code.len() as u64) else {
                    return NextAction::from_exit_code(params.fuel_limit, ExitCode::OutOfGas);
                };
                if !gas.record_cost(gas_cost) {
                    return NextAction::from_exit_code(params.fuel_limit, ExitCode::OutOfGas);
                }
            } else {
                if !gas.record_cost(gas::CREATE) {
                    return NextAction::from_exit_code(params.fuel_limit, ExitCode::OutOfGas);
                }
            }
            let mut gas_limit = gas.remaining();
            gas_limit -= gas_limit / 64;
            if !gas.record_cost(gas_limit) {
                return NextAction::from_exit_code(params.fuel_limit, ExitCode::OutOfGas);
            }
            gas_limit
        } else {
            params.fuel_limit
        };

        // execute a nested call to another binary
        let mut call_outcome = self.create_inner(
            Box::new(CreateInputs {
                caller: context.address,
                scheme,
                value,
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

        let gas_cost = Fuel::new(gas_limit)
            .with_spent(call_outcome.result.gas.spent())
            .with_refund(call_outcome.result.gas.refunded());

        let exit_code = exit_code_from_evm_error(call_outcome.result.result).into_i32();
        NextAction::ExecutionResult(call_outcome.result.output, gas_cost, exit_code)
    }

    pub(crate) fn syscall_emit_log(
        &mut self,
        context: &ContractContext,
        gas: &mut Gas,
        params: SyscallInvocationParams,
    ) -> NextAction {
        let is_gas_free = is_gas_free_call(context);

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
        let gas_cost = if !is_gas_free {
            let Some(gas_cost) = gas::log_cost(topics_len as u8, data.len() as u64) else {
                return NextAction::from_exit_code(params.fuel_limit, ExitCode::OutOfGas);
            };
            if !gas.record_cost(gas_cost) {
                return NextAction::from_exit_code(params.fuel_limit, ExitCode::OutOfGas);
            }
            gas_cost
        } else {
            0
        };

        // write new log into the journal
        self.sdk.write_log(context.address, data, topics);

        NextAction::ExecutionResult(
            Bytes::default(),
            Fuel::new(params.fuel_limit).with_spent(gas_cost),
            ExitCode::Ok.into_i32(),
        )
    }

    pub(crate) fn syscall_destroy_account(
        &mut self,
        context: &ContractContext,
        gas: &mut Gas,
        params: SyscallInvocationParams,
    ) -> NextAction {
        let is_gas_free = is_gas_free_call(context);

        // make sure input is 20 bytes len, and we have enough gas to pay for the call
        if params.input.len() != 20 {
            return NextAction::from_exit_code(params.fuel_limit, ExitCode::MalformedSyscallParams);
        }

        let result = self
            .sdk
            .destroy_account(&context.address, &Address::from_slice(&params.input[0..20]));

        // make sure we have enough gas for this op
        let gas_cost = if !is_gas_free {
            let gas_cost = gas::selfdestruct_cost(
                SpecId::CANCUN,
                SelfDestructResult {
                    had_value: result.had_value,
                    target_exists: result.target_exists,
                    is_cold: result.is_cold,
                    previously_destroyed: result.previously_destroyed,
                },
            );
            if !gas.record_cost(gas_cost) {
                return NextAction::from_exit_code(params.fuel_limit, ExitCode::OutOfGas);
            }
            gas_cost
        } else {
            0
        };

        // return value as bytes with success exit code
        NextAction::ExecutionResult(Bytes::default(), gas_cost.into(), ExitCode::Ok.into_i32())
    }

    pub(crate) fn syscall_balance(
        &mut self,
        context: &ContractContext,
        gas: &mut Gas,
        params: SyscallInvocationParams,
    ) -> NextAction {
        let is_gas_free = is_gas_free_call(context);

        // make sure input is 32 bytes len, and we have enough gas to pay for the call
        if params.input.len() != 20 {
            return NextAction::from_exit_code(params.fuel_limit, ExitCode::MalformedSyscallParams);
        }

        let address = &Address::from_slice(&params.input[0..20]);
        let (result, is_cold) = self.sdk.account(&address);

        // make sure we have enough gas for this op
        let gas_cost = if !is_gas_free {
            let gas_cost = if &context.address == address {
                gas::LOW
            } else if is_cold {
                gas::COLD_ACCOUNT_ACCESS_COST
            } else {
                gas::WARM_STORAGE_READ_COST
            };
            if !gas.record_cost(gas_cost) {
                return NextAction::from_exit_code(params.fuel_limit, ExitCode::OutOfGas);
            }
            gas_cost
        } else {
            0
        };

        // return value as bytes with success exit code
        NextAction::ExecutionResult(
            Bytes::from(result.balance.to_le_bytes::<32>()),
            gas_cost.into(),
            ExitCode::Ok.into_i32(),
        )
    }

    pub(crate) fn syscall_write_preimage(
        &mut self,
        context: &ContractContext,
        gas: &mut Gas,
        params: SyscallInvocationParams,
    ) -> NextAction {
        let is_gas_free = is_gas_free_call(context);

        let preimage = params.input;
        let preimage_hash = self.sdk.native_sdk().keccak256(preimage.as_ref());

        self.sdk
            .write_preimage(context.address, preimage_hash, preimage);

        // return value as bytes with success exit code
        NextAction::ExecutionResult(preimage_hash.0.into(), 0.into(), ExitCode::Ok.into_i32())
    }

    pub(crate) fn syscall_preimage_copy(
        &mut self,
        context: &ContractContext,
        gas: &mut Gas,
        params: SyscallInvocationParams,
    ) -> NextAction {
        let is_gas_free = is_gas_free_call(context);

        // make sure we have at least 32 bytes
        if params.input.len() != 32 {
            return NextAction::from_exit_code(params.fuel_limit, ExitCode::MalformedSyscallParams);
        }
        let preimage_hash = B256::from_slice(&params.input[0..32]);
        let preimage = self.sdk.preimage(&context.address, &preimage_hash);

        // return value as bytes with success exit code
        NextAction::ExecutionResult(
            preimage.unwrap_or_default(),
            0.into(),
            ExitCode::Ok.into_i32(),
        )
    }

    pub(crate) fn syscall_preimage_size(
        &mut self,
        context: &ContractContext,
        gas: &mut Gas,
        params: SyscallInvocationParams,
    ) -> NextAction {
        let is_gas_free = is_gas_free_call(context);

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
        NextAction::ExecutionResult(buffer.into(), gas_cost.into(), ExitCode::Ok.into_i32())
    }

    pub(crate) fn syscall_ext_storage_read(
        &mut self,
        context: &ContractContext,
        gas: &mut Gas,
        params: SyscallInvocationParams,
    ) -> NextAction {
        let is_gas_free = is_gas_free_call(context);

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
            gas_cost.into(),
            ExitCode::Ok.into_i32(),
        )
    }
}
