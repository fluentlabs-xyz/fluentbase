use rwasm_codegen::{rwasm::Error as RwasmError, ReducedModuleError};

#[derive(Debug)]
pub enum RuntimeError {
    ReducedModule(ReducedModuleError),
    Rwasm(RwasmError),
    StorageError(String),
}

impl From<ReducedModuleError> for RuntimeError {
    fn from(value: ReducedModuleError) -> Self {
        Self::ReducedModule(value)
    }
}

impl From<RwasmError> for RuntimeError {
    fn from(value: RwasmError) -> Self {
        Self::Rwasm(value)
    }
}

pub use fluentbase_types::*;

macro_rules! rwasm_error {
    ($error_type:path) => {
        impl From<$error_type> for $crate::types::RuntimeError {
            fn from(value: $error_type) -> Self {
                Self::Rwasm(value.into())
            }
        }
    };
}

rwasm_error!(rwasm::global::GlobalError);
rwasm_error!(rwasm::memory::MemoryError);
rwasm_error!(rwasm::table::TableError);
rwasm_error!(rwasm::linker::LinkerError);
rwasm_error!(rwasm::module::ModuleError);
