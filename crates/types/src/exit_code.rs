use revm_precompile::PrecompileError;
use rwasm::TrapCode;
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
#[derive(Default, Debug, Copy, Clone, Hash, Eq, PartialEq, Display, FromRepr)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[repr(i32)]
pub enum ExitCode {
    /* Basic Error Codes */
    /// Execution is finished without errors
    #[default]
    Ok = 0,
    /// Panic is produced by a program (aka revert)
    Panic = -1,
    /// An internal error (mapped from the errors below for nested EVM calls)
    Err = -2,
    /// An interruption created by runtime (only for system contracts)
    InterruptionCalled = -3,

    /* Fluentbase Runtime Error Codes */
    /// Function can only be invoked as the root entry call
    RootCallOnly = -1002,
    /// Builtin function received malformed or invalid parameters
    MalformedBuiltinParams = -1003,
    /// Exceeded maximum allowed call stack depth
    CallDepthOverflow = -1004,
    /// Exit code must be non-negative, but a negative value was used
    NonNegativeExitCode = -1005,
    /// Generic catch-all error for unknown failures
    UnknownError = -1006,
    /// I/O operation tried to read/write outside allowed buffer bounds
    InputOutputOutOfBounds = -1007,
    /// An error happens inside a precompiled contract
    PrecompileError = -1008,
    /// Passed bytecode into executor is not supported
    NotSupportedBytecode = -1009,
    /// State changed inside immutable call (static=true)
    StateChangeDuringStaticCall = -1010,
    /// Create contract size limit reached (limit depends on the application type)
    CreateContractSizeLimit = -1011,
    /// There is a collision on the contract creation (same address is derived)
    CreateContractCollision = -1012,
    /// Created contract starts with invalid bytes (`0xEF`).
    CreateContractStartingWithEF = -1013,

    /* Trap Error Codes */
    /// Execution reached a code path marked as unreachable
    UnreachableCodeReached = -2001,
    /// Memory access outside the allocated memory range
    MemoryOutOfBounds = -2002,
    /// Table index access outside the allocated table range
    TableOutOfBounds = -2003,
    /// Indirect function call attempted with a null function reference
    IndirectCallToNull = -2004,
    /// Division or remainder by zero occurred
    IntegerDivisionByZero = -2005,
    /// Integer arithmetic operation overflowed the allowed range
    IntegerOverflow = -2006,
    /// Invalid conversion to integer (e.g., from NaN or out-of-range value)
    BadConversionToInteger = -2007,
    /// Stack reached its limit (overflow or underflow)
    StackOverflow = -2008,
    /// Function signature mismatch in a call
    BadSignature = -2009,
    /// Execution ran out of allocated fuel/gas
    OutOfFuel = -2010,
    /// Call an undefined or unregistered external function
    UnknownExternalFunction = -2011,

    /* System Error Codes */
    /// An unexpected fatal execution failure (node should panic or terminate the execution)
    UnexpectedFatalExecutionFailure = -3001,
    /// Missing storage slot
    MissingStorageSlot = -3002,
}

impl core::error::Error for ExitCode {}

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

impl From<TrapCode> for ExitCode {
    fn from(value: TrapCode) -> Self {
        Self::from(&value)
    }
}

impl From<&TrapCode> for ExitCode {
    fn from(value: &TrapCode) -> Self {
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
            TrapCode::UnknownExternalFunction => ExitCode::UnknownExternalFunction,
            TrapCode::InterruptionCalled => ExitCode::InterruptionCalled,
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

pub type EvmExitCode = u32;
