use crate::translator::host::Host;
use crate::translator::translator::Translator;

pub fn lt<H: Host>(_translator: &mut Translator<'_>, _host: &mut H) {}

pub fn gt<H: Host>(_translator: &mut Translator<'_>, _host: &mut H) {}

pub fn slt<H: Host>(_translator: &mut Translator<'_>, _host: &mut H) {}

pub fn sgt<H: Host>(_translator: &mut Translator<'_>, _host: &mut H) {}

pub fn eq<H: Host>(_translator: &mut Translator<'_>, _host: &mut H) {}

pub fn iszero<H: Host>(_translator: &mut Translator<'_>, _host: &mut H) {}

pub fn bitand<H: Host>(_translator: &mut Translator<'_>, _host: &mut H) {}

pub fn bitor<H: Host>(_translator: &mut Translator<'_>, _host: &mut H) {}

pub fn bitxor<H: Host>(_translator: &mut Translator<'_>, _host: &mut H) {}

pub fn not<H: Host>(_translator: &mut Translator<'_>, _host: &mut H) {}

pub fn byte<H: Host>(_translator: &mut Translator<'_>, _host: &mut H) {}

pub fn shl<H: Host>(_translator: &mut Translator<'_>, _host: &mut H) {}

pub fn shr<H: Host>(_translator: &mut Translator<'_>, _host: &mut H) {}

pub fn sar<H: Host>(_translator: &mut Translator<'_>, _host: &mut H) {}
