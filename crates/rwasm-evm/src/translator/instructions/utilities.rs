use crate::translator::{host::Host, translator::Translator};
use alloc::format;
use fluentbase_types::SysFuncIdx;
use rwasm_codegen::InstructionSet;

pub fn wasm_call(
    translator: &mut Translator,
    is_aux: Option<&mut InstructionSet>,
    sys_func_idx: SysFuncIdx,
) -> u64 {
    let _ = translator
        .get_import_linker()
        .resolve_by_index(sys_func_idx as u32)
        .expect(&format!("can't find import function ({:?})", sys_func_idx));
    let index = sys_func_idx as u32;

    let is = is_aux.unwrap_or(translator.result_instruction_set_mut());
    let mut ops_count = is.len() as u64;
    is.op_call(index);

    ops_count = is.len() as u64 - ops_count;
    ops_count
}

pub(super) fn preprocess_op_params(translator: &mut Translator<'_>, host: &mut dyn Host) {
    let opcode = translator.instruction_prev();
    let prev_funcs_len = translator
        .subroutine_data(opcode as u32)
        .expect(&format!("no meta found for 0x{:x?} opcode", opcode))
        .begin_offset as u32;
    let instruction_set = translator.result_instruction_set_mut();
    let is_len = instruction_set.len();
    let return_offset = is_len - prev_funcs_len;
    instruction_set.op_i32_const(return_offset);
}

pub(super) fn replace_with_call_to_subroutine(
    translator: &mut Translator<'_>,
    host: &mut dyn Host,
) {
    preprocess_op_params(translator, host);

    let opcode = translator.instruction_prev();
    let is_len = translator.result_instruction_set_mut().len();

    let sd = translator
        .subroutine_data(opcode as u32)
        .expect(format!("subroutine data not found for opcode 0x{:x?}", opcode).as_str());

    let se = sd.begin_offset as i32 - is_len as i32 + 2 + sd.rel_entry_offset as i32;
    let is = translator.result_instruction_set_mut();
    is.op_br(se);
}
