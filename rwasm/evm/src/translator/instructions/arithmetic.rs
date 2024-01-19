use crate::translator::{
    gas,
    host::Host,
    instructions::utilities::replace_with_call_to_subroutine,
    translator::Translator,
};
#[cfg(test)]
use log::debug;

pub fn wrapped_add<H: Host>(translator: &mut Translator<'_>, host: &mut H) {
    const OP: &str = "ADD";
    #[cfg(test)]
    debug!("op:{}", OP);
    gas!(translator, gas::constants::VERYLOW);

    replace_with_call_to_subroutine(translator, host);
}

pub fn wrapping_mul<H: Host>(translator: &mut Translator<'_>, host: &mut H) {
    const OP: &str = "MUL";
    #[cfg(test)]
    debug!("op:{}", OP);
    let is = translator.result_instruction_set_mut();
    gas!(translator, gas::constants::LOW);

    replace_with_call_to_subroutine(translator, host);
}

pub fn wrapping_sub<H: Host>(translator: &mut Translator<'_>, host: &mut H) {
    const OP: &str = "SUB";
    #[cfg(test)]
    debug!("op:{}", OP);
    let is = translator.result_instruction_set_mut();
    gas!(translator, gas::constants::VERYLOW);

    replace_with_call_to_subroutine(translator, host);
}

pub fn div<H: Host>(translator: &mut Translator<'_>, host: &mut H) {
    const OP: &str = "DIV";
    #[cfg(test)]
    debug!("op:{}", OP);
    let is = translator.result_instruction_set_mut();
    gas!(translator, gas::constants::LOW);

    replace_with_call_to_subroutine(translator, host);
}

pub fn sdiv<H: Host>(translator: &mut Translator<'_>, host: &mut H) {
    const OP: &str = "SDIV";
    #[cfg(test)]
    debug!("op:{}", OP);
    let is = translator.result_instruction_set_mut();
    gas!(translator, gas::constants::LOW);

    replace_with_call_to_subroutine(translator, host);
}

pub fn rem<H: Host>(translator: &mut Translator<'_>, host: &mut H) {
    const OP: &str = "MOD";
    #[cfg(test)]
    debug!("op:{}", OP);
    let is = translator.result_instruction_set_mut();
    gas!(translator, gas::constants::LOW);

    replace_with_call_to_subroutine(translator, host);
}

pub fn smod<H: Host>(translator: &mut Translator<'_>, host: &mut H) {
    const OP: &str = "SMOD";
    #[cfg(test)]
    debug!("op:{}", OP);
    let is = translator.result_instruction_set_mut();
    gas!(translator, gas::constants::LOW);

    replace_with_call_to_subroutine(translator, host);
}

pub fn addmod<H: Host>(translator: &mut Translator<'_>, host: &mut H) {
    const OP: &str = "ADDMOD";
    #[cfg(test)]
    debug!("op:{}", OP);
    let is = translator.result_instruction_set_mut();
    gas!(translator, gas::constants::MID);

    replace_with_call_to_subroutine(translator, host);
}

pub fn mulmod<H: Host>(translator: &mut Translator<'_>, host: &mut H) {
    const OP: &str = "MULMOD";
    #[cfg(test)]
    debug!("op:{}", OP);
    let is = translator.result_instruction_set_mut();
    gas!(translator, gas::constants::MID);

    replace_with_call_to_subroutine(translator, host);
}

pub fn exp<H: Host>(translator: &mut Translator<'_>, host: &mut H) {
    const OP: &str = "EXP";
    #[cfg(test)]
    debug!("op:{}", OP);
    let is = translator.result_instruction_set_mut();
    gas!(translator, 50);

    replace_with_call_to_subroutine(translator, host);
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
    #[cfg(test)]
    debug!("op:{}", OP);
    let is = translator.result_instruction_set_mut();
    gas!(translator, gas::constants::LOW);

    replace_with_call_to_subroutine(translator, host);
}
