use fluentbase_sdk::{Bytes, ExitCode, SyscallInvocationParams};

#[derive(Clone, Debug)]
pub(crate) enum NextAction {
    ExecutionResult {
        exit_code: i32,
        output: Bytes,
        gas_used: u64,
    },
    NestedCall {
        call_id: u32,
        params: SyscallInvocationParams,
        gas_used: u64,
    },
}

impl NextAction {
    pub(crate) fn from_exit_code(gas_used: u64, exit_code: ExitCode) -> Self {
        Self::ExecutionResult {
            exit_code: exit_code.into_i32(),
            output: Default::default(),
            gas_used,
        }
    }
}

#[derive(Debug)]
pub(crate) enum Frame {
    Execute {
        params: SyscallInvocationParams,
        call_id: u32,
    },
    Resume {
        call_id: u32,
        output: Bytes,
        exit_code: i32,
        gas_used: u64,
    },
}
