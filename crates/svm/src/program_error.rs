pub(crate) use crate::solana_program::{
    // decode_error::DecodeError,
    program_error::UNSUPPORTED_SYSVAR,
};
// pub trait PrintProgramError {
//     fn print<E>(&self)
//     where
//         E: 'static + core::error::Error + DecodeError<E> + PrintProgramError + FromPrimitive;
// }

// /// Builtin return values occupy the upper 32 bits
// const BUILTIN_BIT_SHIFT: usize = 32;
// macro_rules! to_builtin {
//     ($error:expr) => {
//         ($error as u64) << BUILTIN_BIT_SHIFT
//     };
// }

// impl<T> From<T> for InstructionError
// where
//     T: ToPrimitive,
// {
//     fn from(error: T) -> Self {
//         let error = error.to_u64().unwrap_or(0xbad_c0de);
//         match error {
//             CUSTOM_ZERO => Self::Custom(0),
//             INVALID_ARGUMENT => Self::InvalidArgument,
//             INVALID_INSTRUCTION_DATA => Self::InvalidInstructionData,
//             INVALID_ACCOUNT_DATA => Self::InvalidAccountData,
//             ACCOUNT_DATA_TOO_SMALL => Self::AccountDataTooSmall,
//             INSUFFICIENT_FUNDS => Self::InsufficientFunds,
//             INCORRECT_PROGRAM_ID => Self::IncorrectProgramId,
//             MISSING_REQUIRED_SIGNATURES => Self::MissingRequiredSignature,
//             ACCOUNT_ALREADY_INITIALIZED => Self::AccountAlreadyInitialized,
//             UNINITIALIZED_ACCOUNT => Self::UninitializedAccount,
//             NOT_ENOUGH_ACCOUNT_KEYS => Self::NotEnoughAccountKeys,
//             ACCOUNT_BORROW_FAILED => Self::AccountBorrowFailed,
//             MAX_SEED_LENGTH_EXCEEDED => Self::MaxSeedLengthExceeded,
//             INVALID_SEEDS => Self::InvalidSeeds,
//             BORSH_IO_ERROR => Self::BorshIoError("Unknown".to_string()),
//             ACCOUNT_NOT_RENT_EXEMPT => Self::AccountNotRentExempt,
//             UNSUPPORTED_SYSVAR => Self::UnsupportedSysvar,
//             ILLEGAL_OWNER => Self::IllegalOwner,
//             MAX_ACCOUNTS_DATA_ALLOCATIONS_EXCEEDED => Self::MaxAccountsDataAllocationsExceeded,
//             INVALID_ACCOUNT_DATA_REALLOC => Self::InvalidRealloc,
//             MAX_INSTRUCTION_TRACE_LENGTH_EXCEEDED => Self::MaxInstructionTraceLengthExceeded,
//             BUILTIN_PROGRAMS_MUST_CONSUME_COMPUTE_UNITS => {
//                 Self::BuiltinProgramsMustConsumeComputeUnits
//             }
//             INVALID_ACCOUNT_OWNER => Self::InvalidAccountOwner,
//             ARITHMETIC_OVERFLOW => Self::ArithmeticOverflow,
//             _ => {
//                 // A valid custom error has no bits set in the upper 32
//                 if error >> BUILTIN_BIT_SHIFT == 0 {
//                     Self::Custom(error as u32)
//                 } else {
//                     Self::InvalidError
//                 }
//             }
//         }
//     }
// }
