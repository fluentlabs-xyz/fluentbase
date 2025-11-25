use alloc::{boxed::Box, string::String, vec::Vec};
use core::{
    fmt::{Display, Formatter},
    str::Utf8Error,
};
use fluentbase_sdk::{debug_log, ExitCode};
use num_derive::FromPrimitive;
use solana_decode_error::DecodeError;
use solana_instruction::error::InstructionError;
use solana_program_error::{PrintProgramError, ProgramError};
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
    fn fmt(&self, f: &mut Formatter<'_>) -> core::fmt::Result {
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
    InvalidInputValue,
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
            RuntimeError::InvalidInputValue => write!(f, "RuntimeError::InvalidInputValue"),
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
    ProgramError(ProgramError),
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
            SvmError::ProgramError(e) => {
                write!(f, "SvmError::ProgramError:{}", e)
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
            SvmError::ProgramError(e) => Box::new(e),
        }
    }
}

/// Errors that may be returned by the Token program.
#[derive(Clone, Debug, Eq, FromPrimitive, PartialEq)]
pub enum TokenError {
    // 0
    /// Lamport balance below rent-exempt threshold.
    // #[error("Lamport balance below rent-exempt threshold")]
    NotRentExempt,
    /// Insufficient funds for the operation requested.
    // #[error("Insufficient funds")]
    InsufficientFunds,
    /// Invalid Mint.
    // #[error("Invalid Mint")]
    InvalidMint,
    /// Account not associated with this Mint.
    // #[error("Account not associated with this Mint")]
    MintMismatch,
    /// Owner does not match.
    // #[error("Owner does not match")]
    OwnerMismatch,

    // 5
    /// This token's supply is fixed and new tokens cannot be minted.
    // #[error("Fixed supply")]
    FixedSupply,
    /// The account cannot be initialized because it is already being used.
    // #[error("Already in use")]
    AlreadyInUse,
    /// Invalid number of provided signers.
    // #[error("Invalid number of provided signers")]
    InvalidNumberOfProvidedSigners,
    /// Invalid number of required signers.
    // #[error("Invalid number of required signers")]
    InvalidNumberOfRequiredSigners,
    /// State is uninitialized.
    // #[error("State is uninitialized")]
    UninitializedState,

    // 10
    /// Instruction does not support native tokens
    // #[error("Instruction does not support native tokens")]
    NativeNotSupported,
    /// Non-native account can only be closed if its balance is zero
    // #[error("Non-native account can only be closed if its balance is zero")]
    NonNativeHasBalance,
    /// Invalid instruction
    // #[error("Invalid instruction")]
    InvalidInstruction,
    /// State is invalid for requested operation.
    // #[error("State is invalid for requested operation")]
    InvalidState,
    /// Operation overflowed
    // #[error("Operation overflowed")]
    Overflow,

    // 15
    /// Account does not support specified authority type.
    // #[error("Account does not support specified authority type")]
    AuthorityTypeNotSupported,
    /// This token mint cannot freeze accounts.
    // #[error("This token mint cannot freeze accounts")]
    MintCannotFreeze,
    /// Account is frozen; all account operations will fail
    // #[error("Account is frozen")]
    AccountFrozen,
    /// Mint decimals mismatch between the client and mint
    // #[error("The provided decimals value different from the Mint decimals")]
    MintDecimalsMismatch,
    /// Instruction does not support non-native tokens
    // #[error("Instruction does not support non-native tokens")]
    NonNativeNotSupported,

    // 20
    /// Extension type does not match already existing extensions
    // #[error("Extension type does not match already existing extensions")]
    ExtensionTypeMismatch,
    /// Extension does not match the base type provided
    // #[error("Extension does not match the base type provided")]
    ExtensionBaseMismatch,
    /// Extension already initialized on this account
    // #[error("Extension already initialized on this account")]
    ExtensionAlreadyInitialized,
    /// An account can only be closed if its confidential balance is zero
    // #[error("An account can only be closed if its confidential balance is zero")]
    ConfidentialTransferAccountHasBalance,
    /// Account not approved for confidential transfers
    // #[error("Account not approved for confidential transfers")]
    ConfidentialTransferAccountNotApproved,

    // 25
    /// Account not accepting deposits or transfers
    // #[error("Account not accepting deposits or transfers")]
    ConfidentialTransferDepositsAndTransfersDisabled,
    /// ElGamal public key mismatch
    // #[error("ElGamal public key mismatch")]
    ConfidentialTransferElGamalPubkeyMismatch,
    /// Balance mismatch
    // #[error("Balance mismatch")]
    ConfidentialTransferBalanceMismatch,
    /// Mint has non-zero supply. Burn all tokens before closing the mint.
    // #[error("Mint has non-zero supply. Burn all tokens before closing the mint")]
    MintHasSupply,
    /// No authority exists to perform the desired operation
    // #[error("No authority exists to perform the desired operation")]
    NoAuthorityExists,

    // 30
    /// Transfer fee exceeds maximum of 10,000 basis points
    // #[error("Transfer fee exceeds maximum of 10,000 basis points")]
    TransferFeeExceedsMaximum,
    /// Mint required for this account to transfer tokens, use
    /// `transfer_checked` or `transfer_checked_with_fee`
    // #[error("Mint required for this account to transfer tokens, use `transfer_checked` or `transfer_checked_with_fee`")]
    MintRequiredForTransfer,
    /// Calculated fee does not match expected fee
    // #[error("Calculated fee does not match expected fee")]
    FeeMismatch,
    /// Fee parameters associated with confidential transfer zero-knowledge
    /// proofs do not match fee parameters in mint
    // #[error(
    //     "Fee parameters associated with zero-knowledge proofs do not match fee parameters in mint"
    // )]
    FeeParametersMismatch,
    /// The owner authority cannot be changed
    // #[error("The owner authority cannot be changed")]
    ImmutableOwner,

    // 35
    /// An account can only be closed if its withheld fee balance is zero,
    /// harvest fees to the mint and try again
    // #[error("An account can only be closed if its withheld fee balance is zero, harvest fees to the mint and try again")]
    AccountHasWithheldTransferFees,
    /// No memo in previous instruction; required for recipient to receive a
    /// transfer
    // #[error("No memo in previous instruction; required for recipient to receive a transfer")]
    NoMemo,
    /// Transfer is disabled for this mint
    // #[error("Transfer is disabled for this mint")]
    NonTransferable,
    /// Non-transferable tokens can't be minted to an account without immutable
    /// ownership
    // #[error("Non-transferable tokens can't be minted to an account without immutable ownership")]
    NonTransferableNeedsImmutableOwnership,
    /// The total number of `Deposit` and `Transfer` instructions to an account
    /// cannot exceed the associated
    /// `maximum_pending_balance_credit_counter`
    // #[error(
    //     "The total number of `Deposit` and `Transfer` instructions to an account cannot exceed
    //         the associated `maximum_pending_balance_credit_counter`"
    // )]
    MaximumPendingBalanceCreditCounterExceeded,

    // 40
    /// The deposit amount for the confidential extension exceeds the maximum
    /// limit
    // #[error("Deposit amount exceeds maximum limit")]
    MaximumDepositAmountExceeded,
    /// CPI Guard cannot be enabled or disabled in CPI
    // #[error("CPI Guard cannot be enabled or disabled in CPI")]
    CpiGuardSettingsLocked,
    /// CPI Guard is enabled, and a program attempted to transfer user funds
    /// without using a delegate
    // #[error("CPI Guard is enabled, and a program attempted to transfer user funds via CPI without using a delegate")]
    CpiGuardTransferBlocked,
    /// CPI Guard is enabled, and a program attempted to burn user funds without
    /// using a delegate
    // #[error(
    //     "CPI Guard is enabled, and a program attempted to burn user funds via CPI without using a delegate"
    // )]
    CpiGuardBurnBlocked,
    /// CPI Guard is enabled, and a program attempted to close an account
    /// without returning lamports to owner
    // #[error("CPI Guard is enabled, and a program attempted to close an account via CPI without returning lamports to owner")]
    CpiGuardCloseAccountBlocked,

    // 45
    /// CPI Guard is enabled, and a program attempted to approve a delegate
    // #[error("CPI Guard is enabled, and a program attempted to approve a delegate via CPI")]
    CpiGuardApproveBlocked,
    /// CPI Guard is enabled, and a program attempted to add or replace an
    /// authority
    // #[error(
    //     "CPI Guard is enabled, and a program attempted to add or replace an authority via CPI"
    // )]
    CpiGuardSetAuthorityBlocked,
    /// Account ownership cannot be changed while CPI Guard is enabled
    // #[error("Account ownership cannot be changed while CPI Guard is enabled")]
    CpiGuardOwnerChangeBlocked,
    /// Extension not found in account data
    // #[error("Extension not found in account data")]
    ExtensionNotFound,
    /// Account does not accept non-confidential transfers
    // #[error("Non-confidential transfers disabled")]
    NonConfidentialTransfersDisabled,

    // 50
    /// An account can only be closed if the confidential withheld fee is zero
    // #[error("An account can only be closed if the confidential withheld fee is zero")]
    ConfidentialTransferFeeAccountHasWithheldFee,
    /// A mint or an account is initialized to an invalid combination of
    /// extensions
    // #[error("A mint or an account is initialized to an invalid combination of extensions")]
    InvalidExtensionCombination,
    /// Extension allocation with overwrite must use the same length
    // #[error("Extension allocation with overwrite must use the same length")]
    InvalidLengthForAlloc,
    /// Failed to decrypt a confidential transfer account
    // #[error("Failed to decrypt a confidential transfer account")]
    AccountDecryption,
    /// Failed to generate a zero-knowledge proof needed for a token instruction
    // #[error("Failed to generate proof")]
    ProofGeneration,

    // 55
    /// An invalid proof instruction offset was provided
    // #[error("An invalid proof instruction offset was provided")]
    InvalidProofInstructionOffset,
    /// Harvest of withheld tokens to mint is disabled
    // #[error("Harvest of withheld tokens to mint is disabled")]
    HarvestToMintDisabled,
    /// Split proof context state accounts not supported for instruction
    // #[error("Split proof context state accounts not supported for instruction")]
    SplitProofContextStateAccountsNotSupported,
    /// Not enough proof context state accounts provided
    // #[error("Not enough proof context state accounts provided")]
    NotEnoughProofContextStateAccounts,
    /// Ciphertext is malformed
    // #[error("Ciphertext is malformed")]
    MalformedCiphertext,

    // 60
    /// Ciphertext arithmetic failed
    // #[error("Ciphertext arithmetic failed")]
    CiphertextArithmeticFailed,
    /// Pedersen commitments did not match
    // #[error("Pedersen commitment mismatch")]
    PedersenCommitmentMismatch,
    /// Range proof length did not match
    // #[error("Range proof length mismatch")]
    RangeProofLengthMismatch,
    /// Illegal transfer amount bit length
    // #[error("Illegal transfer amount bit length")]
    IllegalBitLength,
    /// Fee calculation failed
    // #[error("Fee calculation failed")]
    FeeCalculation,

    //65
    /// Withdraw / Deposit not allowed for confidential-mint-burn
    // #[error("Withdraw / Deposit not allowed for confidential-mint-burn")]
    IllegalMintBurnConversion,
}
// impl From<TokenError> for Error {
//     fn from(e: TokenError) -> Self {
//         match e {
//             TokenError::NotRentExempt => Box::new(e),
//             TokenError::InsufficientFunds => Box::new(e),
//             TokenError::InvalidMint => Box::new(e),
//             TokenError::MintMismatch => Box::new(e),
//             TokenError::OwnerMismatch => Box::new(e),
//             TokenError::FixedSupply => Box::new(e),
//             TokenError::AlreadyInUse => Box::new(e),
//             TokenError::InvalidNumberOfProvidedSigners => Box::new(e),
//             TokenError::InvalidNumberOfRequiredSigners => Box::new(e),
//             TokenError::UninitializedState => Box::new(e),
//             TokenError::NativeNotSupported => Box::new(e),
//             TokenError::NonNativeHasBalance => Box::new(e),
//             TokenError::InvalidInstruction => Box::new(e),
//             TokenError::InvalidState => Box::new(e),
//             TokenError::Overflow => Box::new(e),
//             TokenError::AuthorityTypeNotSupported => Box::new(e),
//             TokenError::MintCannotFreeze => Box::new(e),
//             TokenError::AccountFrozen => Box::new(e),
//             TokenError::MintDecimalsMismatch => Box::new(e),
//             TokenError::NonNativeNotSupported => Box::new(e),
//             TokenError::ExtensionTypeMismatch => Box::new(e),
//             TokenError::ExtensionBaseMismatch => Box::new(e),
//             TokenError::ExtensionAlreadyInitialized => Box::new(e),
//             TokenError::ConfidentialTransferAccountHasBalance => Box::new(e),
//             TokenError::ConfidentialTransferAccountNotApproved => Box::new(e),
//             TokenError::ConfidentialTransferDepositsAndTransfersDisabled => Box::new(e),
//             TokenError::ConfidentialTransferElGamalPubkeyMismatch => Box::new(e),
//             TokenError::ConfidentialTransferBalanceMismatch => Box::new(e),
//             TokenError::MintHasSupply => Box::new(e),
//             TokenError::NoAuthorityExists => Box::new(e),
//             TokenError::TransferFeeExceedsMaximum => Box::new(e),
//             TokenError::MintRequiredForTransfer => Box::new(e),
//             TokenError::FeeMismatch => Box::new(e),
//             TokenError::FeeParametersMismatch => Box::new(e),
//             TokenError::ImmutableOwner => Box::new(e),
//             TokenError::AccountHasWithheldTransferFees => Box::new(e),
//             TokenError::NoMemo => Box::new(e),
//             TokenError::NonTransferable => Box::new(e),
//             TokenError::NonTransferableNeedsImmutableOwnership => Box::new(e),
//             TokenError::MaximumPendingBalanceCreditCounterExceeded => Box::new(e),
//             TokenError::MaximumDepositAmountExceeded => Box::new(e),
//             TokenError::CpiGuardSettingsLocked => Box::new(e),
//             TokenError::CpiGuardTransferBlocked => Box::new(e),
//             TokenError::CpiGuardBurnBlocked => Box::new(e),
//             TokenError::CpiGuardCloseAccountBlocked => Box::new(e),
//             TokenError::CpiGuardApproveBlocked => Box::new(e),
//             TokenError::CpiGuardSetAuthorityBlocked => Box::new(e),
//             TokenError::CpiGuardOwnerChangeBlocked => Box::new(e),
//             TokenError::ExtensionNotFound => Box::new(e),
//             TokenError::NonConfidentialTransfersDisabled => Box::new(e),
//             TokenError::ConfidentialTransferFeeAccountHasWithheldFee => Box::new(e),
//             TokenError::InvalidExtensionCombination => Box::new(e),
//             TokenError::InvalidLengthForAlloc => Box::new(e),
//             TokenError::AccountDecryption => Box::new(e),
//             TokenError::ProofGeneration => Box::new(e),
//             TokenError::InvalidProofInstructionOffset => Box::new(e),
//             TokenError::HarvestToMintDisabled => Box::new(e),
//             TokenError::SplitProofContextStateAccountsNotSupported => Box::new(e),
//             TokenError::NotEnoughProofContextStateAccounts => Box::new(e),
//             TokenError::MalformedCiphertext => Box::new(e),
//             TokenError::CiphertextArithmeticFailed => Box::new(e),
//             TokenError::PedersenCommitmentMismatch => Box::new(e),
//             TokenError::RangeProofLengthMismatch => Box::new(e),
//             TokenError::IllegalBitLength => Box::new(e),
//             TokenError::FeeCalculation => Box::new(e),
//             TokenError::IllegalMintBurnConversion => Box::new(e),
//         }
//     }
// }
impl Display for TokenError {
    fn fmt(&self, f: &mut Formatter<'_>) -> core::fmt::Result {
        match self {
            TokenError::NotRentExempt => write!(f, "TokenError::NotRentExempt"),
            TokenError::InsufficientFunds => write!(f, "TokenError::InsufficientFunds"),
            TokenError::InvalidMint => write!(f, "TokenError::InvalidMint"),
            TokenError::MintMismatch => write!(f, "TokenError::MintMismatch"),
            TokenError::OwnerMismatch => write!(f, "TokenError::OwnerMismatch"),
            TokenError::FixedSupply => write!(f, "TokenError::FixedSupply"),
            TokenError::AlreadyInUse => write!(f, "TokenError::AlreadyInUse"),
            TokenError::InvalidNumberOfProvidedSigners => {
                write!(f, "TokenError::InvalidNumberOfProvidedSigners")
            }
            TokenError::InvalidNumberOfRequiredSigners => {
                write!(f, "TokenError::InvalidNumberOfRequiredSigners")
            }
            TokenError::UninitializedState => write!(f, "TokenError::UninitializedState"),
            TokenError::NativeNotSupported => write!(f, "TokenError::NativeNotSupported"),
            TokenError::NonNativeHasBalance => write!(f, "TokenError::NonNativeHasBalance"),
            TokenError::InvalidInstruction => write!(f, "TokenError::InvalidInstruction"),
            TokenError::InvalidState => write!(f, "TokenError::InvalidState"),
            TokenError::Overflow => write!(f, "TokenError::Overflow"),
            TokenError::AuthorityTypeNotSupported => {
                write!(f, "TokenError::AuthorityTypeNotSupported")
            }
            TokenError::MintCannotFreeze => write!(f, "TokenError::MintCannotFreeze"),
            TokenError::AccountFrozen => write!(f, "TokenError::AccountFrozen"),
            TokenError::MintDecimalsMismatch => write!(f, "TokenError::MintDecimalsMismatch"),
            TokenError::NonNativeNotSupported => write!(f, "TokenError::NonNativeNotSupported"),
            TokenError::ExtensionTypeMismatch => write!(f, "TokenError::ExtensionTypeMismatch"),
            TokenError::ExtensionBaseMismatch => write!(f, "TokenError::ExtensionBaseMismatch"),
            TokenError::ExtensionAlreadyInitialized => {
                write!(f, "TokenError::ExtensionAlreadyInitialized")
            }
            TokenError::ConfidentialTransferAccountHasBalance => {
                write!(f, "TokenError::ConfidentialTransferAccountHasBalance")
            }
            TokenError::ConfidentialTransferAccountNotApproved => {
                write!(f, "TokenError::ConfidentialTransferAccountNotApproved")
            }
            TokenError::ConfidentialTransferDepositsAndTransfersDisabled => write!(
                f,
                "TokenError::ConfidentialTransferDepositsAndTransfersDisabled"
            ),
            TokenError::ConfidentialTransferElGamalPubkeyMismatch => {
                write!(f, "TokenError::ConfidentialTransferElGamalPubkeyMismatch")
            }
            TokenError::ConfidentialTransferBalanceMismatch => {
                write!(f, "TokenError::ConfidentialTransferBalanceMismatch")
            }
            TokenError::MintHasSupply => write!(f, "TokenError::MintHasSupply"),
            TokenError::NoAuthorityExists => write!(f, "TokenError::NoAuthorityExists"),
            TokenError::TransferFeeExceedsMaximum => {
                write!(f, "TokenError::TransferFeeExceedsMaximum")
            }
            TokenError::MintRequiredForTransfer => write!(f, "TokenError::MintRequiredForTransfer"),
            TokenError::FeeMismatch => write!(f, "TokenError::FeeMismatch"),
            TokenError::FeeParametersMismatch => write!(f, "TokenError::FeeParametersMismatch"),
            TokenError::ImmutableOwner => write!(f, "TokenError::ImmutableOwner"),
            TokenError::AccountHasWithheldTransferFees => {
                write!(f, "TokenError::AccountHasWithheldTransferFees")
            }
            TokenError::NoMemo => write!(f, "TokenError::NoMemo"),
            TokenError::NonTransferable => write!(f, "TokenError::NonTransferable"),
            TokenError::NonTransferableNeedsImmutableOwnership => {
                write!(f, "TokenError::NonTransferableNeedsImmutableOwnership")
            }
            TokenError::MaximumPendingBalanceCreditCounterExceeded => {
                write!(f, "TokenError::MaximumPendingBalanceCreditCounterExceeded")
            }
            TokenError::MaximumDepositAmountExceeded => {
                write!(f, "TokenError::MaximumDepositAmountExceeded")
            }
            TokenError::CpiGuardSettingsLocked => write!(f, "TokenError::CpiGuardSettingsLocked"),
            TokenError::CpiGuardTransferBlocked => write!(f, "TokenError::CpiGuardTransferBlocked"),
            TokenError::CpiGuardBurnBlocked => write!(f, "TokenError::CpiGuardBurnBlocked"),
            TokenError::CpiGuardCloseAccountBlocked => {
                write!(f, "TokenError::CpiGuardCloseAccountBlocked")
            }
            TokenError::CpiGuardApproveBlocked => write!(f, "TokenError::CpiGuardApproveBlocked"),
            TokenError::CpiGuardSetAuthorityBlocked => {
                write!(f, "TokenError::CpiGuardSetAuthorityBlocked")
            }
            TokenError::CpiGuardOwnerChangeBlocked => {
                write!(f, "TokenError::CpiGuardOwnerChangeBlocked")
            }
            TokenError::ExtensionNotFound => write!(f, "TokenError::ExtensionNotFound"),
            TokenError::NonConfidentialTransfersDisabled => {
                write!(f, "TokenError::NonConfidentialTransfersDisabled")
            }
            TokenError::ConfidentialTransferFeeAccountHasWithheldFee => write!(
                f,
                "TokenError::ConfidentialTransferFeeAccountHasWithheldFee"
            ),
            TokenError::InvalidExtensionCombination => {
                write!(f, "TokenError::InvalidExtensionCombination")
            }
            TokenError::InvalidLengthForAlloc => write!(f, "TokenError::InvalidLengthForAlloc"),
            TokenError::AccountDecryption => write!(f, "TokenError::AccountDecryption"),
            TokenError::ProofGeneration => write!(f, "TokenError::ProofGeneration"),
            TokenError::InvalidProofInstructionOffset => {
                write!(f, "TokenError::InvalidProofInstructionOffset")
            }
            TokenError::HarvestToMintDisabled => write!(f, "TokenError::HarvestToMintDisabled"),
            TokenError::SplitProofContextStateAccountsNotSupported => {
                write!(f, "TokenError::SplitProofContextStateAccountsNotSupported")
            }
            TokenError::NotEnoughProofContextStateAccounts => {
                write!(f, "TokenError::NotEnoughProofContextStateAccounts")
            }
            TokenError::MalformedCiphertext => write!(f, "TokenError::MalformedCiphertext"),
            TokenError::CiphertextArithmeticFailed => {
                write!(f, "TokenError::CiphertextArithmeticFailed")
            }
            TokenError::PedersenCommitmentMismatch => {
                write!(f, "TokenError::PedersenCommitmentMismatch")
            }
            TokenError::RangeProofLengthMismatch => {
                write!(f, "TokenError::RangeProofLengthMismatch")
            }
            TokenError::IllegalBitLength => write!(f, "TokenError::IllegalBitLength"),
            TokenError::FeeCalculation => write!(f, "TokenError::FeeCalculation"),
            TokenError::IllegalMintBurnConversion => {
                write!(f, "TokenError::IllegalMintBurnConversion")
            }
        }
    }
}
impl core::error::Error for TokenError {}
impl From<TokenError> for ProgramError {
    fn from(e: TokenError) -> Self {
        ProgramError::Custom(e as u32)
    }
}
impl<T> DecodeError<T> for TokenError {
    fn type_of() -> &'static str {
        "TokenError"
    }
}

impl PrintProgramError for TokenError {
    fn print<E>(&self)
    where
        E: 'static + core::error::Error + DecodeError<E> + num_traits::FromPrimitive,
    {
        match self {
            TokenError::NotRentExempt => {
                debug_log!("Error: Lamport balance below rent-exempt threshold")
            }
            TokenError::InsufficientFunds => debug_log!("Error: insufficient funds"),
            TokenError::InvalidMint => debug_log!("Error: Invalid Mint"),
            TokenError::MintMismatch => debug_log!("Error: Account not associated with this Mint"),
            TokenError::OwnerMismatch => debug_log!("Error: owner does not match"),
            TokenError::FixedSupply => debug_log!("Error: the total supply of this token is fixed"),
            TokenError::AlreadyInUse => debug_log!("Error: account or token already in use"),
            TokenError::InvalidNumberOfProvidedSigners => {
                debug_log!("Error: Invalid number of provided signers")
            }
            TokenError::InvalidNumberOfRequiredSigners => {
                debug_log!("Error: Invalid number of required signers")
            }
            TokenError::UninitializedState => debug_log!("Error: State is uninitialized"),
            TokenError::NativeNotSupported => {
                debug_log!("Error: Instruction does not support native tokens")
            }
            TokenError::NonNativeHasBalance => {
                debug_log!("Error: Non-native account can only be closed if its balance is zero")
            }
            TokenError::InvalidInstruction => debug_log!("Error: Invalid instruction"),
            TokenError::InvalidState => debug_log!("Error: Invalid account state for operation"),
            TokenError::Overflow => debug_log!("Error: Operation overflowed"),
            TokenError::AuthorityTypeNotSupported => {
                debug_log!("Error: Account does not support specified authority type")
            }
            TokenError::MintCannotFreeze => {
                debug_log!("Error: This token mint cannot freeze accounts")
            }
            TokenError::AccountFrozen => debug_log!("Error: Account is frozen"),
            TokenError::MintDecimalsMismatch => {
                debug_log!("Error: decimals different from the Mint decimals")
            }
            TokenError::NonNativeNotSupported => {
                debug_log!("Error: Instruction does not support non-native tokens")
            }
            TokenError::ExtensionTypeMismatch => {
                debug_log!("Error: New extension type does not match already existing extensions")
            }
            TokenError::ExtensionBaseMismatch => {
                debug_log!("Error: Extension does not match the base type provided")
            }
            TokenError::ExtensionAlreadyInitialized => {
                debug_log!("Error: Extension already initialized on this account")
            }
            TokenError::ConfidentialTransferAccountHasBalance => {
                debug_log!(
                    "Error: An account can only be closed if its confidential balance is zero"
                )
            }
            TokenError::ConfidentialTransferAccountNotApproved => {
                debug_log!("Error: Account not approved for confidential transfers")
            }
            TokenError::ConfidentialTransferDepositsAndTransfersDisabled => {
                debug_log!("Error: Account not accepting deposits or transfers")
            }
            TokenError::ConfidentialTransferElGamalPubkeyMismatch => {
                debug_log!("Error: ElGamal public key mismatch")
            }
            TokenError::ConfidentialTransferBalanceMismatch => {
                debug_log!("Error: Balance mismatch")
            }
            TokenError::MintHasSupply => {
                debug_log!(
                    "Error: Mint has non-zero supply. Burn all tokens before closing the mint"
                )
            }
            TokenError::NoAuthorityExists => {
                debug_log!("Error: No authority exists to perform the desired operation");
            }
            TokenError::TransferFeeExceedsMaximum => {
                debug_log!("Error: Transfer fee exceeds maximum of 10,000 basis points");
            }
            TokenError::MintRequiredForTransfer => {
                debug_log!("Mint required for this account to transfer tokens, use `transfer_checked` or `transfer_checked_with_fee`");
            }
            TokenError::FeeMismatch => {
                debug_log!("Calculated fee does not match expected fee");
            }
            TokenError::FeeParametersMismatch => {
                debug_log!("Fee parameters associated with zero-knowledge proofs do not match fee parameters in mint")
            }
            TokenError::ImmutableOwner => {
                debug_log!("The owner authority cannot be changed");
            }
            TokenError::AccountHasWithheldTransferFees => {
                debug_log!("Error: An account can only be closed if its withheld fee balance is zero, harvest fees to the mint and try again");
            }
            TokenError::NoMemo => {
                debug_log!("Error: No memo in previous instruction; required for recipient to receive a transfer");
            }
            TokenError::NonTransferable => {
                debug_log!("Transfer is disabled for this mint");
            }
            TokenError::NonTransferableNeedsImmutableOwnership => {
                debug_log!("Non-transferable tokens can't be minted to an account without immutable ownership");
            }
            TokenError::MaximumPendingBalanceCreditCounterExceeded => {
                debug_log!("The total number of `Deposit` and `Transfer` instructions to an account cannot exceed the associated `maximum_pending_balance_credit_counter`");
            }
            TokenError::MaximumDepositAmountExceeded => {
                debug_log!("Deposit amount exceeds maximum limit")
            }
            TokenError::CpiGuardSettingsLocked => {
                debug_log!("CPI Guard status cannot be changed in CPI")
            }
            TokenError::CpiGuardTransferBlocked => {
                debug_log!("CPI Guard is enabled, and a program attempted to transfer user funds without using a delegate")
            }
            TokenError::CpiGuardBurnBlocked => {
                debug_log!("CPI Guard is enabled, and a program attempted to burn user funds without using a delegate")
            }
            TokenError::CpiGuardCloseAccountBlocked => {
                debug_log!("CPI Guard is enabled, and a program attempted to close an account without returning lamports to owner")
            }
            TokenError::CpiGuardApproveBlocked => {
                debug_log!("CPI Guard is enabled, and a program attempted to approve a delegate")
            }
            TokenError::CpiGuardSetAuthorityBlocked => {
                debug_log!(
                    "CPI Guard is enabled, and a program attempted to add or change an authority"
                )
            }
            TokenError::CpiGuardOwnerChangeBlocked => {
                debug_log!("Account ownership cannot be changed while CPI Guard is enabled")
            }
            TokenError::ExtensionNotFound => {
                debug_log!("Extension not found in account data")
            }
            TokenError::NonConfidentialTransfersDisabled => {
                debug_log!("Non-confidential transfers disabled")
            }
            TokenError::ConfidentialTransferFeeAccountHasWithheldFee => {
                debug_log!("Account has non-zero confidential withheld fee")
            }
            TokenError::InvalidExtensionCombination => {
                debug_log!("Mint or account is initialized to an invalid combination of extensions")
            }
            TokenError::InvalidLengthForAlloc => {
                debug_log!("Extension allocation with overwrite must use the same length")
            }
            TokenError::AccountDecryption => {
                debug_log!("Failed to decrypt a confidential transfer account")
            }
            TokenError::ProofGeneration => {
                debug_log!("Failed to generate proof")
            }
            TokenError::InvalidProofInstructionOffset => {
                debug_log!("An invalid proof instruction offset was provided")
            }
            TokenError::HarvestToMintDisabled => {
                debug_log!("Harvest of withheld tokens to mint is disabled")
            }
            TokenError::SplitProofContextStateAccountsNotSupported => {
                debug_log!("Split proof context state accounts not supported for instruction")
            }
            TokenError::NotEnoughProofContextStateAccounts => {
                debug_log!("Not enough proof context state accounts provided")
            }
            TokenError::MalformedCiphertext => {
                debug_log!("Ciphertext is malformed")
            }
            TokenError::CiphertextArithmeticFailed => {
                debug_log!("Ciphertext arithmetic failed")
            }
            TokenError::PedersenCommitmentMismatch => {
                debug_log!("Pedersen commitments did not match")
            }
            TokenError::RangeProofLengthMismatch => {
                debug_log!("Range proof lengths did not match")
            }
            TokenError::IllegalBitLength => {
                debug_log!("Illegal transfer amount bit length")
            }
            TokenError::FeeCalculation => {
                debug_log!("Transfer fee calculation failed")
            }
            TokenError::IllegalMintBurnConversion => {
                debug_log!("Conversions from normal to confidential token balance and vice versa are illegal if the confidential-mint-burn extension is enabled")
            }
        }
    }
}
