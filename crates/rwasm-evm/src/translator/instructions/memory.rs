use crate::{
    consts::SP_BASE_MEM_OFFSET_DEFAULT,
    translator::{
        gas,
        host::Host,
        instructions::utilities::{replace_with_call_to_subroutine, wasm_call},
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
use fluentbase_types::{ExitCode, SysFuncIdx};
#[cfg(test)]
use log::debug;
use rwasm_codegen::InstructionSet;

pub fn mload<H: Host>(translator: &mut Translator<'_>, host: &mut H) {
    const OP: &str = "MLOAD";
    #[cfg(test)]
    debug!("op:{}", OP);
    gas!(translator, gas::constants::VERYLOW);

    replace_with_call_to_subroutine(translator, host);
}

pub fn mstore<H: Host>(translator: &mut Translator<'_>, host: &mut H) {
    const OP: &str = "MSTORE";
    #[cfg(test)]
    debug!("op:{}", OP);
    gas!(translator, gas::constants::VERYLOW);

    replace_with_call_to_subroutine(translator, host);
}

pub fn mstore8<H: Host>(translator: &mut Translator<'_>, host: &mut H) {
    const OP: &str = "MSTORE8";
    #[cfg(test)]
    debug!("op:{}", OP);
    gas!(translator, gas::constants::VERYLOW);

    replace_with_call_to_subroutine(translator, host);
}

pub fn msize<H: Host>(translator: &mut Translator<'_>, host: &mut H) {
    const OP: &str = "MSIZE";
    #[cfg(test)]
    debug!("op:{}", OP);
    gas!(translator, gas::constants::BASE);

    replace_with_call_to_subroutine(translator, host);
}

pub fn mcopy<H: Host>(translator: &mut Translator<'_>, _host: &mut H) {
    const OP: &str = "MCOPY";
    #[cfg(test)]
    debug!("op:{}", OP);
    const OP_PARAMS_COUNT: usize = 3;
    pop!(translator, _dst, _src, len);
    // into usize or fail
    let len = as_usize_or_fail!(translator, len);
    // deduce gas
    gas_or_fail!(translator, gas::calc::verylowcopy_cost(len as u32));
    let is = translator.result_instruction_set_mut();
    if len == 0 {
        sp_drop_u256(is, OP_PARAMS_COUNT as u64);
        return;
    }

    {
        for op_param_idx in 0..OP_PARAMS_COUNT {
            for u256_i64_component_idx in 0..(WASM_I64_IN_EVM_WORD_COUNT - 1) {
                let offset =
                    op_param_idx * EVM_WORD_BYTES + u256_i64_component_idx * WASM_I64_BYTES;
                sp_get_value(is, None);
                if op_param_idx > 0 || u256_i64_component_idx > 0 {
                    is.op_i64_const(offset as u64);
                    is.op_i64_add();
                    is.op_i64_load(0);
                    is.op_i64_or();
                } else {
                    is.op_i64_load(0);
                }
            }
        }
    }

    let mut aux_is = InstructionSet::new();
    aux_is.op_i32_const(ExitCode::UnknownError as i32);
    wasm_call(translator, Some(&mut aux_is), SysFuncIdx::SYS_HALT);
    aux_is.op_unreachable();

    let is = translator.result_instruction_set_mut();
    is.op_br_if_eqz(aux_is.len() as i32 + 1);
    is.extend(&aux_is);

    for i in 0..OP_PARAMS_COUNT {
        is.op_i64_const(SP_BASE_MEM_OFFSET_DEFAULT as u64);
        sp_get_value(
            is,
            Some(-((WASM_I64_BYTES * 3 + i * EVM_WORD_BYTES) as i64)),
        );
        is.op_i64_sub();
        load_i64_const_be(is, None);
    }
    is.op_memory_copy();

    sp_drop_u256(is, OP_PARAMS_COUNT as u64);
}
