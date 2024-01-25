use fluentbase_rwasm::rwasm::ReducedModuleError;

#[derive(Debug)]
pub enum RuntimeError {
    ReducedModule(ReducedModuleError),
    Rwasm(fluentbase_rwasm::Error),
    StorageError(String),
}

impl From<ReducedModuleError> for RuntimeError {
    fn from(value: ReducedModuleError) -> Self {
        Self::ReducedModule(value)
    }
}

impl From<fluentbase_rwasm::Error> for RuntimeError {
    fn from(value: fluentbase_rwasm::Error) -> Self {
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

rwasm_error!(fluentbase_rwasm::global::GlobalError);
rwasm_error!(fluentbase_rwasm::memory::MemoryError);
rwasm_error!(fluentbase_rwasm::table::TableError);
rwasm_error!(fluentbase_rwasm::linker::LinkerError);
rwasm_error!(fluentbase_rwasm::module::ModuleError);
