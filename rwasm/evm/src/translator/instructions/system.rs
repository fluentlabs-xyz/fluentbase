use crate::translator::host::Host;
use crate::translator::translator::Translator;

pub fn keccak256<H: Host>(_translator: &mut Translator<'_>, _host: &mut H) {
    const OP: &str = "KECCAK256";
    panic!("op:{} not implemented", OP);
}

pub fn address<H: Host>(_translator: &mut Translator<'_>, _host: &mut H) {
    const OP: &str = "ADDRESS";
    panic!("op:{} not implemented", OP);
}

pub fn caller<H: Host>(_translator: &mut Translator<'_>, _host: &mut H) {
    const OP: &str = "CALLER";
    panic!("op:{} not implemented", OP);
}

pub fn codesize<H: Host>(_translator: &mut Translator<'_>, _host: &mut H) {
    const OP: &str = "CODESIZE";
    panic!("op:{} not implemented", OP);
}

pub fn codecopy<H: Host>(_translator: &mut Translator<'_>, _host: &mut H) {
    const OP: &str = "CODECOPY";
    panic!("op:{} not implemented", OP);
}

pub fn calldataload<H: Host>(_translator: &mut Translator<'_>, _host: &mut H) {
    const OP: &str = "CALLDATALOAD";
    panic!("op:{} not implemented", OP);
}

pub fn calldatasize<H: Host>(_translator: &mut Translator<'_>, _host: &mut H) {
    const OP: &str = "CALLDATASIZE";
    panic!("op:{} not implemented", OP);
}

pub fn callvalue<H: Host>(_translator: &mut Translator<'_>, _host: &mut H) {
    const OP: &str = "CALLVALUE";
    panic!("op:{} not implemented", OP);
}

pub fn calldatacopy<H: Host>(_translator: &mut Translator<'_>, _host: &mut H) {
    const OP: &str = "CALLDATACOPY";
    panic!("op:{} not implemented", OP);
}

pub fn returndatasize<H: Host>(_translator: &mut Translator<'_>, _host: &mut H) {
    const OP: &str = "RETURNDATASIZE";
    panic!("op:{} not implemented", OP);
}

pub fn returndatacopy<H: Host>(_translator: &mut Translator<'_>, _host: &mut H) {
    const OP: &str = "RETURNDATACOPY";
    panic!("op:{} not implemented", OP);
}

pub fn gas<H: Host>(_translator: &mut Translator<'_>, _host: &mut H) {
    const OP: &str = "GAS";
    panic!("op:{} not implemented", OP);
}
