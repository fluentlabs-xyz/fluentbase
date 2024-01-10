use crate::runtime::RuntimeContext;
use fluentbase_rwasm::{AsContextMut, Caller, Extern, Memory};
pub(crate) use mpt::*;

mod crypto;
mod ecc;
mod mpt;
mod rwasm;
mod sys;
mod wasi;
mod zktrie;

pub(crate) use crypto::*;
pub(crate) use ecc::*;
pub(crate) use mpt::*;
pub(crate) use rwasm::*;
pub(crate) use sys::*;
pub(crate) use wasi::*;
// pub(crate) use zktrie::*;

pub(crate) fn exported_memory_slice<'a, T>(
    caller: &'a mut Caller<RuntimeContext<T>>,
    offset: usize,
    length: usize,
) -> &'a mut [u8] {
    if length == 0 {
        return &mut [];
    }
    let memory = caller.exported_memory().data_mut(caller.as_context_mut());
    if memory.len() > offset {
        return &mut memory[offset..(offset + length)];
    }
    return &mut [];
}

pub(crate) fn exported_memory_vec<T>(
    caller: &mut Caller<RuntimeContext<T>>,
    offset: usize,
    length: usize,
) -> Vec<u8> {
    if length == 0 {
        return Default::default();
    }
    let memory = caller.exported_memory().data_mut(caller.as_context_mut());
    if memory.len() > offset {
        return Vec::from(&memory[offset..(offset + length)]);
    }
    return Default::default();
}
