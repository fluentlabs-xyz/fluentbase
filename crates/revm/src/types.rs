use crate::ExecutionResult;
use fluentbase_sdk::SyscallInvocationParams;
use revm::interpreter::Gas;
use std::boxed::Box;

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
    /// Indicates is interruption happen inside contact deployment.
    pub is_create: bool,
    /// Indicates is interruption happen inside static call.
    pub is_static: bool,
}

/// An interruption outcome.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SystemInterruptionOutcome {
    /// Original inputs.
    pub inputs: Box<SystemInterruptionInputs>,
    /// An interruption execution result.
    /// It can be empty for frame creation,
    /// where we don't know the result until the frame is executed.
    pub result: Option<ExecutionResult>,
    /// Indicated is it a nested frame call or not.
    pub is_frame: bool,
}
