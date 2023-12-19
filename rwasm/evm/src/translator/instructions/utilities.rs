use crate::{
    translator::{host::Host, instructions::opcode, translator::Translator},
    utilities::{
        WASM_I64_BITS,
        WASM_I64_HIGH_32_BIT_MASK,
        WASM_I64_IN_EVM_WORD_COUNT,
        WASM_I64_LOW_32_BIT_MASK,
    },
};
use fluentbase_rwasm::{engine::bytecode::Instruction, module::ImportName, rwasm::InstructionSet};

pub(super) enum SystemFuncs {
    CryptoKeccak256,
    EvmSstore,
    EvmSload,
}

pub(super) fn wasm_call(
    instruction_set: &mut InstructionSet,
    fn_name: SystemFuncs,
    translator: &mut Translator,
) {
    let fn_name = match fn_name {
        SystemFuncs::CryptoKeccak256 => "_crypto_keccak256",
        SystemFuncs::EvmSstore => "_evm_sstore",
        SystemFuncs::EvmSload => "_evm_sload",
    };
    let import_fn_idx =
        translator.get_import_linker().index_mapping()[&ImportName::new("env", fn_name)].0;
    // instruction_set.op_i32_const(instruction_set.len() + 2);
    instruction_set.op_call(import_fn_idx);
}

pub(super) fn preprocess_op_params(
    translator: &mut Translator<'_>,
    host: &mut dyn Host,
    inject_memory_result_offset: bool,
    memory_result_offset_is_first_param: bool,
    // inject_return_offset: bool,
) {
    let opcode = translator.opcode_prev();
    let i64_stack_params_count: usize;
    const MEM_RESULT_OFFSET: usize = 0;
    match opcode {
        opcode::ISZERO | opcode::POP => {
            // i64_stack_params_count = 4;
            i64_stack_params_count = 0;
        }

        opcode::BYTE
        | opcode::EQ
        | opcode::GAS
        | opcode::LT
        | opcode::GT
        | opcode::SAR
        | opcode::SGT
        | opcode::SHL
        | opcode::SHR
        | opcode::SLT
        | opcode::ADD
        | opcode::SIGNEXTEND
        | opcode::SUB
        | opcode::MUL
        | opcode::DIV
        | opcode::MSTORE
        | opcode::MSTORE8
        | opcode::EXP
        | opcode::MOD
        | opcode::SMOD
        | opcode::SDIV => {
            // i64_stack_params_count = 8;
            i64_stack_params_count = 0;
        }

        opcode::MULMOD | opcode::ADDMOD => {
            // i64_stack_params_count = 12;
            i64_stack_params_count = 0;
        }

        opcode::AND | opcode::NOT | opcode::OR | opcode::XOR => {
            i64_stack_params_count = 0;
        }
        _ => {
            panic!("no postprocessing defined for 0x{:x?} opcode", opcode)
        }
    }

    let instruction_set = host.instruction_set();
    let mut aux_params_count = 0;
    if inject_memory_result_offset {
        aux_params_count += 1;
        if memory_result_offset_is_first_param {
            let offset_instruction = instruction_set.instr[instruction_set.len() as usize - 4];
            let offset = match offset_instruction {
                Instruction::I64Const(offset) => offset,
                x => {
                    panic!("unexpected instruction: {:?}", x)
                }
            };
            let mem_result_offset = offset.as_usize();
            instruction_set.op_i32_const(mem_result_offset);
        } else {
            instruction_set.op_i32_const(MEM_RESULT_OFFSET);
        }
    }
    if aux_params_count > 0 {
        let instruction_set_len = instruction_set.len() as usize;
        let last_item_idx = instruction_set_len - 1;
        let aux_params_start_idx = instruction_set_len - aux_params_count;
        let aux_params_end_idx = last_item_idx;
        let aux_params = instruction_set.instr[aux_params_start_idx..=aux_params_end_idx].to_vec();
        let params_start_idx = instruction_set_len - i64_stack_params_count - aux_params_count;
        let params_end_idx = params_start_idx + i64_stack_params_count;
        let params = instruction_set.instr[params_start_idx..params_end_idx].to_vec();

        instruction_set.instr[params_start_idx..params_start_idx + aux_params_count]
            .copy_from_slice(&aux_params);
        instruction_set.instr[params_start_idx + aux_params_count
            ..params_start_idx + aux_params_count + i64_stack_params_count]
            .clone_from_slice(&params);
    }
    // if inject_return_offset {
    let meta = translator
        .subroutine_meta(opcode)
        .expect(&format!("no meta found for 0x{:x?} opcode", opcode));
    let prev_funcs_len = meta.begin_offset as u32;
    instruction_set.op_i32_const(instruction_set.len() + 1 - prev_funcs_len);
    // }
}

// pub(super) fn replace_current_opcode_with_inline_func(
//     translator: &mut Translator<'_>,
//     host: &mut dyn Host,
//     inject_memory_result_offset: bool,
//     memory_result_offset_is_first_param: bool,
// ) {
//     preprocess_op_params(
//         translator,
//         host,
//         inject_memory_result_offset,
//         memory_result_offset_is_first_param,
//         false,
//     );
//
//     let instruction_set = host.instruction_set();
//     let opcode = translator.opcode_prev();
//     let mut instruction_set_replace = translator.inline_instruction_set(opcode).clone();
//     instruction_set
//         .instr
//         .extend(instruction_set_replace.instr.iter());
// }

pub(super) fn replace_current_opcode_with_call_to_subroutine(
    translator: &mut Translator<'_>,
    host: &mut dyn Host,
    inject_memory_result_offset: bool,
    memory_result_offset_is_first_param: bool,
) {
    preprocess_op_params(
        translator,
        host,
        inject_memory_result_offset,
        memory_result_offset_is_first_param,
    );

    let instruction_set = host.instruction_set();
    let opcode = translator.opcode_prev();
    let subroutine_meta = translator
        .subroutine_meta(opcode)
        .expect(format!("subroutine entry not found for opcode 0x{:x?}", opcode).as_str());

    let mut subroutine_entry =
        subroutine_meta.begin_offset as i32 - instruction_set.len() as i32 + 1;
    instruction_set.op_br(subroutine_entry);
}

pub(super) fn duplicate_stack_value(
    instruction_set: &mut InstructionSet,
    stack_pos_shift: &mut i32,
    item_stack_pos: usize,
) {
    instruction_set.op_local_get(item_stack_pos as u32);
    *stack_pos_shift += 1;
}

pub(super) fn evm_word_param_stack_pos(
    stack_pos_shift: i32,
    part_idx: usize,
    is_b_param: bool,
    start_from_be: bool,
) -> usize {
    if start_from_be {
        WASM_I64_IN_EVM_WORD_COUNT * if is_b_param { 0 } else { 1 }
            + part_idx
            + stack_pos_shift as usize
    } else {
        WASM_I64_IN_EVM_WORD_COUNT * if is_b_param { 1 } else { 2 } - part_idx
            + stack_pos_shift as usize
    }
}

pub(super) fn duplicate_i64_part_of_evm_word(
    instruction_set: &mut InstructionSet,
    stack_pos_shift: &mut i32,
    part_idx: usize,
    is_b_param: bool,
    start_from_left: bool,
) {
    duplicate_stack_value(
        instruction_set,
        stack_pos_shift,
        evm_word_param_stack_pos(*stack_pos_shift, part_idx, is_b_param, start_from_left),
    );
}
pub(super) fn i64_shift_part(
    instruction_set: &mut InstructionSet,
    _stack_pos_shift: &mut i32,
    shift_low_high: bool,
) {
    instruction_set.op_i64_const(WASM_I64_BITS / 2);
    if shift_low_high {
        // *stack_pos_shift += 1;
        instruction_set.op_i64_shl();
    // *stack_pos_shift -= 1;
    } else {
        // *stack_pos_shift += 1;
        instruction_set.op_i64_shr_u();
        // *stack_pos_shift -= 1;
    }
}
pub(super) fn fetch_i64_part_as_i32(
    instruction_set: &mut InstructionSet,
    stack_pos_shift: &mut i32,
    drop_high_part: bool,
) {
    instruction_set.op_i64_const(if drop_high_part {
        WASM_I64_LOW_32_BIT_MASK
    } else {
        WASM_I64_HIGH_32_BIT_MASK
    });
    // *stack_pos_shift += 1;
    instruction_set.op_i64_and();
    // *stack_pos_shift -= 1;

    if !drop_high_part {
        i64_shift_part(instruction_set, stack_pos_shift, false);
    }
}
pub(super) fn wasm_add(instruction_set: &mut InstructionSet, stack_pos_shift: &mut i32) {
    instruction_set.op_i64_add();
    *stack_pos_shift -= 1;
}
pub(super) fn wasm_and(instruction_set: &mut InstructionSet, stack_pos_shift: &mut i32) {
    instruction_set.op_i64_and();
    *stack_pos_shift -= 1;
}
pub(super) fn wasm_or(instruction_set: &mut InstructionSet, stack_pos_shift: &mut i32) {
    instruction_set.op_i64_or();
    *stack_pos_shift -= 1;
}
pub(super) fn wasm_xor(instruction_set: &mut InstructionSet, stack_pos_shift: &mut i32) {
    instruction_set.op_i64_xor();
    *stack_pos_shift -= 1;
}
pub(super) fn wasm_not(instruction_set: &mut InstructionSet, _stack_pos_shift: &mut i32) {
    instruction_set.op_i64_const(-1);
    instruction_set.op_i64_sub();
}
pub(super) fn wasm_drop_n(
    instruction_set: &mut InstructionSet,
    stack_pos_shift: &mut i32,
    count: usize,
) {
    for _ in 0..count {
        instruction_set.op_drop();
    }
    *stack_pos_shift -= count as i32;
}
pub(super) fn assign_to_stack_and_drop(
    instruction_set: &mut InstructionSet,
    stack_pos_shift: &mut i32,
    stack_pos: usize,
) {
    instruction_set.op_local_set(stack_pos as u32);
    *stack_pos_shift -= 1;
}
pub(super) fn split_i64_repr_of_i32_sum_into_overflow_and_normal_parts(
    instruction_set: &mut InstructionSet,
    stack_pos_shift: &mut i32,
    do_upgrade_to_high_part: bool,
) {
    // split value onto overflow part (which is greater 0xffffffffff) and normal and them on stack
    // so overflow part is on top puts overflow value on top of the stack and normal value next
    // to it
    duplicate_stack_value(instruction_set, stack_pos_shift, 1);
    // extract overflow part
    fetch_i64_part_as_i32(instruction_set, stack_pos_shift, false);
    duplicate_stack_value(instruction_set, stack_pos_shift, 2);
    // extract normal part
    fetch_i64_part_as_i32(instruction_set, stack_pos_shift, true);
    if do_upgrade_to_high_part {
        i64_shift_part(instruction_set, stack_pos_shift, true);
    }
    // replace initial value with normal part
    instruction_set.op_local_set(3);
    *stack_pos_shift += 1;
}
