use crate::translator::{
    gas,
    host::Host,
    instructions::utilities::replace_with_call_to_subroutine,
    translator::Translator,
};
#[cfg(test)]
use log::debug;

pub fn lt<H: Host>(translator: &mut Translator<'_>, host: &mut H) {
    const OP: &str = "LT";
    #[cfg(test)]
    debug!("op:{}", OP);
    gas!(translator, gas::constants::VERYLOW);

    replace_with_call_to_subroutine(translator, host);
}

pub fn gt<H: Host>(translator: &mut Translator<'_>, host: &mut H) {
    const OP: &str = "GT";
    #[cfg(test)]
    debug!("op:{}", OP);
    gas!(translator, gas::constants::VERYLOW);

    replace_with_call_to_subroutine(translator, host);
}

pub fn slt<H: Host>(translator: &mut Translator<'_>, host: &mut H) {
    const OP: &str = "SLT";
    #[cfg(test)]
    debug!("op:{}", OP);
    gas!(translator, gas::constants::VERYLOW);

    replace_with_call_to_subroutine(translator, host);
}

pub fn sgt<H: Host>(translator: &mut Translator<'_>, host: &mut H) {
    const OP: &str = "SGT";
    #[cfg(test)]
    debug!("op:{}", OP);
    gas!(translator, gas::constants::VERYLOW);

    replace_with_call_to_subroutine(translator, host);
}

pub fn eq<H: Host>(translator: &mut Translator<'_>, host: &mut H) {
    const OP: &str = "EQ";
    #[cfg(test)]
    debug!("op:{}", OP);
    gas!(translator, gas::constants::VERYLOW);

    replace_with_call_to_subroutine(translator, host);
}

pub fn iszero<H: Host>(translator: &mut Translator<'_>, host: &mut H) {
    const OP: &str = "ISZERO";
    #[cfg(test)]
    debug!("op:{}", OP);
    gas!(translator, gas::constants::VERYLOW);

    replace_with_call_to_subroutine(translator, host);
}

pub fn bitand<H: Host>(translator: &mut Translator<'_>, host: &mut H) {
    const OP: &str = "AND";
    #[cfg(test)]
    debug!("op:{}", OP);
    gas!(translator, gas::constants::VERYLOW);

    replace_with_call_to_subroutine(translator, host);
}

pub fn bitor<H: Host>(translator: &mut Translator<'_>, host: &mut H) {
    const OP: &str = "OR";
    #[cfg(test)]
    debug!("op:{}", OP);
    gas!(translator, gas::constants::VERYLOW);

    replace_with_call_to_subroutine(translator, host);
}

pub fn bitxor<H: Host>(translator: &mut Translator<'_>, host: &mut H) {
    const OP: &str = "XOR";
    #[cfg(test)]
    debug!("op:{}", OP);
    gas!(translator, gas::constants::VERYLOW);

    replace_with_call_to_subroutine(translator, host);
}

pub fn not<H: Host>(translator: &mut Translator<'_>, host: &mut H) {
    const OP: &str = "NOT";
    #[cfg(test)]
    debug!("op:{}", OP);
    gas!(translator, gas::constants::VERYLOW);

    replace_with_call_to_subroutine(translator, host);
}

pub fn byte<H: Host>(translator: &mut Translator<'_>, host: &mut H) {
    const OP: &str = "BYTE";
    #[cfg(test)]
    debug!("op:{}", OP);
    gas!(translator, gas::constants::VERYLOW);

    replace_with_call_to_subroutine(translator, host);
}

pub fn shl<H: Host>(translator: &mut Translator<'_>, host: &mut H) {
    const OP: &str = "SHL";
    #[cfg(test)]
    debug!("op:{}", OP);
    gas!(translator, gas::constants::VERYLOW);

    replace_with_call_to_subroutine(translator, host);
}

pub fn shr<H: Host>(translator: &mut Translator<'_>, host: &mut H) {
    const OP: &str = "SHR";
    #[cfg(test)]
    debug!("op:{}", OP);
    gas!(translator, gas::constants::VERYLOW);

    replace_with_call_to_subroutine(translator, host);
}

pub fn sar<H: Host>(translator: &mut Translator<'_>, host: &mut H) {
    const OP: &str = "SAR";
    #[cfg(test)]
    debug!("op:{}", OP);
    gas!(translator, gas::constants::VERYLOW);

    replace_with_call_to_subroutine(translator, host);
}
