use crate::translator::host::Host;
use crate::translator::translator::Translator;

pub fn balance<H: Host>(_translator: &mut Translator<'_>, _host: &mut H) {
    const OP: &str = "BALANCE";
    panic!("op:{} not implemented", OP);
}

pub fn selfbalance<H: Host>(_translator: &mut Translator<'_>, _host: &mut H) {
    const OP: &str = "SELFBALANCE";
    panic!("op:{} not implemented", OP);
}

pub fn extcodesize<H: Host>(_translator: &mut Translator<'_>, _host: &mut H) {
    const OP: &str = "EXTCODESIZE";
    panic!("op:{} not implemented", OP);
}

/// EIP-1052: EXTCODEHASH opcode
pub fn extcodehash<H: Host>(_translator: &mut Translator<'_>, _host: &mut H) {
    const OP: &str = "EXTCODEHASH";
    panic!("op:{} not implemented", OP);
}

pub fn extcodecopy<H: Host>(_translator: &mut Translator<'_>, _host: &mut H) {
    const OP: &str = "EXTCODECOPY";
    panic!("op:{} not implemented", OP);
}

pub fn blockhash<H: Host>(_translator: &mut Translator<'_>, _host: &mut H) {
    const OP: &str = "BLOCKHASH";
    panic!("op:{} not implemented", OP);
}

pub fn sload<H: Host>(_translator: &mut Translator<'_>, _host: &mut H) {
    const OP: &str = "SLOAD";
    panic!("op:{} not implemented", OP);
}

pub fn sstore<H: Host>(_translator: &mut Translator<'_>, _host: &mut H) {
    const OP: &str = "SSTORE";
    panic!("op:{} not implemented", OP);
}

pub fn tstore<H: Host>(_translator: &mut Translator<'_>, _host: &mut H) {
    const OP: &str = "TSTORE";
    panic!("op:{} not implemented", OP);
}

pub fn tload<H: Host>(_translator: &mut Translator<'_>, _host: &mut H) {
    const OP: &str = "TLOAD";
    panic!("op:{} not implemented", OP);
}

pub fn log<const N: usize, H: Host>(_translator: &mut Translator<'_>, _host: &mut H) {
    const OP: &str = "LOG";
    panic!("op:{} not implemented", OP);
}

pub fn selfdestruct<H: Host>(_translator: &mut Translator<'_>, _host: &mut H) {
    const OP: &str = "SELFDESTRUCT";
    panic!("op:{} not implemented", OP);
}

pub fn create<const IS_CREATE2: bool, H: Host>(_translator: &mut Translator<'_>, _host: &mut H) {
    const OP: &str = "CREATE";
    panic!("op:{}(IS_CREATE2:{}) not implemented", OP, IS_CREATE2);
}

pub fn call<H: Host>(_translator: &mut Translator<'_>, _host: &mut H) {
    const OP: &str = "CALL";
    panic!("op:{} not implemented", OP);
}

pub fn call_code<H: Host>(_translator: &mut Translator<'_>, _host: &mut H) {
    const OP: &str = "CALL_CODE";
    panic!("op:{} not implemented", OP);
}

pub fn delegate_call<H: Host>(_translator: &mut Translator<'_>, _host: &mut H) {
    const OP: &str = "DELEGATE_CALL";
    panic!("op:{} not implemented", OP);
}

pub fn static_call<H: Host>(_translator: &mut Translator<'_>, _host: &mut H) {
    const OP: &str = "STATIC_CALL";
    panic!("op:{} not implemented", OP);
}
