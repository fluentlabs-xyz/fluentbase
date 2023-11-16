use crate::translator::host::Host;
use crate::translator::translator::Translator;

pub fn keccak256<H: Host>(translator: &mut Translator<'_>, _host: &mut H) {}

pub fn address<H: Host>(translator: &mut Translator<'_>, _host: &mut H) {}

pub fn caller<H: Host>(translator: &mut Translator<'_>, _host: &mut H) {}

pub fn codesize<H: Host>(translator: &mut Translator<'_>, _host: &mut H) {}

pub fn codecopy<H: Host>(translator: &mut Translator<'_>, _host: &mut H) {}

pub fn calldataload<H: Host>(translator: &mut Translator<'_>, _host: &mut H) {}

pub fn calldatasize<H: Host>(translator: &mut Translator<'_>, _host: &mut H) {}

pub fn callvalue<H: Host>(translator: &mut Translator<'_>, _host: &mut H) {}

pub fn calldatacopy<H: Host>(translator: &mut Translator<'_>, _host: &mut H) {}

pub fn returndatasize<H: Host>(translator: &mut Translator<'_>, _host: &mut H) {}

pub fn returndatacopy<H: Host>(translator: &mut Translator<'_>, _host: &mut H) {}

pub fn gas<H: Host>(translator: &mut Translator<'_>, _host: &mut H) {}
