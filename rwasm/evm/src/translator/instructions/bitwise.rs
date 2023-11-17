use log::debug;

use crate::translator::host::Host;
use crate::translator::instructions::utilities::{
    assign_to_stack_and_drop, duplicate_stack_value, wasm_and, wasm_not, wasm_or, wasm_xor,
};
use crate::translator::translator::Translator;
use crate::utilities::WASM_I64_IN_EVM_WORD_COUNT;

pub fn lt<H: Host>(_translator: &mut Translator<'_>, _host: &mut H) {
    debug!("op:LT");
}

pub fn gt<H: Host>(_translator: &mut Translator<'_>, _host: &mut H) {
    debug!("op:GT");
}

pub fn slt<H: Host>(_translator: &mut Translator<'_>, _host: &mut H) {
    debug!("op:SLT");
}

pub fn sgt<H: Host>(_translator: &mut Translator<'_>, _host: &mut H) {
    debug!("op:SGT");
}

pub fn eq<H: Host>(_translator: &mut Translator<'_>, _host: &mut H) {
    debug!("op:EQ");
}

pub fn iszero<H: Host>(_translator: &mut Translator<'_>, host: &mut H) {
    debug!("op:ISZERO");
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
    debug!("op:AND");
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
    debug!("op:OR");
    let instruction_set = host.instruction_set();

    let mut stack_post_shift = 0;
    for _part_idx in 0..WASM_I64_IN_EVM_WORD_COUNT {
        duplicate_stack_value(instruction_set, &mut stack_post_shift, 5);
        wasm_or(instruction_set, &mut stack_post_shift);
        assign_to_stack_and_drop(instruction_set, &mut stack_post_shift, 5);
    }
}

pub fn bitxor<H: Host>(_translator: &mut Translator<'_>, host: &mut H) {
    debug!("op:XOR");
    let instruction_set = host.instruction_set();

    let mut stack_post_shift = 0;
    for _part_idx in 0..WASM_I64_IN_EVM_WORD_COUNT {
        duplicate_stack_value(instruction_set, &mut stack_post_shift, 5);
        wasm_xor(instruction_set, &mut stack_post_shift);
        assign_to_stack_and_drop(instruction_set, &mut stack_post_shift, 5);
    }
}

pub fn not<H: Host>(_translator: &mut Translator<'_>, host: &mut H) {
    debug!("op:NOT");
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

pub fn byte<H: Host>(_translator: &mut Translator<'_>, _host: &mut H) {
    debug!("op:BYTE");
}

pub fn shl<H: Host>(_translator: &mut Translator<'_>, _host: &mut H) {
    debug!("op:SHL");
}

pub fn shr<H: Host>(_translator: &mut Translator<'_>, _host: &mut H) {
    debug!("op:SHR");
}

pub fn sar<H: Host>(_translator: &mut Translator<'_>, _host: &mut H) {
    debug!("op:SAR");
}
