use fluentbase_rwasm::{
    common::{Trap, TrapCode},
    rwasm::ReducedModuleError,
};

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

pub const STACK_MAX_HEIGHT: usize = 1024;
pub const RECURSIVE_MAX_DEPTH: usize = 1024;

#[derive(Debug, Copy, Clone)]
pub enum ExitCode {
    // fluentbase error codes
    ExecutionHalted = -1001,
    NotSupportedCall = -1003,
    TransactError = -1004,
    RwasmOutputOverflow = -1005,
    InputDecodeFailure = -1006,
    PoseidonError = -1007,
    // trap error codes
    UnreachableCodeReached = -2006,
    MemoryOutOfBounds = -2007,
    TableOutOfBounds = -2008,
    IndirectCallToNull = -2009,
    IntegerDivisionByZero = -2010,
    IntegerOverflow = -2011,
    BadConversionToInteger = -2012,
    StackOverflow = -2013,
    BadSignature = -2014,
    OutOfFuel = -2015,
    GrowthOperationLimited = -2016,
    UnknownError = -2017,
}

pub const STATE_MAIN: u32 = 0;
pub const STATE_DEPLOY: u32 = 1;

impl From<TrapCode> for ExitCode {
    fn from(value: TrapCode) -> Self {
        match value {
            TrapCode::UnreachableCodeReached => ExitCode::UnreachableCodeReached,
            TrapCode::MemoryOutOfBounds => ExitCode::MemoryOutOfBounds,
            TrapCode::TableOutOfBounds => ExitCode::TableOutOfBounds,
            TrapCode::IndirectCallToNull => ExitCode::IndirectCallToNull,
            TrapCode::IntegerDivisionByZero => ExitCode::IntegerDivisionByZero,
            TrapCode::IntegerOverflow => ExitCode::IntegerOverflow,
            TrapCode::BadConversionToInteger => ExitCode::BadConversionToInteger,
            TrapCode::StackOverflow => ExitCode::StackOverflow,
            TrapCode::BadSignature => ExitCode::BadSignature,
            TrapCode::OutOfFuel => ExitCode::OutOfFuel,
            TrapCode::GrowthOperationLimited => ExitCode::GrowthOperationLimited,
        }
    }
}

impl Into<Trap> for ExitCode {
    fn into(self) -> Trap {
        Trap::i32_exit(self as i32)
    }
}

impl Into<i32> for ExitCode {
    fn into(self) -> i32 {
        self as i32
    }
}
