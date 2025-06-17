use crate::helpers::SyscallError;
use alloc::boxed::Box;
use core::fmt::{Display, Formatter};
use fluentbase_types::ExitCode;
use solana_instruction::error::InstructionError;
use solana_rbpf::{elf::ElfError, error::EbpfError};
use solana_transaction_error::TransactionError;

pub type Error = Box<dyn core::error::Error>;

/// Error definitions
#[derive(Debug)]
#[repr(u64)]
pub enum RuntimeError {
    InvalidTransformation,
    InvalidLength,
    InvalidIdx,
    InvalidType,
    InvalidPrefix,
}

impl core::error::Error for RuntimeError {}

impl Display for RuntimeError {
    fn fmt(&self, f: &mut Formatter<'_>) -> core::fmt::Result {
        match self {
            RuntimeError::InvalidTransformation => write!(f, "RuntimeError::InvalidTransformation"),
            RuntimeError::InvalidLength => write!(f, "RuntimeError::InvalidLength"),
            RuntimeError::InvalidIdx => write!(f, "RuntimeError::InvalidIdx"),
            RuntimeError::InvalidType => write!(f, "RuntimeError::InvalidType"),
            RuntimeError::InvalidPrefix => write!(f, "RuntimeError::InvalidPrefix"),
        }
    }
}

#[derive(Debug)]
pub enum SvmError {
    ElfError(ElfError),
    EbpfError(EbpfError),
    TransactionError(TransactionError),
    BincodeEncodeError(bincode::error::EncodeError),
    BincodeDecodeError(bincode::error::DecodeError),
    ExitCode(ExitCode),
    InstructionError(InstructionError),
    SyscallError(SyscallError),
    RuntimeError(RuntimeError),
}

impl From<TransactionError> for SvmError {
    fn from(value: TransactionError) -> Self {
        SvmError::TransactionError(value)
    }
}

impl From<ExitCode> for SvmError {
    fn from(value: ExitCode) -> Self {
        SvmError::ExitCode(value)
    }
}

impl From<bincode::error::EncodeError> for SvmError {
    fn from(value: bincode::error::EncodeError) -> Self {
        SvmError::BincodeEncodeError(value)
    }
}

impl From<bincode::error::DecodeError> for SvmError {
    fn from(value: bincode::error::DecodeError) -> Self {
        SvmError::BincodeDecodeError(value)
    }
}

impl From<InstructionError> for SvmError {
    fn from(value: InstructionError) -> Self {
        SvmError::InstructionError(value)
    }
}

impl From<ElfError> for SvmError {
    fn from(value: ElfError) -> Self {
        SvmError::ElfError(value)
    }
}

impl From<EbpfError> for SvmError {
    fn from(value: EbpfError) -> Self {
        SvmError::EbpfError(value)
    }
}

impl From<SyscallError> for SvmError {
    fn from(value: SyscallError) -> Self {
        SvmError::SyscallError(value)
    }
}

impl From<RuntimeError> for SvmError {
    fn from(value: RuntimeError) -> Self {
        SvmError::RuntimeError(value)
    }
}

impl From<SvmError> for Error {
    fn from(value: SvmError) -> Self {
        match value {
            SvmError::ElfError(e) => Box::new(e),
            SvmError::EbpfError(e) => Box::new(e),
            SvmError::TransactionError(e) => Box::new(e),
            SvmError::BincodeEncodeError(e) => Box::new(e),
            SvmError::BincodeDecodeError(e) => Box::new(e),
            SvmError::ExitCode(e) => Box::new(e),
            SvmError::InstructionError(e) => Box::new(e),
            SvmError::SyscallError(e) => Box::new(e),
            SvmError::RuntimeError(e) => Box::new(e),
        }
    }
}
