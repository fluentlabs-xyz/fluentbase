use alloc::boxed::Box;

use crate::translator::host::Host;
use crate::translator::inner_models::{CallInputs, CallScheme, CreateInputs};
use crate::translator::translator::Translator;

pub fn balance<H: Host>(_translator: &mut Translator<'_>, host: &mut H) {}

/// EIP-1884: Repricing for trie-size-dependent opcodes
pub fn selfbalance<H: Host>(_translator: &mut Translator<'_>, host: &mut H) {}

pub fn extcodesize<H: Host>(_translator: &mut Translator<'_>, host: &mut H) {
}

/// EIP-1052: EXTCODEHASH opcode
pub fn extcodehash<H: Host>(_translator: &mut Translator<'_>, host: &mut H) {
}

pub fn extcodecopy<H: Host>(_translator: &mut Translator<'_>, host: &mut H) {
}

pub fn blockhash<H: Host>(_translator: &mut Translator<'_>, host: &mut H) {
}

pub fn sload<H: Host>(_translator: &mut Translator<'_>, host: &mut H) {
}

pub fn sstore<H: Host>(_translator: &mut Translator<'_>, host: &mut H) {
}

pub fn tstore<H: Host>(_translator: &mut Translator<'_>, host: &mut H) {
}

pub fn tload<H: Host>(_translator: &mut Translator<'_>, host: &mut H) {
}

pub fn log<const N: usize, H: Host>(_translator: &mut Translator<'_>, host: &mut H) {
}

pub fn selfdestruct<H: Host>(_translator: &mut Translator<'_>, host: &mut H) {
}

#[inline(never)]
pub fn prepare_create_inputs<H: Host, const IS_CREATE2: bool>(
    _translator: &mut Translator<'_>,
    host: &mut H,
    create_inputs: &mut Option<Box<CreateInputs>>,
) {
}

pub fn create<const IS_CREATE2: bool, H: Host>(_translator: &mut Translator<'_>, host: &mut H) {}

pub fn call<H: Host>(_translator: &mut Translator<'_>, host: &mut H) {}

pub fn call_code<H: Host>(_translator: &mut Translator<'_>, host: &mut H) {}

pub fn delegate_call<H: Host>(_translator: &mut Translator<'_>, host: &mut H) {}

pub fn static_call<H: Host>(_translator: &mut Translator<'_>, host: &mut H) {}

#[inline(never)]
fn prepare_call_inputs<H: Host>(
    _translator: &mut Translator<'_>,
    scheme: CallScheme,
    host: &mut H,
    result_len: &mut usize,
    result_offset: &mut usize,
    result_call_inputs: &mut Option<Box<CallInputs>>,
) {
}

pub fn call_inner<H: Host>(
    scheme: CallScheme,
    _translator: &mut Translator<'_>,
    host: &mut H,
) {
}
