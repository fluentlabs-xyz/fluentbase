use crate::{FUEL_DENOM_RATE, U256};
use alloy_primitives::{hex, B256};
#[cfg(feature = "rwasm")]
use rwasm::{
    core::{Trap, TrapCode},
    engine::bytecode::FuncIdx,
};
use strum_macros::{Display, FromRepr};

#[derive(Default, Debug, Copy, Clone, Eq, PartialEq, Display, FromRepr)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[repr(i32)]
pub enum ExitCode {
    // warning: when adding new codes doesn't forget to add them to impls below
    #[default]
    Ok = 0,
    Panic = -71, // -71 to be wasi friendly
    // fluentbase error codes
    ExecutionHalted = -1001,
    RootCallOnly = -1002,
    MalformedBuiltinParams = -1003,
    CallDepthOverflow = -1004,
    NonNegativeExitCode = -1005,
    UnknownError = -1006,
    InputOutputOutOfBounds = -1007,
    // trap error codes
    UnreachableCodeReached = -2001,
    MemoryOutOfBounds = -2002,
    TableOutOfBounds = -2003,
    IndirectCallToNull = -2004,
    IntegerDivisionByZero = -2005,
    IntegerOverflow = -2006,
    BadConversionToInteger = -2007,
    StackOverflow = -2008,
    BadSignature = -2009,
    OutOfFuel = -2010,
    GrowthOperationLimited = -2011,
    UnresolvedFunction = -2013,
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
        Self::from(&value)
    }
}
#[cfg(feature = "rwasm")]
impl From<&TrapCode> for ExitCode {
    fn from(value: &TrapCode) -> Self {
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

    // SYS host
    EXIT = 0x0001,
    STATE = 0x0002,
    READ_INPUT = 0x0003,
    INPUT_SIZE = 0x0004,
    WRITE_OUTPUT = 0x0005,
    OUTPUT_SIZE = 0x0006,
    READ_OUTPUT = 0x0007,
    EXEC = 0x0009,
    RESUME = 0x000a,
    FORWARD_OUTPUT = 0x000b,
    CHARGE_FUEL = 0x000c,
    FUEL = 0x000d,
    PREIMAGE_SIZE = 0x000e,
    PREIMAGE_COPY = 0x000f,
    DEBUG_LOG = 0x0010,

    // hashing
    KECCAK256 = 0x0101,
    KECCAK256_PERMUTE = 0x0102,
    POSEIDON = 0x0103,
    POSEIDON_HASH = 0x0104,
    SHA256_EXTEND = 0x0105,
    SHA256_COMPRESS = 0x0106,

    // ed25519
    ED25519_ADD = 0x0107,
    ED25519_DECOMPRESS = 0x0108,

    // secp256k1
    SECP256K1_RECOVER = 0x0110,
    SECP256K1_ADD = 0x0111,
    SECP256K1_DECOMPRESS = 0x0112,
    SECP256K1_DOUBLE = 0x0113,

    // bls12381
    BLS12381_DECOMPRESS = 0x0120,
    BLS12381_ADD = 0x0121,
    BLS12381_DOUBLE = 0x0122,
    BLS12381_FP_ADD = 0x0123,
    BLS12381_FP_SUB = 0x0124,
    BLS12381_FP_MUL = 0x0125,
    BLS12381_FP2_ADD = 0x0126,
    BLS12381_FP2_SUB = 0x0127,
    BLS12381_FP2_MUL = 0x0128,

    // bn254
    BN254_ADD = 0x0130,
    BN254_DOUBLE = 0x0131,
    BN254_FP_ADD = 0x0132,
    BN254_FP_SUB = 0x0133,
    BN254_FP_MUL = 0x0134,
    BN254_FP2_ADD = 0x0135,
    BN254_FP2_SUB = 0x0136,
    BN254_FP2_MUL = 0x0137,

    // uint256
    UINT256_MUL = 0x011D,
    // sp1
    // WRITE_FD = 0x0202,
    // ENTER_UNCONSTRAINED = 0x0203,
    // EXIT_UNCONSTRAINED = 0x0204,
    // COMMIT = 0x0210,
    // COMMIT_DEFERRED_PROOFS = 0x021A,
    // VERIFY_SP1_PROOF = 0x021B,
    // HINT_LEN = 0x02F0,
    // HINT_READ = 0x02F1,
}

impl SysFuncIdx {
    pub fn fuel_cost(&self) -> u32 {
        match self {
            SysFuncIdx::EXIT => 1,
            SysFuncIdx::STATE => 1,
            SysFuncIdx::READ_INPUT => 1,
            SysFuncIdx::INPUT_SIZE => 1,
            SysFuncIdx::WRITE_OUTPUT => 1,
            SysFuncIdx::KECCAK256 => 1,
            SysFuncIdx::POSEIDON => 1,
            SysFuncIdx::POSEIDON_HASH => 1,
            SysFuncIdx::SECP256K1_RECOVER => 1,
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

pub const SYSCALL_ID_STORAGE_READ: B256 = B256::with_last_byte(0x01);
pub const SYSCALL_ID_STORAGE_WRITE: B256 = B256::with_last_byte(0x02);
pub const SYSCALL_ID_CALL: B256 = B256::with_last_byte(0x03);
pub const SYSCALL_ID_STATIC_CALL: B256 = B256::with_last_byte(0x04);
pub const SYSCALL_ID_CALL_CODE: B256 = B256::with_last_byte(0x05);
pub const SYSCALL_ID_DELEGATE_CALL: B256 = B256::with_last_byte(0x06);
pub const SYSCALL_ID_CREATE: B256 = B256::with_last_byte(0x07);
pub const SYSCALL_ID_CREATE2: B256 = B256::with_last_byte(0x08);
pub const SYSCALL_ID_EMIT_LOG: B256 = B256::with_last_byte(0x09);
pub const SYSCALL_ID_DESTROY_ACCOUNT: B256 = B256::with_last_byte(0x0a);
pub const SYSCALL_ID_BALANCE: B256 = B256::with_last_byte(0x0b);
pub const SYSCALL_ID_WRITE_PREIMAGE: B256 = B256::with_last_byte(0x0c);
pub const SYSCALL_ID_PREIMAGE_COPY: B256 = B256::with_last_byte(0x0d);
pub const SYSCALL_ID_PREIMAGE_SIZE: B256 = B256::with_last_byte(0x0e);
pub const SYSCALL_ID_EXT_STORAGE_READ: B256 = B256::with_last_byte(0x0f);
pub const SYSCALL_ID_TRANSIENT_READ: B256 = B256::with_last_byte(0x10);
pub const SYSCALL_ID_TRANSIENT_WRITE: B256 = B256::with_last_byte(0x11);

pub const FUEL_LIMIT_SYSCALL_STORAGE_READ: u64 = 2_100 * FUEL_DENOM_RATE;
pub const FUEL_LIMIT_SYSCALL_STORAGE_WRITE: u64 = 22_100 * FUEL_DENOM_RATE;
pub const FUEL_LIMIT_SYSCALL_EMIT_LOG: u64 = 10_000 * FUEL_DENOM_RATE;
pub const FUEL_LIMIT_SYSCALL_DESTROY_ACCOUNT: u64 = 32_600 * FUEL_DENOM_RATE;
pub const FUEL_LIMIT_SYSCALL_BALANCE: u64 = 2_600 * FUEL_DENOM_RATE;
pub const FUEL_LIMIT_SYSCALL_EXT_STORAGE_READ: u64 = 2_100 * FUEL_DENOM_RATE;
pub const FUEL_LIMIT_SYSCALL_PREIMAGE_SIZE: u64 = 2_600 * FUEL_DENOM_RATE;
pub const FUEL_LIMIT_SYSCALL_TRANSIENT_READ: u64 = 100 * FUEL_DENOM_RATE;
pub const FUEL_LIMIT_SYSCALL_TRANSIENT_WRITE: u64 = 100 * FUEL_DENOM_RATE;
