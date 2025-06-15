use crate::{
    as_usize_or_fail,
    as_usize_saturated,
    gas,
    gas_or_fail,
    pop,
    pop_top,
    push,
    push_b256,
    resize_memory,
    result::InstructionResult,
    EVM,
};
use core::ptr;
use fluentbase_sdk::{ContextReader, SharedAPI, B256, KECCAK_EMPTY, U256};

pub fn keccak256<SDK: SharedAPI>(evm: &mut EVM<SDK>) {
    pop_top!(evm, offset, len_ptr);
    let len = as_usize_or_fail!(evm, len_ptr);
    gas_or_fail!(evm, gas::keccak256_cost(len as u64));
    let hash = if len == 0 {
        KECCAK_EMPTY
    } else {
        let from = as_usize_or_fail!(evm, offset);
        resize_memory!(evm, from, len);
        let input = evm.memory.slice(from, len);
        evm.sdk.keccak256(input)
    };
    *len_ptr = hash.into();
}

pub fn address<SDK: SharedAPI>(evm: &mut EVM<SDK>) {
    gas!(evm, gas::BASE);
    push_b256!(evm, evm.sdk.context().contract_address().into_word());
}

pub fn caller<SDK: SharedAPI>(evm: &mut EVM<SDK>) {
    gas!(evm, gas::BASE);
    push_b256!(evm, evm.sdk.context().contract_caller().into_word());
}

pub fn codesize<SDK: SharedAPI>(evm: &mut EVM<SDK>) {
    gas!(evm, gas::BASE);
    push!(evm, U256::from(evm.analyzed_bytecode.len()));
}

pub fn codecopy<SDK: SharedAPI>(evm: &mut EVM<SDK>) {
    pop!(evm, memory_offset, code_offset, len);
    let len = as_usize_or_fail!(evm, len);
    gas_or_fail!(evm, gas::verylowcopy_cost(len as u64));
    if len == 0 {
        return;
    }
    let memory_offset = as_usize_or_fail!(evm, memory_offset);
    let code_offset = as_usize_saturated!(code_offset);
    resize_memory!(evm, memory_offset, len);
    // Note: this can't panic because we resized memory to fit.
    evm.memory.set_data(
        memory_offset,
        code_offset,
        len,
        evm.analyzed_bytecode.as_slice(),
    );
}

pub fn calldataload<SDK: SharedAPI>(evm: &mut EVM<SDK>) {
    gas!(evm, gas::VERY_LOW);
    pop_top!(evm, offset_ptr);
    let mut word = B256::ZERO;
    let offset = as_usize_saturated!(offset_ptr);
    if offset < evm.input.len() {
        let count = 32.min(evm.input.len() - offset);
        // SAFETY: count is bounded by the calldata length.
        // This is `word[..count].copy_from_slice(input[offset..offset + count])`, written using
        // raw pointers as apparently the compiler cannot optimize the slice version, and using
        // `get_unchecked` twice is uglier.
        debug_assert!(count <= 32 && offset + count <= evm.input.len());
        unsafe {
            ptr::copy_nonoverlapping(evm.input.as_ptr().add(offset), word.as_mut_ptr(), count)
        };
    }
    *offset_ptr = word.into();
}

pub fn calldatasize<SDK: SharedAPI>(evm: &mut EVM<SDK>) {
    gas!(evm, gas::BASE);
    push!(evm, U256::from(evm.input.len()));
}

pub fn callvalue<SDK: SharedAPI>(evm: &mut EVM<SDK>) {
    gas!(evm, gas::BASE);
    push!(evm, evm.sdk.context().contract_value());
}

pub fn calldatacopy<SDK: SharedAPI>(evm: &mut EVM<SDK>) {
    pop!(evm, memory_offset, data_offset, len);
    let len = as_usize_or_fail!(evm, len);
    gas_or_fail!(evm, gas::verylowcopy_cost(len as u64));
    if len == 0 {
        return;
    }
    let memory_offset = as_usize_or_fail!(evm, memory_offset);
    let data_offset = as_usize_saturated!(data_offset);
    resize_memory!(evm, memory_offset, len);

    // Note: this can't panic because we resized memory to fit.
    evm.memory
        .set_data(memory_offset, data_offset, len, &evm.input);
}

/// EIP-211: New opcodes: RETURNDATASIZE and RETURNDATACOPY
pub fn returndatasize<SDK: SharedAPI>(evm: &mut EVM<SDK>) {
    gas!(evm, gas::BASE);
    push!(evm, U256::from(evm.return_data_buffer.len()));
}

/// EIP-211: New opcodes: RETURNDATASIZE and RETURNDATACOPY
pub fn returndatacopy<SDK: SharedAPI>(evm: &mut EVM<SDK>) {
    pop!(evm, memory_offset, offset, len);

    let len = as_usize_or_fail!(evm, len);
    gas_or_fail!(evm, gas::verylowcopy_cost(len as u64));

    let data_offset = as_usize_saturated!(offset);
    let data_end = data_offset.saturating_add(len);

    // Old legacy behavior is to panic if data_end is out of scope of return buffer.
    // This behavior is changed in EOF.
    if data_end > evm.return_data_buffer.len() {
        evm.state = InstructionResult::OutOfOffset;
        return;
    }

    // if len is zero memory is not resized.
    if len == 0 {
        return;
    }

    // resize memory
    let memory_offset = as_usize_or_fail!(evm, memory_offset);
    resize_memory!(evm, memory_offset, len);

    // Note: this can't panic because we resized memory to fit.
    evm.memory
        .set_data(memory_offset, data_offset, len, &evm.return_data_buffer);
}

pub fn gas<SDK: SharedAPI>(evm: &mut EVM<SDK>) {
    gas!(evm, gas::BASE);
    push!(evm, U256::from(evm.gas.remaining()));
}
