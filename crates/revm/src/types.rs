use crate::ExecutionResult;
use fluentbase_sdk::SyscallInvocationParams;
use revm::interpreter::Gas;

///
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SystemInterruptionInputs {
    ///
    pub call_id: u32,
    ///
    pub syscall_params: SyscallInvocationParams,
    ///
    pub gas: Gas,
    ///
    pub is_create: bool,
    ///
    pub is_static: bool,
    ///
    pub is_gas_free: bool,
}

///
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SystemInterruptionOutcome {
    ///
    pub inputs: Box<SystemInterruptionInputs>,
    ///
    pub remaining_gas: Gas,
    ///
    pub result: Option<ExecutionResult>,
    ///
    pub is_frame: bool,
}
