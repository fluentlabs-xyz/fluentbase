use crate::{
    translator::{
        gas,
        host::Host,
        instruction_result::InstructionResult,
        instructions::utilities::replace_with_call_to_subroutine,
        translator::Translator,
    },
    utilities::sp_drop_u256,
};
#[cfg(test)]
use log::debug;

pub fn keccak256<H: Host>(translator: &mut Translator<'_>, host: &mut H) {
    const OP: &str = "KECCAK256";
    #[cfg(test)]
    debug!("op:{}", OP);
    pop!(translator, _from, len);
    let len = as_usize_or_fail!(translator, len);
    gas_or_fail!(translator, gas::calc::keccak256_cost(len as u32));

    replace_with_call_to_subroutine(translator, host);
}

pub fn address<H: Host>(translator: &mut Translator<'_>, host: &mut H) {
    const OP: &str = "ADDRESS";
    #[cfg(test)]
    debug!("op:{}", OP);
    gas!(translator, gas::constants::BASE);

    replace_with_call_to_subroutine(translator, host);
}

pub fn caller<H: Host>(translator: &mut Translator<'_>, host: &mut H) {
    const OP: &str = "CALLER";
    #[cfg(test)]
    debug!("op:{}", OP);
    gas!(translator, gas::constants::BASE);

    replace_with_call_to_subroutine(translator, host);
}

pub fn codesize<H: Host>(translator: &mut Translator<'_>, host: &mut H) {
    const OP: &str = "CODESIZE";
    #[cfg(test)]
    debug!("op:{}", OP);
    gas!(translator, gas::constants::BASE);

    replace_with_call_to_subroutine(translator, host);
}

pub fn codecopy<H: Host>(translator: &mut Translator<'_>, host: &mut H) {
    const OP: &str = "CODECOPY";
    #[cfg(test)]
    debug!("op:{}", OP);
    pop!(translator, _memory_offset, _code_offset, len);
    let len = as_usize_or_fail!(translator, len);
    gas_or_fail!(translator, gas::calc::verylowcopy_cost(len as u32));
    if len == 0 {
        return;
    }

    replace_with_call_to_subroutine(translator, host);
}

pub fn calldataload<H: Host>(translator: &mut Translator<'_>, host: &mut H) {
    const OP: &str = "CALLDATALOAD";
    #[cfg(test)]
    debug!("op:{}", OP);
    gas!(translator, gas::constants::VERYLOW);

    replace_with_call_to_subroutine(translator, host);
}

pub fn calldatasize<H: Host>(translator: &mut Translator<'_>, host: &mut H) {
    const OP: &str = "CALLDATASIZE";
    #[cfg(test)]
    debug!("op:{}", OP);
    gas!(translator, gas::constants::BASE);

    replace_with_call_to_subroutine(translator, host);
}

pub fn callvalue<H: Host>(translator: &mut Translator<'_>, host: &mut H) {
    const OP: &str = "CALLVALUE";
    #[cfg(test)]
    debug!("op:{}", OP);
    gas!(translator, gas::constants::BASE);

    replace_with_call_to_subroutine(translator, host);
}

pub fn calldatacopy<H: Host>(translator: &mut Translator<'_>, host: &mut H) {
    const OP: &str = "CALLDATACOPY";
    #[cfg(test)]
    debug!("op:{}", OP);
    const OP_PARAMS_COUNT: usize = 3;
    pop!(translator, _memory_offset, _data_offset, len);
    let len = as_usize_or_fail!(translator, len);
    gas_or_fail!(translator, gas::calc::verylowcopy_cost(len as u32));
    if len == 0 {
        sp_drop_u256(
            translator.result_instruction_set_mut(),
            OP_PARAMS_COUNT as u64,
        );
        return;
    }

    replace_with_call_to_subroutine(translator, host);
}

pub fn returndatasize<H: Host>(translator: &mut Translator<'_>, _host: &mut H) {
    const OP: &str = "RETURNDATASIZE";
    if cfg!(test) {
        panic!("op:{} not implemented", OP);
    }
    return_with_reason!(translator, InstructionResult::OpcodeNotFound);

    // gas!(translator, gas::constants::BASE);
}

pub fn returndatacopy<H: Host>(translator: &mut Translator<'_>, _host: &mut H) {
    const OP: &str = "RETURNDATACOPY";
    if cfg!(test) {
        panic!("op:{} not implemented", OP);
    }
    return_with_reason!(translator, InstructionResult::OpcodeNotFound);
}

pub fn gas<H: Host>(translator: &mut Translator<'_>, host: &mut H) {
    const OP: &str = "GAS";
    #[cfg(test)]
    debug!("op:{}", OP);
    gas!(translator, gas::constants::BASE);

    replace_with_call_to_subroutine(translator, host);
}
