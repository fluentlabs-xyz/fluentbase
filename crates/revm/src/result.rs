//! Contains the `[RwasmHaltReason]` type.
use fluentbase_sdk::{Bytes, ExitCode};
use revm::{
    context_interface::result::HaltReason,
    interpreter::{FrameInput, Gas, InstructionResult, InterpreterAction, InterpreterResult},
};

// /// The result of an execution operation.
// #[derive(Clone, Debug, PartialEq, Eq)]
// #[cfg_attr(feature = "serde", derive(::serde::Serialize, ::serde::Deserialize))]
// pub struct ExecutionResult {
//     /// The result of the instruction execution.
//     pub result: InterpreterResult,
//     /// The output of the instruction execution.
//     pub output: Bytes,
//     /// The gas usage information.
//     pub gas: Gas,
// }

pub type ExecutionResult = InterpreterResult;

pub fn instruction_result_from_exit_code(
    exit_code: ExitCode,
    is_empty_return_data: bool,
) -> InstructionResult {
    match exit_code {
        /* Basic Error Codes */
        ExitCode::Ok => {
            if is_empty_return_data {
                InstructionResult::Stop
            } else {
                InstructionResult::Return
            }
        }
        ExitCode::Panic => InstructionResult::Revert,
        ExitCode::Err => InstructionResult::UnknownError,
        ExitCode::InterruptionCalled => InstructionResult::Stop,
        /* Fluentbase Runtime Error Codes */
        ExitCode::RootCallOnly => InstructionResult::RootCallOnly,
        ExitCode::MalformedBuiltinParams => InstructionResult::MalformedBuiltinParams,
        ExitCode::CallDepthOverflow => InstructionResult::CallDepthOverflow,
        ExitCode::NonNegativeExitCode => InstructionResult::NonNegativeExitCode,
        ExitCode::UnknownError => InstructionResult::UnknownError,
        ExitCode::InputOutputOutOfBounds => InstructionResult::InputOutputOutOfBounds,
        ExitCode::PrecompileError => InstructionResult::PrecompileError,
        ExitCode::NotSupportedBytecode => InstructionResult::CreateContractStartingWithEF,
        ExitCode::StateChangeDuringStaticCall => InstructionResult::StateChangeDuringStaticCall,
        ExitCode::CreateContractSizeLimit => InstructionResult::CreateContractSizeLimit,
        ExitCode::CreateContractCollision => InstructionResult::CreateCollision,
        ExitCode::CreateContractStartingWithEF => InstructionResult::CreateContractStartingWithEF,
        /* Trap Error Codes */
        ExitCode::UnreachableCodeReached => InstructionResult::UnreachableCodeReached,
        ExitCode::MemoryOutOfBounds => InstructionResult::MemoryOutOfBounds,
        ExitCode::TableOutOfBounds => InstructionResult::TableOutOfBounds,
        ExitCode::IndirectCallToNull => InstructionResult::IndirectCallToNull,
        ExitCode::IntegerDivisionByZero => InstructionResult::IntegerDivisionByZero,
        ExitCode::IntegerOverflow => InstructionResult::IntegerOverflow,
        ExitCode::BadConversionToInteger => InstructionResult::BadConversionToInteger,
        ExitCode::StackOverflow => InstructionResult::StackOverflow,
        ExitCode::BadSignature => InstructionResult::BadSignature,
        ExitCode::OutOfFuel => InstructionResult::OutOfFuel,
        ExitCode::UnknownExternalFunction => InstructionResult::UnknownExternalFunction,
    }
}

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
}

pub enum NextActionOrInterruption {
    NextAction(NextAction),
    Interruption,
}

pub type RwasmHaltReason = HaltReason;
