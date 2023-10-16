use fluentbase_rwasm::common::{Trap, TrapCode};

#[derive(Debug, Copy, Clone)]
pub enum ExitCode {
    EvmStop = -1001,
    MemoryOutOfBounds = -1002,
    NotSupportedCall = -1003,
    TransactError = -1004,
    TransactOutputOverflow = -1005,
    UnreachableCodeReached = -1006,
    TableOutOfBounds = -1007,
    IndirectCallToNull = -1008,
    IntegerDivisionByZero = -1009,
    IntegerOverflow = -1010,
    BadConversionToInteger = -1011,
    StackOverflow = -1012,
    BadSignature = -1013,
    OutOfFuel = -1014,
    GrowthOperationLimited = -1015,
    UnknownError = -1016,
}

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
