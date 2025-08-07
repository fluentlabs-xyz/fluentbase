use alloc::{boxed::Box, string::String, vec::Vec};
use core::{
    fmt,
    fmt::{Display, Formatter},
    str::Utf8Error,
};
use fluentbase_sdk::ExitCode;
use solana_instruction::error::InstructionError;
use solana_pubkey::{Pubkey, PubkeyError};
use solana_rbpf::{elf::ElfError, error::EbpfError};
use solana_transaction_error::TransactionError;

pub type Error = Box<dyn core::error::Error>;

#[derive(Debug, PartialEq, Eq)]
pub enum SyscallError {
    InvalidString(Utf8Error, Vec<u8>),
    Abort,
    Panic(String, u64, u64),
    InvokeContextBorrowFailed,
    MalformedSignerSeed(Utf8Error, Vec<u8>),
    BadSeeds(PubkeyError),
    ProgramNotSupported(Pubkey),
    UnalignedPointer,
    TooManySigners,
    InstructionTooLarge(usize, usize),
    TooManyAccounts,
    CopyOverlapping,
    ReturnDataTooLarge(u64, u64),
    TooManySlices,
    InvalidLength,
    MaxInstructionDataLenExceeded {
        data_len: u64,
        max_data_len: u64,
    },
    MaxInstructionAccountsExceeded {
        num_accounts: u64,
        max_accounts: u64,
    },
    MaxInstructionAccountInfosExceeded {
        num_account_infos: u64,
        max_account_infos: u64,
    },
    InvalidAttribute,
    InvalidPointer,
    ArithmeticOverflow,
}

impl Display for SyscallError {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            SyscallError::InvalidString(_, _) => write!(f, "SyscallError::InvalidString"),
            SyscallError::Abort => write!(f, "SyscallError::Abort"),
            SyscallError::Panic(_, _, _) => write!(f, "SyscallError::Panic"),
            SyscallError::InvokeContextBorrowFailed => {
                write!(f, "SyscallError::InvokeContextBorrowFailed")
            }
            SyscallError::MalformedSignerSeed(_, _) => {
                write!(f, "SyscallError::MalformedSignerSeed")
            }
            SyscallError::BadSeeds(_) => write!(f, "SyscallError::BadSeeds"),
            SyscallError::ProgramNotSupported(_) => write!(f, "SyscallError::ProgramNotSupported"),
            SyscallError::UnalignedPointer => write!(f, "SyscallError::UnalignedPointer"),
            SyscallError::TooManySigners => write!(f, "SyscallError::TooManySigners"),
            SyscallError::InstructionTooLarge(_, _) => {
                write!(f, "SyscallError::InstructionTooLarge")
            }
            SyscallError::TooManyAccounts => write!(f, "SyscallError::TooManyAccounts"),
            SyscallError::CopyOverlapping => write!(f, "SyscallError::CopyOverlapping"),
            SyscallError::ReturnDataTooLarge(_, _) => write!(f, "SyscallError::ReturnDataTooLarge"),
            SyscallError::TooManySlices => write!(f, "SyscallError::TooManySlices"),
            SyscallError::InvalidLength => write!(f, "SyscallError::InvalidLength"),
            SyscallError::MaxInstructionDataLenExceeded { .. } => {
                write!(f, "SyscallError::MaxInstructionDataLenExceeded")
            }
            SyscallError::MaxInstructionAccountsExceeded { .. } => {
                write!(f, "SyscallError::MaxInstructionAccountsExceeded")
            }
            SyscallError::MaxInstructionAccountInfosExceeded { .. } => {
                write!(f, "SyscallError::MaxInstructionAccountInfosExceeded")
            }
            SyscallError::InvalidAttribute => write!(f, "SyscallError::InvalidAttribute"),
            SyscallError::InvalidPointer => write!(f, "SyscallError::InvalidPointer"),
            SyscallError::ArithmeticOverflow => write!(f, "SyscallError::ArithmeticOverflow"),
        }
    }
}

impl core::error::Error for SyscallError {}

#[derive(Debug)]
#[repr(u64)]
pub enum RuntimeError {
    InvalidTransformation,
    InvalidLength,
    InvalidIdx,
    InvalidType,
    InvalidPrefix,
    InvalidConversion,
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
            RuntimeError::InvalidConversion => write!(f, "RuntimeError::InvalidConversion"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Secp256k1RecoverError {
    InvalidHash,
    InvalidRecoveryId,
    InvalidSignature,
}

impl From<u64> for Secp256k1RecoverError {
    fn from(v: u64) -> Secp256k1RecoverError {
        match v {
            1 => Secp256k1RecoverError::InvalidHash,
            2 => Secp256k1RecoverError::InvalidRecoveryId,
            3 => Secp256k1RecoverError::InvalidSignature,
            _ => panic!("Unsupported Secp256k1RecoverError"),
        }
    }
}

impl From<Secp256k1RecoverError> for u64 {
    fn from(v: Secp256k1RecoverError) -> u64 {
        match v {
            Secp256k1RecoverError::InvalidHash => 1,
            Secp256k1RecoverError::InvalidRecoveryId => 2,
            Secp256k1RecoverError::InvalidSignature => 3,
        }
    }
}

impl Display for Secp256k1RecoverError {
    fn fmt(&self, f: &mut Formatter<'_>) -> core::fmt::Result {
        match self {
            Secp256k1RecoverError::InvalidHash => write!(f, "Secp256k1RecoverError::InvalidHash"),
            Secp256k1RecoverError::InvalidRecoveryId => {
                write!(f, "Secp256k1RecoverError::InvalidRecoveryId")
            }
            Secp256k1RecoverError::InvalidSignature => {
                write!(f, "Secp256k1RecoverError::InvalidSignature")
            }
        }
    }
}

impl core::error::Error for Secp256k1RecoverError {}

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
    Secp256k1RecoverError(Secp256k1RecoverError),
}

impl Display for SvmError {
    fn fmt(&self, f: &mut Formatter<'_>) -> core::fmt::Result {
        match self {
            SvmError::TransactionError(e) => {
                write!(f, "SvmError::TransactionError:{}", e)
            }
            SvmError::BincodeEncodeError(e) => {
                write!(f, "SvmError::BincodeEncodeError:{}", e)
            }
            SvmError::BincodeDecodeError(e) => {
                write!(f, "SvmError::BincodeDecodeError:{}", e)
            }
            SvmError::InstructionError(e) => {
                write!(f, "SvmError::InstructionError:{}", e)
            }
            SvmError::ElfError(e) => {
                write!(f, "SvmError::ElfError:{}", e)
            }
            SvmError::EbpfError(e) => {
                write!(f, "SvmError::EbpfError:{}", e)
            }
            SvmError::SyscallError(e) => {
                write!(f, "SvmError::SyscallError:{}", e)
            }
            SvmError::RuntimeError(e) => {
                write!(f, "SvmError::RuntimeError:{}", e)
            }
            SvmError::ExitCode(e) => {
                write!(f, "SvmError::ExitCode:{}", e)
            }
            SvmError::Secp256k1RecoverError(e) => {
                write!(f, "SvmError::Secp256k1RecoverError:{}", e)
            }
        }
    }
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
            SvmError::Secp256k1RecoverError(e) => Box::new(e),
        }
    }
}
