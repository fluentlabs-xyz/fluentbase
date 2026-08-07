//! Contains the `[RwasmHaltReason]` type.
use fluentbase_evm::types::instruction_result_from_exit_code;
use fluentbase_sdk::{Bytes, ExitCode};
use revm::{
    context_interface::result::HaltReason,
    interpreter::{
        return_ok, return_revert, FrameInput, Gas, InstructionResult, InterpreterAction,
        InterpreterResult,
    },
};

pub type ExecutionResult = InterpreterResult;

/// Burn the frame's remaining gas if `result` is an exceptional halt.
///
/// The rWasm boundary charges gas from the fuel the engine reports, and system precompile
/// wrappers only call `sync_evm_gas` on their success path. A malformed input therefore returns
/// an error before anything is charged, and for non-engine-metered precompiles the engine reports
/// almost no consumed fuel, so the supplied gas would be handed back to the caller. Native REVM
/// spends everything when a precompile halts, so every non-success, non-revert outcome must do
/// the same here.
pub fn spend_gas_on_exceptional_halt(result: InstructionResult, gas: &mut Gas) {
    if matches!(result, return_ok!() | return_revert!()) {
        return;
    }
    gas.spend_all();
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

    pub fn error(exit_code: ExitCode, mut gas: Gas) -> Self {
        let result = instruction_result_from_exit_code(exit_code, true);
        spend_gas_on_exceptional_halt(result, &mut gas);
        NextAction::Return(ExecutionResult {
            result,
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

#[cfg(test)]
mod tests {
    use super::*;

    fn gas_after_halt(exit_code: ExitCode) -> u64 {
        let mut gas = Gas::new(100_000);
        assert!(gas.record_regular_cost(1_000));
        spend_gas_on_exceptional_halt(instruction_result_from_exit_code(exit_code, true), &mut gas);
        gas.remaining()
    }

    #[test]
    fn exceptional_halt_burns_the_whole_frame() {
        // A malformed precompile input, an out-of-fuel frame, and a trap are all exceptional
        // halts: none of them may hand gas back to the caller.
        assert_eq!(gas_after_halt(ExitCode::PrecompileError), 0);
        assert_eq!(gas_after_halt(ExitCode::OutOfFuel), 0);
        assert_eq!(gas_after_halt(ExitCode::UnreachableCodeReached), 0);
        assert_eq!(gas_after_halt(ExitCode::MalformedBuiltinParams), 0);
    }

    #[test]
    fn success_and_revert_keep_the_remaining_gas() {
        assert_eq!(gas_after_halt(ExitCode::Ok), 99_000);
        assert_eq!(gas_after_halt(ExitCode::Panic), 99_000);
    }

    #[test]
    fn error_action_reports_a_fully_spent_frame() {
        let NextAction::Return(result) =
            NextAction::error(ExitCode::PrecompileError, Gas::new(500))
        else {
            panic!("expected a returning action");
        };
        assert_eq!(result.result, InstructionResult::PrecompileError);
        assert_eq!(result.gas.remaining(), 0);
    }
}
