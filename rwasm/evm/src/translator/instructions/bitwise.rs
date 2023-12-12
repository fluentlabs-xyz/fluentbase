use crate::translator::{
    host::Host,
    instructions::utilities::{
        replace_current_opcode_with_inline_func,
        replace_current_opcode_with_subroutine,
    },
    translator::Translator,
};
use log::debug;

pub fn lt<H: Host>(translator: &mut Translator<'_>, host: &mut H) {
    const OP: &str = "LT";
    debug!("op:{}", OP);
    replace_current_opcode_with_inline_func(translator, host, true, false);
}

pub fn gt<H: Host>(translator: &mut Translator<'_>, host: &mut H) {
    const OP: &str = "GT";
    debug!("op:{}", OP);
    replace_current_opcode_with_inline_func(translator, host, true, false);
}

pub fn slt<H: Host>(translator: &mut Translator<'_>, host: &mut H) {
    const OP: &str = "SLT";
    debug!("op:{}", OP);
    replace_current_opcode_with_subroutine(translator, host, true, false);
    // replace_current_opcode_with_inline_func(translator, host, true, false);
}

pub fn sgt<H: Host>(translator: &mut Translator<'_>, host: &mut H) {
    const OP: &str = "SGT";
    debug!("op:{}", OP);
    replace_current_opcode_with_subroutine(translator, host, true, false);
    // replace_current_opcode_with_inline_func(translator, host, true, false);
}

pub fn eq<H: Host>(translator: &mut Translator<'_>, host: &mut H) {
    const OP: &str = "EQ";
    debug!("op:{}", OP);
    replace_current_opcode_with_inline_func(translator, host, true, false);
}

pub fn iszero<H: Host>(translator: &mut Translator<'_>, host: &mut H) {
    const OP: &str = "ISZERO";
    debug!("op:{}", OP);

    replace_current_opcode_with_subroutine(translator, host, true, false);
    // replace_current_opcode_with_inline_func(translator, host, true, false);
    // let instruction_set = host.instruction_set();
    //
    // for _part_idx in 0..(WASM_I64_IN_EVM_WORD_COUNT - 1) {
    //     instruction_set.op_i64_or();
    // }
    // instruction_set.op_i64_eqz();
    // for _part_idx in 0..(WASM_I64_IN_EVM_WORD_COUNT - 1) {
    //     instruction_set.op_i64_const(0);
    // }
}

pub fn bitand<H: Host>(translator: &mut Translator<'_>, host: &mut H) {
    const OP: &str = "AND";
    debug!("op:{}", OP);
    replace_current_opcode_with_subroutine(translator, host, true, false);
    // replace_current_opcode_with_inline_func(translator, host, true, false);
    // let instruction_set = host.instruction_set();
    //
    // let mut stack_post_shift = 0;
    // for _part_idx in 0..WASM_I64_IN_EVM_WORD_COUNT {
    //     duplicate_stack_value(
    //         instruction_set,
    //         &mut stack_post_shift,
    //         WASM_I64_IN_EVM_WORD_COUNT + 1,
    //     );
    //     wasm_and(instruction_set, &mut stack_post_shift);
    //     assign_to_stack_and_drop(
    //         instruction_set,
    //         &mut stack_post_shift,
    //         WASM_I64_IN_EVM_WORD_COUNT + 1,
    //     );
    // }
}

pub fn bitor<H: Host>(translator: &mut Translator<'_>, host: &mut H) {
    const OP: &str = "OR";
    debug!("op:{}", OP);
    replace_current_opcode_with_subroutine(translator, host, true, false);
    // replace_current_opcode_with_inline_func(translator, host, true, false);
    // let instruction_set = host.instruction_set();
    //
    // let mut stack_post_shift = 0;
    // for _part_idx in 0..WASM_I64_IN_EVM_WORD_COUNT {
    //     duplicate_stack_value(instruction_set, &mut stack_post_shift, 5);
    //     wasm_or(instruction_set, &mut stack_post_shift);
    //     assign_to_stack_and_drop(instruction_set, &mut stack_post_shift, 5);
    // }
}

pub fn bitxor<H: Host>(translator: &mut Translator<'_>, host: &mut H) {
    const OP: &str = "XOR";
    debug!("op:{}", OP);
    replace_current_opcode_with_subroutine(translator, host, true, false);
    // replace_current_opcode_with_inline_func(translator, host, true, false);
    // let instruction_set = host.instruction_set();

    // let mut stack_post_shift = 0;
    // for _part_idx in 0..WASM_I64_IN_EVM_WORD_COUNT {
    //     duplicate_stack_value(instruction_set, &mut stack_post_shift, 5);
    //     wasm_xor(instruction_set, &mut stack_post_shift);
    //     assign_to_stack_and_drop(instruction_set, &mut stack_post_shift, 5);
    // }
}

pub fn not<H: Host>(translator: &mut Translator<'_>, host: &mut H) {
    const OP: &str = "NOT";
    debug!("op:{}", OP);
    replace_current_opcode_with_subroutine(translator, host, true, false);
    // replace_current_opcode_with_inline_func(translator, host, true, false);
    // let instruction_set = host.instruction_set();
    //
    // let mut stack_post_shift = 0;
    // for part_idx in 0..WASM_I64_IN_EVM_WORD_COUNT {
    //     if part_idx > 0 {
    //         duplicate_stack_value(instruction_set, &mut stack_post_shift, part_idx + 1);
    //         wasm_not(instruction_set, &mut stack_post_shift);
    //         assign_to_stack_and_drop(instruction_set, &mut stack_post_shift, part_idx + 2);
    //     } else {
    //         wasm_not(instruction_set, &mut stack_post_shift);
    //     }
    // }
}

pub fn byte<H: Host>(translator: &mut Translator<'_>, host: &mut H) {
    const OP: &str = "BYTE";
    debug!("op:{}", OP);
    replace_current_opcode_with_inline_func(translator, host, true, false);
}

pub fn shl<H: Host>(translator: &mut Translator<'_>, host: &mut H) {
    const OP: &str = "SHL";
    debug!("op:{}", OP);
    replace_current_opcode_with_inline_func(translator, host, true, false);
}

pub fn shr<H: Host>(translator: &mut Translator<'_>, host: &mut H) {
    const OP: &str = "SHR";
    debug!("op:{}", OP);
    replace_current_opcode_with_inline_func(translator, host, true, false);
}

pub fn sar<H: Host>(translator: &mut Translator<'_>, host: &mut H) {
    const OP: &str = "SAR";
    debug!("op:{}", OP);
    replace_current_opcode_with_subroutine(translator, host, true, false);
    // replace_current_opcode_with_inline_func(translator, host, true, false);
}
