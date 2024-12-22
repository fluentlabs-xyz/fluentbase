use crate::U256;
use alloy_primitives::{b256, hex, B256};
#[cfg(feature = "rwasm")]
use rwasm::{
    core::{Trap, TrapCode},
    engine::bytecode::FuncIdx,
};
use strum_macros::{Display, FromRepr};

pub type Bytes64 = [u8; 64];
pub type Bytes34 = [u8; 34];

pub type Bytes32 = [u8; 32];
pub type Bytes20 = [u8; 20];

#[derive(Default, Clone, Debug)]
pub struct Fuel {
    pub limit: u64,
    pub refund: i64,
    pub spent: u64,
}

impl Fuel {
    pub fn new(limit: u64) -> Self {
        Self {
            limit,
            refund: 0,
            spent: 0,
        }
    }

    pub fn with_refund(mut self, refund: i64) -> Self {
        self.refund = refund;
        self
    }

    pub fn with_spent(mut self, spent: u64) -> Self {
        self.spent = spent;
        self
    }

    pub fn remaining(&self) -> u64 {
        self.limit - self.spent
    }

    pub fn spent(&self) -> u64 {
        self.spent
    }

    pub fn charge(&mut self, value: u64) -> bool {
        if value > self.remaining() {
            return false;
        }
        self.spent += value;
        true
    }

    pub fn refund(&mut self, value: u64) {
        assert!(self.spent >= value);
        self.spent -= value;
    }
}

impl From<u64> for Fuel {
    #[inline]
    fn from(value: u64) -> Self {
        Self {
            limit: value,
            refund: 0,
            spent: 0,
        }
    }
}
impl Into<u64> for Fuel {
    #[inline]
    fn into(self) -> u64 {
        self.limit - self.spent
    }
}

#[derive(Default, Debug, Copy, Clone, Eq, PartialEq, Display, FromRepr)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[repr(i32)]
pub enum ExitCode {
    // warning: when adding new codes doesn't forget to add them to impls below
    #[default]
    Ok = 0,
    Panic = -71,
    // fluentbase error codes
    ExecutionHalted = -1001,
    RootCallOnly = -1003,
    OutputOverflow = -1005,
    PoseidonError = -1007,
    PersistentStorageError = -1008,
    WriteProtection = -1009,
    InsufficientBalance = -1012,
    CreateCollision = -1013,
    ContractSizeLimit = -1014,
    CallDepthOverflow = -1016,
    FatalExternalError = -1017,
    CompilationError = -1018,
    OverflowPayment = -1019,
    PrecompileError = -1025,
    EcrecoverBadSignature = -1026,
    EcrecoverError = -1027,
    NonceOverflow = -1028,
    CreateContractStartingWithEF = -1029,
    OpcodeNotFound = -1030,
    InvalidEfOpcode = -1031,
    InvalidJump = -1032,
    NotActivatedEIP = -1033,
    ImmutableContext = -1034,
    ContextWriteProtection = -1035,
    NonNegativeExitCode = -1036,
    MalformedSyscallParams = -1037,
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
    OutOfGas = -2015,
    GrowthOperationLimited = -2016,
    UnknownError = -2017,
    UnresolvedFunction = -2018,
    StackUnderflow = -2019,
}

pub trait UnwrapExitCode<T> {
    fn unwrap_exit_code(self) -> T;
}

impl<T> UnwrapExitCode<T> for Result<T, ExitCode> {
    fn unwrap_exit_code(self) -> T {
        match self {
            Ok(res) => res,
            Err(err) => panic!("exit code: {} ({})", err, err.into_i32()),
        }
    }
}

impl From<i32> for ExitCode {
    fn from(value: i32) -> Self {
        Self::from_repr(value).unwrap_or(ExitCode::UnknownError)
    }
}

impl ExitCode {
    #[inline]
    pub const fn is_ok(&self) -> bool {
        self.into_i32() == Self::Ok.into_i32()
    }

    #[inline]
    pub const fn is_error(&self) -> bool {
        self.into_i32() != Self::Ok.into_i32()
    }

    /// Returns whether the result is a revert.
    #[inline]
    pub const fn is_revert(self) -> bool {
        self.into_i32() != Self::Ok.into_i32()
    }

    pub const fn into_i32(self) -> i32 {
        self as i32
    }

    #[cfg(feature = "rwasm")]
    pub fn into_trap(self) -> Trap {
        Trap::i32_exit(self as i32)
    }

    /// Encodes Solidity panic message using signature sig4("Panic(uint256)")
    pub fn encode_solidity_panic(&self, panic_buffer: &mut [u8]) {
        assert!(panic_buffer.len() >= 32 + 4);
        panic_buffer[..4].copy_from_slice(&hex!("4e487b71"));
        let exit_code = U256::from(self.into_i32() as u32);
        panic_buffer[4..].copy_from_slice(&exit_code.to_be_bytes::<{ U256::BYTES }>());
    }
}

#[cfg(feature = "rwasm")]
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
            TrapCode::OutOfFuel => ExitCode::OutOfGas,
            TrapCode::GrowthOperationLimited => ExitCode::GrowthOperationLimited,
            TrapCode::UnresolvedFunction => ExitCode::UnresolvedFunction,
        }
    }
}

#[cfg(feature = "rwasm")]
impl Into<Trap> for ExitCode {
    fn into(self) -> Trap {
        self.into_trap()
    }
}
#[cfg(feature = "rwasm")]
impl From<Trap> for ExitCode {
    fn from(value: Trap) -> Self {
        ExitCode::from(&value)
    }
}
#[cfg(feature = "rwasm")]
impl From<&Trap> for ExitCode {
    fn from(value: &Trap) -> Self {
        if let Some(trap_code) = value.trap_code() {
            return ExitCode::from(trap_code);
        }
        if let Some(exit_code) = value.i32_exit_status() {
            return ExitCode::from(exit_code);
        }
        ExitCode::UnknownError
    }
}

impl Into<i32> for ExitCode {
    fn into(self) -> i32 {
        self as i32
    }
}

#[derive(Default, Clone, Copy, Debug, PartialEq, Eq, Hash, Ord, PartialOrd, Display, FromRepr)]
#[repr(u32)]
#[allow(non_camel_case_types)]
pub enum SysFuncIdx {
    #[default]
    UNKNOWN = 0x0000,

    // crypto
    KECCAK256 = 0x0101,
    POSEIDON = 0x0102,
    POSEIDON_HASH = 0x0103,
    ECRECOVER = 0x0104,

    // SYS host
    EXIT = 0x0001,
    STATE = 0x0002,
    READ = 0x0003,
    INPUT_SIZE = 0x0004,
    WRITE = 0x0005,
    OUTPUT_SIZE = 0x0006,
    READ_OUTPUT = 0x0007,
    EXEC = 0x0009,
    RESUME = 0x000a,
    FORWARD_OUTPUT = 0x000b,
    CHARGE_FUEL = 0x000c,
    FUEL = 0x000d,

    // preimage
    PREIMAGE_SIZE = 0x070D,
    PREIMAGE_COPY = 0x070E,

    DEBUG_LOG = 0x0901,
}

impl SysFuncIdx {
    pub fn fuel_cost(&self) -> u32 {
        match self {
            SysFuncIdx::EXIT => 1,
            SysFuncIdx::STATE => 1,
            SysFuncIdx::READ => 1,
            SysFuncIdx::INPUT_SIZE => 1,
            SysFuncIdx::WRITE => 1,
            SysFuncIdx::KECCAK256 => 1,
            SysFuncIdx::POSEIDON => 1,
            SysFuncIdx::POSEIDON_HASH => 1,
            SysFuncIdx::ECRECOVER => 1,
            _ => 1, //unreachable!("not configured fuel for opcode: {:?}", self),
        }
    }
}

impl Into<u32> for SysFuncIdx {
    fn into(self) -> u32 {
        self as u32
    }
}

#[cfg(feature = "rwasm")]
impl Into<FuncIdx> for SysFuncIdx {
    fn into(self) -> FuncIdx {
        (self as u32).into()
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
#[allow(non_camel_case_types)]
pub enum BytecodeType {
    EVM,
    WASM,
}

const EIP7702_SIG_LEN: usize = 2;
/// rWASM binary format signature:
/// - 0xef 0x00 - EIP-3540 compatible prefix
/// - 0x52 - rWASM version number (equal to 'R')
const EIP7702_SIG: [u8; EIP7702_SIG_LEN] = [0xef, 0x01];

const WASM_SIG_LEN: usize = 4;
/// WebAssembly signature (\00ASM)
const WASM_SIG: [u8; WASM_SIG_LEN] = [0x00, 0x61, 0x73, 0x6d];

const RWASM_SIG_LEN: usize = 2;
/// rWASM binary format signature:
/// - 0xef 0x00 - EIP-3540 compatible prefix
/// - 0x52 - rWASM version number (equal to 'R')
const RWASM_SIG: [u8; RWASM_SIG_LEN] = [0xef, 0x52];

impl BytecodeType {
    pub fn from_slice(input: &[u8]) -> Self {
        // default WebAssembly signature
        if input.len() >= WASM_SIG_LEN && input[0..WASM_SIG_LEN] == WASM_SIG {
            return Self::WASM;
        }
        // case for rWASM contracts that are inside genesis
        if input.len() >= RWASM_SIG_LEN && input[0..RWASM_SIG_LEN] == RWASM_SIG {
            return Self::WASM;
        }
        // all the rest are EVM bytecode
        Self::EVM
    }
}

pub const SYSCALL_ID_STORAGE_READ: B256 =
    b256!("4023096842131de08903e3a03a648b5a91209ca2a264e0a3a90f9899431ad227"); // keccak256("_syscall_storage_read")
pub const SYSCALL_ID_STORAGE_WRITE: B256 =
    b256!("126659e43fb4baaff19b992a1869aa0cac8ec5e30b38556fd8cf28e6fd2255b9"); // keccak256("_syscall_storage_write")
pub const SYSCALL_ID_CALL: B256 =
    b256!("1d2e7a52c8548eccd33b1f100ae79c86c1a6a6baa18215f916d395a7095ee3e9"); // keccak256("_syscall_call")
pub const SYSCALL_ID_STATIC_CALL: B256 =
    b256!("c8d75aa83d2d2710550b424cf8ed7ce575348ac9628ae284118ed839ec5003b1"); // keccak256("_syscall_static_call")
pub const SYSCALL_ID_CALL_CODE: B256 =
    b256!("10c6aac9a8c0edaa89d4eb61ccd665b386d1faef9222d1f04b88aa9f43ede6d4"); // keccak256("_syscall_call_code")
pub const SYSCALL_ID_DELEGATE_CALL: B256 =
    b256!("75bd4ec817c86b0736da59cb28bb22979b1547ee30426044e0ded9055ecfee5a"); // keccak256("_syscall_delegate_call")
pub const FAILED_SYSCALL_ID_DELEGATE_CALL: B256 =
    b256!("000000000000000036da59cb28bb22979b1547ee30426044e0ded9055ecfee5a");
pub const SYSCALL_ID_CREATE: B256 =
    b256!("9708d5acbee3bf900474f0e80767e267e15a3c0f8bda6f3f882235855d42a61f"); // keccak256("_syscall_create")
pub const SYSCALL_ID_CREATE2: B256 =
    b256!("ae4ca3b6b3d9965a736c58075c7b05246e0aeb31c16a1be2f2b569c3e6545f2a"); // keccak256("_syscall_create2")
pub const SYSCALL_ID_EMIT_LOG: B256 =
    b256!("505be4983de61b5ab79cdc8164e4db895c4f9548cee794e1e0bccec1dc0b751d"); // keccak256("_syscall_emit_log")
pub const SYSCALL_ID_DESTROY_ACCOUNT: B256 =
    b256!("288b6990f686aff01fe73bc8be3738b4669f5cab8c40076fac1d0abc9c8883d8"); // keccak256("_syscall_destroy_account")
pub const SYSCALL_ID_BALANCE: B256 =
    b256!("cb4021d39709b0f968e88fb3916c04ea18509e666daf1eb14ebd757d0db9e9b2"); // keccak256("_syscall_balance")
pub const SYSCALL_ID_WRITE_PREIMAGE: B256 =
    b256!("d114f5a81f2232d3237cf0e4c72a9a2928f4385fb43e1a0021ed9fe41fb2e8e9"); // keccak256("_syscall_write_preimage")
pub const SYSCALL_ID_PREIMAGE_COPY: B256 =
    b256!("3e98d2443cafeb26748e1eaa1a87e7bf75b170685b56524d8435eb90047e7c3e"); // keccak256("_syscall_preimage_copy")
pub const SYSCALL_ID_PREIMAGE_SIZE: B256 =
    b256!("af119f11b6bece48a4770a5a5aa01003d69518f36a0d9882ddd93e1c9e7bd32a"); // keccak256("_syscall_preimage_size")
pub const SYSCALL_ID_EXT_STORAGE_READ: B256 =
    b256!("25960aed19d8a68d1e45dfed7e5000c174f340980b7942624fa5f12a12cc91cc"); // keccak256("_syscall_ext_storage_read")
pub const SYSCALL_ID_TRANSIENT_READ: B256 =
    b256!("3ef8f86265ed070e9e2226e064013f891f556b3bc8695d7a28a6972ceebdc112"); // keccak256("_syscall_transient_read")
pub const SYSCALL_ID_TRANSIENT_WRITE: B256 =
    b256!("15865fc329a198370698eecf195b1f4a8b99e18253d80deeefedf0b64be49e56"); // keccak256("_syscall_transient_write")

pub const fn syscall_name_by_hash(hash: &B256) -> &str {
    match *hash {
        SYSCALL_ID_STORAGE_READ => "SYSCALL_ID_STORAGE_READ",
        SYSCALL_ID_STORAGE_WRITE => "SYSCALL_ID_STORAGE_WRITE",
        SYSCALL_ID_CALL => "SYSCALL_ID_CALL",
        SYSCALL_ID_STATIC_CALL => "SYSCALL_ID_STATIC_CALL",
        SYSCALL_ID_CALL_CODE => "SYSCALL_ID_CALL_CODE",
        SYSCALL_ID_DELEGATE_CALL => "SYSCALL_ID_DELEGATE_CALL",
        SYSCALL_ID_CREATE => "SYSCALL_ID_CREATE",
        SYSCALL_ID_CREATE2 => "SYSCALL_ID_CREATE2",
        SYSCALL_ID_EMIT_LOG => "SYSCALL_ID_EMIT_LOG",
        SYSCALL_ID_DESTROY_ACCOUNT => "SYSCALL_ID_DESTROY_ACCOUNT",
        SYSCALL_ID_BALANCE => "SYSCALL_ID_BALANCE",
        SYSCALL_ID_WRITE_PREIMAGE => "SYSCALL_ID_WRITE_PREIMAGE",
        SYSCALL_ID_PREIMAGE_COPY => "SYSCALL_ID_PREIMAGE_COPY",
        SYSCALL_ID_PREIMAGE_SIZE => "SYSCALL_ID_PREIMAGE_SIZE",
        SYSCALL_ID_EXT_STORAGE_READ => "SYSCALL_ID_EXT_STORAGE_READ",
        SYSCALL_ID_TRANSIENT_READ => "SYSCALL_ID_TRANSIENT_READ",
        SYSCALL_ID_TRANSIENT_WRITE => "SYSCALL_ID_TRANSIENT_WRITE",
        _ => "EXEC",
    }
}

pub const GAS_LIMIT_SYSCALL_STORAGE_READ: u64 = 2_100;
pub const GAS_LIMIT_SYSCALL_STORAGE_WRITE: u64 = 22_100;
pub const GAS_LIMIT_SYSCALL_EMIT_LOG: u64 = 10_000;
pub const GAS_LIMIT_SYSCALL_DESTROY_ACCOUNT: u64 = 32_600;
pub const GAS_LIMIT_SYSCALL_BALANCE: u64 = 2_600;
pub const GAS_LIMIT_SYSCALL_EXT_STORAGE_READ: u64 = 2_100;
pub const GAS_LIMIT_SYSCALL_PREIMAGE_SIZE: u64 = 2_600;
pub const GAS_LIMIT_SYSCALL_TRANSIENT_READ: u64 = 100;
pub const GAS_LIMIT_SYSCALL_TRANSIENT_WRITE: u64 = 100;
