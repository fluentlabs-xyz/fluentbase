use crate::translator::host::Host;
use crate::translator::translator::Translator;

pub fn mload<H: Host>(translator: &mut Translator<'_>, _host: &mut H) {
}

pub fn mstore<H: Host>(translator: &mut Translator<'_>, _host: &mut H) {
}

pub fn mstore8<H: Host>(translator: &mut Translator<'_>, _host: &mut H) {
}

pub fn msize<H: Host>(translator: &mut Translator<'_>, _host: &mut H) {
}

pub fn mcopy<H: Host>(translator: &mut Translator<'_>, _host: &mut H) {
}
