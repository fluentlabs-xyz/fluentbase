//! Opcode shims that turn host-bound instructions into interruptions.
//!
//! The instruction table mirrors EVM semantics. For opcodes that require
//! host interaction, we emit a SystemInterruption and resume after the host
//! provides a result.
use crate::{
    host::{HostWrapper, HostWrapperImpl},
    types::{InterruptingInterpreter, InterruptionExtension, InterruptionOutcome},
    utils::{global_memory_from_shared_buffer, interrupt_into_action},
};
use alloc::vec::Vec;
use core::{cmp::min, mem::take, ops::Range};
use fluentbase_sdk::{
    syscall::SyscallInterruptExecutor, Address, Bytes, ExitCode, SharedAPI, B256,
    EVM_MAX_INITCODE_SIZE, FUEL_DENOM_RATE, U256,
};
use revm_interpreter::{
    as_u64_saturated, as_usize_or_fail, as_usize_or_fail_ret, as_usize_saturated, gas,
    instruction_table,
    instructions::{
        contract::resize_memory,
        utility::{IntoAddress, IntoU256},
    },
    interpreter_types::{LoopControl, MemoryTr, ReturnData, RuntimeFlag, StackTr},
    peekn, popn, popn_top, push, require_non_staticcall, resize_memory, Host, Instruction,
    InstructionContext, InstructionResult, Interpreter, InterpreterAction, InterpreterTypes, Stack,
};
use revm_primitives::{wasm::wasm_max_code_size, BLOCK_HASH_HISTORY};

macro_rules! unpack_interruption {
    (@frame $context:expr) => {
        if let Some(interruption_outcome) =
            take(&mut $context.interpreter.extend.interruption_outcome)
        {
            if interruption_outcome.halted_frame {
                let result = interruption_outcome.into_interpreter_result();
                $context
                    .interpreter
                    .bytecode
                    .set_action(InterpreterAction::Return(result));
                return;
            }
            Some(interruption_outcome)
        } else {
            None
        }
    };
    ($context:expr) => {
        if let Some(interruption_outcome) =
            take(&mut $context.interpreter.extend.interruption_outcome)
        {
            if !interruption_outcome.exit_code.is_ok() {
                let result = interruption_outcome.into_interpreter_result();
                $context
                    .interpreter
                    .bytecode
                    .set_action(InterpreterAction::Return(result));
                return;
            }
            Some(interruption_outcome)
        } else {
            None
        }
    };
}

fn balance<WIRE: InterpreterTypes<Extend = InterruptionExtension>, H: Host + ?Sized>(
    context: InstructionContext<'_, H, WIRE>,
) {
    popn_top!([], top, context.interpreter);
    if let Some(interruption_outcome) = unpack_interruption!(context) {
        *top = interruption_outcome.into_u256();
        return;
    }
    let address = top.into_address();
    interrupt_into_action(context, |_context, sdk| sdk.balance(&address));
}

fn selfbalance<WIRE: InterpreterTypes<Extend = InterruptionExtension>, H: Host + ?Sized>(
    context: InstructionContext<'_, H, WIRE>,
) {
    if let Some(interruption_outcome) = unpack_interruption!(context) {
        push!(context.interpreter, interruption_outcome.into_u256());
        return;
    }
    interrupt_into_action(context, |_context, sdk| sdk.self_balance());
}

fn extcodesize<WIRE: InterpreterTypes<Extend = InterruptionExtension>, H: Host + ?Sized>(
    context: InstructionContext<'_, H, WIRE>,
) {
    popn_top!([], top, context.interpreter);
    if let Some(interruption_outcome) = unpack_interruption!(context) {
        *top = interruption_outcome.into_u256();
        return;
    }
    let address = top.into_address();
    interrupt_into_action(context, |_context, sdk| sdk.code_size(&address));
}

fn extcodecopy<
    WIRE: InterpreterTypes<Stack = Stack, Extend = InterruptionExtension>,
    H: Host + ?Sized,
>(
    context: InstructionContext<'_, H, WIRE>,
) {
    if let Some(interruption_outcome) = unpack_interruption!(context) {
        popn!(
            [_address, memory_offset, _code_offset, len_u256],
            context.interpreter
        );
        let code = interruption_outcome.output;
        let len = as_usize_or_fail!(context.interpreter, len_u256);
        let memory_offset = as_usize_or_fail!(context.interpreter, memory_offset);
        resize_memory!(context.interpreter, memory_offset, len);
        context
            .interpreter
            .memory
            .set_data(memory_offset, 0, len, &code);
        return;
    }
    peekn!(
        [address, _memory_offset, code_offset, len_u256],
        context.interpreter
    );
    let address = address.into_address();
    let len = as_usize_or_fail!(context.interpreter, len_u256);
    let offset = as_usize_saturated!(code_offset);
    interrupt_into_action(context, |_context, sdk| {
        sdk.code_copy(&address, offset as u64, len as u64)
    });
}

fn extcodehash<WIRE: InterpreterTypes<Extend = InterruptionExtension>, H: Host + ?Sized>(
    context: InstructionContext<'_, H, WIRE>,
) {
    popn_top!([], top, context.interpreter);
    if let Some(interruption_outcome) = unpack_interruption!(context) {
        *top = interruption_outcome.into_b256().into();
        return;
    }
    let address = top.into_address();
    interrupt_into_action(context, |_context, sdk| sdk.code_hash(&address));
}

fn blockhash<WIRE: InterpreterTypes<Extend = InterruptionExtension>, H: Host + ?Sized>(
    context: InstructionContext<'_, H, WIRE>,
) {
    popn_top!([], number, context.interpreter);
    if let Some(interruption_outcome) = unpack_interruption!(context) {
        *number = interruption_outcome.into_b256().into();
        return;
    }
    let requested_number = *number;
    let block_number = context.host.block_number();
    let Some(diff) = block_number.checked_sub(requested_number) else {
        *number = U256::ZERO;
        gas!(context.interpreter, gas::BLOCKHASH);
        return;
    };
    let diff = as_u64_saturated!(diff);
    // blockhash should push zero if number is same as current block number.
    if diff == 0 || diff > BLOCK_HASH_HISTORY {
        *number = U256::ZERO;
        gas!(context.interpreter, gas::BLOCKHASH);
        return;
    }
    interrupt_into_action(context, |_context, sdk| {
        sdk.block_hash(as_u64_saturated!(requested_number))
    });
}

fn sload<WIRE: InterpreterTypes<Extend = InterruptionExtension>, H: Host + ?Sized>(
    context: InstructionContext<'_, H, WIRE>,
) {
    popn_top!([], index, context.interpreter);
    if let Some(interruption_outcome) = unpack_interruption!(context) {
        *index = interruption_outcome.into_u256();
        return;
    }
    let index = index.clone();
    interrupt_into_action(context, |_context, sdk| sdk.storage(&index));
}

fn sstore<
    WIRE: InterpreterTypes<Extend = InterruptionExtension>,
    H: Host + HostWrapper + ?Sized,
>(
    context: InstructionContext<'_, H, WIRE>,
) {
    if let Some(_interruption_outcome) = unpack_interruption!(context) {
        return;
    }
    require_non_staticcall!(context.interpreter);
    popn!([index, value], context.interpreter);
    interrupt_into_action(context, |_context, sdk| sdk.write_storage(index, value));
}

fn tload<WIRE: InterpreterTypes<Extend = InterruptionExtension>, H: Host + ?Sized>(
    context: InstructionContext<'_, H, WIRE>,
) {
    popn_top!([], index, context.interpreter);
    if let Some(interruption_outcome) = unpack_interruption!(context) {
        *index = interruption_outcome.into_u256();
        return;
    }
    let index = index.clone();
    interrupt_into_action(context, |_context, sdk| sdk.transient_storage(&index));
}

fn tstore<WIRE: InterpreterTypes<Extend = InterruptionExtension>, H: Host + ?Sized>(
    context: InstructionContext<'_, H, WIRE>,
) {
    if let Some(_interruption_outcome) = unpack_interruption!(context) {
        return;
    }
    require_non_staticcall!(context.interpreter);
    popn!([index, value], context.interpreter);
    interrupt_into_action(context, |_context, sdk| {
        sdk.write_transient_storage(index, value)
    });
}

fn log<const N: usize, H: Host + ?Sized>(
    context: InstructionContext<'_, H, impl InterpreterTypes<Extend = InterruptionExtension>>,
) {
    if let Some(_interruption_outcome) = unpack_interruption!(context) {
        return;
    }
    require_non_staticcall!(context.interpreter);

    popn!([offset, len], context.interpreter);
    let len = as_usize_or_fail!(context.interpreter, len);
    let data = if len == 0 {
        Bytes::new()
    } else {
        let offset = as_usize_or_fail!(context.interpreter, offset);
        resize_memory!(context.interpreter, offset, len);
        Bytes::copy_from_slice(context.interpreter.memory.slice_len(offset, len).as_ref())
    };
    if context.interpreter.stack.len() < N {
        context.interpreter.halt(InstructionResult::StackUnderflow);
        return;
    }
    let Some(topics) = context.interpreter.stack.popn::<N>() else {
        context.interpreter.halt(InstructionResult::StackUnderflow);
        return;
    };
    let topics: Vec<B256> = topics.into_iter().map(B256::from).collect();
    interrupt_into_action(context, |_context, sdk| {
        sdk.emit_log(&topics, data.as_ref())
    });
}

fn create<
    WIRE: InterpreterTypes<Extend = InterruptionExtension>,
    const IS_CREATE2: bool,
    H: Host + HostWrapper + ?Sized,
>(
    context: InstructionContext<'_, H, WIRE>,
) {
    if let Some(interruption_outcome) = unpack_interruption!(@frame context) {
        context.interpreter.return_data.clear();
        match interruption_outcome.exit_code {
            ExitCode::Ok => {
                debug_assert_eq!(interruption_outcome.output.len(), 20);
                let created_address = Address::from_slice(interruption_outcome.output.as_ref());
                push!(context.interpreter, created_address.into_u256());
            }
            ExitCode::Panic => {
                context
                    .interpreter
                    .return_data
                    .set_buffer(interruption_outcome.output);
                push!(context.interpreter, U256::ZERO);
            }
            _ => {
                push!(context.interpreter, U256::ZERO);
            }
        }
        return;
    }
    require_non_staticcall!(context.interpreter);
    popn!([value, code_offset, len], context.interpreter);
    let code_len = as_usize_or_fail!(context.interpreter, len);
    let mut init_code = Bytes::new();
    let init_gas_cost = if code_len != 0 {
        let code_offset = as_usize_or_fail!(context.interpreter, code_offset);
        // EIP-3860: Limit and meter initcode
        let max_initcode_size = if code_len >= 4
            && context.interpreter.memory.size() >= code_offset.checked_add(4).unwrap_or(usize::MAX)
        {
            let prefix = context.interpreter.memory.slice_len(code_offset, 4);
            wasm_max_code_size(&*prefix).unwrap_or(EVM_MAX_INITCODE_SIZE)
        } else {
            EVM_MAX_INITCODE_SIZE
        };
        // The limit is set as double of max contract bytecode size
        if code_len > max_initcode_size {
            context
                .interpreter
                .halt(InstructionResult::CreateInitCodeSizeLimit);
            return;
        }
        let init_gas_cost = gas::initcode_cost(code_len);
        if init_gas_cost > context.interpreter.gas.remaining() {
            context.interpreter.halt(InstructionResult::OutOfGas);
            return;
        }
        resize_memory!(context.interpreter, code_offset, code_len);
        init_code = Bytes::copy_from_slice(
            context
                .interpreter
                .memory
                .slice_len(code_offset, code_len)
                .as_ref(),
        );
        init_gas_cost
    } else {
        0
    };
    let create_gas_cost = if IS_CREATE2 {
        let Some(gas) = gas::create2_cost(init_code.len().try_into().unwrap()) else {
            context.interpreter.halt(InstructionResult::OutOfGas);
            return;
        };
        gas
    } else {
        gas::CREATE
    };
    // EIP-3860: Check we have enough gas before doing CREATE
    if init_gas_cost + create_gas_cost > context.interpreter.gas.remaining() {
        context.interpreter.halt(InstructionResult::OutOfGas);
        return;
    }
    let salt: Option<U256> = if IS_CREATE2 {
        popn!([salt], context.interpreter);
        Some(salt)
    } else {
        None
    };
    interrupt_into_action(context, |_context, sdk| {
        sdk.create(salt, &value, init_code.as_ref())
    });
}

fn insert_call_outcome<
    WIRE: InterpreterTypes<Extend = InterruptionExtension, Stack = Stack>,
    H: Host + ?Sized,
>(
    context: InstructionContext<'_, H, WIRE>,
    interruption_outcome: InterruptionOutcome,
) {
    let Some(return_memory_offset) = get_memory_output_range(context.interpreter) else {
        return;
    };
    let out_offset = return_memory_offset.start;
    let out_len = return_memory_offset.len();
    context
        .interpreter
        .return_data
        .set_buffer(interruption_outcome.output.clone());
    let target_len = min(out_len, interruption_outcome.output.len());
    match interruption_outcome.exit_code {
        ExitCode::Ok => {
            context
                .interpreter
                .memory
                .set(out_offset, &interruption_outcome.output[..target_len]);
            push!(context.interpreter, U256::from(1));
        }
        ExitCode::Panic => {
            context
                .interpreter
                .memory
                .set(out_offset, &interruption_outcome.output[..target_len]);
            push!(context.interpreter, U256::ZERO);
        }
        _ => {
            push!(context.interpreter, U256::ZERO);
        }
    }
}

#[inline]
fn get_memory_input_range<WIRE: InterpreterTypes<Extend = InterruptionExtension, Stack = Stack>>(
    interpreter: &mut Interpreter<WIRE>,
) -> Option<Range<usize>> {
    // We peek here, because we pop in [get_memory_output_range] function call
    peekn!([in_offset, in_len, out_offset, out_len], interpreter, None);
    let mut in_range = resize_memory(interpreter, in_offset, in_len)?;
    if !in_range.is_empty() {
        let offset = interpreter.memory.local_memory_offset();
        in_range = in_range.start.saturating_add(offset)..in_range.end.saturating_add(offset);
    }
    resize_memory(interpreter, out_offset, out_len)?;
    Some(in_range)
}

#[inline]
fn get_memory_output_range<
    WIRE: InterpreterTypes<Extend = InterruptionExtension, Stack = Stack>,
>(
    interpreter: &mut Interpreter<WIRE>,
) -> Option<Range<usize>> {
    // We pop here because we peeked in [get_memory_input_range] function call
    popn!(
        [_in_offset, _in_len, out_offset, out_len],
        interpreter,
        None
    );
    let out_offset = as_usize_or_fail_ret!(interpreter, out_offset, None);
    let out_len = as_usize_or_fail_ret!(interpreter, out_len, None);
    Some(out_offset..out_offset + out_len)
}

fn call<
    WIRE: InterpreterTypes<Extend = InterruptionExtension, Stack = Stack>,
    H: Host + HostWrapper + ?Sized,
>(
    context: InstructionContext<'_, H, WIRE>,
) {
    if let Some(interruption_outcome) = unpack_interruption!(@frame context) {
        insert_call_outcome(context, interruption_outcome);
        return;
    }
    popn!([local_gas_limit, to, value], context.interpreter);
    let to = to.into_address();
    // Max gas limit is not possible in real ethereum situation.
    let local_gas_limit = u64::try_from(local_gas_limit).unwrap_or(u64::MAX);
    let has_transfer = !value.is_zero();
    if context.interpreter.runtime_flag.is_static() && has_transfer {
        context
            .interpreter
            .halt(InstructionResult::CallNotAllowedInsideStatic);
        return;
    }
    let Some(in_range) = get_memory_input_range(context.interpreter) else {
        return;
    };
    interrupt_into_action(context, |context, sdk| {
        let input = global_memory_from_shared_buffer(&context, in_range);
        // TODO(dmitry123): I know that wrapping mul works here, but why?
        let fuel_limit = Some(local_gas_limit.wrapping_mul(FUEL_DENOM_RATE));
        sdk.call(to, value, input.as_ref(), fuel_limit)
    });
}

fn call_code<
    WIRE: InterpreterTypes<Extend = InterruptionExtension, Stack = Stack>,
    H: Host + HostWrapper + ?Sized,
>(
    context: InstructionContext<'_, H, WIRE>,
) {
    if let Some(interruption_outcome) = unpack_interruption!(@frame context) {
        insert_call_outcome(context, interruption_outcome);
        return;
    }
    popn!([local_gas_limit, to, value], context.interpreter);
    let to = to.into_address();
    // Max gas limit is not possible in real ethereum situation.
    let local_gas_limit = u64::try_from(local_gas_limit).unwrap_or(u64::MAX);
    let Some(in_range) = get_memory_input_range(context.interpreter) else {
        return;
    };
    interrupt_into_action(context, |context, sdk| {
        let input = global_memory_from_shared_buffer(&context, in_range);
        // TODO(dmitry123): I know that wrapping mul works here, but why?
        let fuel_limit = Some(local_gas_limit.wrapping_mul(FUEL_DENOM_RATE));
        sdk.call_code(to, value, input.as_ref(), fuel_limit)
    });
}

fn delegate_call<
    WIRE: InterpreterTypes<Extend = InterruptionExtension, Stack = Stack>,
    H: Host + HostWrapper + ?Sized,
>(
    context: InstructionContext<'_, H, WIRE>,
) {
    if let Some(interruption_outcome) = unpack_interruption!(@frame context) {
        insert_call_outcome(context, interruption_outcome);
        return;
    }
    popn!([local_gas_limit, to], context.interpreter);
    let to = to.into_address();
    // Max gas limit is not possible in real ethereum situation.
    let local_gas_limit = u64::try_from(local_gas_limit).unwrap_or(u64::MAX);
    let Some(in_range) = get_memory_input_range(context.interpreter) else {
        return;
    };
    interrupt_into_action(context, |context, sdk| {
        let input = global_memory_from_shared_buffer(&context, in_range);
        // TODO(dmitry123): I know that wrapping mul works here, but why?
        let fuel_limit = Some(local_gas_limit.wrapping_mul(FUEL_DENOM_RATE));
        sdk.delegate_call(to, input.as_ref(), fuel_limit)
    });
}

fn static_call<
    WIRE: InterpreterTypes<Extend = InterruptionExtension, Stack = Stack>,
    H: Host + HostWrapper + ?Sized,
>(
    context: InstructionContext<'_, H, WIRE>,
) {
    if let Some(interruption_outcome) = unpack_interruption!(@frame context) {
        insert_call_outcome(context, interruption_outcome);
        return;
    }
    popn!([local_gas_limit, to], context.interpreter);
    let to = to.into_address();
    // Max gas limit is not possible in real ethereum situation.
    let local_gas_limit = u64::try_from(local_gas_limit).unwrap_or(u64::MAX);
    let Some(in_range) = get_memory_input_range(context.interpreter) else {
        return;
    };
    interrupt_into_action(context, |context, sdk| {
        let input = global_memory_from_shared_buffer(&context, in_range);
        // TODO(dmitry123): I know that wrapping mul works here, but why?
        let fuel_limit = Some(local_gas_limit.wrapping_mul(FUEL_DENOM_RATE));
        sdk.static_call(to, input.as_ref(), fuel_limit)
    });
}

fn selfdestruct<
    WIRE: InterpreterTypes<Extend = InterruptionExtension, Stack = Stack>,
    H: Host + ?Sized,
>(
    context: InstructionContext<'_, H, WIRE>,
) {
    if let Some(_interruption_outcome) = unpack_interruption!(context) {
        context.interpreter.halt(InstructionResult::SelfDestruct);
        return;
    }
    require_non_staticcall!(context.interpreter);
    popn!([target], context.interpreter);
    let target = target.into_address();
    interrupt_into_action(context, |_context, sdk| sdk.destroy_account(target));
}

/// Build an instruction table matching EVM semantics with interruption-aware handlers.
pub const fn interruptable_instruction_table<'a, SDK: SharedAPI>(
) -> [Instruction<InterruptingInterpreter, HostWrapperImpl<'a, SDK>>; 256] {
    let mut table = instruction_table::<InterruptingInterpreter, HostWrapperImpl<'a, SDK>>();
    use revm_bytecode::opcode::*;
    table[BALANCE as usize] = balance;
    table[EXTCODESIZE as usize] = extcodesize;
    table[EXTCODECOPY as usize] = extcodecopy;
    table[EXTCODEHASH as usize] = extcodehash;
    table[BLOCKHASH as usize] = blockhash;
    table[SELFBALANCE as usize] = selfbalance;
    table[SLOAD as usize] = sload;
    table[SSTORE as usize] = sstore;
    table[TLOAD as usize] = tload;
    table[TSTORE as usize] = tstore;
    table[LOG0 as usize] = log::<0, _>;
    table[LOG1 as usize] = log::<1, _>;
    table[LOG2 as usize] = log::<2, _>;
    table[LOG3 as usize] = log::<3, _>;
    table[LOG4 as usize] = log::<4, _>;
    table[CREATE as usize] = create::<_, false, _>;
    table[CALL as usize] = call;
    table[CALLCODE as usize] = call_code;
    table[DELEGATECALL as usize] = delegate_call;
    table[CREATE2 as usize] = create::<_, true, _>;
    table[STATICCALL as usize] = static_call;
    table[SELFDESTRUCT as usize] = selfdestruct;
    table
}
