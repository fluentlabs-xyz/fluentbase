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
