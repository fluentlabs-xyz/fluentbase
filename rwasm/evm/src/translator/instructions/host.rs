use crate::translator::{
    host::Host,
    instructions::utilities::{replace_current_opcode_with_call_to_subroutine, wasm_call},
    translator::Translator,
};
use fluentbase_runtime::SysFuncIdx;

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

pub fn blockhash<H: Host>(translator: &mut Translator<'_>, host: &mut H) {
    const OP: &str = "BLOCKHASH";
    // debug!("op:{}", OP);
    replace_current_opcode_with_call_to_subroutine(translator, host);
}

pub fn sload<H: Host>(translator: &mut Translator<'_>, host: &mut H) {
    const OP: &str = "SLOAD";
    // debug!("op:{}", OP);
    replace_current_opcode_with_call_to_subroutine(translator, host);
}

pub fn sstore<H: Host>(translator: &mut Translator<'_>, host: &mut H) {
    const OP: &str = "SSTORE";
    // debug!("op:{}", OP);
    wasm_call(translator, host.instruction_set(), SysFuncIdx::ZKTRIE_STORE);
}

pub fn tstore<H: Host>(translator: &mut Translator<'_>, host: &mut H) {
    const OP: &str = "TSTORE";
    // debug!("op:{}", OP);
    replace_current_opcode_with_call_to_subroutine(translator, host);
}

pub fn tload<H: Host>(translator: &mut Translator<'_>, host: &mut H) {
    const OP: &str = "TLOAD";
    // debug!("op:{}", OP);
    replace_current_opcode_with_call_to_subroutine(translator, host);
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
