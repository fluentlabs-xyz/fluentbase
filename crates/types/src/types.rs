use crate::U256;
use alloy_primitives::hex;
#[cfg(feature = "rwasm")]
use rwasm::{
    core::{Trap, TrapCode},
    engine::bytecode::FuncIdx,
};
use strum_macros::{Display, FromRepr};

pub type Bytes32 = [u8; 32];
pub type Bytes20 = [u8; 20];

pub struct Fuel {
    pub limit: u64,
    pub spent: u64,
}

impl Fuel {
    pub fn remaining(&self) -> u64 {
        self.limit - self.spent
    }

    pub fn charge(&mut self, value: u64) {
        self.spent += value;
    }
}

impl From<u64> for Fuel {
    #[inline]
    fn from(value: u64) -> Self {
        Self {
            limit: value,
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
        value
            .i32_exit_status()
            .map(ExitCode::from)
            .unwrap_or(ExitCode::UnknownError)
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
    READ_CONTEXT = 0x000e,

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

/// WebAssembly signature (\0ASM)
const WASM_SIG: [u8; 4] = [0x00, 0x61, 0x73, 0x6d];

/// rWASM binary format signature:
/// - 0xef 0x00 - EIP-3540 compatible prefix
/// - 0x52 - rWASM version number (equal to 'R')
const RWASM_SIG: [u8; 3] = [0xef, 0x00, 0x52];

impl BytecodeType {
    pub fn from_slice(input: &[u8]) -> Self {
        // default WebAssembly signature (\0ASM)
        if input.len() >= WASM_SIG.len() && input[0..4] == WASM_SIG {
            return Self::WASM;
        }
        // case for rWASM contracts that are inside genesis
        if input.len() >= RWASM_SIG.len() && input[0..3] == RWASM_SIG {
            return Self::WASM;
        }
        // all the rest are EVM bytecode
        Self::EVM
    }
}
