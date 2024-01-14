use fluentbase_rwasm::engine::{bytecode::FuncIdx, CompiledFunc};
use strum::IntoEnumIterator;
use strum_macros::EnumIter;

#[allow(non_camel_case_types)]
#[derive(Default, Clone, Copy, Debug, PartialEq, Eq, Hash, Ord, PartialOrd, EnumIter)]
pub enum SysFuncIdx {
    #[default]
    UNKNOWN = 0x0000,
    // SYS host functions (starts with 0xAA00)
    SYS_HALT = 0xA001,       // fluentbase_v1alpha::_sys_halt
    SYS_STATE = 0xA002,      // fluentbase_v1alpha::_sys_state
    SYS_READ = 0xA003,       // fluentbase_v1alpha::_sys_read
    SYS_INPUT_SIZE = 0xA004, // fluentbase_v1alpha::_sys_input_size
    SYS_WRITE = 0xA005,      // fluentbase_v1alpha::_sys_write
    // WASI runtime
    WASI_PROC_EXIT = 0xB001,         // wasi_snapshot_preview1::proc_exit
    WASI_FD_WRITE = 0xB002,          // wasi_snapshot_preview1::fd_write
    WASI_ENVIRON_SIZES_GET = 0xB003, // wasi_snapshot_preview1::environ_sizes_get
    WASI_ENVIRON_GET = 0xB004,       // wasi_snapshot_preview1::environ_get
    WASI_ARGS_SIZES_GET = 0xB005,    // wasi_snapshot_preview1::args_sizes_get
    WASI_ARGS_GET = 0xB006,          // wasi_snapshot_preview1::args_get
    // RWASM runtime
    RWASM_TRANSACT = 0xC001, // fluentbase_v1alpha::_rwasm_transact
    RWASM_COMPILE = 0xC002,  // fluentbase_v1alpha::_rwasm_compile
    // zktrie functions
    ZKTRIE_OPEN = 0xDD01,
    ZKTRIE_UPDATE_NONCE = 0xDD02,
    ZKTRIE_UPDATE_BALANCE = 0xDD03,
    ZKTRIE_UPDATE_STORAGE_ROOT = 0xDD04,
    ZKTRIE_UPDATE_CODE_HASH = 0xDD05,
    ZKTRIE_UPDATE_CODE_SIZE = 0xDD06,
    ZKTRIE_GET_NONCE = 0xDD07,
    ZKTRIE_GET_BALANCE = 0xDD08,
    ZKTRIE_GET_STORAGE_ROOT = 0xDD09,
    ZKTRIE_GET_CODE_HASH = 0xDD0A,
    ZKTRIE_GET_CODE_SIZE = 0xDD0B,
    ZKTRIE_UPDATE_STORE = 0xDD0C,
    ZKTRIE_GET_STORE = 0xDD0D,
    // mpt trie
    MPT_OPEN = 0xDF01,
    MPT_UPDATE = 0xDF02,
    MPT_GET = 0xDF03,
    MPT_GET_ROOT = 0xDF04,
    // crypto/ecc
    CRYPTO_KECCAK256 = 0xE001,
    CRYPTO_POSEIDON = 0xE002,
    CRYPTO_POSEIDON2 = 0xE003,
    CRYPTO_ECRECOVER = 0xE004,
}

impl SysFuncIdx {
    pub fn fuel_cost(&self) -> u32 {
        match self {
            SysFuncIdx::SYS_HALT => 1,
            SysFuncIdx::SYS_STATE => 1,
            SysFuncIdx::SYS_READ => 1,
            SysFuncIdx::SYS_INPUT_SIZE => 1,
            SysFuncIdx::SYS_WRITE => 1,
            SysFuncIdx::WASI_PROC_EXIT => 1,
            SysFuncIdx::WASI_FD_WRITE => 1,
            SysFuncIdx::WASI_ENVIRON_SIZES_GET => 1,
            SysFuncIdx::WASI_ENVIRON_GET => 1,
            SysFuncIdx::WASI_ARGS_SIZES_GET => 1,
            SysFuncIdx::WASI_ARGS_GET => 1,
            SysFuncIdx::RWASM_TRANSACT => 1,
            SysFuncIdx::RWASM_COMPILE => 1,
            SysFuncIdx::ZKTRIE_OPEN => 1,
            SysFuncIdx::ZKTRIE_UPDATE_NONCE => 1,
            SysFuncIdx::ZKTRIE_UPDATE_BALANCE => 1,
            SysFuncIdx::ZKTRIE_UPDATE_STORAGE_ROOT => 1,
            SysFuncIdx::ZKTRIE_UPDATE_CODE_HASH => 1,
            SysFuncIdx::ZKTRIE_UPDATE_CODE_SIZE => 1,
            SysFuncIdx::ZKTRIE_GET_NONCE => 1,
            SysFuncIdx::ZKTRIE_GET_BALANCE => 1,
            SysFuncIdx::ZKTRIE_GET_STORAGE_ROOT => 1,
            SysFuncIdx::ZKTRIE_GET_CODE_HASH => 1,
            SysFuncIdx::ZKTRIE_GET_CODE_SIZE => 1,
            SysFuncIdx::ZKTRIE_UPDATE_STORE => 1,
            SysFuncIdx::ZKTRIE_GET_STORE => 1,
            SysFuncIdx::MPT_OPEN => 1,
            SysFuncIdx::MPT_UPDATE => 1,
            SysFuncIdx::MPT_GET => 1,
            SysFuncIdx::MPT_GET_ROOT => 1,
            SysFuncIdx::CRYPTO_KECCAK256 => 1,
            SysFuncIdx::CRYPTO_POSEIDON => 1,
            SysFuncIdx::CRYPTO_POSEIDON2 => 1,
            SysFuncIdx::CRYPTO_ECRECOVER => 1,
            _ => unreachable!("not configured fuel for opcode: {:?}", self),
        }
    }
}

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
        FuncIdx::from(self as u16)
    }
}
