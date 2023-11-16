use crate::translator::host::Host;
use crate::translator::translator::Translator;

/// EIP-1344: ChainID opcode
pub fn chainid<H: Host>(translator: &mut Translator<'_>, host: &mut H) {
}

pub fn coinbase<H: Host>(translator: &mut Translator<'_>, host: &mut H) {
}

pub fn timestamp<H: Host>(translator: &mut Translator<'_>, host: &mut H) {
}

pub fn number<H: Host>(translator: &mut Translator<'_>, host: &mut H) {
}

pub fn difficulty<H: Host>(translator: &mut Translator<'_>, host: &mut H) {
}

pub fn gaslimit<H: Host>(translator: &mut Translator<'_>, host: &mut H) {
}

pub fn gasprice<H: Host>(translator: &mut Translator<'_>, host: &mut H) {
}

/// EIP-3198: BASEFEE opcode
pub fn basefee<H: Host>(translator: &mut Translator<'_>, host: &mut H) {
}

pub fn origin<H: Host>(translator: &mut Translator<'_>, host: &mut H) {
}

pub fn blob_hash<H: Host>(translator: &mut Translator<'_>, host: &mut H) {
}

pub fn blob_basefee<H: Host>(translator: &mut Translator<'_>, host: &mut H) {
}
