use crate::{
    translator::{
        gas,
        host::Host,
        instruction_result::InstructionResult,
        instructions::{
            opcode,
            opcode::{compute_push_count, JUMP, JUMPDEST, JUMPI, PUSH0, PUSH32},
            utilities::replace_with_call_to_subroutine,
        },
        translator::Translator,
    },
    utilities::{
        invalid_op_gen,
        load_i64_const,
        not_found_op_gen,
        sp_drop_u256,
        sp_get_offset,
        stop_op_gen,
        EVM_WORD_BYTES,
        WASM_I64_BYTES,
        WASM_I64_IN_EVM_WORD_COUNT,
    },
};
use core::{i64, u64};
#[cfg(test)]
use log::debug;

pub const JUMP_BR_INDIRECT_ARG_REL_OFFSET: usize = 6; // too heavy to replace with dyn comp
pub fn jump<H: Host>(translator: &mut Translator<'_>, _host: &mut H) {
    const OP: &str = "JUMP";
    const OPCODE: u8 = JUMP;
    #[cfg(test)]
    debug!("op:{}", OP);
    gas!(translator, gas::constants::MID);
    pop!(translator, _dest);
    const OP_PARAMS_COUNT: u64 = 1;

    let pc_from = translator.program_counter() - 1;
    let prev_opcode = unsafe { *translator.instruction_pointer_prev };
    if prev_opcode < PUSH0 || prev_opcode > PUSH32 {
        #[cfg(test)]
        panic!("expected PUSH{} opcode", prev_opcode);
        return_with_reason!(translator, InstructionResult::InvalidJump);
    }
    let push_count = compute_push_count(prev_opcode);
    let pc_prev = translator.program_counter_prev();
    let bytes_before = pc_from - pc_prev - 1;
    if bytes_before != push_count {
        #[cfg(test)]
        panic!("expected distance {} got {}", push_count, bytes_before);
        return_with_reason!(translator, InstructionResult::InvalidJump);
    };
    let mut pc_to_arr = [0u8; WASM_I64_BYTES];
    let mut bytes_to_fetch = if bytes_before < WASM_I64_BYTES {
        bytes_before
    } else {
        WASM_I64_BYTES
    };
    let pc_to_slice =
        translator.get_bytecode_slice(Some(-1 - bytes_to_fetch as isize), bytes_to_fetch);
    pc_to_arr[WASM_I64_BYTES - pc_to_slice.len()..].copy_from_slice(pc_to_slice);
    let pc_to = i64::from_be_bytes(pc_to_arr);

    let instr = translator.instruction_at_pc(pc_to as usize);
    if instr == None || instr.unwrap() != JUMPDEST {
        return_with_reason!(translator, InstructionResult::InvalidJump);
    }

    translator.jumps_add(OPCODE, pc_from, pc_to as usize);

    let is = translator.result_instruction_set_mut();
    #[cfg(test)]
    let is_before_len = is.len();
    sp_drop_u256(is, OP_PARAMS_COUNT);

    let is_current_len = is.len();
    #[cfg(test)]
    debug!(
        "hint: JUMP_BR_INDIRECT_ARG_OFFSET={}",
        is_current_len - is_before_len
    );
    // by default: just skips itself (replaced with real value later)
    is.op_i64_const(is_current_len as i64);
    is.op_br_indirect(2); // for i64_const and br_indirect itself
}

pub const JUMPI_BR_INDIRECT_ARG_REL_OFFSET: usize = 31; // too heavy to replace with dyn comp
pub fn jumpi<H: Host>(translator: &mut Translator<'_>, _host: &mut H) {
    const OP: &str = "JUMPI";
    const OPCODE: u8 = JUMPI;
    #[cfg(test)]
    debug!("op:{}", OP);
    gas!(translator, gas::constants::HIGH);
    pop!(translator, _dest, _value);
    const OP_PARAMS_COUNT: u64 = 2;

    let pc_from = translator.program_counter() - 1;
    let prev_opcode = unsafe { *translator.instruction_pointer_prev };
    if prev_opcode < PUSH0 || prev_opcode > PUSH32 {
        #[cfg(test)]
        panic!("expected PUSHX opcode");
        return_with_reason!(translator, InstructionResult::InvalidJump);
    }
    let push_count = compute_push_count(prev_opcode);
    let pc_prev = translator.program_counter_prev();
    let bytes_before = pc_from - pc_prev - 1;
    if bytes_before != push_count {
        #[cfg(test)]
        panic!("expected distance {} got {}", push_count, bytes_before);
        return_with_reason!(translator, InstructionResult::InvalidJump);
    };
    // const WASM_I64_BYTES_TMP: usize = 4;
    let mut pc_to_arr = [0u8; WASM_I64_BYTES];
    let mut bytes_to_fetch = if bytes_before < WASM_I64_BYTES {
        bytes_before
    } else {
        WASM_I64_BYTES
    };
    let pc_to_slice =
        translator.get_bytecode_slice(Some(-1 - bytes_to_fetch as isize), bytes_to_fetch);
    pc_to_arr[WASM_I64_BYTES - pc_to_slice.len()..].copy_from_slice(pc_to_slice);
    let pc_to = u64::from_be_bytes(pc_to_arr);

    let instr = translator.instruction_at_pc(pc_to as usize);
    if instr == None || instr.unwrap() != JUMPDEST {
        return_with_reason!(translator, InstructionResult::InvalidJump);
    }

    translator.jumps_add(OPCODE, pc_from, pc_to as usize);
    let is = translator.result_instruction_set_mut();
    #[cfg(test)]
    let is_before_len = is.len();

    sp_get_offset(is, None);
    sp_drop_u256(is, OP_PARAMS_COUNT);

    // fetch conditional param and make decision based on it
    is.op_i64_const(EVM_WORD_BYTES);
    is.op_i64_add();

    is.op_local_get(1);
    for i in 0..WASM_I64_IN_EVM_WORD_COUNT {
        if i > 0 {
            is.op_local_get(2);
            is.op_i64_const(i * WASM_I64_BYTES);
            is.op_i64_add();
        }
        load_i64_const(is, None);
        if i > 0 {
            is.op_i64_or();
        }
    }
    is.op_local_set(1);
    is.op_br_if_eqz(3);

    let is_current_len = is.len();
    #[cfg(test)]
    debug!(
        "hint: JUMPI_BR_INDIRECT_ARG_OFFSET={}",
        is_current_len - is_before_len
    );
    // by default: just skips itself (replaced with real value later)
    is.op_i64_const(is_current_len as i64);
    is.op_br_indirect(2); // for const and br_indirect itself
}

pub fn jumpdest<H: Host>(translator: &mut Translator<'_>, _host: &mut H) {
    const OP: &str = "JUMPDEST";
    #[cfg(test)]
    debug!("op:{}", OP);
    gas!(translator, gas::constants::JUMPDEST);
}

pub fn pc<H: Host>(translator: &mut Translator<'_>, _host: &mut H) {
    const OP: &str = "PC";
    if cfg!(test) {
        panic!("op:{} not implemented", OP);
    }
    return_with_reason!(translator, InstructionResult::OpcodeNotFound);
    // gas!(translator, gas::constants::BASE);
}

pub fn ret<H: Host>(translator: &mut Translator<'_>, host: &mut H) {
    const OP: &str = "RET";
    #[cfg(test)]
    debug!("op:{}", OP);
    replace_with_call_to_subroutine(translator, host);
}

pub fn revert<H: Host>(translator: &mut Translator<'_>, host: &mut H) {
    const OP: &str = "REVERT";
    #[cfg(test)]
    debug!("op:{}", OP);

    replace_with_call_to_subroutine(translator, host);
}

pub fn stop<H: Host>(translator: &mut Translator<'_>, _host: &mut H) {
    stop_op_gen(translator);
}

pub fn invalid<H: Host>(translator: &mut Translator<'_>, _host: &mut H) {
    invalid_op_gen(translator);
}

pub fn not_found<H: Host>(translator: &mut Translator<'_>, _host: &mut H) {
    not_found_op_gen(translator);
}
