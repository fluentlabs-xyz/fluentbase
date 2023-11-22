use crate::translator::host::Host;
use crate::translator::instruction_result::InstructionResult;
use crate::translator::translator::Translator;

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

pub fn ret<H: Host>(_translator: &mut Translator<'_>, _host: &mut H) {
    const OP: &str = "RET";
    panic!("op:{} not implemented", OP);
}

pub fn revert<H: Host>(_translator: &mut Translator<'_>, _host: &mut H) {
    const OP: &str = "REVERT";
    panic!("op:{} not implemented", OP);
}

pub fn stop<H: Host>(translator: &mut Translator<'_>, host: &mut H) {
    translator.instruction_result = InstructionResult::Stop;
    host.instruction_set().op_unreachable();
}

pub fn invalid<H: Host>(translator: &mut Translator<'_>, _host: &mut H) {
    translator.instruction_result = InstructionResult::InvalidFEOpcode;
}

pub fn not_found<H: Host>(translator: &mut Translator<'_>, _host: &mut H) {
    translator.instruction_result = InstructionResult::OpcodeNotFound;
}
