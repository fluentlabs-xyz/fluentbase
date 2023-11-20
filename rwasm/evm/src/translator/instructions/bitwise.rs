use fluentbase_rwasm::instruction_set;
use fluentbase_rwasm::rwasm::InstructionSet;
use log::debug;

use crate::translator::host::Host;
use crate::translator::instructions::utilities::{
    assign_to_stack_and_drop, duplicate_stack_value, wasm_and, wasm_not, wasm_or, wasm_xor,
};
use crate::translator::translator::Translator;
use crate::utilities::WASM_I64_IN_EVM_WORD_COUNT;

pub fn lt<H: Host>(_translator: &mut Translator<'_>, _host: &mut H) {
    const OP: &str = "LT";
    panic!("op:{} not implemented", OP);
}

pub fn gt<H: Host>(_translator: &mut Translator<'_>, _host: &mut H) {
    const OP: &str = "GT";
    panic!("op:{} not implemented", OP);
}

pub fn slt<H: Host>(_translator: &mut Translator<'_>, _host: &mut H) {
    const OP: &str = "SLT";
    panic!("op:{} not implemented", OP);
}

pub fn sgt<H: Host>(_translator: &mut Translator<'_>, _host: &mut H) {
    const OP: &str = "SGT";
    panic!("op:{} not implemented", OP);
}

pub fn eq<H: Host>(_translator: &mut Translator<'_>, _host: &mut H) {
    const OP: &str = "EQ";
    panic!("op:{} not implemented", OP);
}

pub fn iszero<H: Host>(_translator: &mut Translator<'_>, host: &mut H) {
    const OP: &str = "ISZERO";
    debug!("op:{}", OP);
    let instruction_set = host.instruction_set();

    for _part_idx in 0..(WASM_I64_IN_EVM_WORD_COUNT - 1) {
        instruction_set.op_i64_or();
    }
    instruction_set.op_i64_eqz();
    for _part_idx in 0..(WASM_I64_IN_EVM_WORD_COUNT - 1) {
        instruction_set.op_i64_const(0);
    }
}

pub fn bitand<H: Host>(_translator: &mut Translator<'_>, host: &mut H) {
    const OP: &str = "AND";
    debug!("op:{}", OP);
    let instruction_set = host.instruction_set();

    let mut stack_post_shift = 0;
    for _part_idx in 0..WASM_I64_IN_EVM_WORD_COUNT {
        duplicate_stack_value(
            instruction_set,
            &mut stack_post_shift,
            WASM_I64_IN_EVM_WORD_COUNT + 1,
        );
        wasm_and(instruction_set, &mut stack_post_shift);
        assign_to_stack_and_drop(
            instruction_set,
            &mut stack_post_shift,
            WASM_I64_IN_EVM_WORD_COUNT + 1,
        );
    }
}

pub fn bitor<H: Host>(_translator: &mut Translator<'_>, host: &mut H) {
    const OP: &str = "OR";
    debug!("op:{}", OP);
    let instruction_set = host.instruction_set();

    let mut stack_post_shift = 0;
    for _part_idx in 0..WASM_I64_IN_EVM_WORD_COUNT {
        duplicate_stack_value(instruction_set, &mut stack_post_shift, 5);
        wasm_or(instruction_set, &mut stack_post_shift);
        assign_to_stack_and_drop(instruction_set, &mut stack_post_shift, 5);
    }
}

pub fn bitxor<H: Host>(_translator: &mut Translator<'_>, host: &mut H) {
    const OP: &str = "XOR";
    debug!("op:{}", OP);
    let instruction_set = host.instruction_set();

    let mut stack_post_shift = 0;
    for _part_idx in 0..WASM_I64_IN_EVM_WORD_COUNT {
        duplicate_stack_value(instruction_set, &mut stack_post_shift, 5);
        wasm_xor(instruction_set, &mut stack_post_shift);
        assign_to_stack_and_drop(instruction_set, &mut stack_post_shift, 5);
    }
}

pub fn not<H: Host>(_translator: &mut Translator<'_>, host: &mut H) {
    const OP: &str = "NOT";
    debug!("op:{}", OP);
    let instruction_set = host.instruction_set();

    let mut stack_post_shift = 0;
    for part_idx in 0..WASM_I64_IN_EVM_WORD_COUNT {
        if part_idx > 0 {
            duplicate_stack_value(instruction_set, &mut stack_post_shift, part_idx + 1);
            wasm_not(instruction_set, &mut stack_post_shift);
            assign_to_stack_and_drop(instruction_set, &mut stack_post_shift, part_idx + 2);
        } else {
            wasm_not(instruction_set, &mut stack_post_shift);
        }
    }
}

pub fn byte<H: Host>(translator: &mut Translator<'_>, host: &mut H) {
    const OP: &str = "BYTE";
    debug!("op:{}", OP);
    let instruction_set = host.instruction_set();
    let co = translator.current_opcode();
    let instruction_set_replace = translator.get_opcode_snippet(co);
    instruction_set
        .instr
        .extend(instruction_set_replace.instr.iter());
}

pub fn shl<H: Host>(_translator: &mut Translator<'_>, _host: &mut H) {
    const OP: &str = "SHL";
    panic!("op:{} not implemented", OP);
}

pub fn shr<H: Host>(_translator: &mut Translator<'_>, _host: &mut H) {
    const OP: &str = "SHR";
    panic!("op:{} not implemented", OP);
}

pub fn sar<H: Host>(_translator: &mut Translator<'_>, _host: &mut H) {
    const OP: &str = "SAR";
    panic!("op:{} not implemented", OP);
}
