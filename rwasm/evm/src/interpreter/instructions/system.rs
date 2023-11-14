use crate::interpreter::host::Host;
use crate::interpreter::instruction_result::InstructionResult;
use crate::interpreter::interpreter::Translator;
use crate::primitives::{B256, KECCAK_EMPTY, U256};

pub fn keccak256<H: Host>(interpreter: &mut Translator<'_>, _host: &mut H) {
    pop!(interpreter, from, len);
    let len = as_usize_or_fail!(interpreter, len);
    // gas_or_fail!(interpreter, gas::keccak256_cost(len as u64));
    // let hash = if len == 0 {
    //     KECCAK_EMPTY
    // } else {
    //     let from = as_usize_or_fail!(interpreter, from);
    //     shared_memory_resize!(interpreter, from, len);
    //     crate::primitives::keccak256(interpreter.shared_memory.slice(from, len))
    // };

    // push_b256!(interpreter, hash);
}

pub fn address<H: Host>(interpreter: &mut Translator<'_>, _host: &mut H) {
    // gas!(interpreter, gas::BASE);
    // push_b256!(interpreter, interpreter.contract.address.into_word());
}

pub fn caller<H: Host>(interpreter: &mut Translator<'_>, _host: &mut H) {
    // gas!(interpreter, gas::BASE);
    // push_b256!(interpreter, interpreter.contract.caller.into_word());
}

pub fn codesize<H: Host>(interpreter: &mut Translator<'_>, _host: &mut H) {
    // gas!(interpreter, gas::BASE);
    // push!(interpreter, U256::from(interpreter.contract.bytecode.len()));
}

pub fn codecopy<H: Host>(interpreter: &mut Translator<'_>, _host: &mut H) {
    pop!(interpreter, memory_offset, code_offset, len);
    let len = as_usize_or_fail!(interpreter, len);
    // gas_or_fail!(interpreter, gas::verylowcopy_cost(len as u64));
    if len == 0 {
        return;
    }
    let memory_offset = as_usize_or_fail!(interpreter, memory_offset);
    let code_offset = as_usize_saturated!(code_offset);
    shared_memory_resize!(interpreter, memory_offset, len);

    // Safety: set_data is unsafe function and memory_resize ensures us that it is safe to call it
    // interpreter.shared_memory.set_data(
    //     memory_offset,
    //     code_offset,
    //     len,
    //     interpreter.contract.bytecode.original_bytecode_slice(),
    // );
}

pub fn calldataload<H: Host>(interpreter: &mut Translator<'_>, _host: &mut H) {
    // gas!(interpreter, gas::VERYLOW);
    pop!(interpreter, index);
    let index = as_usize_saturated!(index);
    // let load = if index < interpreter.contract.input.len() {
    //     let have_bytes = 32.min(interpreter.contract.input.len() - index);
    //     let mut bytes = [0u8; 32];
    //     bytes[..have_bytes].copy_from_slice(&interpreter.contract.input[index..index + have_bytes]);
    //     B256::new(bytes)
    // } else {
    //     B256::ZERO
    // };
    let load = B256::ZERO;

    push_b256!(interpreter, load);
}

pub fn calldatasize<H: Host>(interpreter: &mut Translator<'_>, _host: &mut H) {
    // gas!(interpreter, gas::BASE);
    // push!(interpreter, U256::from(interpreter.contract.input.len()));
}

pub fn callvalue<H: Host>(interpreter: &mut Translator<'_>, _host: &mut H) {
    // gas!(interpreter, gas::BASE);
    // push!(interpreter, interpreter.contract.value);
}

pub fn calldatacopy<H: Host>(interpreter: &mut Translator<'_>, _host: &mut H) {
    pop!(interpreter, memory_offset, data_offset, len);
    let len = as_usize_or_fail!(interpreter, len);
    // gas_or_fail!(interpreter, gas::verylowcopy_cost(len as u64));
    if len == 0 {
        return;
    }
    let memory_offset = as_usize_or_fail!(interpreter, memory_offset);
    let data_offset = as_usize_saturated!(data_offset);
    shared_memory_resize!(interpreter, memory_offset, len);

    // Safety: set_data is unsafe function and memory_resize ensures us that it is safe to call it
    // interpreter.shared_memory.set_data(
    //     memory_offset,
    //     data_offset,
    //     len,
    //     &interpreter.contract.input,
    // );
}

/// EIP-211: New opcodes: RETURNDATASIZE and RETURNDATACOPY
pub fn returndatasize<H: Host /* , SPEC: Spec */>(interpreter: &mut Translator<'_>, _host: &mut H) {
    check!(interpreter, BYZANTIUM);
    // gas!(interpreter, gas::BASE);
    // push!(
    //     interpreter,
    //     U256::from(interpreter.return_data_buffer.len())
    // );
}

/// EIP-211: New opcodes: RETURNDATASIZE and RETURNDATACOPY
pub fn returndatacopy<H: Host /* , SPEC: Spec */>(interpreter: &mut Translator<'_>, _host: &mut H) {
    check!(interpreter, BYZANTIUM);
    pop!(interpreter, memory_offset, offset, len);
    let len = as_usize_or_fail!(interpreter, len);
    // gas_or_fail!(interpreter, gas::verylowcopy_cost(len as u64));
    let data_offset = as_usize_saturated!(offset);
    let (data_end, overflow) = data_offset.overflowing_add(len);
    if overflow || data_end > interpreter.return_data_buffer.len() {
        interpreter.instruction_result = InstructionResult::OutOfOffset;
        return;
    }
    if len != 0 {
        let memory_offset = as_usize_or_fail!(interpreter, memory_offset);
        shared_memory_resize!(interpreter, memory_offset, len);
        // interpreter.shared_memory.set(
        //     memory_offset,
        //     &interpreter.return_data_buffer[data_offset..data_end],
        // );
    }
}

pub fn gas<H: Host>(interpreter: &mut Translator<'_>, _host: &mut H) {
    // gas!(interpreter, gas::BASE);
    // push!(interpreter, U256::from(interpreter.gas.remaining()));
}
