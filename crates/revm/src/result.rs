//! Contains the `[RwasmHaltReason]` type.
use fluentbase_evm::types::instruction_result_from_exit_code;
use fluentbase_sdk::{Bytes, ExitCode};
use revm::{
    context_interface::result::HaltReason,
    interpreter::{FrameInput, Gas, InterpreterAction, InterpreterResult},
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

pub type RwasmHaltReason = HaltReason;
