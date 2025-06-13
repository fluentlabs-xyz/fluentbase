use crate::{
    as_u64_saturated,
    as_usize_or_fail,
    as_usize_saturated,
    gas,
    gas::warm_cold_cost,
    gas_or_fail,
    pop,
    pop_address,
    pop_top,
    push,
    push_b256,
    require_non_staticcall,
    resize_memory,
    result::InstructionResult,
    unwrap_syscall,
    utils::instruction_result_from_exit_code,
    BLOCK_HASH_HISTORY,
    EVM,
};
use alloc::vec::Vec;
use core::cmp::min;
use fluentbase_sdk::{
    calc_preimage_address,
    debug_log,
    ContextReader,
    SharedAPI,
    B256,
    KECCAK_EMPTY,
    PROTECTED_STORAGE_SLOT_0,
    U256,
};

pub fn balance<SDK: SharedAPI>(evm: &mut EVM<SDK>) {
    pop_address!(evm, address);
    let result = unwrap_syscall!(evm, evm.sdk.balance(&address));
    push!(evm, result);
}

/// EIP-1884: Repricing for trie-size-dependent opcodes
pub fn selfbalance<SDK: SharedAPI>(evm: &mut EVM<SDK>) {
    let result = unwrap_syscall!(evm, evm.sdk.self_balance());
    push!(evm, result);
}

pub fn extcodesize<SDK: SharedAPI>(evm: &mut EVM<SDK>) {
    pop_address!(evm, address);
    let delegated_code_hash = evm
        .sdk
        .delegated_storage(&address, &PROTECTED_STORAGE_SLOT_0.into());
    assert!(
        !delegated_code_hash.status.is_error(),
        "evm: delegated storage failed with error ({:?})",
        delegated_code_hash.status
    );
    let is_delegated = delegated_code_hash.status.is_ok();
    let (delegated_code_hash, is_cold_accessed, _is_empty) = delegated_code_hash.data;
    let preimage_address = if is_delegated {
        calc_preimage_address(&delegated_code_hash.into())
    } else {
        address
    };
    let code_size = evm.sdk.code_size(&preimage_address);
    if !code_size.status.is_ok() {
        evm.state = instruction_result_from_exit_code(code_size.status);
        return;
    }
    gas!(evm, warm_cold_cost(is_cold_accessed));
    push!(evm, U256::from(code_size.data));
}

/// EIP-1052: EXTCODEHASH opcode
pub fn extcodehash<SDK: SharedAPI>(evm: &mut EVM<SDK>) {
    pop_address!(evm, address);
    let delegated_code_hash = evm
        .sdk
        .delegated_storage(&address, &PROTECTED_STORAGE_SLOT_0.into());
    assert!(
        !delegated_code_hash.status.is_error(),
        "evm: delegated storage failed with error ({:?})",
        delegated_code_hash.status
    );
    let is_delegated = delegated_code_hash.status.is_ok();
    let (mut delegated_code_hash, is_cold_accessed, is_empty) = delegated_code_hash.data;
    // for delegated accounts, we can instantly return code hash
    // since the account is managed by the same runtime and store EVM code hash in this field
    if is_delegated {
        // if delegated code hash is zero, then it might be a contract deployment stage,
        // for non-empty account return KECCAK_EMPTY
        if delegated_code_hash == U256::ZERO && !is_empty {
            delegated_code_hash = Into::<U256>::into(KECCAK_EMPTY);
        }
        // charge the gas according to the cold/warm access to the account
        gas!(evm, warm_cold_cost(is_cold_accessed));
        // there is no need to request code hash for a delegated account
        // since we already know the result and can safely push it
        push!(evm, delegated_code_hash);
        return;
    };
    let code_hash = evm.sdk.code_hash(&address);
    if !code_hash.status.is_ok() {
        evm.state = instruction_result_from_exit_code(code_hash.status);
        return;
    }
    gas!(evm, warm_cold_cost(is_cold_accessed));
    push_b256!(evm, code_hash.data);
}

pub fn extcodecopy<SDK: SharedAPI>(evm: &mut EVM<SDK>) {
    pop_address!(evm, address);
    pop!(evm, memory_offset, code_offset, len_u256);
    debug_log!(
        "EXTCODECOPY: address={} memory_offset={} code_offset={} len_u256={}",
        address,
        memory_offset,
        code_offset,
        len_u256
    );

    let delegated_code_hash = evm
        .sdk
        .delegated_storage(&address, &PROTECTED_STORAGE_SLOT_0.into());
    assert!(
        !delegated_code_hash.status.is_error(),
        "evm: delegated storage failed with error ({:?})",
        delegated_code_hash.status
    );
    let is_delegated = delegated_code_hash.status.is_ok();
    let (delegated_code_hash, is_cold_accessed, _) = delegated_code_hash.data;

    let code_length = as_usize_or_fail!(evm, len_u256);
    gas_or_fail!(
        evm,
        gas::extcodecopy_cost(code_length as u64, is_cold_accessed)
    );
    if code_length == 0 {
        return;
    }

    let memory_offset = as_usize_or_fail!(evm, memory_offset);
    let code_offset = as_usize_saturated!(code_offset);

    let preimage_address = if is_delegated {
        calc_preimage_address(&delegated_code_hash.into())
    } else {
        address
    };
    let code = unwrap_syscall!(@gasless evm, evm.sdk.code_copy(&preimage_address, code_offset as u64, code_length as u64));
    let code_offset = min(code_offset, code.len());
    resize_memory!(evm, memory_offset, code_length as usize);
    // Note: this can't panic because we resized memory to fit.
    evm.memory
        .set_data(memory_offset, code_offset, code_length, &code);
}

pub fn blockhash<SDK: SharedAPI>(evm: &mut EVM<SDK>) {
    gas!(evm, gas::BLOCKHASH);
    pop_top!(evm, number);
    let number_u64 = as_u64_saturated!(number);
    let block_number = evm.sdk.context().block_number();
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

pub fn sload<SDK: SharedAPI>(evm: &mut EVM<SDK>) {
    pop_top!(evm, index);
    let result = unwrap_syscall!(evm, evm.sdk.storage(&index));
    *index = result;
}

pub fn sstore<SDK: SharedAPI>(evm: &mut EVM<SDK>) {
    require_non_staticcall!(evm);
    pop!(evm, index, value);
    evm.sync_evm_gas();
    unwrap_syscall!(evm, evm.sdk.write_storage(index, value));
}

/// EIP-1153: Transient storage opcodes
/// Store value to transient storage
pub fn tstore<SDK: SharedAPI>(evm: &mut EVM<SDK>) {
    require_non_staticcall!(evm);
    pop!(evm, index, value);
    unwrap_syscall!(evm, evm.sdk.write_transient_storage(index, value));
}

/// EIP-1153: Transient storage opcodes
/// Load value from transient storage
pub fn tload<SDK: SharedAPI>(evm: &mut EVM<SDK>) {
    pop_top!(evm, index);
    let result = unwrap_syscall!(evm, evm.sdk.transient_storage(index));
    *index = result;
}

pub fn log<const N: usize, SDK: SharedAPI>(evm: &mut EVM<SDK>) {
    require_non_staticcall!(evm);
    pop!(evm, offset, len);
    let len = as_usize_or_fail!(evm, len);
    let data = if len != 0 {
        let offset = as_usize_or_fail!(evm, offset);
        resize_memory!(evm, offset, len);
        evm.memory.slice(offset, len)
    } else {
        &[]
    };
    if evm.stack.len() < N {
        evm.state = InstructionResult::StackUnderflow;
        return;
    }
    let mut topics = Vec::with_capacity(N);
    for _ in 0..N {
        // SAFETY: stack bounds already checked few lines above
        topics.push(B256::from(unsafe { evm.stack.pop_unsafe() }));
    }
    unwrap_syscall!(evm, evm.sdk.emit_log(&topics, data));
}

pub fn selfdestruct<SDK: SharedAPI>(evm: &mut EVM<SDK>) {
    require_non_staticcall!(evm);
    pop_address!(evm, target);
    unwrap_syscall!(evm, evm.sdk.destroy_account(target));
    evm.state = InstructionResult::SelfDestruct;
}
