use crate::{blended::BlendedRuntime, helpers::exit_code_from_evm_error, types::NextAction};
use alloc::{boxed::Box, vec::Vec};
use fluentbase_sdk::{
    byteorder::{ByteOrder, LittleEndian},
    debug_log,
    Address,
    Bytes,
    ContractContext,
    ExitCode,
    SovereignAPI,
    SyscallInvocationParams,
    B256,
    STATE_MAIN,
    SYSCALL_ID_BALANCE,
    SYSCALL_ID_CALL,
    SYSCALL_ID_CALL_CODE,
    SYSCALL_ID_CREATE,
    SYSCALL_ID_CREATE2,
    SYSCALL_ID_DELEGATE_CALL,
    SYSCALL_ID_DESTROY_ACCOUNT,
    SYSCALL_ID_EMIT_LOG,
    SYSCALL_ID_EXT_STORAGE_READ,
    SYSCALL_ID_PREIMAGE_COPY,
    SYSCALL_ID_PREIMAGE_SIZE,
    SYSCALL_ID_STATIC_CALL,
    SYSCALL_ID_STORAGE_READ,
    SYSCALL_ID_STORAGE_WRITE,
    SYSCALL_ID_TRANSIENT_READ,
    SYSCALL_ID_TRANSIENT_WRITE,
    SYSCALL_ID_WRITE_PREIMAGE,
    U256,
};
use revm_interpreter::{
    gas,
    AccountLoad,
    CallInputs,
    CallScheme,
    CallValue,
    CreateInputs,
    Eip7702CodeLoad,
    SStoreResult,
    SelfDestructResult,
    StateLoad,
};
use revm_primitives::{CreateScheme, SpecId, MAX_INITCODE_SIZE};

fn is_gas_free_call(_address: &Address) -> bool {
    false
    // address == &PRECOMPILE_EVM
}

impl<SDK: SovereignAPI> BlendedRuntime<SDK> {
    pub(crate) fn process_syscall(
        &mut self,
        contract_context: &ContractContext,
        params: SyscallInvocationParams,
        call_depth: u32,
    ) -> NextAction {
        debug_log!(
            "process_syscall({}): fuel={} input_len={} state={}",
            fluentbase_sdk::syscall_name_by_hash(&params.code_hash),
            params.fuel_limit,
            params.input.len(),
            params.state,
        );
        // only main state can be forwarded to the other contract as a nested call,
        // other states can be only used by root
        let inner_gas_used = self.inner_gas_spend.take();
        let next_action = match params.code_hash {
            SYSCALL_ID_STORAGE_READ => self.syscall_storage_read(contract_context, params),
            SYSCALL_ID_STORAGE_WRITE => self.syscall_storage_write(contract_context, params),
            SYSCALL_ID_CALL => self.syscall_call(contract_context, params, call_depth),
            SYSCALL_ID_STATIC_CALL => {
                self.syscall_static_call(contract_context, params, call_depth)
            }
            SYSCALL_ID_DELEGATE_CALL => {
                self.syscall_delegate_call(contract_context, params, call_depth)
            }
            SYSCALL_ID_CALL_CODE => self.syscall_call_code(contract_context, params, call_depth),
            SYSCALL_ID_CREATE => self.syscall_create::<false>(contract_context, params, call_depth),
            SYSCALL_ID_CREATE2 => self.syscall_create::<true>(contract_context, params, call_depth),
            SYSCALL_ID_EMIT_LOG => self.syscall_emit_log(contract_context, params),
            SYSCALL_ID_DESTROY_ACCOUNT => self.syscall_destroy_account(contract_context, params),
            SYSCALL_ID_BALANCE => self.syscall_balance(contract_context, params),
            SYSCALL_ID_WRITE_PREIMAGE => self.syscall_write_preimage(contract_context, params),
            SYSCALL_ID_PREIMAGE_COPY => self.syscall_preimage_copy(contract_context, params),
            SYSCALL_ID_PREIMAGE_SIZE => self.syscall_preimage_size(contract_context, params),
            SYSCALL_ID_EXT_STORAGE_READ => self.syscall_ext_storage_read(contract_context, params),
            SYSCALL_ID_TRANSIENT_READ => self.syscall_transient_read(contract_context, params),
            SYSCALL_ID_TRANSIENT_WRITE => self.syscall_transient_write(contract_context, params),
            _ => NextAction::from_exit_code(params.fuel_limit, ExitCode::MalformedSyscallParams),
        };
        match next_action {
            NextAction::ExecutionResult { gas_used, .. }
            | NextAction::NestedCall { gas_used, .. } => {
                self.inner_gas_spend = Some(inner_gas_used.unwrap_or_default() + gas_used);
                next_action
            }
        }
    }

    pub(crate) fn syscall_storage_read(
        &mut self,
        context: &ContractContext,
        params: SyscallInvocationParams,
    ) -> NextAction {
        let is_gas_free = is_gas_free_call(&context.bytecode_address);

        // make sure input is 32 bytes len, and we have enough gas to pay for the call
        if params.input.len() != 32 {
            return NextAction::from_exit_code(params.fuel_limit, ExitCode::MalformedSyscallParams);
        } else if params.state != STATE_MAIN {
            return NextAction::from_exit_code(params.fuel_limit, ExitCode::MalformedSyscallParams);
        }

        // read value from storage
        let slot = U256::from_le_slice(&params.input[0..32].as_ref());
        let (value, is_cold) = self.sdk.storage(&context.address, &slot);

        let gas_cost = if !is_gas_free {
            let gas_cost = if is_cold {
                gas::COLD_SLOAD_COST
            } else {
                gas::WARM_STORAGE_READ_COST
            };
            if gas_cost > params.fuel_limit {
                return NextAction::from_exit_code(params.fuel_limit, ExitCode::OutOfGas);
            }
            gas_cost
        } else {
            0
        };

        debug_log!(
            " - storage_read: address={}, slot={} value={}, gas={}",
            context.address,
            B256::from(slot.to_be_bytes::<32>()),
            B256::from(value.to_be_bytes::<32>()),
            gas_cost
        );

        // return value as bytes with success exit code
        NextAction::ExecutionResult {
            exit_code: ExitCode::Ok.into_i32(),
            output: value.to_le_bytes::<32>().into(),
            gas_used: gas_cost,
        }
    }

    pub(crate) fn syscall_storage_write(
        &mut self,
        context: &ContractContext,
        params: SyscallInvocationParams,
    ) -> NextAction {
        let is_gas_free = is_gas_free_call(&context.bytecode_address);

        // make sure input is 32 bytes len, and we have enough gas to pay for the call
        if params.input.len() != 64 {
            return NextAction::from_exit_code(params.fuel_limit, ExitCode::MalformedSyscallParams);
        } else if params.state != STATE_MAIN {
            return NextAction::from_exit_code(params.fuel_limit, ExitCode::MalformedSyscallParams);
        }

        let slot = U256::from_le_slice(&params.input[0..32]);
        let new_value = U256::from_le_slice(&params.input[32..64]);

        let (original_value, _) = self.sdk.committed_storage(&context.address, &slot);
        let (present_value, is_cold) = self.sdk.storage(&context.address, &slot);
        self.sdk.write_storage(context.address, slot, new_value);

        let gas_cost = if !is_gas_free {
            let gas_cost = match gas::sstore_cost(
                SpecId::CANCUN,
                &SStoreResult {
                    original_value,
                    present_value,
                    new_value,
                },
                params.fuel_limit,
                is_cold,
            )
            .filter(|gas_cost| *gas_cost <= params.fuel_limit)
            {
                Some(gas_cost) => gas_cost,
                None => return NextAction::from_exit_code(params.fuel_limit, ExitCode::OutOfGas),
            };
            gas_cost
        } else {
            0
        };

        debug_log!(
            "- storage_write: address={} slot={} value={} prev_value={} gas={}",
            context.address,
            B256::from(slot.to_be_bytes::<32>()),
            B256::from(new_value.to_be_bytes::<32>()),
            B256::from(present_value.to_be_bytes::<32>()),
            gas_cost
        );

        let _is_cold = self.sdk.write_storage(context.address, slot, new_value);

        NextAction::ExecutionResult {
            exit_code: ExitCode::Ok.into_i32(),
            output: Default::default(),
            gas_used: gas_cost,
        }
    }

    pub(crate) fn syscall_call(
        &mut self,
        context: &ContractContext,
        params: SyscallInvocationParams,
        call_depth: u32,
    ) -> NextAction {
        let is_gas_free = is_gas_free_call(&context.bytecode_address);

        // make sure we have enough bytes inside input params, where:
        // - 20 bytes for the target address
        // - 32 bytes for the call value
        if params.input.len() < 20 + 32 {
            return NextAction::from_exit_code(params.fuel_limit, ExitCode::MalformedSyscallParams);
        } else if params.state != STATE_MAIN {
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

        let (call_cost, gas_limit) = if !is_gas_free {
            // make sure we have enough gas for the call
            let call_cost = gas::call_cost(
                SpecId::CANCUN,
                !value.is_zero(),
                AccountLoad {
                    load: Eip7702CodeLoad::new_not_delegated((), is_cold),
                    is_empty: account.is_empty(),
                },
            );
            if call_cost > params.fuel_limit {
                return NextAction::from_exit_code(params.fuel_limit, ExitCode::OutOfGas);
            }
            let mut gas_limit = params.fuel_limit - call_cost;
            gas_limit = core::cmp::min(gas_limit - gas_limit / 64, params.fuel_limit);
            if gas_limit + call_cost > params.fuel_limit {
                return NextAction::from_exit_code(params.fuel_limit, ExitCode::OutOfGas);
            }
            // add call stipend if there is a value to be transferred.
            if has_transfer {
                gas_limit = gas_limit.saturating_add(gas::CALL_STIPEND);
            }
            (call_cost, gas_limit)
        } else {
            (0, params.fuel_limit)
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

        NextAction::ExecutionResult {
            exit_code,
            output,
            gas_used: call_cost + gas.spent(),
        }
    }

    pub(crate) fn syscall_static_call(
        &mut self,
        context: &ContractContext,
        params: SyscallInvocationParams,
        call_depth: u32,
    ) -> NextAction {
        let is_gas_free = is_gas_free_call(&context.bytecode_address);

        // make sure we have enough bytes inside input params, where:
        if params.input.len() < 20 {
            return NextAction::from_exit_code(params.fuel_limit, ExitCode::MalformedSyscallParams);
        } else if params.state != STATE_MAIN {
            return NextAction::from_exit_code(params.fuel_limit, ExitCode::MalformedSyscallParams);
        }

        let target_address = Address::from_slice(&params.input[0..20]);
        let contract_input = params.input.slice(20..);

        let (_, is_cold) = self.sdk.account(&target_address);

        let (call_cost, gas_limit) = if !is_gas_free {
            // make sure we have enough gas for the call
            let call_cost = gas::call_cost(
                SpecId::CANCUN,
                false,
                AccountLoad {
                    load: Eip7702CodeLoad::new_not_delegated((), is_cold),
                    is_empty: false,
                },
            );
            if call_cost > params.fuel_limit {
                return NextAction::from_exit_code(params.fuel_limit, ExitCode::OutOfGas);
            }
            let mut gas_limit = params.fuel_limit - call_cost;
            gas_limit = core::cmp::min(gas_limit - gas_limit / 64, params.fuel_limit);
            if gas_limit + call_cost > params.fuel_limit {
                return NextAction::from_exit_code(params.fuel_limit, ExitCode::OutOfGas);
            }
            (call_cost, gas_limit)
        } else {
            (0, params.fuel_limit)
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

        NextAction::ExecutionResult {
            exit_code,
            output,
            gas_used: call_cost + gas.spent(),
        }
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

        let bytecode_address = Address::from_slice(&params.input[0..20]);
        let contract_input = params.input.slice(20..);

        let (_, is_cold) = self.sdk.account(&bytecode_address);

        let is_gas_free = is_gas_free_call(&bytecode_address);

        let (call_cost, gas_limit) = if !is_gas_free {
            // make sure we have enough gas for the call
            let call_cost = gas::call_cost(
                SpecId::CANCUN,
                false,
                AccountLoad {
                    load: Eip7702CodeLoad::new_not_delegated((), is_cold),
                    is_empty: false,
                },
            );
            if call_cost > params.fuel_limit {
                return NextAction::from_exit_code(params.fuel_limit, ExitCode::OutOfGas);
            }

            let mut gas_limit = params.fuel_limit - call_cost;
            gas_limit = core::cmp::min(gas_limit - gas_limit / 64, params.fuel_limit);
            if gas_limit + call_cost > params.fuel_limit {
                return NextAction::from_exit_code(params.fuel_limit, ExitCode::OutOfGas);
            }
            (call_cost, gas_limit)
        } else {
            (0, params.fuel_limit)
        };

        debug_log!(
            " - delegate_call: address={} gas={} gas_limit={}",
            bytecode_address,
            call_cost,
            gas_limit
        );

        // execute a nested call to another binary
        let (output, gas, exit_code) = self.call_inner(
            Box::new(CallInputs {
                input: contract_input,
                return_memory_offset: Default::default(),
                gas_limit,
                bytecode_address,
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

        NextAction::ExecutionResult {
            exit_code,
            output,
            gas_used: call_cost + gas.spent(),
        }
    }

    pub(crate) fn syscall_call_code(
        &mut self,
        context: &ContractContext,
        params: SyscallInvocationParams,
        call_depth: u32,
    ) -> NextAction {
        let is_gas_free = is_gas_free_call(&context.bytecode_address);

        // make sure we have enough bytes inside input params, where:
        // - 20 bytes for the target address
        // - 32 bytes for the transfer value
        if params.input.len() < 20 + 32 {
            return NextAction::from_exit_code(params.fuel_limit, ExitCode::MalformedSyscallParams);
        } else if params.state != STATE_MAIN {
            return NextAction::from_exit_code(params.fuel_limit, ExitCode::MalformedSyscallParams);
        }

        let target_address = Address::from_slice(&params.input[0..20]);
        let value = U256::from_le_slice(&params.input[20..52]);
        let contract_input = params.input.slice(52..);

        let (_, is_cold) = self.sdk.account(&target_address);

        let (call_cost, gas_limit) = if !is_gas_free {
            // make sure we have enough gas for the call
            let call_cost = gas::call_cost(
                SpecId::CANCUN,
                !value.is_zero(),
                AccountLoad {
                    load: Eip7702CodeLoad::new_not_delegated((), is_cold),
                    is_empty: false,
                },
            );
            if call_cost > params.fuel_limit {
                return NextAction::from_exit_code(params.fuel_limit, ExitCode::OutOfGas);
            }
            let mut gas_limit = params.fuel_limit - call_cost;
            gas_limit = core::cmp::min(gas_limit - gas_limit / 64, params.fuel_limit);
            if gas_limit + call_cost > params.fuel_limit {
                return NextAction::from_exit_code(params.fuel_limit, ExitCode::OutOfGas);
            }
            // add call stipend if there is a value to be transferred.
            if !value.is_zero() {
                gas_limit = gas_limit.saturating_add(gas::CALL_STIPEND);
            }
            (call_cost, gas_limit)
        } else {
            (0, params.fuel_limit)
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

        NextAction::ExecutionResult {
            exit_code,
            output,
            gas_used: call_cost + gas.spent(),
        }
    }

    pub(crate) fn syscall_create<const IS_CREATE2: bool>(
        &mut self,
        context: &ContractContext,
        params: SyscallInvocationParams,
        call_depth: u32,
    ) -> NextAction {
        let is_gas_free = is_gas_free_call(&context.bytecode_address);

        if params.state != STATE_MAIN {
            return NextAction::from_exit_code(params.fuel_limit, ExitCode::MalformedSyscallParams);
        }

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

        // make sure we don't exceed max possible init code
        let max_initcode_size = self
            .env
            .cfg
            .limit_contract_code_size
            .map(|limit| limit.saturating_mul(2))
            .unwrap_or(MAX_INITCODE_SIZE);
        if init_code.len() > max_initcode_size {
            return NextAction::from_exit_code(params.fuel_limit, ExitCode::ContractSizeLimit);
        }

        let (create_cost, gas_limit) = if !is_gas_free {
            let init_cost = if init_code.len() > 0 {
                gas::initcode_cost(init_code.len() as u64)
            } else {
                0
            };
            if init_cost > params.fuel_limit {
                return NextAction::from_exit_code(params.fuel_limit, ExitCode::OutOfGas);
            }
            // calc gas cost for CREATE/CREATE2 opcode call
            let create_cost = if IS_CREATE2 {
                let Some(gas_cost) = gas::create2_cost(init_code.len() as u64) else {
                    return NextAction::from_exit_code(params.fuel_limit, ExitCode::OutOfGas);
                };
                gas_cost
            } else {
                gas::CREATE
            };
            if init_cost + create_cost > params.fuel_limit {
                return NextAction::from_exit_code(params.fuel_limit, ExitCode::OutOfGas);
            }
            let mut gas_limit = params.fuel_limit - init_cost - create_cost;
            gas_limit -= gas_limit / 64;
            if gas_limit + init_cost + create_cost > params.fuel_limit {
                return NextAction::from_exit_code(params.fuel_limit, ExitCode::OutOfGas);
            }
            (init_cost + create_cost, gas_limit)
        } else {
            (0, params.fuel_limit)
        };

        // execute a nested call to another binary
        let mut create_outcome = self.create_inner(
            Box::new(CreateInputs {
                caller: context.address,
                scheme,
                value,
                init_code,
                gas_limit,
            }),
            call_depth + 1,
        );

        create_outcome.result.output = if create_outcome.result.is_ok() {
            Bytes::copy_from_slice(create_outcome.address.unwrap().as_slice())
        } else {
            create_outcome.result.output
        };

        let exit_code = exit_code_from_evm_error(create_outcome.result.result).into_i32();

        NextAction::ExecutionResult {
            exit_code,
            output: create_outcome.result.output,
            gas_used: create_cost + create_outcome.result.gas.spent(),
        }
    }

    pub(crate) fn syscall_emit_log(
        &mut self,
        context: &ContractContext,
        params: SyscallInvocationParams,
    ) -> NextAction {
        let is_gas_free = is_gas_free_call(&context.bytecode_address);

        if params.input.len() < 1 {
            return NextAction::from_exit_code(params.fuel_limit, ExitCode::MalformedSyscallParams);
        } else if params.state != STATE_MAIN {
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
            if gas_cost > params.fuel_limit {
                return NextAction::from_exit_code(params.fuel_limit, ExitCode::OutOfGas);
            }
            gas_cost
        } else {
            0
        };

        // write new log into the journal
        self.sdk.write_log(context.address, data, topics);

        NextAction::ExecutionResult {
            exit_code: ExitCode::Ok.into_i32(),
            output: Default::default(),
            gas_used: gas_cost,
        }
    }

    pub(crate) fn syscall_destroy_account(
        &mut self,
        context: &ContractContext,
        params: SyscallInvocationParams,
    ) -> NextAction {
        let is_gas_free = is_gas_free_call(&context.bytecode_address);

        // make sure input is 20 bytes len, and we have enough gas to pay for the call
        if params.input.len() != 20 {
            return NextAction::from_exit_code(params.fuel_limit, ExitCode::MalformedSyscallParams);
        } else if params.state != STATE_MAIN {
            return NextAction::from_exit_code(params.fuel_limit, ExitCode::MalformedSyscallParams);
        }

        let result = self
            .sdk
            .destroy_account(&context.address, &Address::from_slice(&params.input[0..20]));

        // make sure we have enough gas for this op
        let gas_cost = if !is_gas_free {
            let gas_cost = gas::selfdestruct_cost(
                SpecId::CANCUN,
                StateLoad::new(
                    SelfDestructResult {
                        had_value: result.had_value,
                        target_exists: result.target_exists,
                        previously_destroyed: result.previously_destroyed,
                    },
                    result.is_cold,
                ),
            );
            if gas_cost > params.fuel_limit {
                return NextAction::from_exit_code(params.fuel_limit, ExitCode::OutOfGas);
            }
            gas_cost
        } else {
            0
        };

        // return value as bytes with success exit code
        NextAction::ExecutionResult {
            exit_code: ExitCode::Ok.into_i32(),
            output: Default::default(),
            gas_used: gas_cost,
        }
    }

    pub(crate) fn syscall_balance(
        &mut self,
        context: &ContractContext,
        params: SyscallInvocationParams,
    ) -> NextAction {
        let is_gas_free = is_gas_free_call(&context.bytecode_address);

        // make sure input is 32 bytes len, and we have enough gas to pay for the call
        if params.input.len() != 20 {
            return NextAction::from_exit_code(params.fuel_limit, ExitCode::MalformedSyscallParams);
        } else if params.state != STATE_MAIN {
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
            if gas_cost > params.fuel_limit {
                return NextAction::from_exit_code(params.fuel_limit, ExitCode::OutOfGas);
            }
            gas_cost
        } else {
            0
        };

        // return value as bytes with success exit code
        NextAction::ExecutionResult {
            exit_code: ExitCode::Ok.into_i32(),
            output: Bytes::from(result.balance.to_le_bytes::<32>()),
            gas_used: gas_cost,
        }
    }

    pub(crate) fn syscall_write_preimage(
        &mut self,
        context: &ContractContext,
        params: SyscallInvocationParams,
    ) -> NextAction {
        let _is_gas_free = is_gas_free_call(&context.bytecode_address);

        if params.state != STATE_MAIN {
            return NextAction::from_exit_code(params.fuel_limit, ExitCode::MalformedSyscallParams);
        }

        let preimage_hash = SDK::keccak256(params.input.as_ref());

        self.sdk
            .write_preimage(context.address, preimage_hash, params.input);

        // return value as bytes with success exit code
        NextAction::ExecutionResult {
            exit_code: ExitCode::Ok.into_i32(),
            output: preimage_hash.0.into(),
            gas_used: 0,
        }
    }

    pub(crate) fn syscall_preimage_copy(
        &mut self,
        context: &ContractContext,
        params: SyscallInvocationParams,
    ) -> NextAction {
        let _is_gas_free = is_gas_free_call(&context.bytecode_address);

        // make sure we have at least 32 bytes
        if params.input.len() != 32 {
            return NextAction::from_exit_code(params.fuel_limit, ExitCode::MalformedSyscallParams);
        } else if params.state != STATE_MAIN {
            return NextAction::from_exit_code(params.fuel_limit, ExitCode::MalformedSyscallParams);
        }

        let preimage_hash = B256::from_slice(&params.input[0..32]);
        let preimage = self.sdk.preimage(&context.address, &preimage_hash);

        debug_log!(
            " - preimage_copy: len={} hash={}",
            preimage.as_ref().map(|v| v.len()).unwrap_or(0),
            preimage_hash
        );

        // return value as bytes with success exit code
        NextAction::ExecutionResult {
            exit_code: ExitCode::Ok.into_i32(),
            output: preimage.unwrap_or_default(),
            gas_used: 0,
        }
    }

    pub(crate) fn syscall_preimage_size(
        &mut self,
        context: &ContractContext,
        params: SyscallInvocationParams,
    ) -> NextAction {
        let is_gas_free = is_gas_free_call(&context.bytecode_address);

        // make sure we have at least 32 bytes
        if params.input.len() != 32 {
            return NextAction::from_exit_code(params.fuel_limit, ExitCode::MalformedSyscallParams);
        } else if params.state != STATE_MAIN {
            return NextAction::from_exit_code(params.fuel_limit, ExitCode::MalformedSyscallParams);
        }

        let preimage_hash = B256::from_slice(&params.input[0..32]);

        let gas_cost = if !is_gas_free {
            let gas_cost = gas::warm_cold_cost(false);
            if gas_cost > params.fuel_limit {
                return NextAction::from_exit_code(params.fuel_limit, ExitCode::OutOfGas);
            }
            gas_cost
        } else {
            0
        };
        let preimage_size = self
            .sdk
            .preimage_size(&context.address, &preimage_hash)
            .unwrap_or_default();

        debug_log!(
            " - preimage_size: size={} gas={} hash={}",
            preimage_size,
            gas_cost,
            preimage_hash
        );

        let mut buffer = [0u8; 4];
        LittleEndian::write_u32(&mut buffer, preimage_size);

        // return value as bytes with success exit code
        NextAction::ExecutionResult {
            exit_code: ExitCode::Ok.into_i32(),
            output: buffer.into(),
            gas_used: gas_cost,
        }
    }

    pub(crate) fn syscall_ext_storage_read(
        &mut self,
        context: &ContractContext,
        params: SyscallInvocationParams,
    ) -> NextAction {
        let is_gas_free = is_gas_free_call(&context.bytecode_address);

        // make sure input is 32 bytes len, and we have enough gas to pay for the call
        if params.input.len() != 20 + 32 {
            return NextAction::from_exit_code(params.fuel_limit, ExitCode::MalformedSyscallParams);
        } else if params.state != STATE_MAIN {
            return NextAction::from_exit_code(params.fuel_limit, ExitCode::MalformedSyscallParams);
        }

        let ext_address = Address::from_slice(&params.input[0..20]);
        let slot = U256::from_le_slice(&params.input[20..52].as_ref());

        // read value from storage
        let (value, is_cold) = self.sdk.storage(&ext_address, &slot);

        let gas_cost = if !is_gas_free {
            let gas_cost = if is_cold {
                gas::COLD_SLOAD_COST
            } else {
                gas::WARM_STORAGE_READ_COST
            };
            if params.fuel_limit < gas_cost {
                return NextAction::from_exit_code(params.fuel_limit, ExitCode::OutOfGas);
            }
            gas_cost
        } else {
            0
        };

        // return value as bytes with success exit code
        NextAction::ExecutionResult {
            exit_code: ExitCode::Ok.into_i32(),
            output: value.to_le_bytes::<32>().into(),
            gas_used: gas_cost,
        }
    }

    pub(crate) fn syscall_transient_read(
        &mut self,
        context: &ContractContext,
        params: SyscallInvocationParams,
    ) -> NextAction {
        let is_gas_free = is_gas_free_call(&context.bytecode_address);

        // make sure input is 32 bytes len, and we have enough gas to pay for the call
        if params.input.len() != 32 {
            return NextAction::from_exit_code(params.fuel_limit, ExitCode::MalformedSyscallParams);
        } else if params.state != STATE_MAIN {
            return NextAction::from_exit_code(params.fuel_limit, ExitCode::MalformedSyscallParams);
        }

        // read value from storage
        let slot = U256::from_le_slice(&params.input[0..32].as_ref());
        let value = self.sdk.transient_storage(&context.address, &slot);

        let gas_cost = if !is_gas_free {
            let gas_cost = gas::WARM_STORAGE_READ_COST;
            if gas_cost > params.fuel_limit {
                return NextAction::from_exit_code(params.fuel_limit, ExitCode::OutOfGas);
            }
            gas_cost
        } else {
            0
        };

        debug_log!(
            " - transient_read: slot={} value={}, gas={}",
            B256::from(slot.to_be_bytes::<32>()),
            B256::from(value.to_be_bytes::<32>()),
            gas_cost
        );

        // return value as bytes with success exit code
        NextAction::ExecutionResult {
            exit_code: ExitCode::Ok.into_i32(),
            output: value.to_le_bytes::<32>().into(),
            gas_used: gas_cost,
        }
    }

    pub(crate) fn syscall_transient_write(
        &mut self,
        context: &ContractContext,
        params: SyscallInvocationParams,
    ) -> NextAction {
        let is_gas_free = is_gas_free_call(&context.bytecode_address);

        // make sure input is 32 bytes len, and we have enough gas to pay for the call
        if params.input.len() != 64 {
            return NextAction::from_exit_code(params.fuel_limit, ExitCode::MalformedSyscallParams);
        } else if params.state != STATE_MAIN {
            return NextAction::from_exit_code(params.fuel_limit, ExitCode::MalformedSyscallParams);
        }

        let slot = U256::from_le_slice(&params.input[0..32]);
        let value = U256::from_le_slice(&params.input[32..64]);

        let gas_cost = if !is_gas_free {
            let gas_cost = gas::WARM_STORAGE_READ_COST;
            if gas_cost > params.fuel_limit {
                return NextAction::from_exit_code(params.fuel_limit, ExitCode::OutOfGas);
            }
            gas_cost
        } else {
            0
        };

        debug_log!(
            "- transient_write: slot={} value={} gas={}",
            B256::from(slot.to_be_bytes::<32>()),
            B256::from(value.to_be_bytes::<32>()),
            gas_cost
        );

        self.sdk
            .write_transient_storage(context.address, slot, value);

        NextAction::ExecutionResult {
            exit_code: ExitCode::Ok.into_i32(),
            output: Default::default(),
            gas_used: gas_cost,
        }
    }
}
