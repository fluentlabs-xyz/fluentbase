use crate::translator::{
    host::Host,
    instructions::utilities::replace_with_call_to_subroutine,
    translator::Translator,
};
#[cfg(test)]
use log::debug;

/// EIP-1344: ChainID opcode
pub fn chainid<H: Host>(translator: &mut Translator<'_>, host: &mut H) {
    const OP: &str = "CHAINID";
    #[cfg(test)]
    debug!("op:{}", OP);
    replace_with_call_to_subroutine(translator, host);
}

pub fn coinbase<H: Host>(translator: &mut Translator<'_>, host: &mut H) {
    const OP: &str = "COINBASE";
    #[cfg(test)]
    debug!("op:{}", OP);
    replace_with_call_to_subroutine(translator, host);
}

pub fn timestamp<H: Host>(translator: &mut Translator<'_>, host: &mut H) {
    const OP: &str = "TIMESTAMP";
    #[cfg(test)]
    debug!("op:{}", OP);
    replace_with_call_to_subroutine(translator, host);
}

pub fn number<H: Host>(translator: &mut Translator<'_>, host: &mut H) {
    const OP: &str = "NUMBER";
    #[cfg(test)]
    debug!("op:{}", OP);
    replace_with_call_to_subroutine(translator, host);
}

pub fn difficulty<H: Host>(translator: &mut Translator<'_>, host: &mut H) {
    const OP: &str = "DIFFICULTY";
    #[cfg(test)]
    debug!("op:{}", OP);
    replace_with_call_to_subroutine(translator, host);
}

pub fn gaslimit<H: Host>(translator: &mut Translator<'_>, host: &mut H) {
    const OP: &str = "GASLIMIT";
    #[cfg(test)]
    debug!("op:{}", OP);
    replace_with_call_to_subroutine(translator, host);
}

pub fn gasprice<H: Host>(translator: &mut Translator<'_>, host: &mut H) {
    const OP: &str = "GASPRICE";
    #[cfg(test)]
    debug!("op:{}", OP);
    replace_with_call_to_subroutine(translator, host);
}

pub fn basefee<H: Host>(translator: &mut Translator<'_>, host: &mut H) {
    const OP: &str = "BASEFEE";
    #[cfg(test)]
    debug!("op:{}", OP);
    replace_with_call_to_subroutine(translator, host);
}

pub fn blob_basefee<H: Host>(translator: &mut Translator<'_>, host: &mut H) {
    const OP: &str = "BLOBBASEFEE";
    #[cfg(test)]
    debug!("op:{}", OP);
    replace_with_call_to_subroutine(translator, host);
}

pub fn origin<H: Host>(translator: &mut Translator<'_>, host: &mut H) {
    const OP: &str = "ORIGIN";
    #[cfg(test)]
    debug!("op:{}", OP);
    replace_with_call_to_subroutine(translator, host);
}

pub fn blob_hash<H: Host>(translator: &mut Translator<'_>, host: &mut H) {
    const OP: &str = "BLOB_HASH";
    #[cfg(test)]
    debug!("op:{}", OP);
    replace_with_call_to_subroutine(translator, host);
}
