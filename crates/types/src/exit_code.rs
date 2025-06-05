use precompile::{PrecompileError};
use rwasm::RwasmError;
use strum_macros::{Display, FromRepr};

/// Exit codes representing various execution outcomes and error conditions.
///
/// This enum defines the possible exit codes that can be returned by the execution
/// environment.
///
/// The codes are grouped into several categories:
/// - Basic status codes (0 to -2)
/// - Fluentbase-specific error codes (-1000 and below)
/// - Trap error codes (-2000 and below)
///
/// **Note**: Exit codes cannot be positive, as positive values are used to represent
/// call indices in interrupted executions.
///
/// Exit codes are used to represent the outcome of execution across different
/// environments.
/// This makes their interpretation somewhat nuanced.
///
/// Within applications, developers can use any exit code, but there are conventions:
/// 1. `Ok` (0) — Indicates successful execution.
/// 2. `Panic` (-1) — Controlled application revert (intended error, no gas penalty).
/// 3. Any other code is treated as an error with a gas penalty and is mapped to `Err` (-2).
///
/// Technically, developers *can* return trap codes, but it's generally pointless:
/// they are replaced by the `Err` (-2) code and still incur gas penalties.
/// If the error code cannot be determined, it is mapped to `UnknownError` (-1006),
/// which also results in a gas fine.
///
/// The SDK provides helper functions such as `evm_exit` and `evm_panic` to simplify
/// returning error codes.
/// These produce Solidity-compatible error outputs.
///
/// The `exit` function remains available to all developers.
/// In some cases, an application may want to simulate trap behavior or explicitly exit with a gas
/// penalty to comply with gas consumption standards (for instance, EVM runtime).
///
/// This behavior is similar to Solidity, where only three exit codes are supported:
/// - For legacy contracts: `Ok = 1`, `Revert = 0`, `Err = 2`
/// - For EOF contracts: `Ok = 0`, `Revert = 1`, `Err = 2`
///
/// ## Basic Status Codes
/// * `Ok` (0) - Successful execution
/// * `Panic` (-1) - Execution panic
/// * `Err` (-2) - General error
///
/// ## Fluentbase Error Codes
/// * `RootCallOnly` - Operation restricted to root-level calls
/// * `MalformedBuiltinParams` - Invalid parameters passed to builtin function
/// * `CallDepthOverflow` - Call stack depth exceeded limit
/// * `NonNegativeExitCode` - Exit code must be negative
/// * `UnknownError` - Unspecified error condition
/// * `InputOutputOutOfBounds` - I/O operation exceeded bounds
/// * `PrecompileError` - Error in precompiled contract execution
///
/// ## Trap Error Codes
/// * `UnreachableCodeReached` - Execution reached unreachable code
/// * `MemoryOutOfBounds` - Memory access violation
/// * `TableOutOfBounds` - Table access violation
/// * `IndirectCallToNull` - Attempted call to null function pointer
/// * `IntegerDivisionByZero` - Division by zero
/// * `IntegerOverflow` - Integer overflow occurred
/// * `BadConversionToInteger` - Invalid integer conversion
/// * `StackOverflow` - Stack limit exceeded
/// * `BadSignature` - Invalid function signature
/// * `OutOfFuel` - Insufficient gas/fuel for execution
/// * `GrowthOperationLimited` - Growth operation exceeded limits
/// * `UnresolvedFunction` - Function not found
#[derive(Default, Debug, Copy, Clone, Eq, PartialEq, Display, FromRepr)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[repr(i32)]
pub enum ExitCode {
    // warning: when adding new codes doesn't forget to add them to impls below
    #[default]
    Ok = 0,
    Panic = -1,
    Err = -2,
    // fluentbase error codes
    RootCallOnly = -1002,
    MalformedBuiltinParams = -1003,
    CallDepthOverflow = -1004,
    NonNegativeExitCode = -1005,
    UnknownError = -1006,
    InputOutputOutOfBounds = -1007,
    PrecompileError = -1008,
    // trap error codes
    UnreachableCodeReached = -2001,
    MemoryOutOfBounds = -2002,
    TableOutOfBounds = -2003,
    IndirectCallToNull = -2004,
    IntegerDivisionByZero = -2005,
    IntegerOverflow = -2006,
    BadConversionToInteger = -2007,
    StackOverflow = -2008,
    BadSignature = -2009,
    OutOfFuel = -2010,
    GrowthOperationLimited = -2011,
    UnresolvedFunction = -2013,
}

pub trait UnwrapExitCode<T> {
    fn unwrap_exit_code(self) -> T;
}

impl<T> UnwrapExitCode<T> for Result<T, ExitCode> {
    fn unwrap_exit_code(self) -> T {
        match self {
            Ok(res) => res,
            Err(err) => panic!("exit code: {} ({})", err, err.into_i32()),
        }
    }
}

impl From<i32> for ExitCode {
    fn from(value: i32) -> Self {
        Self::from_repr(value).unwrap_or(ExitCode::UnknownError)
    }
}

impl Into<i32> for ExitCode {
    fn into(self) -> i32 {
        self as i32
    }
}

impl ExitCode {
    pub fn is_ok(&self) -> bool {
        self == &Self::Ok
    }

    /// Returns whether the result is a revert.
    pub fn is_revert(&self) -> bool {
        self == &Self::Panic
    }

    pub fn is_error(&self) -> bool {
        !self.is_ok() && !self.is_revert()
    }

    pub const fn into_i32(self) -> i32 {
        self as i32
    }
}

impl From<RwasmError> for ExitCode {
    fn from(value: RwasmError) -> Self {
        Self::from(&value)
    }
}

impl From<&RwasmError> for ExitCode {
    fn from(value: &RwasmError) -> Self {
        match value {
            RwasmError::UnreachableCodeReached => ExitCode::UnreachableCodeReached,
            RwasmError::MemoryOutOfBounds => ExitCode::MemoryOutOfBounds,
            RwasmError::TableOutOfBounds => ExitCode::TableOutOfBounds,
            RwasmError::IndirectCallToNull => ExitCode::IndirectCallToNull,
            RwasmError::IntegerDivisionByZero => ExitCode::IntegerDivisionByZero,
            RwasmError::IntegerOverflow => ExitCode::IntegerOverflow,
            RwasmError::BadConversionToInteger => ExitCode::BadConversionToInteger,
            RwasmError::StackOverflow => ExitCode::StackOverflow,
            RwasmError::BadSignature => ExitCode::BadSignature,
            RwasmError::OutOfFuel => ExitCode::OutOfFuel,
            RwasmError::GrowthOperationLimited => ExitCode::GrowthOperationLimited,
            RwasmError::UnresolvedFunction => ExitCode::UnresolvedFunction,
            _ => ExitCode::UnknownError,
        }
    }
}

impl From<PrecompileError> for ExitCode {
    fn from(err: PrecompileError) -> Self {
        Self::from(&err)
    }
}
impl From<&PrecompileError> for ExitCode {
    fn from(err: &PrecompileError) -> Self {
        match err {
            PrecompileError::OutOfGas => ExitCode::OutOfFuel,
            _ => ExitCode::PrecompileError,
        }
    }
}
// impl From<PrecompileErrors> for ExitCode {
//     fn from(err: PrecompileErrors) -> Self {
//         Self::from(&err)
//     }
// }
// impl From<&PrecompileErrors> for ExitCode {
//     fn from(err: &PrecompileErrors) -> Self {
//         match err {
//             PrecompileErrors::Error(err) => ExitCode::from(err),
//             PrecompileErrors::Fatal { .. } => ExitCode::PrecompileError,
//         }
//     }
// }
