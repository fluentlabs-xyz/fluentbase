use crate::{
    translator::{
        host::Host,
        instruction_result::InstructionResult,
        instructions::{
            opcode::JUMP,
            utilities::{replace_current_opcode_with_call_to_subroutine, wasm_call, SystemFunc},
        },
        translator::Translator,
    },
    utilities::sp_drop_u256,
};
use log::debug;

pub fn jump<H: Host>(translator: &mut Translator<'_>, host: &mut H) {
    const OP: &str = "JUMP";
    debug!("op:{}", OP);
    const OP_PARAMS_COUNT: u64 = 1;

    let pc_from = translator.program_counter() - 1;
    let pc_to_slice = translator.get_bytecode_slice(Some(-9), 8);
    let pc_to = usize::from_be_bytes(pc_to_slice.try_into().unwrap());
    translator.jumps_to_process_add(pc_from, pc_to);
    let is = host.instruction_set();

    sp_drop_u256(is, OP_PARAMS_COUNT);

    let is_current_offset = is.len() as i64;
    let br_indirect_offset = 0; // by default: just skips itself
    is.op_i64_const(is_current_offset + br_indirect_offset);
    is.op_br_indirect(2); // for const and br_indirect itself
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
    wasm_call(translator, host.instruction_set(), SystemFunc::SysHalt);
}

pub fn not_found<H: Host>(translator: &mut Translator<'_>, _host: &mut H) {
    translator.instruction_result = InstructionResult::OpcodeNotFound;
}
