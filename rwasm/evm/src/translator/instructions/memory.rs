use crate::translator::host::Host;
use crate::translator::translator::Translator;

pub fn mload<H: Host>(translator: &mut Translator<'_>, _host: &mut H) {
    const OP: &str = "MLOAD";
    panic!("op:{} not implemented", OP);
}

pub fn mstore<H: Host>(translator: &mut Translator<'_>, _host: &mut H) {
    const OP: &str = "MSTORE";
    panic!("op:{} not implemented", OP);
}

pub fn mstore8<H: Host>(translator: &mut Translator<'_>, _host: &mut H) {
    const OP: &str = "MSORE8";
    panic!("op:{} not implemented", OP);
}

pub fn msize<H: Host>(translator: &mut Translator<'_>, _host: &mut H) {
    const OP: &str = "MSIZE";
    panic!("op:{} not implemented", OP);
}

pub fn mcopy<H: Host>(translator: &mut Translator<'_>, _host: &mut H) {
    const OP: &str = "MCOPY";
    panic!("op:{} not implemented", OP);
}
