use crate::{
    consts::SP_BASE_MEM_OFFSET_DEFAULT,
    translator::{
        host::Host,
        instruction_result::InstructionResult,
        instructions::{
            control::jump,
            opcode::{JUMP, JUMPI},
            utilities::replace_current_opcode_with_call_to_subroutine,
        },
        translator::Translator,
    },
    utilities::{align_to_evm_word_array, EVM_WORD_BYTES, WASM_I64_IN_EVM_WORD_COUNT},
};
use fluentbase_rwasm::rwasm::InstructionSet;
use log::debug;

pub fn pop<H: Host>(translator: &mut Translator<'_>, host: &mut H) {
    const OP: &str = "POP";
    debug!("op:{}", OP);
    replace_current_opcode_with_call_to_subroutine(translator, host);
}

pub fn push<const N: usize, H: Host>(translator: &mut Translator<'_>, host: &mut H) {
    const OP: &str = "PUSH";

    let next_opcode = translator.get_bytecode_byte(Some(N as isize));

    // if [JUMP].contains(&next_opcode) {
    //     jump(translator, host);
    //
    //     translator.instruction_pointer_inc(N + 1);
    //     translator.instruction_result = InstructionResult::Continue;
    // } else {
    let data = translator.get_bytecode_slice(None, N);

    let is = host.instruction_set();
    let mut is_aux = InstructionSet::new();

    {
        if N == 0 {
            for _ in 0..WASM_I64_IN_EVM_WORD_COUNT {
                is_aux.op_i64_const(0);
            }
            return;
        }
        let data_padded = align_to_evm_word_array(data, true);
        if let Err(_) = data_padded {
            translator.instruction_result = InstructionResult::OutOfOffset;
            return;
        }
        let data_padded = data_padded.unwrap();

        // let is = host.instruction_set();
        for i in 1..=4 {
            is_aux.op_i64_const(SP_BASE_MEM_OFFSET_DEFAULT);
            is_aux.op_i64_const(SP_BASE_MEM_OFFSET_DEFAULT);
            is_aux.op_i64_load(0);
            is_aux.op_i64_sub();
            is_aux.op_i64_const(8 * i);
            is_aux.op_i64_sub();
        }
        for chunk in data_padded.chunks(8) {
            let v = i64::from_le_bytes(chunk.try_into().unwrap());
            is_aux.op_i64_const(v);
            is_aux.op_i64_store(0);
        }
        // compute new SP and update it in memory
        is_aux.op_i64_const(SP_BASE_MEM_OFFSET_DEFAULT);
        is_aux.op_i64_const(SP_BASE_MEM_OFFSET_DEFAULT);
        is_aux.op_i64_load(0);
        is_aux.op_i64_const(EVM_WORD_BYTES);
        is_aux.op_i64_add();
        is_aux.op_i64_store(0);
    }
    debug!(
        "op:{}{} data:{:?} res_instr_count:{}",
        OP,
        N,
        data,
        is_aux.len()
    );

    is.extend(&is_aux);

    translator.instruction_pointer_inc(N);
    translator.instruction_result = InstructionResult::Continue;
    // }
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
