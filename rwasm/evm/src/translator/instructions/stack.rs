use crate::{
    translator::{host::Host, instruction_result::InstructionResult, translator::Translator},
    utilities::{
        align_to_evm_word_array,
        iterate_over_wasm_i64_chunks,
        WASM_I64_IN_EVM_WORD_COUNT,
    },
};
use log::debug;

pub fn pop<H: Host>(_translator: &mut Translator<'_>, host: &mut H) {
    const OP: &str = "PUSH";
    debug!("op:{}", OP);
    let instruction_set = host.instruction_set();
    for _ in 0..WASM_I64_IN_EVM_WORD_COUNT {
        instruction_set.op_drop();
    }
}

pub fn push<const N: usize, H: Host>(translator: &mut Translator<'_>, host: &mut H) {
    const OP: &str = "PUSH";
    let ip = translator.instruction_pointer;
    let data = unsafe { core::slice::from_raw_parts(ip, N) };
    debug!("op:{}{} data:{:x?}", OP, N, data);

    let instruction_set = host.instruction_set();

    if N == 0 {
        for _ in 0..WASM_I64_IN_EVM_WORD_COUNT {
            instruction_set.op_i64_const(0);
        }
        return;
    }
    let data_padded = align_to_evm_word_array(data, true);
    if let Err(_) = data_padded {
        translator.instruction_result = InstructionResult::OutOfOffset;
        return;
    }
    let data_padded = data_padded.unwrap();
    for bytes in iterate_over_wasm_i64_chunks(&data_padded) {
        let v = i64::from_be_bytes(bytes.try_into().unwrap());
        instruction_set.op_i64_const(v);
    }

    translator.instruction_pointer = unsafe { ip.add(N) };
    translator.instruction_result = InstructionResult::Continue;
}

pub fn dup<const N: usize, H: Host>(_translator: &mut Translator<'_>, host: &mut H) {
    const OP: &str = "DUP";
    debug!("op:{}{}", OP, N);
    let instruction_set = host.instruction_set();
    for _ in 0..WASM_I64_IN_EVM_WORD_COUNT {
        instruction_set.op_local_get((WASM_I64_IN_EVM_WORD_COUNT * N) as u32);
    }
}

pub fn swap<const N: usize, H: Host>(_translator: &mut Translator<'_>, host: &mut H) {
    const OP: &str = "SWAP";
    debug!("op:{}{}", OP, N);
    let instruction_set = host.instruction_set();
    // TODO to reduce computation we need swap function inside rwasm so we can decrease number of
    // ops to x/2
    for i in 0..WASM_I64_IN_EVM_WORD_COUNT {
        instruction_set.op_local_get((1 + i * 2) as u32);
    }
    for i in 0..WASM_I64_IN_EVM_WORD_COUNT {
        instruction_set.op_local_get((1 + (N + 1) * WASM_I64_IN_EVM_WORD_COUNT + i * 2) as u32);
    }

    for i in 0..WASM_I64_IN_EVM_WORD_COUNT {
        instruction_set.op_local_set((WASM_I64_IN_EVM_WORD_COUNT * 3 - i * 2) as u32)
    }
    for i in 0..WASM_I64_IN_EVM_WORD_COUNT {
        instruction_set.op_local_set(((N + 1) * WASM_I64_IN_EVM_WORD_COUNT - i * 2) as u32)
    }
}
