use crate::translator::{
    gas,
    host::Host,
    instruction_result::InstructionResult,
    instructions::utilities::replace_with_call_to_subroutine,
    translator::Translator,
};
use alloy_primitives::U256;
#[cfg(test)]
use log::debug;

pub fn balance<H: Host>(translator: &mut Translator<'_>, _host: &mut H) {
    const OP: &str = "BALANCE";
    if cfg!(test) {
        panic!("op:{} not implemented", OP);
    }
    return_with_reason!(translator, InstructionResult::OpcodeNotFound);
    // gas!(
    //     interpreter,
    //     if SPEC::enabled(ISTANBUL) {
    //         // EIP-1884: Repricing for trie-size-dependent opcodes
    //         gas::account_access_gas::<SPEC>(is_cold)
    //     } else if SPEC::enabled(TANGERINE) {
    //         400
    //     } else {
    //         20
    //     }
    // );
}

pub fn selfbalance<H: Host>(translator: &mut Translator<'_>, _host: &mut H) {
    const OP: &str = "SELFBALANCE";
    if cfg!(test) {
        panic!("op:{} not implemented", OP);
    }
    return_with_reason!(translator, InstructionResult::OpcodeNotFound);
    // gas!(translator, gas::constants::LOW);
}

pub fn extcodesize<H: Host>(translator: &mut Translator<'_>, _host: &mut H) {
    const OP: &str = "EXTCODESIZE";
    if cfg!(test) {
        panic!("op:{} not implemented", OP);
    }
    return_with_reason!(translator, InstructionResult::OpcodeNotFound);
    // if SPEC::enabled(BERLIN) {
    //     gas!(
    //         interpreter,
    //         if is_cold {
    //             COLD_ACCOUNT_ACCESS_COST
    //         } else {
    //             WARM_STORAGE_READ_COST
    //         }
    //     );
    // } else if SPEC::enabled(TANGERINE) {
    //     gas!(interpreter, 700);
    // } else {
    //     gas!(interpreter, 20);
    // }
}

/// EIP-1052: EXTCODEHASH opcode
pub fn extcodehash<H: Host>(translator: &mut Translator<'_>, _host: &mut H) {
    const OP: &str = "EXTCODEHASH";
    if cfg!(test) {
        panic!("op:{} not implemented", OP);
    }
    return_with_reason!(translator, InstructionResult::OpcodeNotFound);
    // if SPEC::enabled(BERLIN) {
    //     gas!(
    //         interpreter,
    //         if is_cold {
    //             COLD_ACCOUNT_ACCESS_COST
    //         } else {
    //             WARM_STORAGE_READ_COST
    //         }
    //     );
    // } else if SPEC::enabled(ISTANBUL) {
    //     gas!(interpreter, 700);
    // } else {
    //     gas!(interpreter, 400);
    // }
}

pub fn extcodecopy<H: Host>(translator: &mut Translator<'_>, _host: &mut H) {
    const OP: &str = "EXTCODECOPY";
    if cfg!(test) {
        panic!("op:{} not implemented", OP);
    }
    return_with_reason!(translator, InstructionResult::OpcodeNotFound);
    // let len = as_usize_or_fail!(interpreter, len_u256);
    // gas_or_fail!(
    //     interpreter,
    //     gas::extcodecopy_cost::<SPEC>(len as u64, is_cold)
    // );
    // if len == 0 {
    //     return;
    // }
}

pub fn blockhash<H: Host>(translator: &mut Translator<'_>, host: &mut H) {
    const OP: &str = "BLOCKHASH";
    #[cfg(test)]
    debug!("op:{}", OP);
    gas!(translator, gas::constants::BLOCKHASH);

    replace_with_call_to_subroutine(translator, host);
}

pub fn sload<H: Host>(translator: &mut Translator<'_>, host: &mut H) {
    const OP: &str = "SLOAD";
    #[cfg(test)]
    debug!("op:{}", OP);
    pop!(translator, _index);
    // const OP_PARAMS_COUNT: u64 = 1;
    // let is = translator.result_instruction_set();
    // sp_get_offset(is);
    // is.op_local_get(1); // save to the same word
    // wasm_call(translator, is, SysFuncIdx::ZKTRIE_LOAD);
    // TODO hot/cold logic
    // let Some((value, is_cold)) = host.sload(interpreter.contract.address, index) else {
    //     interpreter.instruction_result = InstructionResult::FatalExternalError;
    //     return;
    // };
    gas!(translator, gas::calc::sload_cost(/*is_cold*/));

    replace_with_call_to_subroutine(translator, host);
}

pub fn sstore<H: Host>(translator: &mut Translator<'_>, host: &mut H) {
    const OP: &str = "SSTORE";
    #[cfg(test)]
    debug!("op:{}", OP);
    // const OP_PARAMS_COUNT: u64 = 2;
    // let is = translator.result_instruction_set();
    // sp_get_offset(is, None);
    // sp_get_offset(is, Some(EVM_WORD_BYTES as i64));
    // wasm_call(translator, is, SysFuncIdx::ZKTRIE_STORE);
    // sp_drop_u256(is, OP_PARAMS_COUNT);
    pop!(translator, _index, _value);
    // TODO hot/cold logic
    // let Some((original, old, new, is_cold)) =
    //     host.sstore(interpreter.contract.address, index, value)
    //     else {
    //         interpreter.instruction_result = InstructionResult::FatalExternalError;
    //         return;
    //     };
    // gas_or_fail!(translator, {
    //     let remaining_gas = translator.gas.remaining();
    //     gas::calc::::sstore_cost(original, old, new, remaining_gas, false)
    // });
    gas::calc::sstore_cost().map(|v| gas!(translator, v));

    replace_with_call_to_subroutine(translator, host);
}

pub fn tstore<H: Host>(translator: &mut Translator<'_>, host: &mut H) {
    const OP: &str = "TSTORE";
    #[cfg(test)]
    debug!("op:{}", OP);
    gas!(translator, gas::constants::WARM_STORAGE_READ_COST);

    replace_with_call_to_subroutine(translator, host);
}

pub fn tload<H: Host>(translator: &mut Translator<'_>, host: &mut H) {
    const OP: &str = "TLOAD";
    #[cfg(test)]
    debug!("op:{}", OP);
    gas!(translator, gas::constants::WARM_STORAGE_READ_COST);

    replace_with_call_to_subroutine(translator, host);
}

pub fn log<const N: usize, H: Host>(translator: &mut Translator<'_>, _host: &mut H) {
    const OP: &str = "LOG";
    if cfg!(test) {
        panic!("op:{} not implemented", OP);
    }
    return_with_reason!(translator, InstructionResult::OpcodeNotFound);
}

pub fn selfdestruct<H: Host>(translator: &mut Translator<'_>, _host: &mut H) {
    const OP: &str = "SELFDESTRUCT";
    if cfg!(test) {
        panic!("op:{} not supported", OP);
    }
    return_with_reason!(translator, InstructionResult::OpcodeNotFound);
}

pub fn create<const IS_CREATE2: bool, H: Host>(translator: &mut Translator<'_>, host: &mut H) {
    const OP: &str = "CREATE";
    #[cfg(test)]
    debug!("op:{}", OP);
    pop!(translator, _value, _code_offset, len);
    let len = as_usize_or_fail!(translator, len);
    if IS_CREATE2 {
        pop!(translator, _salt);
        gas!(translator, gas::constants::CREATE);
    } else {
        gas::calc::create2_cost(len).map(|gas| gas!(translator, gas));
    }

    replace_with_call_to_subroutine(translator, host);
}

pub fn call<H: Host>(translator: &mut Translator<'_>, host: &mut H) {
    const OP: &str = "CALL";
    #[cfg(test)]
    debug!("op:{}", OP);
    pop!(translator, _local_gas_limit, _to);
    pop!(translator, value);
    pop!(translator, in_offset, in_len, out_offset, out_len);
    as_usize_or_fail!(translator, in_offset);
    as_usize_or_fail!(translator, in_len);
    as_usize_or_fail!(translator, out_offset);
    as_usize_or_fail!(translator, out_len);
    gas!(
        translator,
        gas::calc::call_cost(value, false, false, true, true,)
    );

    replace_with_call_to_subroutine(translator, host);
}

pub fn call_code<H: Host>(translator: &mut Translator<'_>, _host: &mut H) {
    const OP: &str = "CALL_CODE";
    if cfg!(test) {
        panic!("op:{} not supported", OP);
    }
    return_with_reason!(translator, InstructionResult::OpcodeNotFound);
}

pub fn delegate_call<H: Host>(translator: &mut Translator<'_>, host: &mut H) {
    const OP: &str = "DELEGATE_CALL";
    #[cfg(test)]
    debug!("op:{}", OP);
    pop!(translator, _local_gas_limit, _to);
    pop!(translator, in_offset, in_len, out_offset, out_len);
    as_usize_or_fail!(translator, in_offset);
    as_usize_or_fail!(translator, in_len);
    as_usize_or_fail!(translator, out_offset);
    as_usize_or_fail!(translator, out_len);
    gas!(
        translator,
        gas::calc::call_cost(U256::ZERO, false, false, false, false)
    );

    replace_with_call_to_subroutine(translator, host);
}

pub fn static_call<H: Host>(translator: &mut Translator<'_>, host: &mut H) {
    const OP: &str = "STATIC_CALL";
    #[cfg(test)]
    debug!("op:{}", OP);
    pop!(translator, _local_gas_limit, _to);
    pop!(translator, in_offset, in_len, out_offset, out_len);
    as_usize_or_fail!(translator, in_offset);
    as_usize_or_fail!(translator, in_len);
    as_usize_or_fail!(translator, out_offset);
    as_usize_or_fail!(translator, out_len);
    gas!(
        translator,
        gas::calc::call_cost(U256::ZERO, false, false, false, true)
    );

    replace_with_call_to_subroutine(translator, host);
}
