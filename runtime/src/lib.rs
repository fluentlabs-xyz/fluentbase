// #![feature(local_key_cell_methods)]
#![feature(slice_group_by)]
#![allow(dead_code, unreachable_patterns, unused_macros, unused_imports)]

extern crate core;

pub use crate::zktrie::*;
use fluentbase_rwasm::{rwasm::ReducedModuleError, Caller};
pub use instruction::*;
pub use macros::*;
pub use platform::*;
pub use runtime::*;
pub use types::*;

mod crypto;
mod eth_typ;
mod eth_types;
mod evm;
mod hash;
mod instruction;
mod keccak_hash;
mod macros;
mod mpt;
mod mpt_helpers;
mod platform;
mod runtime;
mod rwasm;
mod secp256k1;
#[cfg(test)]
mod tests;
mod types;
mod zktrie;
mod zktrie_helpers;

#[derive(Debug)]
pub enum RuntimeError {
    ReducedModule(ReducedModuleError),
    Rwasm(fluentbase_rwasm::Error),
}

impl From<ReducedModuleError> for RuntimeError {
    fn from(value: ReducedModuleError) -> Self {
        Self::ReducedModule(value)
    }
}

macro_rules! rwasm_error {
    ($error_type:path) => {
        impl From<$error_type> for RuntimeError {
            fn from(value: $error_type) -> Self {
                Self::Rwasm(value.into())
            }
        }
    };
}

rwasm_error!(fluentbase_rwasm::global::GlobalError);
rwasm_error!(fluentbase_rwasm::memory::MemoryError);
rwasm_error!(fluentbase_rwasm::table::TableError);
rwasm_error!(fluentbase_rwasm::linker::LinkerError);
rwasm_error!(fluentbase_rwasm::module::ModuleError);

impl From<fluentbase_rwasm::Error> for RuntimeError {
    fn from(value: fluentbase_rwasm::Error) -> Self {
        Self::Rwasm(value)
    }
}

pub trait StateHandler<D> {
    // sys calls
    fn sys_halt(&mut self, _caller: &Caller<D>, _exit_code: u32) {}
    fn sys_write(&mut self, _caller: &Caller<D>, _offset: u32, _length: u32) {}
    fn sys_read(&mut self, _caller: &Caller<D>, _target: u32, _offset: u32, _length: u32) {}
    // evm calls
    fn evm_return(&mut self, _caller: &Caller<D>, _offset: u32, _length: u32) {}
}

#[derive(Default, Debug)]
#[allow(dead_code)]
pub struct MemoryStateHandler {
    input: Vec<u8>,
    exit_code: u32,
    output: Vec<u8>,
}

impl StateHandler<()> for MemoryStateHandler {
    fn sys_halt(&mut self, _caller: &Caller<()>, exit_code: u32) {
        self.exit_code = exit_code;
    }

    fn sys_write(&mut self, _caller: &Caller<()>, _offset: u32, _length: u32) {}
    fn sys_read(&mut self, _caller: &Caller<()>, _target: u32, _offset: u32, _length: u32) {}

    fn evm_return(&mut self, _caller: &Caller<()>, _offset: u32, _length: u32) {}
}
