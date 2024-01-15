use crate::{
    consts::SP_BASE_MEM_OFFSET_DEFAULT,
    translator::{
        host::Host,
        instructions::utilities::{
            replace_current_opcode_with_call_to_subroutine,
            wasm_call,
            SystemFunc,
        },
        translator::Translator,
    },
    utilities::{
        load_i64_const_be,
        sp_drop_u256,
        sp_get_value,
        EVM_WORD_BYTES,
        WASM_I64_BYTES,
        WASM_I64_IN_EVM_WORD_COUNT,
    },
};
use fluentbase_rwasm::rwasm::InstructionSet;
use log::debug;

pub fn mload<H: Host>(translator: &mut Translator<'_>, host: &mut H) {
    const OP: &str = "MLOAD";
    debug!("op:{}", OP);
    replace_current_opcode_with_call_to_subroutine(translator, host);
}

pub fn mstore<H: Host>(translator: &mut Translator<'_>, host: &mut H) {
    const OP: &str = "MSTORE";
    debug!("op:{}", OP);
    replace_current_opcode_with_call_to_subroutine(translator, host);
}

pub fn mstore8<H: Host>(translator: &mut Translator<'_>, host: &mut H) {
    const OP: &str = "MSORE8";
    debug!("op:{}", OP);
    replace_current_opcode_with_call_to_subroutine(translator, host);
}

pub fn msize<H: Host>(translator: &mut Translator<'_>, host: &mut H) {
    const OP: &str = "MSIZE";
    debug!("op:{}", OP);
    replace_current_opcode_with_call_to_subroutine(translator, host);
}

pub fn mcopy<H: Host>(translator: &mut Translator<'_>, host: &mut H) {
    const OP: &str = "MCOPY";

    const OP_PARAMS_COUNT: u64 = 3;
    let is = host.instruction_set();

    for op_param_idx in 0..OP_PARAMS_COUNT {
        for u256_i64_component_idx in 0..(WASM_I64_IN_EVM_WORD_COUNT - 1) {
            let offset = op_param_idx * EVM_WORD_BYTES as u64
                + u256_i64_component_idx as u64 * WASM_I64_BYTES as u64;
            sp_get_value(is);
            if op_param_idx > 0 || u256_i64_component_idx > 0 {
                is.op_i64_const(offset);
                is.op_i64_add();
                is.op_i64_load(0);
                is.op_i64_or();
            } else {
                is.op_i64_load(0);
            }
        }
    }
    let mut aux_is = InstructionSet::new();
    wasm_call(translator, &mut aux_is, SystemFunc::SysHalt);
    aux_is.op_unreachable();
    is.op_br_if_eqz(aux_is.len() as i32 + 1);
    is.extend(&aux_is);

    for i in 0..OP_PARAMS_COUNT {
        is.op_i64_const(SP_BASE_MEM_OFFSET_DEFAULT as u64);
        sp_get_value(is);
        let offset = WASM_I64_BYTES as u64 * 3 + i * EVM_WORD_BYTES as u64;
        is.op_i64_const(offset);
        is.op_i64_sub();
        is.op_i64_sub();
        load_i64_const_be(is, None);
    }
    is.op_memory_copy();

    sp_drop_u256(is, OP_PARAMS_COUNT);
}
