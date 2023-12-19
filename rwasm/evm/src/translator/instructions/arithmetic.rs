use crate::translator::{
    host::Host,
    instructions::utilities::replace_current_opcode_with_call_to_subroutine,
    translator::Translator,
};
use log::debug;

pub fn wrapped_add<H: Host>(translator: &mut Translator<'_>, host: &mut H) {
    const OP: &str = "ADD";
    debug!("op:{}", OP);
    // replace_current_opcode_with_inline_func(translator, host, true);
    replace_current_opcode_with_call_to_subroutine(translator, host, false, false);
}

pub fn wrapping_mul<H: Host>(translator: &mut Translator<'_>, host: &mut H) {
    const OP: &str = "MUL";
    debug!("op:{}", OP);
    // replace_current_opcode_with_inline_func(translator, host, true, false);
    replace_current_opcode_with_call_to_subroutine(translator, host, false, false);
}

pub fn wrapping_sub<H: Host>(translator: &mut Translator<'_>, host: &mut H) {
    const OP: &str = "SUB";
    debug!("op:{}", OP);
    // replace_current_opcode_with_code_snippet(translator, host, true);
    replace_current_opcode_with_call_to_subroutine(translator, host, false, false);
}

pub fn div<H: Host>(translator: &mut Translator<'_>, host: &mut H) {
    const OP: &str = "DIV";
    debug!("op:{}", OP);
    replace_current_opcode_with_call_to_subroutine(translator, host, false, false);
}

pub fn sdiv<H: Host>(translator: &mut Translator<'_>, host: &mut H) {
    const OP: &str = "SDIV";
    debug!("op:{}", OP);
    replace_current_opcode_with_call_to_subroutine(translator, host, false, false);
}

pub fn arithmetic_mod<H: Host>(translator: &mut Translator<'_>, host: &mut H) {
    const OP: &str = "MOD";
    debug!("op:{}", OP);
    replace_current_opcode_with_call_to_subroutine(translator, host, false, false);
}

pub fn smod<H: Host>(translator: &mut Translator<'_>, host: &mut H) {
    const OP: &str = "SMOD";
    debug!("op:{}", OP);
    replace_current_opcode_with_call_to_subroutine(translator, host, false, false);
}

pub fn addmod<H: Host>(translator: &mut Translator<'_>, host: &mut H) {
    const OP: &str = "ADDMOD";
    debug!("op:{}", OP);
    replace_current_opcode_with_call_to_subroutine(translator, host, false, false);
}

pub fn mulmod<H: Host>(translator: &mut Translator<'_>, host: &mut H) {
    const OP: &str = "MULMOD";
    debug!("op:{}", OP);
    replace_current_opcode_with_call_to_subroutine(translator, host, false, false);
}

pub fn exp<H: Host>(translator: &mut Translator<'_>, host: &mut H) {
    const OP: &str = "EXP";
    debug!("op:{}", OP);
    replace_current_opcode_with_call_to_subroutine(translator, host, false, false);
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
pub fn signextend<H: Host>(translator: &mut Translator<'_>, host: &mut H) {
    const OP: &str = "SIGNEXTEND";
    debug!("op:{}", OP);
    replace_current_opcode_with_call_to_subroutine(translator, host, false, false);
}
