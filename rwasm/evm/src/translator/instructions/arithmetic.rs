use crate::translator::{
    host::Host,
    instructions::utilities::replace_current_opcode_with_subroutine,
    translator::Translator,
};
use log::debug;

pub fn wrapped_add<H: Host>(translator: &mut Translator<'_>, host: &mut H) {
    const OP: &str = "ADD";
    debug!("op:{}", OP);
    // replace_current_opcode_with_inline_func(translator, host, true);
    replace_current_opcode_with_subroutine(translator, host, true, false);
    // let instruction_set = host.instruction_set();
    //
    // let mut stack_pos_shift = 0;
    //
    // for subpart_idx in 0..(WASM_I64_IN_EVM_WORD_COUNT * 2) {
    //     let part_idx = subpart_idx / 2;
    //     let fetch_low_part = subpart_idx % 2 == 0;
    //     // extract i64 part of B evm
    //     duplicate_i64_part_of_evm_word(
    //         instruction_set,
    //         &mut stack_pos_shift,
    //         part_idx,
    //         true,
    //         false,
    //     );
    //     // stack: i64_part_of_B
    //
    //     // extract low part of B
    //     fetch_i64_part_as_i32(instruction_set, &mut stack_pos_shift, fetch_low_part);
    //     // stack: subpart_B
    //
    //     // extract i64 part of A
    //     duplicate_i64_part_of_evm_word(
    //         instruction_set,
    //         &mut stack_pos_shift,
    //         part_idx,
    //         false,
    //         false,
    //     );
    //     // stack: i64_part_of_A subpart_B
    //
    //     // extract low part of A
    //     fetch_i64_part_as_i32(instruction_set, &mut stack_pos_shift, fetch_low_part);
    //     // stack: subpart_A subpart_B
    //
    //     // sum low parts
    //     wasm_add(instruction_set, &mut stack_pos_shift);
    //     // stack: sum_of_subpart_A_and_subpart_B
    //
    //     //
    //     if subpart_idx != 0 {
    //         // add overflow amount (which must be on stack) to the sum of parts
    //         wasm_add(instruction_set, &mut stack_pos_shift);
    //         // stack: sum_of_subpart_A_and_subpart_B_with_overflow_amount
    //     }
    //
    //     split_i64_repr_of_i32_sum_into_overflow_and_normal_parts(
    //         instruction_set,
    //         &mut stack_pos_shift,
    //         !fetch_low_part,
    //     );
    // }
    //
    // // drop last overflow value
    // wasm_drop_n(instruction_set, &mut stack_pos_shift, 1);
    //
    // let mut stack_pos_shift = 0;
    // const BASE: usize = WASM_I64_IN_EVM_WORD_COUNT * 2;
    // for i in 0..WASM_I64_IN_EVM_WORD_COUNT {
    //     let items_base_pos = BASE - i * 2;
    //     let assign_pos = BASE - i + 1;
    //     duplicate_stack_value(instruction_set, &mut stack_pos_shift, items_base_pos);
    //     duplicate_stack_value(instruction_set, &mut stack_pos_shift, items_base_pos - 1);
    //     wasm_add(instruction_set, &mut stack_pos_shift);
    //     assign_to_stack_and_drop(instruction_set, &mut stack_pos_shift, assign_pos);
    // }
    // wasm_drop_n(
    //     instruction_set,
    //     &mut stack_pos_shift,
    //     WASM_I64_IN_EVM_WORD_COUNT,
    // );
}

pub fn wrapping_mul<H: Host>(translator: &mut Translator<'_>, host: &mut H) {
    const OP: &str = "MUL";
    debug!("op:{}", OP);
    // replace_current_opcode_with_inline_func(translator, host, true, false);
    replace_current_opcode_with_subroutine(translator, host, true, false);
}

pub fn wrapping_sub<H: Host>(translator: &mut Translator<'_>, host: &mut H) {
    const OP: &str = "SUB";
    debug!("op:{}", OP);
    // replace_current_opcode_with_code_snippet(translator, host, true);
    replace_current_opcode_with_subroutine(translator, host, true, false);
}

pub fn div<H: Host>(_translator: &mut Translator<'_>, _host: &mut H) {
    const OP: &str = "DIV";
    panic!("op:{} not implemented", OP);
}

pub fn sdiv<H: Host>(_translator: &mut Translator<'_>, _host: &mut H) {
    const OP: &str = "SDIV";
    panic!("op:{} not implemented", OP);
}

pub fn arithmetic_mod<H: Host>(_translator: &mut Translator<'_>, _host: &mut H) {
    const OP: &str = "MOD";
    panic!("op:{} not implemented", OP);
}

pub fn smod<H: Host>(_translator: &mut Translator<'_>, _host: &mut H) {
    const OP: &str = "SMOD";
    panic!("op:{} not implemented", OP);
}

pub fn addmod<H: Host>(_translator: &mut Translator<'_>, _host: &mut H) {
    const OP: &str = "ADDMOD";
    panic!("op:{} not implemented", OP);
}

pub fn mulmod<H: Host>(_translator: &mut Translator<'_>, _host: &mut H) {
    const OP: &str = "MULMOD";
    panic!("op:{} not implemented", OP);
}

pub fn exp<H: Host>(_translator: &mut Translator<'_>, _host: &mut H) {
    const OP: &str = "EXP";
    panic!("op:{} not implemented", OP);
}

/// In the yellow paper `SIGNEXTEND` is defined to take two inputs, we will call them
/// `x` and `y`, and produce one output. The first `t` bits of the output (numbering from the
/// left, starting from 0) are equal to the `t`-th bit of `y`, where `t` is equal to
/// `256 - 8(x + 1)`. The remaining bits of the output are equal to the corresponding bits of `y`.
/// Note: if `x >= 32` then the output is equal to `y` since `t <= 0`. To efficiently implement
/// this algorithm in the case `x < 32` we do the following. Let `b` be equal to the `t`-th bit
/// of `y` and let `s = 255 - t = 8x + 7` (this is effectively the same index as `t`, but
/// numbering the bits from the right instead of the left). We can create a bit mask which is all
/// zeros up to and including the `t`-th bit, and all ones afterwards by computing the quantity
/// `2^s - 1`. We can use this mask to compute the output depending on the value of `b`.
/// If `b == 1` then the yellow paper says the output should be all ones up to
/// and including the `t`-th bit, followed by the remaining bits of `y`; this is equal to
/// `y | !mask` where `|` is the bitwise `OR` and `!` is bitwise negation. Similarly, if
/// `b == 0` then the yellow paper says the output should start with all zeros, then end with
/// bits from `b`; this is equal to `y & mask` where `&` is bitwise `AND`.
pub fn signextend<H: Host>(_translator: &mut Translator<'_>, _host: &mut H) {
    const OP: &str = "SIGNEXTEND";
    panic!("op:{} not implemented", OP);
}
