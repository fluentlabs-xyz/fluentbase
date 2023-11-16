use crate::utilities::{
    WASM_I64_BITS, WASM_I64_HIGH_32_BIT_MASK, WASM_I64_IN_EVM_WORD_COUNT, WASM_I64_LOW_32_BIT_MASK,
};
use fluentbase_rwasm::rwasm::InstructionSet;

pub fn duplicate(
    instruction_set: &mut InstructionSet,
    stack_pos_shift: &mut i32,
    item_stack_pos: usize,
) {
    instruction_set.op_local_get(item_stack_pos as u32);
    *stack_pos_shift += 1;
}

pub fn evm_word_param_stack_pos(stack_pos_shift: i32, part_idx: usize, is_b_param: bool) -> usize {
    WASM_I64_IN_EVM_WORD_COUNT * if is_b_param { 1 } else { 2 } - part_idx
        + stack_pos_shift as usize
}

pub fn extract_i64_part_of_evm_word(
    instruction_set: &mut InstructionSet,
    stack_pos_shift: &mut i32,
    part_idx: usize,
    is_b_param: bool,
) {
    duplicate(
        instruction_set,
        stack_pos_shift,
        evm_word_param_stack_pos(*stack_pos_shift, part_idx, is_b_param),
    );
}
pub fn i64_shift_part(
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
pub fn fetch_i64_part_as_i32(
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
pub fn add(instruction_set: &mut InstructionSet, stack_pos_shift: &mut i32) {
    instruction_set.op_i64_add();
    *stack_pos_shift -= 1;
}
pub fn drop_n(instruction_set: &mut InstructionSet, stack_pos_shift: &mut i32, count: usize) {
    for _ in 0..count {
        instruction_set.op_drop();
    }
    *stack_pos_shift -= count as i32;
}
pub fn assign_and_drop(
    instruction_set: &mut InstructionSet,
    stack_pos_shift: &mut i32,
    assign_stack_pos: u32,
) {
    instruction_set.op_local_set(assign_stack_pos);
    *stack_pos_shift -= 1;
}
pub fn split_i64_repr_of_i32_sum_into_overflow_and_normal_parts(
    instruction_set: &mut InstructionSet,
    stack_pos_shift: &mut i32,
    do_upgrade_to_high_part: bool,
) {
    // split value onto overflow part (which is greater 0xffffffffff) and normal and them on stack so overflow part is on top
    // puts overflow value on top of the stack and normal value next to it
    duplicate(instruction_set, stack_pos_shift, 1);
    // extract overflow part
    fetch_i64_part_as_i32(instruction_set, stack_pos_shift, false);
    duplicate(instruction_set, stack_pos_shift, 2);
    // extract normal part
    fetch_i64_part_as_i32(instruction_set, stack_pos_shift, true);
    if do_upgrade_to_high_part {
        i64_shift_part(instruction_set, stack_pos_shift, true);
    }
    // replace initial value with normal part
    instruction_set.op_local_set(3);
    *stack_pos_shift += 1;
}
