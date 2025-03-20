use crate::unwrap_syscall;
use alloc::{vec, vec::Vec};
use core::cmp::min;
use fluentbase_sdk::{BlockContextReader, SharedAPI, SyscallStatus, FUEL_DENOM_RATE};
use revm_interpreter::{
    as_u64_saturated,
    as_usize_or_fail,
    as_usize_saturated,
    gas,
    interpreter::Interpreter,
    pop,
    pop_address,
    pop_top,
    primitives::{Bytes, B256, BLOCK_HASH_HISTORY, U256},
    push,
    push_b256,
    refund,
    require_non_staticcall,
    resize_memory,
    InstructionResult,
};

pub fn balance<SDK: SharedAPI>(interpreter: &mut Interpreter, sdk: &mut SDK) {
    pop_address!(interpreter, address);
    let result = unwrap_syscall!(interpreter, sdk.balance(&address));
    push!(interpreter, result);
}

/// EIP-1884: Repricing for trie-size-dependent opcodes
pub fn selfbalance<SDK: SharedAPI>(interpreter: &mut Interpreter, sdk: &mut SDK) {
    let result = unwrap_syscall!(interpreter, sdk.self_balance());
    push!(interpreter, result);
}

pub fn extcodesize<SDK: SharedAPI>(interpreter: &mut Interpreter, sdk: &mut SDK) {
    pop_address!(interpreter, address);
    let result = unwrap_syscall!(interpreter, sdk.code_size(&address));
    push!(interpreter, U256::from(result));
}

/// EIP-1052: EXTCODEHASH opcode
pub fn extcodehash<SDK: SharedAPI>(interpreter: &mut Interpreter, sdk: &mut SDK) {
    pop_address!(interpreter, address);
    let result = unwrap_syscall!(interpreter, sdk.code_hash(&address));
    push_b256!(interpreter, result);
}

pub fn extcodecopy<SDK: SharedAPI>(interpreter: &mut Interpreter, sdk: &mut SDK) {
    pop_address!(interpreter, address);
    pop!(interpreter, memory_offset, code_offset, len_u256);
    let result = unwrap_syscall!(interpreter, sdk.code_size(&address));
    let memory_offset = as_usize_or_fail!(interpreter, memory_offset);
    let code_offset = min(as_usize_saturated!(code_offset), result as usize);
    let len = as_usize_or_fail!(interpreter, len_u256);
    let mut buffer = vec![0u8; len];
    unwrap_syscall!(
        interpreter,
        sdk.code_copy(&address, code_offset as u32, &mut buffer)
    );
    if len == 0 {
        return;
    }
    resize_memory!(interpreter, memory_offset, len);
    interpreter
        .shared_memory
        .set_data(memory_offset, 0, len, &buffer);
}

pub fn blockhash<SDK: SharedAPI>(interpreter: &mut Interpreter, sdk: &mut SDK) {
    gas!(interpreter, gas::BLOCKHASH);
    pop_top!(interpreter, number);
    let number_u64 = as_u64_saturated!(number);
    let block_number = sdk.context().block_number();
    let hash = match block_number.checked_sub(number_u64) {
        Some(diff) => {
            if diff > 0 && diff <= BLOCK_HASH_HISTORY {
                todo!("implement block hash history")
            } else {
                B256::ZERO
            }
        }
        None => B256::ZERO,
    };
    *number = U256::from_be_bytes(hash.0);
}

pub fn sload<SDK: SharedAPI>(interpreter: &mut Interpreter, sdk: &mut SDK) {
    pop_top!(interpreter, index);
    let result = unwrap_syscall!(interpreter, sdk.storage(&index));
    *index = result;
}

pub fn sstore<SDK: SharedAPI>(interpreter: &mut Interpreter, sdk: &mut SDK) {
    require_non_staticcall!(interpreter);
    pop!(interpreter, index, value);
    unwrap_syscall!(interpreter, sdk.write_storage(index, value));
}

/// EIP-1153: Transient storage opcodes
/// Store value to transient storage
pub fn tstore<SDK: SharedAPI>(interpreter: &mut Interpreter, sdk: &mut SDK) {
    require_non_staticcall!(interpreter);
    pop!(interpreter, index, value);
    unwrap_syscall!(interpreter, sdk.write_transient_storage(index, value));
}

/// EIP-1153: Transient storage opcodes
/// Load value from transient storage
pub fn tload<SDK: SharedAPI>(interpreter: &mut Interpreter, sdk: &mut SDK) {
    pop_top!(interpreter, index);
    let result = unwrap_syscall!(interpreter, sdk.transient_storage(index));
    *index = result;
}

pub fn log<const N: usize, SDK: SharedAPI>(interpreter: &mut Interpreter, sdk: &mut SDK) {
    require_non_staticcall!(interpreter);
    pop!(interpreter, offset, len);
    let len = as_usize_or_fail!(interpreter, len);
    // gas_or_fail!(interpreter, gas::log_cost(N as u8, len as u64));
    let data = if len != 0 {
        let offset = as_usize_or_fail!(interpreter, offset);
        resize_memory!(interpreter, offset, len);
        Bytes::copy_from_slice(interpreter.shared_memory.slice(offset, len))
    } else {
        Bytes::new()
    };
    if interpreter.stack.len() < N {
        interpreter.instruction_result = InstructionResult::StackUnderflow;
        return;
    }
    let mut topics = Vec::with_capacity(N);
    for _ in 0..N {
        // SAFETY: stack bounds already checked few lines above
        topics.push(B256::from(unsafe { interpreter.stack.pop_unsafe() }));
    }
    unwrap_syscall!(interpreter, sdk.emit_log(data.clone(), &topics));
}

pub fn selfdestruct<SDK: SharedAPI>(interpreter: &mut Interpreter, sdk: &mut SDK) {
    require_non_staticcall!(interpreter);
    pop_address!(interpreter, target);
    unwrap_syscall!(interpreter, sdk.destroy_account(target));
    interpreter.instruction_result = InstructionResult::SelfDestruct;
}
