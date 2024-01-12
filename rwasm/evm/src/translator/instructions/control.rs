use crate::translator::{
    host::Host,
    instruction_result::InstructionResult,
    instructions::utilities::{
        replace_current_opcode_with_call_to_subroutine,
        wasm_call,
        SystemFuncs,
    },
    translator::Translator,
};
use log::debug;

pub fn jump<H: Host>(_translator: &mut Translator<'_>, _host: &mut H) {
    const OP: &str = "JUMP";
    panic!("op:{} not implemented", OP);
}

pub fn jumpi<H: Host>(_translator: &mut Translator<'_>, _host: &mut H) {
    const OP: &str = "JUMPI";
    panic!("op:{} not implemented", OP);
}

pub fn jumpdest<H: Host>(_translator: &mut Translator<'_>, _host: &mut H) {
    const OP: &str = "JUMPDEST";
    panic!("op:{} not implemented", OP);
}

pub fn pc<H: Host>(_translator: &mut Translator<'_>, _host: &mut H) {
    const OP: &str = "PC";
    panic!("op:{} not implemented", OP);
}

pub fn ret<H: Host>(translator: &mut Translator<'_>, host: &mut H) {
    const OP: &str = "RET";
    debug!("op:{}", OP);
    replace_current_opcode_with_call_to_subroutine(translator, host);
}

pub fn revert<H: Host>(translator: &mut Translator<'_>, host: &mut H) {
    const OP: &str = "REVERT";
    debug!("op:{}", OP);
    replace_current_opcode_with_call_to_subroutine(translator, host);
}

pub fn stop<H: Host>(translator: &mut Translator<'_>, host: &mut H) {
    translator.instruction_result = InstructionResult::Stop;
    let is = host.instruction_set();
    is.op_return();
    is.op_unreachable();
}

pub fn invalid<H: Host>(translator: &mut Translator<'_>, host: &mut H) {
    translator.instruction_result = InstructionResult::InvalidFEOpcode;
    wasm_call(host.instruction_set(), SystemFuncs::SysHalt, translator)
}

pub fn not_found<H: Host>(translator: &mut Translator<'_>, _host: &mut H) {
    translator.instruction_result = InstructionResult::OpcodeNotFound;
}
