use crate::{
    as_usize_or_fail, gas, pop, pop_address, require_non_staticcall, resize_memory,
    result::InstructionResult,
    utils::{get_memory_input_and_out_ranges, insert_call_outcome, insert_create_outcome},
    EVM,
};
use fluentbase_sdk::{
    Bytes, SharedAPI, EVM_MAX_INITCODE_SIZE, FUEL_DENOM_RATE, SVM_ELF_MAGIC_BYTES,
    SVM_MAX_CODE_SIZE, U256, WASM_MAGIC_BYTES, WASM_MAX_CODE_SIZE,
};

pub fn create<const IS_CREATE2: bool, SDK: SharedAPI>(evm: &mut EVM<SDK>) {
    require_non_staticcall!(evm);
    pop!(evm, value, code_offset, len);
    let code_len = as_usize_or_fail!(evm, len);
    let mut init_code = Bytes::new();
    let init_gas_cost = if code_len != 0 {
        let code_offset = as_usize_or_fail!(evm, code_offset);
        let max_initcode_size = if code_len >= 4 {
            let prefix = evm.memory.try_slice(code_offset, 4);
            if prefix == Some(&WASM_MAGIC_BYTES) {
                WASM_MAX_CODE_SIZE
            } else if prefix == Some(&SVM_ELF_MAGIC_BYTES) {
                SVM_MAX_CODE_SIZE
            } else {
                EVM_MAX_INITCODE_SIZE
            }
        } else {
            EVM_MAX_INITCODE_SIZE
        };
        if code_len > max_initcode_size {
            evm.state = InstructionResult::CreateInitCodeSizeLimit;
            return;
        }
        let init_gas_cost = gas::initcode_cost(code_len as u64);
        if init_gas_cost > evm.gas.remaining() {
            evm.state = InstructionResult::OutOfGas;
            return;
        }
        resize_memory!(evm, code_offset, code_len);
        init_code = Bytes::copy_from_slice(evm.memory.slice(code_offset, code_len));
        init_gas_cost
    } else {
        0
    };
    let gas_cost = if IS_CREATE2 {
        let Some(gas) = gas::create2_cost(init_code.len().try_into().unwrap()) else {
            evm.state = InstructionResult::OutOfGas;
            return;
        };
        gas
    } else {
        gas::CREATE
    };
    if init_gas_cost + gas_cost > evm.gas.remaining() {
        evm.state = InstructionResult::OutOfGas;
        return;
    }
    let salt: Option<U256> = if IS_CREATE2 {
        pop!(evm, salt);
        Some(salt)
    } else {
        None
    };
    // we should sync gas before doing call
    // to make sure gas is synchronized between different runtimes
    evm.sync_evm_gas();
    let result = evm.sdk.create(salt, &value, init_code.as_ref());
    insert_create_outcome(evm, result)
}

pub fn call<SDK: SharedAPI>(evm: &mut EVM<SDK>) {
    pop!(evm, local_gas_limit);
    pop_address!(evm, to);
    // max gas limit is not possible in a real ethereum situation.
    let local_gas_limit = u64::try_from(local_gas_limit).unwrap_or(u64::MAX);
    pop!(evm, value);
    let has_transfer = !value.is_zero();
    if evm.is_static && has_transfer {
        evm.state = InstructionResult::CallNotAllowedInsideStatic;
        return;
    }
    let Some((input, return_memory_offset)) = get_memory_input_and_out_ranges(evm) else {
        return;
    };
    // we should sync gas before doing call
    // to make sure gas is synchronized between different runtimes
    evm.sync_evm_gas();
    let result = evm.sdk.call(
        to,
        value,
        input.as_ref(),
        Some(local_gas_limit.wrapping_mul(FUEL_DENOM_RATE)),
    );
    insert_call_outcome(evm, result, return_memory_offset);
}

pub fn call_code<SDK: SharedAPI>(evm: &mut EVM<SDK>) {
    pop!(evm, local_gas_limit);
    pop_address!(evm, to);
    // max gas limit is not possible in real ethereum situation.
    let local_gas_limit = u64::try_from(local_gas_limit).unwrap_or(u64::MAX);
    pop!(evm, value);
    let Some((input, return_memory_offset)) = get_memory_input_and_out_ranges(evm) else {
        return;
    };
    // we should sync gas before doing call
    // to make sure gas is synchronized between different runtimes
    evm.sync_evm_gas();
    let result = evm.sdk.call_code(
        to,
        value,
        input.as_ref(),
        Some(local_gas_limit.wrapping_mul(FUEL_DENOM_RATE)),
    );
    insert_call_outcome(evm, result, return_memory_offset);
}

pub fn delegate_call<SDK: SharedAPI>(evm: &mut EVM<SDK>) {
    pop!(evm, local_gas_limit);
    pop_address!(evm, to);
    // max gas limit is not possible in real ethereum situation.
    let local_gas_limit = u64::try_from(local_gas_limit).unwrap_or(u64::MAX);
    let Some((input, return_memory_offset)) = get_memory_input_and_out_ranges(evm) else {
        return;
    };
    // we should sync gas before doing call
    // to make sure gas is synchronized between different runtimes
    evm.sync_evm_gas();
    let result = evm.sdk.delegate_call(
        to,
        input.as_ref(),
        Some(local_gas_limit.wrapping_mul(FUEL_DENOM_RATE)),
    );
    insert_call_outcome(evm, result, return_memory_offset);
}

pub fn static_call<SDK: SharedAPI>(evm: &mut EVM<SDK>) {
    pop!(evm, local_gas_limit);
    pop_address!(evm, to);
    // max gas limit is not possible in a real ethereum situation.
    let local_gas_limit = u64::try_from(local_gas_limit).unwrap_or(u64::MAX);
    let Some((input, return_memory_offset)) = get_memory_input_and_out_ranges(evm) else {
        return;
    };
    // we should sync gas before doing call
    // to make sure gas is synchronized between different runtimes
    evm.sync_evm_gas();
    let result = evm.sdk.static_call(
        to,
        input.as_ref(),
        Some(local_gas_limit.wrapping_mul(FUEL_DENOM_RATE)),
    );
    insert_call_outcome(evm, result, return_memory_offset);
}
