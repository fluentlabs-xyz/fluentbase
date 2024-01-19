use crate::{
    translator::{
        gas,
        host::Host,
        instructions::utilities::{replace_with_call_to_subroutine, wasm_call},
        translator::Translator,
    },
    utilities::{sp_drop_u256, sp_get_offset, EVM_WORD_BYTES},
};
use fluentbase_runtime::SysFuncIdx;
#[cfg(test)]
use log::debug;

pub fn balance<H: Host>(translator: &mut Translator<'_>, _host: &mut H) {
    const OP: &str = "BALANCE";
    panic!("op:{} not implemented", OP);
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

pub fn selfbalance<H: Host>(translator: &mut Translator<'_>, host: &mut H) {
    const OP: &str = "SELFBALANCE";
    panic!("op:{} not implemented", OP);
    let is = translator.result_instruction_set_mut();
    gas!(translator, gas::constants::LOW);
}

pub fn extcodesize<H: Host>(translator: &mut Translator<'_>, _host: &mut H) {
    const OP: &str = "EXTCODESIZE";
    panic!("op:{} not implemented", OP);
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
    panic!("op:{} not implemented", OP);
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
    panic!("op:{} not implemented", OP);
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
    let is = translator.result_instruction_set_mut();
    gas!(translator, gas::constants::BLOCKHASH);

    replace_with_call_to_subroutine(translator, host);
}

pub fn sload<H: Host>(translator: &mut Translator<'_>, host: &mut H) {
    const OP: &str = "SLOAD";
    #[cfg(test)]
    debug!("op:{}", OP);
    pop!(translator, index);
    // const OP_PARAMS_COUNT: u64 = 1;
    // let is = translator.result_instruction_set();
    // sp_get_offset(is);
    // is.op_local_get(1); // save to the same word
    // wasm_call(translator, is, SysFuncIdx::ZKTRIE_LOAD);
    // TODO what todo with hot/cold logic
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
    // TODO gas (think)
    pop!(translator, index, value);
    // TODO what todo with hot/cold logic
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

    replace_with_call_to_subroutine(translator, host);
}

pub fn tstore<H: Host>(translator: &mut Translator<'_>, host: &mut H) {
    const OP: &str = "TSTORE";
    #[cfg(test)]
    debug!("op:{}", OP);
    let is = translator.result_instruction_set_mut();
    gas!(translator, gas::constants::WARM_STORAGE_READ_COST);

    replace_with_call_to_subroutine(translator, host);
}

pub fn tload<H: Host>(translator: &mut Translator<'_>, host: &mut H) {
    const OP: &str = "TLOAD";
    #[cfg(test)]
    debug!("op:{}", OP);
    let is = translator.result_instruction_set_mut();
    gas!(translator, gas::constants::WARM_STORAGE_READ_COST);

    replace_with_call_to_subroutine(translator, host);
}

pub fn log<const N: usize, H: Host>(translator: &mut Translator<'_>, _host: &mut H) {
    const OP: &str = "LOG";
    panic!("op:{} not implemented", OP);
}

pub fn selfdestruct<H: Host>(translator: &mut Translator<'_>, _host: &mut H) {
    const OP: &str = "SELFDESTRUCT";
    panic!("op:{} not implemented", OP);
}

pub fn create<const IS_CREATE2: bool, H: Host>(translator: &mut Translator<'_>, _host: &mut H) {
    const OP: &str = "CREATE";
    panic!("op:{}(IS_CREATE2:{}) not implemented", OP, IS_CREATE2);
}

pub fn call<H: Host>(translator: &mut Translator<'_>, _host: &mut H) {
    const OP: &str = "CALL";
    panic!("op:{} not implemented", OP);
}

pub fn call_code<H: Host>(translator: &mut Translator<'_>, _host: &mut H) {
    const OP: &str = "CALL_CODE";
    panic!("op:{} not implemented", OP);
}

pub fn delegate_call<H: Host>(translator: &mut Translator<'_>, _host: &mut H) {
    const OP: &str = "DELEGATE_CALL";
    panic!("op:{} not implemented", OP);
}

pub fn static_call<H: Host>(translator: &mut Translator<'_>, _host: &mut H) {
    const OP: &str = "STATIC_CALL";
    panic!("op:{} not implemented", OP);
}
