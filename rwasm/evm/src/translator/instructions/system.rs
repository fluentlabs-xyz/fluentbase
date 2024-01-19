use crate::translator::{
    gas,
    host::Host,
    instructions::utilities::replace_with_call_to_subroutine,
    translator::Translator,
};
#[cfg(test)]
use log::debug;

pub fn keccak256<H: Host>(translator: &mut Translator<'_>, host: &mut H) {
    const OP: &str = "KECCAK256";
    #[cfg(test)]
    debug!("op:{}", OP);
    pop!(translator, from, len);
    let len = as_usize_or_fail!(translator, len);
    gas_or_fail!(translator, gas::calc::keccak256_cost(len as u32));

    replace_with_call_to_subroutine(translator, host);
}

pub fn address<H: Host>(translator: &mut Translator<'_>, host: &mut H) {
    const OP: &str = "ADDRESS";
    #[cfg(test)]
    debug!("op:{}", OP);
    let is = translator.result_instruction_set_mut();
    gas!(translator, gas::constants::BASE);

    replace_with_call_to_subroutine(translator, host);
}

pub fn caller<H: Host>(translator: &mut Translator<'_>, host: &mut H) {
    const OP: &str = "CALLER";
    #[cfg(test)]
    debug!("op:{}", OP);
    let is = translator.result_instruction_set_mut();
    gas!(translator, gas::constants::BASE);

    replace_with_call_to_subroutine(translator, host);
}

pub fn codesize<H: Host>(translator: &mut Translator<'_>, host: &mut H) {
    const OP: &str = "CODESIZE";
    #[cfg(test)]
    debug!("op:{}", OP);
    let is = translator.result_instruction_set_mut();
    gas!(translator, gas::constants::BASE);

    replace_with_call_to_subroutine(translator, host);
}

pub fn codecopy<H: Host>(translator: &mut Translator<'_>, _host: &mut H) {
    const OP: &str = "CODECOPY";
    panic!("op:{} not implemented", OP);
    // TODO gas (not implemented opcode)
}

pub fn calldataload<H: Host>(translator: &mut Translator<'_>, host: &mut H) {
    const OP: &str = "CALLDATALOAD";
    #[cfg(test)]
    debug!("op:{}", OP);
    let is = translator.result_instruction_set_mut();
    gas!(translator, gas::constants::VERYLOW);

    replace_with_call_to_subroutine(translator, host);
}

pub fn calldatasize<H: Host>(translator: &mut Translator<'_>, host: &mut H) {
    const OP: &str = "CALLDATASIZE";
    #[cfg(test)]
    debug!("op:{}", OP);
    let is = translator.result_instruction_set_mut();
    gas!(translator, gas::constants::BASE);

    replace_with_call_to_subroutine(translator, host);
}

pub fn callvalue<H: Host>(translator: &mut Translator<'_>, host: &mut H) {
    const OP: &str = "CALLVALUE";
    #[cfg(test)]
    debug!("op:{}", OP);
    let is = translator.result_instruction_set_mut();
    gas!(translator, gas::constants::BASE);

    replace_with_call_to_subroutine(translator, host);
}

pub fn calldatacopy<H: Host>(translator: &mut Translator<'_>, host: &mut H) {
    const OP: &str = "CALLDATACOPY";
    #[cfg(test)]
    debug!("op:{}", OP);
    pop!(translator, memory_offset, data_offset, len);
    let len = as_usize_or_fail!(translator, len);
    gas_or_fail!(translator, gas::calc::verylowcopy_cost(len as u32));
    // if len == 0 { // TODO just drop params on stack?
    //     return;
    // }

    replace_with_call_to_subroutine(translator, host);
}

pub fn returndatasize<H: Host>(translator: &mut Translator<'_>, host: &mut H) {
    const OP: &str = "RETURNDATASIZE";
    panic!("op:{} not implemented", OP);
    let is = translator.result_instruction_set_mut();
    gas!(translator, gas::constants::BASE);
}

pub fn returndatacopy<H: Host>(translator: &mut Translator<'_>, _host: &mut H) {
    const OP: &str = "RETURNDATACOPY";
    panic!("op:{} not implemented", OP);
}

pub fn gas<H: Host>(translator: &mut Translator<'_>, host: &mut H) {
    const OP: &str = "GAS";
    #[cfg(test)]
    debug!("op:{}", OP);
    gas!(translator, gas::constants::BASE);

    replace_with_call_to_subroutine(translator, host);
}
