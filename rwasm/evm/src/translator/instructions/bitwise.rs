use crate::translator::{
    host::Host,
    instructions::utilities::replace_current_opcode_with_call_to_subroutine,
    translator::Translator,
};

pub fn lt<H: Host>(translator: &mut Translator<'_>, host: &mut H) {
    const OP: &str = "LT";
    // debug!("op:{}", OP);
    replace_current_opcode_with_call_to_subroutine(translator, host);
}

pub fn gt<H: Host>(translator: &mut Translator<'_>, host: &mut H) {
    const OP: &str = "GT";
    // debug!("op:{}", OP);
    replace_current_opcode_with_call_to_subroutine(translator, host);
}

pub fn slt<H: Host>(translator: &mut Translator<'_>, host: &mut H) {
    const OP: &str = "SLT";
    // debug!("op:{}", OP);
    replace_current_opcode_with_call_to_subroutine(translator, host);
}

pub fn sgt<H: Host>(translator: &mut Translator<'_>, host: &mut H) {
    const OP: &str = "SGT";
    // debug!("op:{}", OP);
    replace_current_opcode_with_call_to_subroutine(translator, host);
}

pub fn eq<H: Host>(translator: &mut Translator<'_>, host: &mut H) {
    const OP: &str = "EQ";
    // debug!("op:{}", OP);
    replace_current_opcode_with_call_to_subroutine(translator, host);
}

pub fn iszero<H: Host>(translator: &mut Translator<'_>, host: &mut H) {
    const OP: &str = "ISZERO";
    // debug!("op:{}", OP);

    replace_current_opcode_with_call_to_subroutine(translator, host);
}

pub fn bitand<H: Host>(translator: &mut Translator<'_>, host: &mut H) {
    const OP: &str = "AND";
    // debug!("op:{}", OP);
    replace_current_opcode_with_call_to_subroutine(translator, host);
}

pub fn bitor<H: Host>(translator: &mut Translator<'_>, host: &mut H) {
    const OP: &str = "OR";
    // debug!("op:{}", OP);
    replace_current_opcode_with_call_to_subroutine(translator, host);
}

pub fn bitxor<H: Host>(translator: &mut Translator<'_>, host: &mut H) {
    const OP: &str = "XOR";
    // debug!("op:{}", OP);
    replace_current_opcode_with_call_to_subroutine(translator, host);
}

pub fn not<H: Host>(translator: &mut Translator<'_>, host: &mut H) {
    const OP: &str = "NOT";
    // debug!("op:{}", OP);
    replace_current_opcode_with_call_to_subroutine(translator, host);
}

pub fn byte<H: Host>(translator: &mut Translator<'_>, host: &mut H) {
    const OP: &str = "BYTE";
    // debug!("op:{}", OP);
    replace_current_opcode_with_call_to_subroutine(translator, host);
}

pub fn shl<H: Host>(translator: &mut Translator<'_>, host: &mut H) {
    const OP: &str = "SHL";
    // debug!("op:{}", OP);
    replace_current_opcode_with_call_to_subroutine(translator, host);
}

pub fn shr<H: Host>(translator: &mut Translator<'_>, host: &mut H) {
    const OP: &str = "SHR";
    // debug!("op:{}", OP);
    replace_current_opcode_with_call_to_subroutine(translator, host);
}

pub fn sar<H: Host>(translator: &mut Translator<'_>, host: &mut H) {
    const OP: &str = "SAR";
    // debug!("op:{}", OP);
    replace_current_opcode_with_call_to_subroutine(translator, host);
}
