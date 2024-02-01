use rwasm::{
    common::{Trap, TrapCode},
    engine::{bytecode::FuncIdx, CompiledFunc},
};
#[cfg(feature = "std")]
use strum::IntoEnumIterator;

#[derive(Debug, Copy, Clone)]
pub enum ExitCode {
    Ok = 0,
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
}

impl ExitCode {
    pub fn into_i32(self) -> i32 {
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
    // SYS host functions (starts with 0x0000)
    SYS_HALT = 0x0001,        // fluentbase_v1alpha::_sys_halt
    SYS_WRITE = 0x0005,       // fluentbase_v1alpha::_sys_write
    SYS_INPUT_SIZE = 0x0004,  // fluentbase_v1alpha::_sys_input_size
    SYS_READ = 0x0003,        // fluentbase_v1alpha::_sys_read
    SYS_OUTPUT_SIZE = 0x0006, // fluentbase_v1alpha::_sys_output_size
    SYS_READ_OUTPUT = 0x0007, // fluentbase_v1alpha::_sys_read_output
    SYS_EXEC = 0x0008,        // fluentbase_v1alpha::_sys_exec
    SYS_STATE = 0x0002,       // fluentbase_v1alpha::_sys_state
    // RWASM
    RWASM_TRANSACT = 0x000A, // fluentbase_v1alpha::_rwasm_transact
    RWASM_COMPILE = 0x000B,  // fluentbase_v1alpha::_rwasm_compile
    RWASM_CREATE = 0x000C,   // fluentbase_v1alpha::_rwasm_create
    // crypto functions
    CRYPTO_KECCAK256 = 0x0101, // fluentbase_v1alpha::_sys_keccak256
    CRYPTO_POSEIDON = 0x0102,  // fluentbase_v1alpha::_sys_poseidon
    CRYPTO_POSEIDON2 = 0x0103, // fluentbase_v1alpha::_sys_poseidon2
    CRYPTO_ECRECOVER = 0x0104, // fluentbase_v1alpha::_sys_ecrecover
    // preimage functions
    PREIMAGE_SIZE = 0x0701, // fluentbase_v1alpha::_preimage_size
    PREIMAGE_COPY = 0x0702, // fluentbase_v1alpha::_preimage_copy
    // zktrie functions (0x5A54 means ZT)
    ZKTRIE_OPEN = 0x0201,     // fluentbase_v1alpha::_zktrie_open
    ZKTRIE_UPDATE = 0x0202,   // fluentbase_v1alpha::_zktrie_update
    ZKTRIE_FIELD = 0x0203,    // fluentbase_v1alpha::_zktrie_field
    ZKTRIE_ROOT = 0x0204,     // fluentbase_v1alpha::_zktrie_root
    ZKTRIE_ROLLBACK = 0x0205, // fluentbase_v1alpha::_zktrie_rollback
    ZKTRIE_COMMIT = 0x0206,   // fluentbase_v1alpha::_zktrie_commit
    // statedb functions
    STATEDB_GET_CODE = 0x0501,      // fluentbase_v1alpha::_statedb_get_code
    STATEDB_GET_CODE_SIZE = 0x0502, // fluentbase_v1alpha::_statedb_get_code_size
    STATEDB_UPDATE_CODE = 0x0503,   // fluentbase_v1alpha::_statedb_update_code
    STATEDB_GET_STORAGE = 0x0504,   // fluentbase_v1alpha::_statedb_get_storage
    STATEDB_UPDATE_STORAGE = 0x0505, // fluentbase_v1alpha::_statedb_update_storage
    STATEDB_EMIT_LOG = 0x0506,      // fluentbase_v1alpha::_statedb_add_log
    STATEDB_GET_BALANCE = 0x0507,   // fluentbase_v1alpha::_statedb_get_balance
    STATEDB_GET_CODE_HASH = 0x0508, // fluentbase_v1alpha::_statedb_get_code_hash
    // WASI runtime (0x5741 means WA)
    WASI_PROC_EXIT = 0x0301,         // wasi_snapshot_preview1::proc_exit
    WASI_FD_WRITE = 0x0302,          // wasi_snapshot_preview1::fd_write
    WASI_ENVIRON_SIZES_GET = 0x0303, // wasi_snapshot_preview1::environ_sizes_get
    WASI_ENVIRON_GET = 0x0304,       // wasi_snapshot_preview1::environ_get
    WASI_ARGS_SIZES_GET = 0x0305,    // wasi_snapshot_preview1::args_sizes_get
    WASI_ARGS_GET = 0x0306,          // wasi_snapshot_preview1::args_get
    // mpt trie (0x4D54 means MT)
    MPT_OPEN = 0x0401,
    MPT_UPDATE = 0x0402,
    MPT_GET = 0x0403,
    MPT_GET_ROOT = 0x0404,
}

impl SysFuncIdx {
    pub fn fuel_cost(&self) -> u32 {
        match self {
            SysFuncIdx::SYS_HALT => 1,
            SysFuncIdx::SYS_STATE => 1,
            SysFuncIdx::SYS_READ => 1,
            SysFuncIdx::SYS_INPUT_SIZE => 1,
            SysFuncIdx::SYS_WRITE => 1,
            SysFuncIdx::RWASM_TRANSACT => 1,
            SysFuncIdx::RWASM_COMPILE => 1,
            SysFuncIdx::RWASM_CREATE => 1,
            SysFuncIdx::CRYPTO_KECCAK256 => 1,
            SysFuncIdx::CRYPTO_POSEIDON => 1,
            SysFuncIdx::CRYPTO_POSEIDON2 => 1,
            SysFuncIdx::CRYPTO_ECRECOVER => 1,
            SysFuncIdx::ZKTRIE_OPEN => 1,
            SysFuncIdx::ZKTRIE_UPDATE => 1,
            SysFuncIdx::ZKTRIE_FIELD => 1,
            SysFuncIdx::ZKTRIE_ROOT => 1,
            SysFuncIdx::ZKTRIE_ROLLBACK => 1,
            SysFuncIdx::ZKTRIE_COMMIT => 1,
            SysFuncIdx::STATEDB_GET_STORAGE => 1,
            SysFuncIdx::STATEDB_UPDATE_STORAGE => 1,
            SysFuncIdx::STATEDB_EMIT_LOG => 1,
            SysFuncIdx::STATEDB_GET_BALANCE => 1,
            SysFuncIdx::STATEDB_GET_CODE_HASH => 1,
            SysFuncIdx::WASI_PROC_EXIT => 1,
            SysFuncIdx::WASI_FD_WRITE => 1,
            SysFuncIdx::WASI_ENVIRON_SIZES_GET => 1,
            SysFuncIdx::WASI_ENVIRON_GET => 1,
            SysFuncIdx::WASI_ARGS_SIZES_GET => 1,
            SysFuncIdx::WASI_ARGS_GET => 1,
            SysFuncIdx::MPT_OPEN => 1,
            SysFuncIdx::MPT_UPDATE => 1,
            SysFuncIdx::MPT_GET => 1,
            SysFuncIdx::MPT_GET_ROOT => 1,
            _ => 1, //unreachable!("not configured fuel for opcode: {:?}", self),
        }
    }
}

#[cfg(feature = "std")]
impl From<FuncIdx> for SysFuncIdx {
    fn from(value: FuncIdx) -> Self {
        for item in Self::iter() {
            if value.to_u32() == item as u32 {
                return item;
            }
        }
        Self::UNKNOWN
    }
}

impl Into<CompiledFunc> for SysFuncIdx {
    fn into(self) -> CompiledFunc {
        CompiledFunc::from(self as u32)
    }
}

impl Into<FuncIdx> for SysFuncIdx {
    fn into(self) -> FuncIdx {
        FuncIdx::from(self as u32)
    }
}
