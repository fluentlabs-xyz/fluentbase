use crate::translator::{host::Host, translator::Translator};
use fluentbase_rwasm::{module::ImportName, rwasm::InstructionSet};

pub(super) enum SystemFuncs {
    CryptoKeccak256,
    EvmSstore,
    EvmSload,
}

pub(super) fn wasm_call(
    instruction_set: &mut InstructionSet,
    fn_name: SystemFuncs,
    translator: &mut Translator,
) {
    let fn_name = match fn_name {
        SystemFuncs::CryptoKeccak256 => "_crypto_keccak256",
        SystemFuncs::EvmSstore => "_evm_sstore",
        SystemFuncs::EvmSload => "_evm_sload",
    };
    let import_fn_idx =
        translator.get_import_linker().index_mapping()[&ImportName::new("env", fn_name)].0;
    instruction_set.op_call(import_fn_idx);
}

pub(super) fn preprocess_op_params(translator: &mut Translator<'_>, host: &mut dyn Host) {
    let opcode = translator.opcode_prev();
    let instruction_set = host.instruction_set();
    let meta = translator
        .subroutine_data(opcode)
        .expect(&format!("no meta found for 0x{:x?} opcode", opcode));
    let prev_funcs_len = meta.begin_offset as u32;
    instruction_set.op_i32_const(instruction_set.len() + 1 - prev_funcs_len);
}

pub(super) fn replace_current_opcode_with_call_to_subroutine(
    translator: &mut Translator<'_>,
    host: &mut dyn Host,
) {
    preprocess_op_params(translator, host);

    let is = host.instruction_set();
    let op = translator.opcode_prev();
    let sd = translator
        .subroutine_data(op)
        .expect(format!("subroutine data not found for opcode 0x{:x?}", op).as_str());

    let se = sd.begin_offset as i32 - is.len() as i32 + 1/* + sd.rel_entry_offset as i32*/;
    is.op_br(se);
}
