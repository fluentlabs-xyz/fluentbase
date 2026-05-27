//! Contains the `[RwasmHaltReason]` type.
use fluentbase_evm::types::instruction_result_from_exit_code;
use fluentbase_sdk::{Bytes, ExitCode};
use revm::{
    context_interface::result::HaltReason,
    interpreter::{FrameInput, Gas, InstructionResult, InterpreterAction, InterpreterResult},
};

pub type ExecutionResult = InterpreterResult;

/// Next actions to be executed
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum NextAction {
    /// New frame
    NewFrame(FrameInput),
    /// Interpreter finished execution.
    Return(ExecutionResult),
    /// An interrupted call flag
    InterruptionResult,
}

impl NextAction {
    pub fn into_interpreter_action(self) -> InterpreterAction {
        match self {
            NextAction::NewFrame(frame_input) => InterpreterAction::NewFrame(frame_input),
            NextAction::Return(result) => InterpreterAction::Return(result),
            NextAction::InterruptionResult => unreachable!(),
        }
    }

    pub fn error(exit_code: ExitCode, gas: Gas) -> Self {
        NextAction::Return(ExecutionResult {
            result: instruction_result_from_exit_code(exit_code, true),
            output: Bytes::default(),
            gas,
        })
    }

    pub fn out_of_fuel(gas: Gas) -> Self {
        Self::error(ExitCode::OutOfFuel, gas)
    }
}

pub enum NextActionOrInterruption {
    NextAction(NextAction),
    Interruption,
}

/// rWasm/Fluentbase-specific halt reason.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum RwasmHaltReason {
    /// Base EVM halt reason.
    Base(HaltReason),
    /// Function can only be invoked as the root entry call.
    RootCallOnly,
    /// Builtin function received malformed or invalid parameters.
    MalformedBuiltinParams,
    /// Exceeded maximum allowed call stack depth.
    CallDepthOverflow,
    /// Exit code must be negative, but a non-negative value was used.
    NonNegativeExitCode,
    /// Generic catch-all error for unknown failures.
    UnknownError,
    /// I/O operation tried to read/write outside allowed buffer bounds.
    InputOutputOutOfBounds,
    /// Execution reached a code path marked as unreachable.
    UnreachableCodeReached,
    /// Memory access outside the allocated memory range.
    MemoryOutOfBounds,
    /// Table index access outside the allocated table range.
    TableOutOfBounds,
    /// Indirect function call attempted with a null function reference.
    IndirectCallToNull,
    /// Division or remainder by zero occurred.
    IntegerDivisionByZero,
    /// Integer arithmetic operation overflowed the allowed range.
    IntegerOverflow,
    /// Invalid conversion to integer.
    BadConversionToInteger,
    /// Function signature mismatch in a call.
    BadSignature,
    /// Execution ran out of allocated fuel/gas.
    OutOfFuel,
    /// Call an undefined or unregistered external function.
    UnknownExternalFunction,
}

impl From<HaltReason> for RwasmHaltReason {
    fn from(value: HaltReason) -> Self {
        Self::Base(value)
    }
}

impl From<InstructionResult> for RwasmHaltReason {
    fn from(value: InstructionResult) -> Self {
        match value {
            InstructionResult::RootCallOnly => Self::RootCallOnly,
            InstructionResult::MalformedBuiltinParams => Self::MalformedBuiltinParams,
            InstructionResult::CallDepthOverflow => Self::CallDepthOverflow,
            InstructionResult::NonNegativeExitCode => Self::NonNegativeExitCode,
            InstructionResult::UnknownError => Self::UnknownError,
            InstructionResult::InputOutputOutOfBounds => Self::InputOutputOutOfBounds,
            InstructionResult::UnreachableCodeReached => Self::UnreachableCodeReached,
            InstructionResult::MemoryOutOfBounds => Self::MemoryOutOfBounds,
            InstructionResult::TableOutOfBounds => Self::TableOutOfBounds,
            InstructionResult::IndirectCallToNull => Self::IndirectCallToNull,
            InstructionResult::IntegerDivisionByZero => Self::IntegerDivisionByZero,
            InstructionResult::IntegerOverflow => Self::IntegerOverflow,
            InstructionResult::BadConversionToInteger => Self::BadConversionToInteger,
            InstructionResult::BadSignature => Self::BadSignature,
            InstructionResult::OutOfFuel => Self::OutOfFuel,
            InstructionResult::UnknownExternalFunction => Self::UnknownExternalFunction,
            result => Self::Base(result.into()),
        }
    }
}
