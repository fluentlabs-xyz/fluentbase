use crate::translator::{
    host::Host,
    instructions::utilities::replace_current_opcode_with_call_to_subroutine,
    translator::Translator,
};
use log::debug;

pub fn mload<H: Host>(translator: &mut Translator<'_>, host: &mut H) {
    const OP: &str = "MLOAD";
    debug!("op:{}", OP);
    replace_current_opcode_with_call_to_subroutine(translator, host);
}

pub fn mstore<H: Host>(translator: &mut Translator<'_>, host: &mut H) {
    const OP: &str = "MSTORE";
    debug!("op:{}", OP);
    replace_current_opcode_with_call_to_subroutine(translator, host);
}

pub fn mstore8<H: Host>(translator: &mut Translator<'_>, host: &mut H) {
    const OP: &str = "MSORE8";
    debug!("op:{}", OP);
    replace_current_opcode_with_call_to_subroutine(translator, host);
}

pub fn msize<H: Host>(translator: &mut Translator<'_>, host: &mut H) {
    const OP: &str = "MSIZE";
    debug!("op:{}", OP);
    replace_current_opcode_with_call_to_subroutine(translator, host);
}

pub fn mcopy<H: Host>(_translator: &mut Translator<'_>, _host: &mut H) {
    const OP: &str = "MCOPY";
    panic!("op:{} not implemented", OP);
}
