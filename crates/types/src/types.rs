use core::fmt;
use core::fmt::Formatter;
use rwasm::{
    core::{Trap, TrapCode},
    engine::bytecode::FuncIdx,
};

pub type Bytes32 = [u8; 32];
pub type Bytes20 = [u8; 20];

#[derive(Default, Debug, Copy, Clone, Eq, PartialEq)]
#[cfg_attr(feature = "std", derive(strum_macros::EnumIter))]
pub enum ExitCode {
    // warning: when adding new codes don't forget to add them to impls below
    #[default]
    Ok = 0,
    Panic = -71,
    // fluentbase error codes
    ExecutionHalted = -1001,
    NotSupportedCall = -1003,
    TransactError = -1004,
    OutputOverflow = -1005,
    InputDecodeFailure = -1006,
    PoseidonError = -1007,
    PersistentStorageError = -1008,
    WriteProtection = -1009,
    CreateError = -1010,
    PreimageUnavailable = -1011,
    InsufficientBalance = -1012,
    CreateCollision = -1013,
    ContractSizeLimit = -1014,
    StorageSlotOverflow = -1015,
    CallDepthOverflow = -1016,
    FatalExternalError = -1017,
    CompilationError = -1018,
    OverflowPayment = -1019,
    EVMCreateError = -1020,
    EVMCreateRevert = -1021,
    EVMCallError = -1022,
    EVMCallRevert = -1023,
    EVMNotFound = -1024,
    PrecompileError = -1025,
    EcrecoverBadSignature = -1026,
    EcrecoverError = -1027,
    NonceOverflow = -1028,
    CreateContractStartingWithEF = 1029,
    // trap error codes
    UnreachableCodeReached = -2006,
    MemoryOutOfBounds = -2007,
    TableOutOfBounds = -2008,
    IndirectCallToNull = -2009,
    IntegerDivisionByZero = -2010,
    IntegerOverflow = -2011,
    BadConversionToInteger = -2012,
    StackOverflow = -2013,
    BadSignature = -2014,
    OutOfFuel = -2015,
    GrowthOperationLimited = -2016,
    UnknownError = -2017,
    UnresolvedFunction = -2018,
}

impl fmt::Display for ExitCode {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            ExitCode::Ok => write!(f, "ok"),
            ExitCode::Panic => write!(f, "panic"),
            ExitCode::ExecutionHalted => write!(f, "execution halted"),
            ExitCode::NotSupportedCall => write!(f, "not supported call"),
            ExitCode::TransactError => write!(f, "transact error"),
            ExitCode::OutputOverflow => write!(f, "output overflow"),
            ExitCode::InputDecodeFailure => write!(f, "input decode failure"),
            ExitCode::PoseidonError => write!(f, "poseidon error"),
            ExitCode::PersistentStorageError => write!(f, "persistent storage error"),
            ExitCode::WriteProtection => write!(f, "write protection"),
            ExitCode::CreateError => write!(f, "create error"),
            ExitCode::PreimageUnavailable => write!(f, "preimage unavailable"),
            ExitCode::InsufficientBalance => write!(f, "insufficient balance"),
            ExitCode::CreateCollision => write!(f, "create collision"),
            ExitCode::ContractSizeLimit => write!(f, "contract size limit"),
            ExitCode::StorageSlotOverflow => write!(f, "storage slot overflow"),
            ExitCode::CallDepthOverflow => write!(f, "call depth overflow"),
            ExitCode::FatalExternalError => write!(f, "fatal external error"),
            ExitCode::CompilationError => write!(f, "compilation error"),
            ExitCode::OverflowPayment => write!(f, "overflow payment"),
            ExitCode::EVMCreateError => write!(f, "evm create error"),
            ExitCode::EVMCreateRevert => write!(f, "evm create revert"),
            ExitCode::EVMCallError => write!(f, "evm call error"),
            ExitCode::EVMCallRevert => write!(f, "evm call revert"),
            ExitCode::EVMNotFound => write!(f, "evm not found"),
            ExitCode::PrecompileError => write!(f, "precompile error"),
            ExitCode::EcrecoverBadSignature => write!(f, "ecrecover bad signature"),
            ExitCode::EcrecoverError => write!(f, "ecrecover error"),
            ExitCode::NonceOverflow => write!(f, "nonce overflow"),
            ExitCode::CreateContractStartingWithEF => write!(f, "create contract starting with ef"),
            ExitCode::UnreachableCodeReached => write!(f, "unreachable code reached"),
            ExitCode::MemoryOutOfBounds => write!(f, "memory out of bounds"),
            ExitCode::TableOutOfBounds => write!(f, "table out of bounds"),
            ExitCode::IndirectCallToNull => write!(f, "indirect call to null"),
            ExitCode::IntegerDivisionByZero => write!(f, "integer division by zero"),
            ExitCode::IntegerOverflow => write!(f, "integer overflow"),
            ExitCode::BadConversionToInteger => write!(f, "bad conversion to integer"),
            ExitCode::StackOverflow => write!(f, "stack overflow"),
            ExitCode::BadSignature => write!(f, "bad signature"),
            ExitCode::OutOfFuel => write!(f, "out of fuel"),
            ExitCode::GrowthOperationLimited => write!(f, "growth operation limited"),
            ExitCode::UnknownError => write!(f, "unknown error"),
            ExitCode::UnresolvedFunction => write!(f, "unresolved function"),
        }
    }
}

impl From<i32> for ExitCode {
    fn from(value: i32) -> Self {
        if value == Self::Ok.into_i32() {
            Self::Ok
        } else if value == Self::Panic.into_i32() {
            Self::Panic
        } else {
            Self::TransactError
        }
    }
}

impl ExitCode {
    pub const fn is_ok(&self) -> bool {
        self.into_i32() == Self::Ok.into_i32()
    }

    pub const fn is_error(&self) -> bool {
        self.into_i32() != Self::Ok.into_i32()
    }

    pub const fn into_i32(self) -> i32 {
        self as i32
    }

    pub fn into_trap(self) -> Trap {
        Trap::i32_exit(self as i32)
    }
}

impl From<TrapCode> for ExitCode {
    fn from(value: TrapCode) -> Self {
        match value {
            TrapCode::UnreachableCodeReached => ExitCode::UnreachableCodeReached,
            TrapCode::MemoryOutOfBounds => ExitCode::MemoryOutOfBounds,
            TrapCode::TableOutOfBounds => ExitCode::TableOutOfBounds,
            TrapCode::IndirectCallToNull => ExitCode::IndirectCallToNull,
            TrapCode::IntegerDivisionByZero => ExitCode::IntegerDivisionByZero,
            TrapCode::IntegerOverflow => ExitCode::IntegerOverflow,
            TrapCode::BadConversionToInteger => ExitCode::BadConversionToInteger,
            TrapCode::StackOverflow => ExitCode::StackOverflow,
            TrapCode::BadSignature => ExitCode::BadSignature,
            TrapCode::OutOfFuel => ExitCode::OutOfFuel,
            TrapCode::GrowthOperationLimited => ExitCode::GrowthOperationLimited,
            TrapCode::UnresolvedFunction => ExitCode::UnresolvedFunction,
        }
    }
}

impl Into<Trap> for ExitCode {
    fn into(self) -> Trap {
        self.into_trap()
    }
}

impl Into<i32> for ExitCode {
    fn into(self) -> i32 {
        self as i32
    }
}

#[allow(non_camel_case_types)]
#[derive(Default, Clone, Copy, Debug, PartialEq, Eq, Hash, Ord, PartialOrd)]
#[cfg_attr(feature = "std", derive(strum_macros::EnumIter))]
pub enum SysFuncIdx {
    #[default]
    UNKNOWN = 0x0000,

    // crypto
    CRYPTO_KECCAK256 = 0x0101,
    CRYPTO_POSEIDON = 0x0102,
    CRYPTO_POSEIDON2 = 0x0103,
    CRYPTO_ECRECOVER = 0x0104,

    // SYS host
    SYS_HALT = 0x0001,
    SYS_STATE = 0x0002,
    SYS_READ = 0x0003,
    SYS_INPUT_SIZE = 0x0004,
    SYS_WRITE = 0x0005,
    SYS_OUTPUT_SIZE = 0x0006,
    SYS_READ_OUTPUT = 0x0007,
    SYS_EXEC_HASH = 0x0009,
    SYS_FORWARD_OUTPUT = 0x000a,

    // jzkt
    JZKT_OPEN = 0x0701,
    JZKT_CHECKPOINT = 0x0702,
    JZKT_GET = 0x0703,
    JZKT_UPDATE = 0x0704,
    JZKT_UPDATE_PREIMAGE = 0x0705,
    JZKT_REMOVE = 0x0706,
    JZKT_COMPUTE_ROOT = 0x0707,
    JZKT_EMIT_LOG = 0x0708,
    JZKT_COMMIT = 0x0709,
    JZKT_ROLLBACK = 0x070A,
    JZKT_PREIMAGE_SIZE = 0x070D,
    JZKT_PREIMAGE_COPY = 0x070E,
}

impl SysFuncIdx {
    pub fn fuel_cost(&self) -> u32 {
        match self {
            SysFuncIdx::SYS_HALT => 1,
            SysFuncIdx::SYS_STATE => 1,
            SysFuncIdx::SYS_READ => 1,
            SysFuncIdx::SYS_INPUT_SIZE => 1,
            SysFuncIdx::SYS_WRITE => 1,
            SysFuncIdx::CRYPTO_KECCAK256 => 1,
            SysFuncIdx::CRYPTO_POSEIDON => 1,
            SysFuncIdx::CRYPTO_POSEIDON2 => 1,
            SysFuncIdx::CRYPTO_ECRECOVER => 1,
            SysFuncIdx::JZKT_OPEN => 1,
            SysFuncIdx::JZKT_UPDATE => 1,
            SysFuncIdx::JZKT_GET => 1,
            SysFuncIdx::JZKT_COMPUTE_ROOT => 1,
            SysFuncIdx::JZKT_ROLLBACK => 1,
            SysFuncIdx::JZKT_COMMIT => 1,
            _ => 1, //unreachable!("not configured fuel for opcode: {:?}", self),
        }
    }
}

impl From<u32> for SysFuncIdx {
    fn from(value: u32) -> Self {
        match value {
            0x0000 => Self::UNKNOWN,

            // crypto
            0x0101 => Self::CRYPTO_KECCAK256,
            0x0102 => Self::CRYPTO_POSEIDON,
            0x0103 => Self::CRYPTO_POSEIDON2,
            0x0104 => Self::CRYPTO_ECRECOVER,

            // SYS host
            0x0001 => Self::SYS_HALT,
            0x0002 => Self::SYS_STATE,
            0x0003 => Self::SYS_READ,
            0x0004 => Self::SYS_INPUT_SIZE,
            0x0005 => Self::SYS_WRITE,
            0x0006 => Self::SYS_OUTPUT_SIZE,
            0x0007 => Self::SYS_READ_OUTPUT,
            0x0009 => Self::SYS_EXEC_HASH,
            0x000a => Self::SYS_FORWARD_OUTPUT,

            // jzkt
            0x0701 => Self::JZKT_OPEN,
            0x0702 => Self::JZKT_CHECKPOINT,
            0x0703 => Self::JZKT_GET,
            0x0704 => Self::JZKT_UPDATE,
            0x0705 => Self::JZKT_UPDATE_PREIMAGE,
            0x0706 => Self::JZKT_REMOVE,
            0x0707 => Self::JZKT_COMPUTE_ROOT,
            0x0708 => Self::JZKT_EMIT_LOG,
            0x0709 => Self::JZKT_COMMIT,
            0x070A => Self::JZKT_ROLLBACK,
            0x070D => Self::JZKT_PREIMAGE_SIZE,
            0x070E => Self::JZKT_PREIMAGE_COPY,

            _ => Self::UNKNOWN,
        }
    }
}

impl Into<u32> for SysFuncIdx {
    fn into(self) -> u32 {
        self as u32
    }
}

impl Into<FuncIdx> for SysFuncIdx {
    fn into(self) -> FuncIdx {
        (self as u32).into()
    }
}
