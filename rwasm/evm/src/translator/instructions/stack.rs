use crate::{
    consts::SP_BASE_MEM_OFFSET_DEFAULT,
    translator::{
        host::Host,
        instruction_result::InstructionResult,
        instructions::utilities::replace_current_opcode_with_call_to_subroutine,
        translator::Translator,
    },
    utilities::{align_to_evm_word_array, WASM_I64_IN_EVM_WORD_COUNT},
};
use log::debug;

pub fn pop<H: Host>(translator: &mut Translator<'_>, host: &mut H) {
    const OP: &str = "POP";
    debug!("op:{}", OP);
    replace_current_opcode_with_call_to_subroutine(translator, host);
}

pub fn push<const N: usize, H: Host>(translator: &mut Translator<'_>, host: &mut H) {
    const OP: &str = "PUSH";
    let ip = translator.instruction_pointer;
    let data = unsafe { core::slice::from_raw_parts(ip, N) };
    debug!("op:{}{} data:{:?}", OP, N, data);

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

    let is = host.instruction_set();
    for i in 1..=4 {
        is.op_i64_const(SP_BASE_MEM_OFFSET_DEFAULT);
        is.op_i64_const(SP_BASE_MEM_OFFSET_DEFAULT);
        is.op_i64_load(0);
        is.op_i64_sub();
        is.op_i64_const(8 * i);
        is.op_i64_sub();
    }
    for chunk in data_padded.chunks(8) {
        let v = i64::from_le_bytes(chunk.try_into().unwrap());
        is.op_i64_const(v);
        is.op_i64_store(0);
    }
    // compute new SP and update it in memory
    is.op_i64_const(SP_BASE_MEM_OFFSET_DEFAULT);
    is.op_i64_const(SP_BASE_MEM_OFFSET_DEFAULT);
    is.op_i64_load(0);
    is.op_i64_const(32);
    is.op_i64_add();
    is.op_i64_store(0);

    translator.instruction_pointer = unsafe { ip.add(N) };
    translator.instruction_result = InstructionResult::Continue;
}

pub fn dup<const N: usize, H: Host>(translator: &mut Translator<'_>, host: &mut H) {
    const OP: &str = "DUP";
    debug!("op:{}{}", OP, N);
    replace_current_opcode_with_call_to_subroutine(translator, host);
}

pub fn swap<const N: usize, H: Host>(translator: &mut Translator<'_>, host: &mut H) {
    const OP: &str = "SWAP";
    debug!("op:{}{}", OP, N);
    replace_current_opcode_with_call_to_subroutine(translator, host);
}
