use crate::{
    consts::SP_BASE_MEM_OFFSET_DEFAULT,
    primitives::U256,
    translator::{
        gas,
        host::Host,
        instruction_result::InstructionResult,
        instructions::utilities::replace_with_call_to_subroutine,
        translator::Translator,
    },
    utilities::{align_to_evm_word_array, EVM_WORD_BYTES, WASM_I64_IN_EVM_WORD_COUNT},
};
use fluentbase_rwasm::rwasm::InstructionSet;
#[cfg(test)]
use log::debug;

pub fn pop<H: Host>(translator: &mut Translator<'_>, host: &mut H) {
    const OP: &str = "POP";
    #[cfg(test)]
    debug!("op:{}", OP);
    let is = translator.result_instruction_set_mut();
    gas!(translator, gas::constants::BASE);

    replace_with_call_to_subroutine(translator, host);
}

pub fn push<const N: usize, H: Host>(translator: &mut Translator<'_>, host: &mut H) {
    const OP: &str = "PUSH";
    #[cfg(test)]
    debug!("op:{}", OP);

    let next_opcode = translator.get_bytecode_byte(Some(N as isize));

    // if [JUMP].contains(&next_opcode) {
    //     jump(translator, host);
    //
    //     translator.instruction_pointer_inc(N + 1);
    //     translator.instruction_result = InstructionResult::Continue;
    // } else {
    // let data = translator.get_bytecode_slice(None, N);

    let mut is_aux = InstructionSet::new();

    {
        if N == 0 {
            gas!(translator, gas::constants::BASE);
            if let Err(result) = translator.stack.push(U256::ZERO) {
                translator.instruction_result = result;
            }
            for _ in 0..WASM_I64_IN_EVM_WORD_COUNT {
                is_aux.op_i64_const(0);
            }
            return;
        }
        gas!(translator, gas::constants::VERYLOW);
        if let Err(result) = translator
            .stack
            .push_slice(unsafe { core::slice::from_raw_parts(translator.instruction_pointer, N) })
        {
            translator.instruction_result = result;
            return;
        }
        let data_padded = align_to_evm_word_array(translator.get_bytecode_slice(None, N), true);
        if let Err(_) = data_padded {
            translator.instruction_result = InstructionResult::OutOfOffset;
            return;
        }
        let data_padded = data_padded.unwrap();

        // let is = translator.result_instruction_set();
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
    // debug!(
    //     "op:{}{} data:{:?} res_instr_count:{}",
    //     OP,
    //     N,
    //     data,
    //     is_aux.len()
    // );

    let is = translator.result_instruction_set_mut();
    is.extend(&is_aux);

    translator.instruction_pointer_inc(N);
    translator.instruction_result = InstructionResult::Continue;
    // }
}

pub fn dup<const N: usize, H: Host>(translator: &mut Translator<'_>, host: &mut H) {
    const OP: &str = "DUP";
    #[cfg(test)]
    debug!("op:{}", OP);
    let is = translator.result_instruction_set_mut();
    gas!(translator, gas::constants::VERYLOW);

    replace_with_call_to_subroutine(translator, host);
}

pub fn swap<const N: usize, H: Host>(translator: &mut Translator<'_>, host: &mut H) {
    const OP: &str = "SWAP";
    #[cfg(test)]
    debug!("op:{}", OP);
    let is = translator.result_instruction_set_mut();
    gas!(translator, gas::constants::VERYLOW);

    replace_with_call_to_subroutine(translator, host);
}
