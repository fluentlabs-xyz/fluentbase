use alloc::string::String;
use core::fmt::{Display, Formatter};
use fluentbase_sdk::ExitCode;
use serde::{Deserialize, Serialize};

/// Reasons the runtime might have rejected an instruction.
///
/// Members of this enum must not be removed, but new ones can be added.
/// Also, it is crucial that meta-information if any that comes along with
/// an error be consistent across software versions.  For example, it is
/// dangerous to include error strings from 3rd party crates because they could
/// change at any time and changes to them are difficult to detect.
#[derive(Serialize, Deserialize, Debug, /*Error, */ PartialEq, Eq, Clone)]
pub enum InstructionError {
    /// Deprecated! Use CustomError instead!
    /// The program instruction returned an error
    // #[error("generic instruction error")]
    GenericError,

    /// The arguments provided to a program were invalid
    // #[error("invalid program argument")]
    InvalidArgument,

    /// An instruction's data contents were invalid
    // #[error("invalid instruction data")]
    InvalidInstructionData,

    /// An account's data contents was invalid
    // #[error("invalid account data for instruction")]
    InvalidAccountData,

    /// An account's data was too small
    // #[error("account data too small for instruction")]
    AccountDataTooSmall,

    /// An account's balance was too small to complete the instruction
    // #[error("insufficient funds for instruction")]
    InsufficientFunds,

    /// The account did not have the expected program id
    // #[error("incorrect program id for instruction")]
    IncorrectProgramId,

    /// A signature was required but not found
    // #[error("missing required signature for instruction")]
    MissingRequiredSignature,

    /// An initialize instruction was sent to an account that has already been initialized.
    // #[error("instruction requires an uninitialized account")]
    AccountAlreadyInitialized,

    /// An attempt to operate on an account that hasn't been initialized.
    // #[error("instruction requires an initialized account")]
    UninitializedAccount,

    /// Program's instruction lamport balance does not equal the balance after the instruction
    // #[error("sum of account balances before and after instruction do not match")]
    UnbalancedInstruction,

    /// Program illegally modified an account's program id
    // #[error("instruction illegally modified the program id of an account")]
    ModifiedProgramId,

    /// Program spent the lamports of an account that doesn't belong to it
    // #[error("instruction spent from the balance of an account it does not own")]
    ExternalAccountLamportSpend,

    /// Program modified the data of an account that doesn't belong to it
    // #[error("instruction modified data of an account it does not own")]
    ExternalAccountDataModified,

    /// Read-only account's lamports modified
    // #[error("instruction changed the balance of a read-only account")]
    ReadonlyLamportChange,

    /// Read-only account's data was modified
    // #[error("instruction modified data of a read-only account")]
    ReadonlyDataModified,

    /// An account was referenced more than once in a single instruction
    // Deprecated, instructions can now contain duplicate accounts
    // #[error("instruction contains duplicate accounts")]
    DuplicateAccountIndex,

    /// Executable bit on account changed, but shouldn't have
    // #[error("instruction changed executable bit of an account")]
    ExecutableModified,

    /// Rent_epoch account changed, but shouldn't have
    // #[error("instruction modified rent epoch of an account")]
    RentEpochModified,

    /// The instruction expected additional account keys
    // #[error("insufficient account keys for instruction")]
    NotEnoughAccountKeys,

    /// Program other than the account's owner changed the size of the account data
    // #[error("program other than the account's owner changed the size of the account data")]
    AccountDataSizeChanged,

    /// The instruction expected an executable account
    // #[error("instruction expected an executable account")]
    AccountNotExecutable,

    /// Failed to borrow a reference to account data, already borrowed
    // #[error("instruction tries to borrow reference for an account which is already borrowed")]
    AccountBorrowFailed,

    /// Account data has an outstanding reference after a program's execution
    // #[error("instruction left account with an outstanding borrowed reference")]
    AccountBorrowOutstanding,

    /// The same account was multiply passed to an on-chain program's entrypoint, but the program
    /// modified them differently.  A program can only modify one instance of the account because
    /// the runtime cannot determine which changes to pick or how to merge them if both are modified
    // #[error("instruction modifications of multiply-passed account differ")]
    DuplicateAccountOutOfSync,

    /// Allows on-chain programs to implement program-specific error types and see them returned
    /// by the Solana runtime. A program-specific error may be any type that is represented as
    /// or serialized to a u32 integer.
    // #[error("custom program error: {0:#x}")]
    Custom(u32),

    /// The return value from the program was invalid.  Valid errors are either a defined builtin
    /// error value or a user-defined error in the lower 32 bits.
    // #[error("program returned invalid error code")]
    InvalidError,

    /// Executable account's data was modified
    // #[error("instruction changed executable accounts data")]
    ExecutableDataModified,

    /// Executable account's lamports modified
    // #[error("instruction changed the balance of an executable account")]
    ExecutableLamportChange,

    /// Executable accounts must be rent exempt
    // #[error("executable accounts must be rent exempt")]
    ExecutableAccountNotRentExempt,

    /// Unsupported program id
    // #[error("Unsupported program id")]
    UnsupportedProgramId,

    /// Cross-program invocation call depth too deep
    // #[error("Cross-program invocation call depth too deep")]
    CallDepth,

    /// An account required by the instruction is missing
    // #[error("An account required by the instruction is missing")]
    MissingAccount,

    /// Cross-program invocation reentrancy not allowed for this instruction
    // #[error("Cross-program invocation reentrancy not allowed for this instruction")]
    ReentrancyNotAllowed,

    /// Length of the seed is too long for address generation
    // #[error("Length of the seed is too long for address generation")]
    MaxSeedLengthExceeded,

    /// Provided seeds do not result in a valid address
    // #[error("Provided seeds do not result in a valid address")]
    InvalidSeeds,

    /// Failed to reallocate account data of this length
    // #[error("Failed to reallocate account data")]
    InvalidRealloc,

    /// Computational budget exceeded
    // #[error("Computational budget exceeded")]
    ComputationalBudgetExceeded,

    /// Cross-program invocation with unauthorized signer or writable account
    // #[error("Cross-program invocation with unauthorized signer or writable account")]
    PrivilegeEscalation,

    /// Failed to create program execution environment
    // #[error("Failed to create program execution environment")]
    ProgramEnvironmentSetupFailure,

    /// Program failed to complete
    // #[error("Program failed to complete")]
    ProgramFailedToComplete,

    /// Program failed to compile
    // #[error("Program failed to compile")]
    ProgramFailedToCompile,

    /// Account is immutable
    // #[error("Account is immutable")]
    Immutable,

    /// Incorrect authority provided
    // #[error("Incorrect authority provided")]
    IncorrectAuthority,

    /// Failed to serialize or deserialize account data
    ///
    /// Warning: This error should never be emitted by the runtime.
    ///
    /// This error includes strings from the underlying 3rd party Borsh crate
    /// which can be dangerous because the error strings could change across
    /// Borsh versions. Only programs can use this error because they are
    /// consistent across Solana software versions.
    ///
    // #[error("Failed to serialize or deserialize account data: {0}")]
    BorshIoError(String),

    /// An account does not have enough lamports to be rent-exempt
    // #[error("An account does not have enough lamports to be rent-exempt")]
    AccountNotRentExempt,

    /// Invalid account owner
    // #[error("Invalid account owner")]
    InvalidAccountOwner,

    /// Program arithmetic overflowed
    // #[error("Program arithmetic overflowed")]
    ArithmeticOverflow,

    /// Unsupported sysvar
    // #[error("Unsupported sysvar")]
    UnsupportedSysvar,

    /// Illegal account owner
    // #[error("Provided owner is not allowed")]
    IllegalOwner,

    /// Accounts data allocations exceeded the maximum allowed per transaction
    // #[error("Accounts data allocations exceeded the maximum allowed per transaction")]
    MaxAccountsDataAllocationsExceeded,

    /// Max accounts exceeded
    // #[error("Max accounts exceeded")]
    MaxAccountsExceeded,

    /// Max instruction trace length exceeded
    // #[error("Max instruction trace length exceeded")]
    MaxInstructionTraceLengthExceeded,

    /// Builtin programs must consume compute units
    // #[error("Builtin programs must consume compute units")]
    BuiltinProgramsMustConsumeComputeUnits,
    // Note: For any new error added here an equivalent ProgramError and its
    // conversions must also be added
}

impl Display for InstructionError {
    fn fmt(&self, f: &mut Formatter<'_>) -> core::fmt::Result {
        match self {
            InstructionError::GenericError => f.write_str("GenericError"),
            InstructionError::InvalidArgument => f.write_str("InvalidArgument"),
            InstructionError::InvalidInstructionData => f.write_str("InvalidInstructionData"),
            InstructionError::InvalidAccountData => f.write_str("InvalidAccountData"),
            InstructionError::AccountDataTooSmall => f.write_str("AccountDataTooSmall"),
            InstructionError::InsufficientFunds => f.write_str("InsufficientFunds"),
            InstructionError::IncorrectProgramId => f.write_str("IncorrectProgramId"),
            InstructionError::MissingRequiredSignature => f.write_str("MissingRequiredSignature"),
            InstructionError::AccountAlreadyInitialized => f.write_str("AccountAlreadyInitialized"),
            InstructionError::UninitializedAccount => f.write_str("UninitializedAccount"),
            InstructionError::UnbalancedInstruction => f.write_str("UnbalancedInstruction"),
            InstructionError::ModifiedProgramId => f.write_str("ModifiedProgramId"),
            InstructionError::ExternalAccountLamportSpend => {
                f.write_str("ExternalAccountLamportSpend")
            }
            InstructionError::ExternalAccountDataModified => {
                f.write_str("ExternalAccountDataModified")
            }
            InstructionError::ReadonlyLamportChange => f.write_str("ReadonlyLamportChange"),
            InstructionError::ReadonlyDataModified => f.write_str("ReadonlyDataModified"),
            InstructionError::DuplicateAccountIndex => f.write_str("DuplicateAccountIndex"),
            InstructionError::ExecutableModified => f.write_str("ExecutableModified"),
            InstructionError::RentEpochModified => f.write_str("RentEpochModified"),
            InstructionError::NotEnoughAccountKeys => f.write_str("NotEnoughAccountKeys"),
            InstructionError::AccountDataSizeChanged => f.write_str("AccountDataSizeChanged"),
            InstructionError::AccountNotExecutable => f.write_str("AccountNotExecutable"),
            InstructionError::AccountBorrowFailed => f.write_str("AccountBorrowFailed"),
            InstructionError::AccountBorrowOutstanding => f.write_str("AccountBorrowOutstanding"),
            InstructionError::DuplicateAccountOutOfSync => f.write_str("DuplicateAccountOutOfSync"),
            InstructionError::Custom(_) => f.write_str("Custom"),
            InstructionError::InvalidError => f.write_str("InvalidError"),
            InstructionError::ExecutableDataModified => f.write_str("ExecutableDataModified"),
            InstructionError::ExecutableLamportChange => f.write_str("ExecutableLamportChange"),
            InstructionError::ExecutableAccountNotRentExempt => {
                f.write_str("ExecutableAccountNotRentExempt")
            }
            InstructionError::UnsupportedProgramId => f.write_str("UnsupportedProgramId"),
            InstructionError::CallDepth => f.write_str("CallDepth"),
            InstructionError::MissingAccount => f.write_str("MissingAccount"),
            InstructionError::ReentrancyNotAllowed => f.write_str("ReentrancyNotAllowed"),
            InstructionError::MaxSeedLengthExceeded => f.write_str("MaxSeedLengthExceeded"),
            InstructionError::InvalidSeeds => f.write_str("InvalidSeeds"),
            InstructionError::InvalidRealloc => f.write_str("InvalidRealloc"),
            InstructionError::ComputationalBudgetExceeded => {
                f.write_str("ComputationalBudgetExceeded")
            }
            InstructionError::PrivilegeEscalation => f.write_str("PrivilegeEscalation"),
            InstructionError::ProgramEnvironmentSetupFailure => {
                f.write_str("ProgramEnvironmentSetupFailure")
            }
            InstructionError::ProgramFailedToComplete => f.write_str("ProgramFailedToComplete"),
            InstructionError::ProgramFailedToCompile => f.write_str("ProgramFailedToCompile"),
            InstructionError::Immutable => f.write_str("Immutable"),
            InstructionError::IncorrectAuthority => f.write_str("IncorrectAuthority"),
            InstructionError::BorshIoError(_) => f.write_str("BorshIoError"),
            InstructionError::AccountNotRentExempt => f.write_str("AccountNotRentExempt"),
            InstructionError::InvalidAccountOwner => f.write_str("InvalidAccountOwner"),
            InstructionError::ArithmeticOverflow => f.write_str("ArithmeticOverflow"),
            InstructionError::UnsupportedSysvar => f.write_str("UnsupportedSysvar"),
            InstructionError::IllegalOwner => f.write_str("IllegalOwner"),
            InstructionError::MaxAccountsDataAllocationsExceeded => {
                f.write_str("MaxAccountsDataAllocationsExceeded")
            }
            InstructionError::MaxAccountsExceeded => f.write_str("MaxAccountsExceeded"),
            InstructionError::MaxInstructionTraceLengthExceeded => {
                f.write_str("MaxInstructionTraceLengthExceeded")
            }
            InstructionError::BuiltinProgramsMustConsumeComputeUnits => {
                f.write_str("BuiltinProgramsMustConsumeComputeUnits")
            }
        }
    }
}

impl core::error::Error for InstructionError {}

/// Reasons a transaction might be rejected.
#[derive(
    /*Error, Serialize, Deserialize, */ Debug,
    PartialEq,
    Eq,
    Clone, /* AbiExample, AbiEnumVisitor,*/
)]
pub enum TransactionError {
    /// An account is already being processed in another transaction in a way
    /// that does not support parallelism
    // #[error("Account in use")]
    AccountInUse,

    /// A `Pubkey` appears twice in the transaction's `account_keys`.  Instructions can reference
    /// `Pubkey`s more than once but the message must contain a list with no duplicate keys
    // #[error("Account loaded twice")]
    AccountLoadedTwice,

    /// Attempt to debit an account but found no record of a prior credit.
    // #[error("Attempt to debit an account but found no record of a prior credit.")]
    AccountNotFound,

    /// Attempt to load a program that does not exist
    // #[error("Attempt to load a program that does not exist")]
    ProgramAccountNotFound,

    /// The from `Pubkey` does not have sufficient balance to pay the fee to schedule the transaction
    // #[error("Insufficient funds for fee")]
    InsufficientFundsForFee,

    /// This account may not be used to pay transaction fees
    // #[error("This account may not be used to pay transaction fees")]
    InvalidAccountForFee,

    /// The bank has seen this transaction before. This can occur under normal operation
    /// when a UDP packet is duplicated, as a user error from a client not updating
    /// its `recent_blockhash`, or as a double-spend attack.
    // #[error("This transaction has already been processed")]
    AlreadyProcessed,

    /// The bank has not seen the given `recent_blockhash` or the transaction is too old and
    /// the `recent_blockhash` has been discarded.
    // #[error("Blockhash not found")]
    BlockhashNotFound,

    /// An error occurred while processing an instruction. The first element of the tuple
    /// indicates the instruction index in which the error occurred.
    // #[error("Error processing Instruction {0}: {1}")]
    InstructionError(u8, InstructionError),

    /// Loader call chain is too deep
    // #[error("Loader call chain is too deep")]
    CallChainTooDeep,

    /// Transaction requires a fee but has no signature present
    // #[error("Transaction requires a fee but has no signature present")]
    MissingSignatureForFee,

    /// Transaction contains an invalid account reference
    // #[error("Transaction contains an invalid account reference")]
    InvalidAccountIndex,

    /// Transaction did not pass signature verification
    // #[error("Transaction did not pass signature verification")]
    SignatureFailure,

    /// This program may not be used for executing instructions
    // #[error("This program may not be used for executing instructions")]
    InvalidProgramForExecution,

    /// Transaction failed to sanitize accounts offsets correctly
    /// implies that account locks are not taken for this TX, and should
    /// not be unlocked.
    // #[error("Transaction failed to sanitize accounts offsets correctly")]
    SanitizeFailure,

    // #[error("Transactions are currently disabled due to cluster maintenance")]
    ClusterMaintenance,

    /// Transaction processing left an account with an outstanding borrowed reference
    // #[error("Transaction processing left an account with an outstanding borrowed reference")]
    AccountBorrowOutstanding,

    /// Transaction would exceed max Block Cost Limit
    // #[error("Transaction would exceed max Block Cost Limit")]
    WouldExceedMaxBlockCostLimit,

    /// Transaction version is unsupported
    // #[error("Transaction version is unsupported")]
    UnsupportedVersion,

    /// Transaction loads a writable account that cannot be written
    // #[error("Transaction loads a writable account that cannot be written")]
    InvalidWritableAccount,

    /// Transaction would exceed max account limit within the block
    // #[error("Transaction would exceed max account limit within the block")]
    WouldExceedMaxAccountCostLimit,

    /// Transaction would exceed account data limit within the block
    // #[error("Transaction would exceed account data limit within the block")]
    WouldExceedAccountDataBlockLimit,

    /// Transaction locked too many accounts
    // #[error("Transaction locked too many accounts")]
    TooManyAccountLocks,

    /// Address lookup table not found
    // #[error("Transaction loads an address table account that doesn't exist")]
    AddressLookupTableNotFound,

    /// Attempted to lookup addresses from an account owned by the wrong program
    // #[error("Transaction loads an address table account with an invalid owner")]
    InvalidAddressLookupTableOwner,

    /// Attempted to lookup addresses from an invalid account
    // #[error("Transaction loads an address table account with invalid data")]
    InvalidAddressLookupTableData,

    /// Address table lookup uses an invalid index
    // #[error("Transaction address table lookup uses an invalid index")]
    InvalidAddressLookupTableIndex,

    /// Transaction leaves an account with a lower balance than rent-exempt minimum
    // #[error("Transaction leaves an account with a lower balance than rent-exempt minimum")]
    InvalidRentPayingAccount,

    /// Transaction would exceed max Vote Cost Limit
    // #[error("Transaction would exceed max Vote Cost Limit")]
    WouldExceedMaxVoteCostLimit,

    /// Transaction would exceed total account data limit
    // #[error("Transaction would exceed total account data limit")]
    WouldExceedAccountDataTotalLimit,

    /// Transaction contains a duplicate instruction that is not allowed
    // #[error("Transaction contains a duplicate instruction ({0}) that is not allowed")]
    DuplicateInstruction(u8),

    /// Transaction results in an account with insufficient funds for rent
    // #[error(
    //     "Transaction results in an account ({account_index}) with insufficient funds for rent"
    // )]
    InsufficientFundsForRent {
        account_index: u8,
    },

    /// Transaction exceeded max loaded accounts data size cap
    // #[error("Transaction exceeded max loaded accounts data size cap")]
    MaxLoadedAccountsDataSizeExceeded,

    /// LoadedAccountsDataSizeLimit set for transaction must be greater than 0.
    // #[error("LoadedAccountsDataSizeLimit set for transaction must be greater than 0.")]
    InvalidLoadedAccountsDataSizeLimit,

    /// Sanitized transaction differed before/after feature activiation. Needs to be resanitized.
    // #[error("ResanitizationNeeded")]
    ResanitizationNeeded,

    /// Program execution is temporarily restricted on an account.
    // #[error("Execution of the program referenced by account at index {account_index} is temporarily restricted.")]
    ProgramExecutionTemporarilyRestricted {
        account_index: u8,
    },

    /// The total balance before the transaction does not equal the total balance after the transaction
    // #[error("Sum of account balances before and after transaction do not match")]
    UnbalancedTransaction,
}

impl Display for TransactionError {
    fn fmt(&self, f: &mut Formatter<'_>) -> core::fmt::Result {
        match self {
            TransactionError::AccountInUse => f.write_str("AccountInUse"),
            TransactionError::AccountLoadedTwice => f.write_str("AccountLoadedTwice"),
            TransactionError::AccountNotFound => f.write_str("AccountNotFound"),
            TransactionError::ProgramAccountNotFound => f.write_str("ProgramAccountNotFound"),
            TransactionError::InsufficientFundsForFee => f.write_str("InsufficientFundsForFee"),
            TransactionError::InvalidAccountIndex => f.write_str("InvalidAccountIndex"),
            TransactionError::SignatureFailure => f.write_str("SignatureFailure"),
            TransactionError::InvalidProgramForExecution => {
                f.write_str("InvalidProgramForExecution")
            }
            TransactionError::SanitizeFailure => f.write_str("SanitizeFailure"),
            TransactionError::ClusterMaintenance => f.write_str("ClusterMaintenance"),
            TransactionError::AccountBorrowOutstanding => f.write_str("AccountBorrowOutstanding"),
            TransactionError::InvalidAccountForFee => f.write_str("InvalidAccountForFee"),
            TransactionError::AlreadyProcessed => f.write_str("AlreadyProcessed"),
            TransactionError::BlockhashNotFound => f.write_str("BlockhashNotFound"),
            TransactionError::InstructionError(_, _) => f.write_str("InstructionError"),
            TransactionError::CallChainTooDeep => f.write_str("CallChainTooDeep"),
            TransactionError::MissingSignatureForFee => f.write_str("MissingSignatureForFee"),
            TransactionError::WouldExceedMaxBlockCostLimit => {
                f.write_str("WouldExceedMaxBlockCostLimit")
            }
            TransactionError::UnsupportedVersion => f.write_str("UnsupportedVersion"),
            TransactionError::InvalidWritableAccount => f.write_str("InvalidWritableAccount"),
            TransactionError::WouldExceedMaxAccountCostLimit => {
                f.write_str("WouldExceedMaxAccountCostLimit")
            }
            TransactionError::WouldExceedAccountDataBlockLimit => {
                f.write_str("WouldExceedAccountDataBlockLimit")
            }
            TransactionError::TooManyAccountLocks => f.write_str("TooManyAccountLocks"),
            TransactionError::AddressLookupTableNotFound => {
                f.write_str("AddressLookupTableNotFound")
            }
            TransactionError::InvalidAddressLookupTableOwner => {
                f.write_str("InvalidAddressLookupTableOwner")
            }
            TransactionError::InvalidAddressLookupTableData => {
                f.write_str("InvalidAddressLookupTableData")
            }
            TransactionError::InvalidAddressLookupTableIndex => {
                f.write_str("InvalidAddressLookupTableIndex")
            }
            TransactionError::InvalidRentPayingAccount => f.write_str("InvalidRentPayingAccount"),
            TransactionError::WouldExceedMaxVoteCostLimit => {
                f.write_str("WouldExceedMaxVoteCostLimit")
            }
            TransactionError::WouldExceedAccountDataTotalLimit => {
                f.write_str("WouldExceedAccountDataTotalLimit")
            }
            TransactionError::DuplicateInstruction(_) => f.write_str("DuplicateInstruction"),
            TransactionError::InsufficientFundsForRent { .. } => {
                f.write_str("InsufficientFundsForRent")
            }
            TransactionError::MaxLoadedAccountsDataSizeExceeded => {
                f.write_str("MaxLoadedAccountsDataSizeExceeded")
            }
            TransactionError::InvalidLoadedAccountsDataSizeLimit => {
                f.write_str("InvalidLoadedAccountsDataSizeLimit")
            }
            TransactionError::ResanitizationNeeded => f.write_str("ResanitizationNeeded"),
            TransactionError::ProgramExecutionTemporarilyRestricted { .. } => {
                f.write_str("ProgramExecutionTemporarilyRestricted")
            }
            TransactionError::UnbalancedTransaction => f.write_str("UnbalancedTransaction"),
        }
    }
}

impl core::error::Error for TransactionError {}

#[derive(Debug)]
pub enum SvmError {
    TransactionError(TransactionError),
    BincodeError(bincode::Error),
    ExitCode(ExitCode),
    InstructionError(InstructionError),
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

impl From<bincode::Error> for SvmError {
    fn from(value: bincode::Error) -> Self {
        SvmError::BincodeError(value)
    }
}

impl From<InstructionError> for SvmError {
    fn from(value: InstructionError) -> Self {
        SvmError::InstructionError(value)
    }
}
