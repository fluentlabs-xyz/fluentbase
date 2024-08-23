use fluentbase_sdk::{Bytes, ExitCode, Fuel, SyscallInvocationParams};

#[derive(Clone, Debug)]
pub(crate) enum NextAction {
    ExecutionResult(Bytes, Fuel, i32),
    NestedCall(u32, SyscallInvocationParams),
}

impl NextAction {
    pub(crate) fn from_exit_code(fuel_spent: u64, exit_code: ExitCode) -> Self {
        Self::ExecutionResult(Bytes::default(), fuel_spent.into(), exit_code.into_i32())
    }
}

#[derive(Debug)]
pub(crate) enum Frame {
    Execute(SyscallInvocationParams, u32),
    Resume(u32, Bytes, i32, u64),
}

impl Frame {
    pub(crate) fn call_id(&self) -> u32 {
        match self {
            Frame::Execute(_, call_id) => *call_id,
            Frame::Resume(call_id, _, _, _) => *call_id,
        }
    }
}
