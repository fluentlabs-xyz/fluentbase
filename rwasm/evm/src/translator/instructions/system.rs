use crate::translator::{
    host::Host,
    instructions::utilities::{wasm_call, SystemFuncs},
    translator::Translator,
};
use fluentbase_rwasm::module::ImportName;
use log::debug;

pub fn keccak256<H: Host>(translator: &mut Translator<'_>, host: &mut H) {
    const OP: &str = "KECCAK256";
    debug!("op:{}", OP);
    let instruction_set = host.instruction_set();

    // data offset
    instruction_set.op_i64_const(4);
    // data len
    instruction_set.op_i64_const(4);
    // out offset
    instruction_set.op_i64_const(0);

    wasm_call(instruction_set, SystemFuncs::CryptoKeccak256, translator);

    // remove params from stack
    (0..8).for_each(|_| instruction_set.op_drop());
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
