use crate::translator::{
    host::Host,
    instructions::utilities::replace_current_opcode_with_call_to_subroutine,
    translator::Translator,
};
use log::debug;

/// EIP-1344: ChainID opcode
pub fn chainid<H: Host>(translator: &mut Translator<'_>, host: &mut H) {
    const OP: &str = "CHAINID";
    debug!("op:{}", OP);
    replace_current_opcode_with_call_to_subroutine(translator, host);
}

pub fn coinbase<H: Host>(translator: &mut Translator<'_>, host: &mut H) {
    const OP: &str = "COINBASE";
    debug!("op:{}", OP);
    replace_current_opcode_with_call_to_subroutine(translator, host);
}

pub fn timestamp<H: Host>(translator: &mut Translator<'_>, host: &mut H) {
    const OP: &str = "TIMESTAMP";
    debug!("op:{}", OP);
    replace_current_opcode_with_call_to_subroutine(translator, host);
}

pub fn number<H: Host>(translator: &mut Translator<'_>, host: &mut H) {
    const OP: &str = "NUMBER";
    debug!("op:{}", OP);
    replace_current_opcode_with_call_to_subroutine(translator, host);
}

pub fn difficulty<H: Host>(translator: &mut Translator<'_>, host: &mut H) {
    const OP: &str = "DIFFICULTY";
    debug!("op:{}", OP);
    replace_current_opcode_with_call_to_subroutine(translator, host);
}

pub fn gaslimit<H: Host>(translator: &mut Translator<'_>, host: &mut H) {
    const OP: &str = "GASLIMIT";
    debug!("op:{}", OP);
    replace_current_opcode_with_call_to_subroutine(translator, host);
}

pub fn gasprice<H: Host>(translator: &mut Translator<'_>, host: &mut H) {
    const OP: &str = "GASPRICE";
    debug!("op:{}", OP);
    replace_current_opcode_with_call_to_subroutine(translator, host);
}

pub fn basefee<H: Host>(translator: &mut Translator<'_>, host: &mut H) {
    const OP: &str = "BASEFEE";
    debug!("op:{}", OP);
    replace_current_opcode_with_call_to_subroutine(translator, host);
}

pub fn blob_basefee<H: Host>(translator: &mut Translator<'_>, host: &mut H) {
    const OP: &str = "BLOBBASEFEE";
    debug!("op:{}", OP);
    replace_current_opcode_with_call_to_subroutine(translator, host);
}

pub fn origin<H: Host>(translator: &mut Translator<'_>, host: &mut H) {
    const OP: &str = "ORIGIN";
    debug!("op:{}", OP);
    replace_current_opcode_with_call_to_subroutine(translator, host);
}

pub fn blob_hash<H: Host>(translator: &mut Translator<'_>, host: &mut H) {
    const OP: &str = "BLOB_HASH";
    debug!("op:{}", OP);
    replace_current_opcode_with_call_to_subroutine(translator, host);
}
