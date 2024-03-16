use rwasm::{rwasm::BinaryFormatError, Error as RwasmError};

#[derive(Debug)]
pub enum RuntimeError {
    BinaryFormatError(BinaryFormatError),
    Rwasm(RwasmError),
    StorageError(String),
    MissingEntrypoint,
}

impl From<BinaryFormatError> for RuntimeError {
    fn from(value: BinaryFormatError) -> Self {
        Self::BinaryFormatError(value)
    }
}

impl From<RwasmError> for RuntimeError {
    fn from(value: RwasmError) -> Self {
        Self::Rwasm(value)
    }
}

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
