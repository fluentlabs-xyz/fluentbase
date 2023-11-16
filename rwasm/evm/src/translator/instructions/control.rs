use crate::translator::host::Host;
use crate::translator::instruction_result::InstructionResult;
use crate::translator::translator::Translator;

pub fn jump<H: Host>(translator: &mut Translator<'_>, _host: &mut H) {}

pub fn jumpi<H: Host>(translator: &mut Translator<'_>, _host: &mut H) {}

pub fn jumpdest<H: Host>(translator: &mut Translator<'_>, _host: &mut H) {}

pub fn pc<H: Host>(translator: &mut Translator<'_>, _host: &mut H) {}

#[inline(always)]
fn return_inner(_translator: &mut Translator<'_>, result: InstructionResult) {}

pub fn ret<H: Host>(translator: &mut Translator<'_>, _host: &mut H) {}

/// EIP-140: REVERT instruction
pub fn revert<H: Host /* , SPEC: Spec */>(translator: &mut Translator<'_>, _host: &mut H) {}

pub fn stop<H: Host>(translator: &mut Translator<'_>, _host: &mut H) {
    translator.instruction_result = InstructionResult::Stop;
}

pub fn invalid<H: Host>(translator: &mut Translator<'_>, _host: &mut H) {
    translator.instruction_result = InstructionResult::InvalidFEOpcode;
}

pub fn not_found<H: Host>(translator: &mut Translator<'_>, _host: &mut H) {
    translator.instruction_result = InstructionResult::OpcodeNotFound;
}
