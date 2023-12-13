#![allow(dead_code, unreachable_patterns, unused_macros, unused_imports)]

extern crate core;

use fluentbase_rwasm::{rwasm::ReducedModuleError, Caller};
pub use macros::*;
pub use platform::*;
pub use runtime::*;
pub use types::*;
// pub use zktrie::*;

mod fuel;
mod instruction;
mod macros;
mod platform;
mod runtime;
#[cfg(test)]
mod tests;
mod types;

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
