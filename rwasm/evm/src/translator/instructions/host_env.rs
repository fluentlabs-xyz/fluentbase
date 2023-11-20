use crate::translator::host::Host;
use crate::translator::translator::Translator;

/// EIP-1344: ChainID opcode
pub fn chainid<H: Host>(_translator: &mut Translator<'_>, _host: &mut H) {
    const OP: &str = "CHAINID";
    panic!("op:{} not implemented", OP);
}

pub fn coinbase<H: Host>(_translator: &mut Translator<'_>, _host: &mut H) {
    const OP: &str = "COINBASE";
    panic!("op:{} not implemented", OP);
}

pub fn timestamp<H: Host>(_translator: &mut Translator<'_>, _host: &mut H) {
    const OP: &str = "TIMESTAMP";
    panic!("op:{} not implemented", OP);
}

pub fn number<H: Host>(_translator: &mut Translator<'_>, _host: &mut H) {
    const OP: &str = "NUMBER";
    panic!("op:{} not implemented", OP);
}

pub fn difficulty<H: Host>(_translator: &mut Translator<'_>, _host: &mut H) {
    const OP: &str = "DIFFICULTY";
    panic!("op:{} not implemented", OP);
}

pub fn gaslimit<H: Host>(_translator: &mut Translator<'_>, _host: &mut H) {
    const OP: &str = "GASLIMIT";
    panic!("op:{} not implemented", OP);
}

pub fn gasprice<H: Host>(_translator: &mut Translator<'_>, _host: &mut H) {
    const OP: &str = "GASPRICE";
    panic!("op:{} not implemented", OP);
}

pub fn basefee<H: Host>(_translator: &mut Translator<'_>, _host: &mut H) {
    const OP: &str = "BASEFEE";
    panic!("op:{} not implemented", OP);
}

pub fn origin<H: Host>(_translator: &mut Translator<'_>, _host: &mut H) {
    const OP: &str = "ORIGIN";
    panic!("op:{} not implemented", OP);
}

pub fn blob_hash<H: Host>(_translator: &mut Translator<'_>, _host: &mut H) {
    const OP: &str = "BLOB_HASH";
    panic!("op:{} not implemented", OP);
}

pub fn blob_basefee<H: Host>(_translator: &mut Translator<'_>, _host: &mut H) {
    const OP: &str = "BLOB_BASEFEE";
    panic!("op:{} not implemented", OP);
}
