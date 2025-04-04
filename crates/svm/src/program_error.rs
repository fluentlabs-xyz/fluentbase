use num_traits::{FromPrimitive, ToPrimitive};
use solana_program::decode_error::DecodeError;
use solana_program::program_error::CUSTOM_ZERO;
use solana_program::program_error::INVALID_ARGUMENT;
use solana_program::program_error::INVALID_INSTRUCTION_DATA;
use solana_program::program_error::INVALID_ACCOUNT_DATA;
use solana_program::program_error::ACCOUNT_DATA_TOO_SMALL;
use solana_program::program_error::INSUFFICIENT_FUNDS;
use solana_program::program_error::INCORRECT_PROGRAM_ID;
use solana_program::program_error::MISSING_REQUIRED_SIGNATURES;
use solana_program::program_error::ACCOUNT_ALREADY_INITIALIZED;
use solana_program::program_error::UNINITIALIZED_ACCOUNT;
use solana_program::program_error::NOT_ENOUGH_ACCOUNT_KEYS;
use solana_program::program_error::ACCOUNT_BORROW_FAILED;
use solana_program::program_error::MAX_SEED_LENGTH_EXCEEDED;
use solana_program::program_error::INVALID_SEEDS;
use solana_program::program_error::BORSH_IO_ERROR;
use solana_program::program_error::ACCOUNT_NOT_RENT_EXEMPT;
use solana_program::program_error::UNSUPPORTED_SYSVAR;
use solana_program::program_error::ILLEGAL_OWNER;
use solana_program::program_error::MAX_ACCOUNTS_DATA_ALLOCATIONS_EXCEEDED;
use solana_program::program_error::INVALID_ACCOUNT_DATA_REALLOC;
use solana_program::program_error::MAX_INSTRUCTION_TRACE_LENGTH_EXCEEDED;
use solana_program::program_error::BUILTIN_PROGRAMS_MUST_CONSUME_COMPUTE_UNITS;
use solana_program::program_error::INVALID_ACCOUNT_OWNER;
use solana_program::program_error::ARITHMETIC_OVERFLOW;
use crate::error::InstructionError;
use crate::alloc::string::ToString;

pub trait PrintProgramError {
    fn print<E>(&self)
    where
        E: 'static + core::error::Error + DecodeError<E> + PrintProgramError + FromPrimitive;
}

/// Builtin return values occupy the upper 32 bits
const BUILTIN_BIT_SHIFT: usize = 32;
macro_rules! to_builtin {
    ($error:expr) => {
        ($error as u64) << BUILTIN_BIT_SHIFT
    };
}

impl<T> From<T> for InstructionError
where
    T: ToPrimitive,
{
    fn from(error: T) -> Self {
        let error = error.to_u64().unwrap_or(0xbad_c0de);
        match error {
            CUSTOM_ZERO => Self::Custom(0),
            INVALID_ARGUMENT => Self::InvalidArgument,
            INVALID_INSTRUCTION_DATA => Self::InvalidInstructionData,
            INVALID_ACCOUNT_DATA => Self::InvalidAccountData,
            ACCOUNT_DATA_TOO_SMALL => Self::AccountDataTooSmall,
            INSUFFICIENT_FUNDS => Self::InsufficientFunds,
            INCORRECT_PROGRAM_ID => Self::IncorrectProgramId,
            MISSING_REQUIRED_SIGNATURES => Self::MissingRequiredSignature,
            ACCOUNT_ALREADY_INITIALIZED => Self::AccountAlreadyInitialized,
            UNINITIALIZED_ACCOUNT => Self::UninitializedAccount,
            NOT_ENOUGH_ACCOUNT_KEYS => Self::NotEnoughAccountKeys,
            ACCOUNT_BORROW_FAILED => Self::AccountBorrowFailed,
            MAX_SEED_LENGTH_EXCEEDED => Self::MaxSeedLengthExceeded,
            INVALID_SEEDS => Self::InvalidSeeds,
            BORSH_IO_ERROR => Self::BorshIoError("Unknown".to_string()),
            ACCOUNT_NOT_RENT_EXEMPT => Self::AccountNotRentExempt,
            UNSUPPORTED_SYSVAR => Self::UnsupportedSysvar,
            ILLEGAL_OWNER => Self::IllegalOwner,
            MAX_ACCOUNTS_DATA_ALLOCATIONS_EXCEEDED => Self::MaxAccountsDataAllocationsExceeded,
            INVALID_ACCOUNT_DATA_REALLOC => Self::InvalidRealloc,
            MAX_INSTRUCTION_TRACE_LENGTH_EXCEEDED => Self::MaxInstructionTraceLengthExceeded,
            BUILTIN_PROGRAMS_MUST_CONSUME_COMPUTE_UNITS => {
                Self::BuiltinProgramsMustConsumeComputeUnits
            }
            INVALID_ACCOUNT_OWNER => Self::InvalidAccountOwner,
            ARITHMETIC_OVERFLOW => Self::ArithmeticOverflow,
            _ => {
                // A valid custom error has no bits set in the upper 32
                if error >> BUILTIN_BIT_SHIFT == 0 {
                    Self::Custom(error as u32)
                } else {
                    Self::InvalidError
                }
            }
        }
    }
}
