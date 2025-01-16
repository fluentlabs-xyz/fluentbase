use rwasm::{
    core::{HostError, TrapCode},
    errors::MemoryError,
    rwasm::BinaryFormatError,
};

pub const N_DEFAULT_STACK_SIZE: usize = 1024;
pub const N_MAX_STACK_SIZE: usize = 4096;

#[derive(Debug)]
pub enum RwasmError {
    MalformedBinary,
    TrapCode(TrapCode),
    UnknownExternalFunction(u32),
    ExecutionHalted(i32),
    MemoryError(MemoryError),
    HostInterruption(Box<dyn HostError>),
}

impl RwasmError {
    pub fn unwrap_exit_code(&self) -> i32 {
        match self {
            RwasmError::ExecutionHalted(exit_code) => *exit_code,
            _ => unreachable!("runtime: can't unwrap exit code from error"),
        }
    }
}

impl From<BinaryFormatError> for RwasmError {
    fn from(_value: BinaryFormatError) -> Self {
        Self::MalformedBinary
    }
}
impl From<TrapCode> for RwasmError {
    fn from(value: TrapCode) -> Self {
        Self::TrapCode(value)
    }
}
impl From<MemoryError> for RwasmError {
    fn from(value: MemoryError) -> Self {
        Self::MemoryError(value)
    }
}
