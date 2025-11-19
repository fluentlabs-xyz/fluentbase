use crate::ExecutionResult;
use fluentbase_sdk::SyscallInvocationParams;
use revm::interpreter::Gas;

/// A system interruption input params
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SystemInterruptionInputs {
    /// The call identifier (used for recover).
    pub call_id: u32,
    /// Interruptions params (code hash, inputs, gas limits, etc.).
    pub syscall_params: SyscallInvocationParams,
    /// A gas snapshot assigned before the interruption.
    /// We need this to calculate the final amount of gas charged for the entire interruption.
    pub gas: Gas,
}

/// An interruption outcome.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SystemInterruptionOutcome {
    /// Original inputs.
    pub inputs: SystemInterruptionInputs,
    /// An interruption execution result.
    /// It can be empty for frame creation,
    /// where we don't know the result until the frame is executed.
    pub result: Option<ExecutionResult>,
    /// Indicates was the frame halted before execution.
    /// When we do CALL-like op we can halt execution during the frame creation, we
    /// should handle this to forward inside the system runtime to make sure all frames
    /// are terminated gracefully.
    pub halted_frame: bool,
}
