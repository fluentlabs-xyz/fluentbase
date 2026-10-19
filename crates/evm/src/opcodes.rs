//! Opcode shims that turn host-bound instructions into interruptions.
//!
//! The instruction table mirrors EVM semantics. For opcodes that require
//! host interaction, we emit a SystemInterruption and resume after the host
//! provides a result.

use crate::{
    as_u64_or_fail,
    host::{HostWrapper, HostWrapperImpl},
    types::{InterruptingInterpreter, InterruptionExtension, InterruptionOutcome},
    utils::{global_memory_from_shared_buffer, interrupt_into_action},
};
use alloc::{borrow::Cow, vec, vec::Vec};
use core::{cmp::min, mem::take, ops::Range};
use fluentbase_sdk::{
    syscall::*, Address, Bytes, ExitCode, SystemAPI, B256, EVM_MAX_INITCODE_SIZE, FUEL_DENOM_RATE,
    STATE_MAIN, U256,
};
use revm_interpreter::{
    as_u64_saturated, as_usize_or_fail, as_usize_saturated, gas, instruction_table,
    instructions::{
        contract::resize_memory,
        system::copy_cost_and_memory_resize,
        utility::{IntoAddress, IntoU256},
    },
    interpreter_types::{LoopControl, MemoryTr, ReturnData, RuntimeFlag, StackTr},
    peekn, popn, popn_top, push, require_non_staticcall, Host, Instruction, InstructionContext,
    InstructionExecResult, InstructionResult, Interpreter, InterpreterAction, InterpreterTypes,
    Stack,
};
use revm_primitives::{wasm::wasm_max_code_size, BLOCK_HASH_HISTORY};

#[doc(hidden)]
#[macro_export]
#[collapse_debuginfo(yes)]
macro_rules! _count {
    (@count) => { 0 };
    (@count $head:tt $($tail:tt)*) => { 1 + _count!(@count $($tail)*) };
    ($($arg:tt)*) => { _count!(@count $($arg)*) };
}

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
                return Err(InstructionResult::Suspend);
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
                return Err(InstructionResult::Suspend);
            }
            Some(interruption_outcome)
        } else {
            None
        }
    };
}

fn balance<WIRE: InterpreterTypes<Extend = InterruptionExtension>, H: HostWrapper + ?Sized>(
    context: InstructionContext<'_, H, WIRE>,
) -> InstructionExecResult {
    popn_top!([], top, context.interpreter);
    if let Some(interruption_outcome) = unpack_interruption!(context) {
        *top = interruption_outcome.into_u256();
        return Ok(());
    }
    let address = top.into_address();
    let mut buffer = [0u8; encode::balance_size_hint()];
    encode::balance_into(&mut &mut buffer[..], &address);
    context.host.sdk_mut().insert_interruption_income(
        SYSCALL_ID_BALANCE,
        Cow::Borrowed(&buffer[..]),
        None,
        STATE_MAIN,
    );
    interrupt_into_action(context)
}

fn selfbalance<WIRE: InterpreterTypes<Extend = InterruptionExtension>, H: HostWrapper + ?Sized>(
    context: InstructionContext<'_, H, WIRE>,
) -> InstructionExecResult {
    if let Some(interruption_outcome) = unpack_interruption!(context) {
        push!(context.interpreter, interruption_outcome.into_u256());
        return Ok(());
    }
    let mut buffer = [0u8; encode::self_balance_size_hint()];
    encode::self_balance_into(&mut &mut buffer[..]);
    context.host.sdk_mut().insert_interruption_income(
        SYSCALL_ID_SELF_BALANCE,
        Cow::Borrowed(&buffer[..]),
        None,
        STATE_MAIN,
    );
    interrupt_into_action(context)
}

fn extcodesize<WIRE: InterpreterTypes<Extend = InterruptionExtension>, H: HostWrapper + ?Sized>(
    context: InstructionContext<'_, H, WIRE>,
) -> InstructionExecResult {
    popn_top!([], top, context.interpreter);
    if let Some(interruption_outcome) = unpack_interruption!(context) {
        *top = interruption_outcome.into_u256();
        return Ok(());
    }
    let address = top.into_address();
    let mut buffer = [0u8; encode::code_size_size_hint()];
    encode::code_size_into(&mut &mut buffer[..], &address);
    context.host.sdk_mut().insert_interruption_income(
        SYSCALL_ID_CODE_SIZE,
        Cow::Borrowed(&buffer[..]),
        None,
        STATE_MAIN,
    );
    interrupt_into_action(context)
}

fn extcodecopy<
    WIRE: InterpreterTypes<Stack = Stack, Extend = InterruptionExtension>,
    H: Host + HostWrapper + ?Sized,
>(
    context: InstructionContext<'_, H, WIRE>,
) -> InstructionExecResult {
    if let Some(interruption_outcome) = unpack_interruption!(context) {
        popn!(
            [_address, memory_offset, _code_offset, len_u256],
            context.interpreter
        );
        let code = interruption_outcome.output;
        let len = as_usize_or_fail!(context.interpreter, len_u256);
        // A zero length ignores the memory offset, so it must not be narrowed here
        // either, otherwise we'd reject an offset the pre-interruption phase accepted.
        if len == 0 {
            return Ok(());
        }
        // Non-zero length means the offset was already validated before the interruption.
        let memory_offset_usize = as_usize_or_fail!(context.interpreter, memory_offset);
        context
            .interpreter
            .memory
            .set_data(memory_offset_usize, 0, len, &code);
        return Ok(());
    }
    peekn!(
        [address, memory_offset, code_offset, len_u256],
        context.interpreter
    );
    let address = address.into_address();
    let len = as_usize_or_fail!(context.interpreter, len_u256);
    let offset = as_usize_saturated!(code_offset);

    // Resize memory before interruption to make sure we have enough gas for the call
    if len != 0 {
        // Fail on casting of memory_offset only if len is not zero.
        let memory_offset_usize = as_usize_or_fail!(context.interpreter, memory_offset);
        // Resize memory to fit the code
        context
            .interpreter
            .resize_memory(context.host.gas_params(), memory_offset_usize, len)?;
    }

    let mut buffer = [0u8; encode::code_copy_size_hint()];
    encode::code_copy_into(&mut &mut buffer[..], &address, offset as u64, len as u64);
    context.host.sdk_mut().insert_interruption_income(
        SYSCALL_ID_CODE_COPY,
        Cow::Borrowed(&buffer[..]),
        None,
        STATE_MAIN,
    );
    interrupt_into_action(context)
}

fn extcodehash<WIRE: InterpreterTypes<Extend = InterruptionExtension>, H: HostWrapper + ?Sized>(
    context: InstructionContext<'_, H, WIRE>,
) -> InstructionExecResult {
    popn_top!([], top, context.interpreter);
    if let Some(interruption_outcome) = unpack_interruption!(context) {
        *top = interruption_outcome.into_b256().into();
        return Ok(());
    }
    let address = top.into_address();
    let mut buffer = [0u8; encode::code_hash_size_hint()];
    encode::code_hash_into(&mut &mut buffer[..], &address);
    context.host.sdk_mut().insert_interruption_income(
        SYSCALL_ID_CODE_HASH,
        Cow::Borrowed(&buffer[..]),
        None,
        STATE_MAIN,
    );
    interrupt_into_action(context)
}

fn blockhash<WIRE: InterpreterTypes<Extend = InterruptionExtension>, H: HostWrapper + ?Sized>(
    context: InstructionContext<'_, H, WIRE>,
) -> InstructionExecResult {
    popn_top!([], number, context.interpreter);
    if let Some(interruption_outcome) = unpack_interruption!(context) {
        *number = interruption_outcome.into_b256().into();
        return Ok(());
    }
    let requested_number = *number;
    let block_number = context.host.block_number();
    let Some(diff) = block_number.checked_sub(requested_number) else {
        *number = U256::ZERO;
        gas!(context.interpreter, gas::BLOCKHASH);
        return Ok(());
    };
    let diff = as_u64_saturated!(diff);
    // blockhash should push zero if the number is the same as the current block number.
    if diff == 0 || diff > BLOCK_HASH_HISTORY {
        *number = U256::ZERO;
        gas!(context.interpreter, gas::BLOCKHASH);
        return Ok(());
    }
    let requested_number = as_u64_saturated!(requested_number);
    let mut buffer = [0u8; encode::block_hash_size_hint()];
    encode::block_hash_into(&mut &mut buffer[..], requested_number);
    context.host.sdk_mut().insert_interruption_income(
        SYSCALL_ID_BLOCK_HASH,
        Cow::Borrowed(&buffer[..]),
        None,
        STATE_MAIN,
    );
    interrupt_into_action(context)
}

fn sload<WIRE: InterpreterTypes<Extend = InterruptionExtension>, H: HostWrapper + ?Sized>(
    context: InstructionContext<'_, H, WIRE>,
) -> InstructionExecResult {
    popn_top!([], index, context.interpreter);
    if let Some(interruption_outcome) = unpack_interruption!(context) {
        *index = interruption_outcome.into_u256();
        return Ok(());
    }
    let index = *index;
    let mut buffer = [0u8; encode::storage_read_size_hint()];
    encode::storage_read_into(&mut &mut buffer[..], &index);
    context.host.sdk_mut().insert_interruption_income(
        SYSCALL_ID_STORAGE_READ,
        Cow::Borrowed(&buffer[..]),
        None,
        STATE_MAIN,
    );
    interrupt_into_action(context)
}

fn sstore<
    WIRE: InterpreterTypes<Extend = InterruptionExtension>,
    H: Host + HostWrapper + ?Sized,
>(
    context: InstructionContext<'_, H, WIRE>,
) -> InstructionExecResult {
    if let Some(_interruption_outcome) = unpack_interruption!(context) {
        return Ok(());
    }
    require_non_staticcall!(context.interpreter);
    popn!([index, value], context.interpreter);
    let mut buffer = [0u8; encode::storage_write_size_hint()];
    encode::storage_write_into(&mut &mut buffer[..], &index, &value);
    context.host.sdk_mut().insert_interruption_income(
        SYSCALL_ID_STORAGE_WRITE,
        Cow::Borrowed(&buffer[..]),
        None,
        STATE_MAIN,
    );
    interrupt_into_action(context)
}

fn tload<WIRE: InterpreterTypes<Extend = InterruptionExtension>, H: HostWrapper + ?Sized>(
    context: InstructionContext<'_, H, WIRE>,
) -> InstructionExecResult {
    popn_top!([], index, context.interpreter);
    if let Some(interruption_outcome) = unpack_interruption!(context) {
        *index = interruption_outcome.into_u256();
        return Ok(());
    }
    let index = *index;
    let mut buffer = [0u8; encode::transient_read_size_hint()];
    encode::transient_read_into(&mut &mut buffer[..], &index);
    context.host.sdk_mut().insert_interruption_income(
        SYSCALL_ID_TRANSIENT_READ,
        Cow::Borrowed(&buffer[..]),
        None,
        STATE_MAIN,
    );
    interrupt_into_action(context)
}

fn tstore<WIRE: InterpreterTypes<Extend = InterruptionExtension>, H: HostWrapper + ?Sized>(
    context: InstructionContext<'_, H, WIRE>,
) -> InstructionExecResult {
    if let Some(_interruption_outcome) = unpack_interruption!(context) {
        return Ok(());
    }
    require_non_staticcall!(context.interpreter);
    popn!([index, value], context.interpreter);
    let mut buffer = [0u8; encode::transient_write_size_hint()];
    encode::transient_write_into(&mut &mut buffer[..], &index, &value);
    context.host.sdk_mut().insert_interruption_income(
        SYSCALL_ID_TRANSIENT_WRITE,
        Cow::Borrowed(&buffer[..]),
        None,
        STATE_MAIN,
    );
    interrupt_into_action(context)
}

fn log<const N: usize, H: HostWrapper + ?Sized>(
    context: InstructionContext<'_, H, impl InterpreterTypes<Extend = InterruptionExtension>>,
) -> InstructionExecResult {
    if let Some(_interruption_outcome) = unpack_interruption!(context) {
        return Ok(());
    }
    require_non_staticcall!(context.interpreter);

    popn!([offset, len], context.interpreter);
    let len = as_usize_or_fail!(context.interpreter, len);
    let data = if len == 0 {
        Bytes::new()
    } else {
        let offset = as_usize_or_fail!(context.interpreter, offset);
        context
            .interpreter
            .resize_memory(context.host.gas_params(), offset, len)?;
        Bytes::copy_from_slice(context.interpreter.memory.slice_len(offset, len).as_ref())
    };
    if context.interpreter.stack.len() < N {
        return Err(InstructionResult::StackUnderflow);
    }
    let Some(topics) = context.interpreter.stack.popn::<N>() else {
        return Err(InstructionResult::StackUnderflow);
    };
    let topics: Vec<B256> = topics.into_iter().map(B256::from).collect();
    let mut buffer = vec![0u8; encode::emit_log_size_hint(topics.len(), data.len())];
    encode::emit_log_into(&mut &mut buffer[..], &topics, data.as_ref());
    context.host.sdk_mut().insert_interruption_income(
        SYSCALL_ID_EMIT_LOG,
        Cow::Owned(buffer),
        None,
        STATE_MAIN,
    );
    interrupt_into_action(context)
}

fn create<
    WIRE: InterpreterTypes<Extend = InterruptionExtension>,
    const IS_CREATE2: bool,
    H: Host + HostWrapper + ?Sized,
>(
    context: InstructionContext<'_, H, WIRE>,
) -> InstructionExecResult {
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
        return Ok(());
    }
    require_non_staticcall!(context.interpreter);
    popn!([value, code_offset, len], context.interpreter);
    let code_len = as_u64_or_fail!(context.interpreter, len);
    let mut init_code = Bytes::new();
    let init_gas_cost = if code_len != 0 {
        let code_offset = as_usize_or_fail!(context.interpreter, code_offset);
        // EIP-3860: Limit and meter initcode
        let max_initcode_size = if code_len >= 4
            && context.interpreter.memory.size() >= code_offset.saturating_add(4)
        {
            let prefix = context.interpreter.memory.slice_len(code_offset, 4);
            wasm_max_code_size(&*prefix).unwrap_or(EVM_MAX_INITCODE_SIZE)
        } else {
            EVM_MAX_INITCODE_SIZE
        };
        // The limit is set as double of max contract bytecode size
        if code_len > max_initcode_size as u64 {
            return Err(InstructionResult::CreateInitCodeSizeLimit);
        }
        let init_gas_cost = context.host.gas_params().initcode_cost(code_len as usize);
        if init_gas_cost > context.interpreter.gas.remaining() {
            return Err(InstructionResult::OutOfGas);
        }
        context.interpreter.resize_memory(
            context.host.gas_params(),
            code_offset,
            code_len as usize,
        )?;
        init_code = Bytes::copy_from_slice(
            context
                .interpreter
                .memory
                .slice_len(code_offset, code_len as usize)
                .as_ref(),
        );
        init_gas_cost
    } else {
        0
    };
    let create_gas_cost = if IS_CREATE2 {
        context.host.gas_params().create2_cost(init_code.len())
    } else {
        context.host.gas_params().create_cost()
    };
    // EIP-3860: Check we have enough gas before doing CREATE
    if init_gas_cost + create_gas_cost > context.interpreter.gas.remaining() {
        return Err(InstructionResult::OutOfGas);
    }
    let salt: Option<U256> = if IS_CREATE2 {
        popn!([salt], context.interpreter);
        Some(salt)
    } else {
        None
    };
    let mut buffer = vec![0u8; encode::create_size_hint(init_code.len(), salt.is_some())];
    encode::create_into(
        &mut &mut buffer[..],
        salt.as_ref(),
        &value,
        init_code.as_ref(),
    );
    context.host.sdk_mut().insert_interruption_income(
        if IS_CREATE2 {
            SYSCALL_ID_CREATE2
        } else {
            SYSCALL_ID_CREATE
        },
        Cow::Owned(buffer),
        None,
        STATE_MAIN,
    );
    interrupt_into_action(context)
}

fn insert_call_outcome<
    WIRE: InterpreterTypes<Extend = InterruptionExtension, Stack = Stack>,
    H: HostWrapper + ?Sized,
>(
    context: InstructionContext<'_, H, WIRE>,
    interruption_outcome: InterruptionOutcome,
) -> InstructionExecResult {
    let return_memory_offset = get_memory_output_range(context.interpreter)?;
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
    Ok(())
}

#[inline]
fn get_memory_input_range<
    WIRE: InterpreterTypes<Extend = InterruptionExtension, Stack = Stack>,
    H: Host + HostWrapper + ?Sized,
>(
    context: &mut InstructionContext<'_, H, WIRE>,
) -> Result<Range<usize>, InstructionResult> {
    // We peek here because we pop in [get_memory_output_range] function call
    peekn!(
        [in_offset, in_len, out_offset, out_len],
        context.interpreter
    );
    let mut in_range = resize_memory(
        context.interpreter,
        context.host.gas_params(),
        in_offset,
        in_len,
    )?;
    if !in_range.is_empty() {
        let offset = context.interpreter.memory.local_memory_offset();
        in_range = in_range.start.saturating_add(offset)..in_range.end.saturating_add(offset);
    }
    resize_memory(
        context.interpreter,
        context.host.gas_params(),
        out_offset,
        out_len,
    )?;
    Ok(in_range)
}

#[inline]
fn get_memory_output_range<
    WIRE: InterpreterTypes<Extend = InterruptionExtension, Stack = Stack>,
>(
    interpreter: &mut Interpreter<WIRE>,
) -> Result<Range<usize>, InstructionResult> {
    // We pop here because we peeked in [get_memory_input_range] function call
    popn!([_in_offset, _in_len, out_offset, out_len], interpreter);
    let out_len = as_usize_or_fail!(interpreter, out_len);
    if out_len == 0 {
        // Same sentinel `resize_memory` returns for an empty range: the offset is
        // ignored, so narrowing it here would reject an otherwise valid output range.
        return Ok(usize::MAX..usize::MAX);
    }
    // Non-zero length means [get_memory_input_range] already validated and resized this range.
    let out_offset = as_usize_or_fail!(interpreter, out_offset);
    Ok(out_offset..out_offset.saturating_add(out_len))
}

fn call<
    WIRE: InterpreterTypes<Extend = InterruptionExtension, Stack = Stack>,
    H: Host + HostWrapper + ?Sized,
>(
    mut context: InstructionContext<'_, H, WIRE>,
) -> InstructionExecResult {
    if let Some(interruption_outcome) = unpack_interruption!(@frame context) {
        return insert_call_outcome(context, interruption_outcome);
    }
    popn!([local_gas_limit, to, value], context.interpreter);
    let to = to.into_address();
    // Max gas limit is not possible in a real ethereum situation.
    let local_gas_limit = u64::try_from(local_gas_limit).unwrap_or(u64::MAX);
    let has_transfer = !value.is_zero();
    if context.interpreter.runtime_flag.is_static() && has_transfer {
        return Err(InstructionResult::CallNotAllowedInsideStatic);
    }
    let in_range = get_memory_input_range(&mut context)?;
    let input = global_memory_from_shared_buffer(&context, in_range);
    let mut buffer = vec![0u8; encode::call_size_hint(input.len(), true)];
    encode::call_into(&mut &mut buffer[..], to, Some(value), input.as_ref());
    drop(input);
    context.host.sdk_mut().insert_interruption_income(
        SYSCALL_ID_CALL,
        Cow::Owned(buffer),
        Some(local_gas_limit.saturating_mul(FUEL_DENOM_RATE)),
        STATE_MAIN,
    );
    interrupt_into_action(context)
}

fn call_code<
    WIRE: InterpreterTypes<Extend = InterruptionExtension, Stack = Stack>,
    H: Host + HostWrapper + ?Sized,
>(
    mut context: InstructionContext<'_, H, WIRE>,
) -> InstructionExecResult {
    if let Some(interruption_outcome) = unpack_interruption!(@frame context) {
        return insert_call_outcome(context, interruption_outcome);
    }
    popn!([local_gas_limit, to, value], context.interpreter);
    let to = to.into_address();
    // Max gas limit is not possible in a real ethereum situation.
    let local_gas_limit = u64::try_from(local_gas_limit).unwrap_or(u64::MAX);
    let in_range = get_memory_input_range(&mut context)?;
    let input = global_memory_from_shared_buffer(&context, in_range);
    let mut buffer = vec![0u8; encode::call_size_hint(input.len(), true)];
    encode::call_into(&mut &mut buffer[..], to, Some(value), input.as_ref());
    drop(input);
    context.host.sdk_mut().insert_interruption_income(
        SYSCALL_ID_CALL_CODE,
        Cow::Owned(buffer),
        Some(local_gas_limit.saturating_mul(FUEL_DENOM_RATE)),
        STATE_MAIN,
    );
    interrupt_into_action(context)
}

fn delegate_call<
    WIRE: InterpreterTypes<Extend = InterruptionExtension, Stack = Stack>,
    H: Host + HostWrapper + ?Sized,
>(
    mut context: InstructionContext<'_, H, WIRE>,
) -> InstructionExecResult {
    if let Some(interruption_outcome) = unpack_interruption!(@frame context) {
        return insert_call_outcome(context, interruption_outcome);
    }
    popn!([local_gas_limit, to], context.interpreter);
    let to = to.into_address();
    // Max gas limit is not possible in a real ethereum situation.
    let local_gas_limit = u64::try_from(local_gas_limit).unwrap_or(u64::MAX);
    let in_range = get_memory_input_range(&mut context)?;
    let input = global_memory_from_shared_buffer(&context, in_range);
    let mut buffer = vec![0u8; encode::call_size_hint(input.len(), false)];
    encode::call_into(&mut &mut buffer[..], to, None, input.as_ref());
    drop(input);
    context.host.sdk_mut().insert_interruption_income(
        SYSCALL_ID_DELEGATE_CALL,
        Cow::Owned(buffer),
        Some(local_gas_limit.saturating_mul(FUEL_DENOM_RATE)),
        STATE_MAIN,
    );
    interrupt_into_action(context)
}

fn static_call<
    WIRE: InterpreterTypes<Extend = InterruptionExtension, Stack = Stack>,
    H: Host + HostWrapper + ?Sized,
>(
    mut context: InstructionContext<'_, H, WIRE>,
) -> InstructionExecResult {
    if let Some(interruption_outcome) = unpack_interruption!(@frame context) {
        return insert_call_outcome(context, interruption_outcome);
    }
    popn!([local_gas_limit, to], context.interpreter);
    let to = to.into_address();
    // Max gas limit is not possible in a real ethereum situation.
    let local_gas_limit = u64::try_from(local_gas_limit).unwrap_or(u64::MAX);
    let in_range = get_memory_input_range(&mut context)?;
    let input = global_memory_from_shared_buffer(&context, in_range);
    let mut buffer = vec![0u8; encode::call_size_hint(input.len(), false)];
    encode::call_into(&mut &mut buffer[..], to, None, input.as_ref());
    drop(input);
    context.host.sdk_mut().insert_interruption_income(
        SYSCALL_ID_STATIC_CALL,
        Cow::Owned(buffer),
        Some(local_gas_limit.saturating_mul(FUEL_DENOM_RATE)),
        STATE_MAIN,
    );
    interrupt_into_action(context)
}

fn selfdestruct<
    WIRE: InterpreterTypes<Extend = InterruptionExtension, Stack = Stack>,
    H: HostWrapper + ?Sized,
>(
    context: InstructionContext<'_, H, WIRE>,
) -> InstructionExecResult {
    if let Some(_interruption_outcome) = unpack_interruption!(context) {
        return Err(InstructionResult::SelfDestruct);
    }
    require_non_staticcall!(context.interpreter);
    popn!([target], context.interpreter);
    let target = target.into_address();
    let mut buffer = [0u8; encode::destroy_account_size_hint()];
    encode::destroy_account_into(&mut &mut buffer[..], &target);
    context.host.sdk_mut().insert_interruption_income(
        SYSCALL_ID_DESTROY_ACCOUNT,
        Cow::Borrowed(&buffer[..]),
        None,
        STATE_MAIN,
    );
    interrupt_into_action(context)
}

/// EIP-211: New opcodes: RETURNDATASIZE and RETURNDATACOPY
fn returndatacopy<WIRE: InterpreterTypes, H: HostWrapper + ?Sized>(
    context: InstructionContext<'_, H, WIRE>,
) -> InstructionExecResult {
    popn!([memory_offset, offset, len], context.interpreter);

    let len = as_u64_or_fail!(context.interpreter, len);
    let data_offset = as_u64_saturated!(offset);

    // Old legacy behavior is to panic if data_end is out of the scope of the return buffer.
    let data_end = data_offset.saturating_add(len);
    if data_end > context.interpreter.return_data.buffer().len() as u64 {
        return Err(InstructionResult::OutOfOffset);
    }

    let Some(memory_offset) = copy_cost_and_memory_resize(
        context.interpreter,
        context.host.gas_params(),
        memory_offset,
        len as usize,
    )?
    else {
        return Ok(());
    };

    // Note: This can't panic because we resized memory to fit.
    context.interpreter.memory.set_data(
        memory_offset,
        data_offset as usize,
        len as usize,
        context.interpreter.return_data.buffer(),
    );
    Ok(())
}

/// Build an instruction table matching EVM semantics with interruption-aware handlers.
pub const fn interruptable_instruction_table<'a, SDK: SystemAPI>(
) -> [Instruction<InterruptingInterpreter, HostWrapperImpl<'a, SDK>>; 256] {
    let mut table = instruction_table::<InterruptingInterpreter, HostWrapperImpl<'a, SDK>>();
    use revm_bytecode::opcode::*;
    table[BALANCE as usize] = Instruction::new(balance);
    table[EXTCODESIZE as usize] = Instruction::new(extcodesize);
    table[EXTCODECOPY as usize] = Instruction::new(extcodecopy);
    table[RETURNDATACOPY as usize] = Instruction::new(returndatacopy);
    table[EXTCODEHASH as usize] = Instruction::new(extcodehash);
    table[BLOCKHASH as usize] = Instruction::new(blockhash);
    table[SELFBALANCE as usize] = Instruction::new(selfbalance);
    table[SLOAD as usize] = Instruction::new(sload);
    table[SSTORE as usize] = Instruction::new(sstore);
    table[TLOAD as usize] = Instruction::new(tload);
    table[TSTORE as usize] = Instruction::new(tstore);
    table[LOG0 as usize] = Instruction::new(log::<0, _>);
    table[LOG1 as usize] = Instruction::new(log::<1, _>);
    table[LOG2 as usize] = Instruction::new(log::<2, _>);
    table[LOG3 as usize] = Instruction::new(log::<3, _>);
    table[LOG4 as usize] = Instruction::new(log::<4, _>);
    table[CREATE as usize] = Instruction::new(create::<_, false, _>);
    table[CALL as usize] = Instruction::new(call);
    table[CALLCODE as usize] = Instruction::new(call_code);
    table[DELEGATECALL as usize] = Instruction::new(delegate_call);
    table[CREATE2 as usize] = Instruction::new(create::<_, true, _>);
    table[STATICCALL as usize] = Instruction::new(static_call);
    table[SELFDESTRUCT as usize] = Instruction::new(selfdestruct);
    table
}

/// Static charges for delegated execution at Osaka. Host syscalls charge the
/// complete cost of intercepted opcodes, including their resumed dispatch.
pub const fn interruptable_gas_table() -> revm_interpreter::GasTable {
    use revm_bytecode::opcode::*;
    let mut table =
        revm_interpreter::instructions::gas_table_spec(revm_primitives::hardfork::SpecId::OSAKA);
    let intercepted = [
        BALANCE,
        EXTCODESIZE,
        EXTCODECOPY,
        EXTCODEHASH,
        BLOCKHASH,
        SELFBALANCE,
        SLOAD,
        SSTORE,
        TLOAD,
        TSTORE,
        LOG0,
        LOG1,
        LOG2,
        LOG3,
        LOG4,
        CREATE,
        CALL,
        CALLCODE,
        DELEGATECALL,
        CREATE2,
        STATICCALL,
        SELFDESTRUCT,
    ];
    let mut i = 0;
    while i < intercepted.len() {
        table[intercepted[i] as usize] = 0;
        i += 1;
    }
    table
}

#[cfg(test)]
mod tests {
    use super::interruptable_gas_table;
    use revm_bytecode::opcode;

    #[test]
    fn delegated_gas_table_avoids_recharging_interrupted_opcodes() {
        let table = interruptable_gas_table();
        for opcode in [
            opcode::BALANCE,
            opcode::EXTCODESIZE,
            opcode::EXTCODECOPY,
            opcode::EXTCODEHASH,
            opcode::BLOCKHASH,
            opcode::SELFBALANCE,
            opcode::SLOAD,
            opcode::SSTORE,
            opcode::TLOAD,
            opcode::TSTORE,
            opcode::LOG0,
            opcode::LOG1,
            opcode::LOG2,
            opcode::LOG3,
            opcode::LOG4,
            opcode::CREATE,
            opcode::CALL,
            opcode::CALLCODE,
            opcode::DELEGATECALL,
            opcode::CREATE2,
            opcode::STATICCALL,
            opcode::SELFDESTRUCT,
        ] {
            assert_eq!(table[opcode as usize], 0);
        }
        assert_eq!(table[opcode::RETURNDATACOPY as usize], 3);
        assert_eq!(table[opcode::ADD as usize], 3);
        assert_eq!(table[opcode::CLZ as usize], 5);
    }
}
