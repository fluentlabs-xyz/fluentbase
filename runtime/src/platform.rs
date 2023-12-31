use fluentbase_rwasm::{
    engine::{bytecode::FuncIdx, CompiledFunc},
    RwOp,
};
use strum::IntoEnumIterator;
use strum_macros::EnumIter;

#[allow(non_camel_case_types)]
#[derive(Default, Clone, Copy, Debug, PartialEq, Eq, Hash, Ord, PartialOrd, EnumIter)]
pub enum SysFuncIdx {
    #[default]
    UNKNOWN = 0x0000,
    // SYS host functions (starts with 0xAA00)
    SYS_HALT = 0xA001,  // env::_sys_halt
    SYS_STATE = 0xA002, // env::_sys_state
    SYS_READ = 0xA003,  // env::_sys_read
    SYS_INPUT = 0xA004, // env::_sys_input
    SYS_WRITE = 0xA005, // env::_sys_write
    // WASI runtime
    WASI_PROC_EXIT = 0xB001,         // wasi_snapshot_preview1::proc_exit
    WASI_FD_WRITE = 0xB002,          // wasi_snapshot_preview1::fd_write
    WASI_ENVIRON_SIZES_GET = 0xB003, // wasi_snapshot_preview1::environ_sizes_get
    WASI_ENVIRON_GET = 0xB004,       // wasi_snapshot_preview1::environ_get
    WASI_ARGS_SIZES_GET = 0xB005,    // wasi_snapshot_preview1::args_sizes_get
    WASI_ARGS_GET = 0xB006,          // wasi_snapshot_preview1::args_get
    // RWASM runtime
    RWASM_TRANSACT = 0xC001, // env::_rwasm_transact
    RWASM_COMPILE = 0xC002,  // env::_rwasm_compile
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
    ECC_SECP256K1_VERIFY = 0xE104,
    ECC_SECP256K1_RECOVER = 0xE105,
    // EVM
    EVM_SLOAD = 0xFF01,
    EVM_SSTORE = 0xFF02,
    EVM_CALLER = 0xFF03,
    EVM_CALLVALUE = 0xFF04,
    EVM_ADDRESS = 0xFF05,
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

impl SysFuncIdx {
    pub fn get_rw_rows(&self) -> Vec<RwOp> {
        match self {
            SysFuncIdx::SYS_HALT => {
                vec![RwOp::StackRead(0)]
            }
            _ => vec![],
        }
    }
}

// SYS host functions (starts with 0xAA00)
// pub const IMPORT_SYS_HALT: u16 = 0xAA01;
// pub const IMPORT_SYS_WRITE: u16 = 0xAA02;
// pub const IMPORT_SYS_READ: u16 = 0xAA03;

// EVM-compatible host functions (starts with 0xEE00)
// pub const IMPORT_EVM_STOP: u16 = 0xEE01;
// pub const IMPORT_EVM_RETURN: u16 = 0xEE02;
// pub const IMPORT_EVM_KECCAK256: u16 = 0xEE03;
// pub const IMPORT_EVM_ADDRESS: u16 = 0xEE04;
// pub const IMPORT_EVM_BALANCE: u16 = 0xEE05;
// pub const IMPORT_EVM_ORIGIN: u16 = 0xEE06;
// pub const IMPORT_EVM_CALLER: u16 = 0xEE07;
// pub const IMPORT_EVM_CALLVALUE: u16 = 0xEE08;
// pub const IMPORT_EVM_CALLDATALOAD: u16 = 0xEE09;
// pub const IMPORT_EVM_CALLDATASIZE: u16 = 0xEE0A;
// pub const IMPORT_EVM_CALLDATACOPY: u16 = 0xEE0B;
// pub const IMPORT_EVM_CODESIZE: u16 = 0xEE0C;
// pub const IMPORT_EVM_CODECOPY: u16 = 0xEE0D;
// pub const IMPORT_EVM_GASPRICE: u16 = 0xEE0E;
// pub const IMPORT_EVM_EXTCODESIZE: u16 = 0xEE0F;
// pub const IMPORT_EVM_EXTCODECOPY: u16 = 0xEE10;
// pub const IMPORT_EVM_EXTCODEHASH: u16 = 0xEE11;
// pub const IMPORT_EVM_RETURNDATASIZE: u16 = 0xEE12;
// pub const IMPORT_EVM_RETURNDATACOPY: u16 = 0xEE13;
// pub const IMPORT_EVM_BLOCKHASH: u16 = 0xEE14;
// pub const IMPORT_EVM_COINBASE: u16 = 0xEE15;
// pub const IMPORT_EVM_TIMESTAMP: u16 = 0xEE16;
// pub const IMPORT_EVM_NUMBER: u16 = 0xEE17;
// pub const IMPORT_EVM_DIFFICULTY: u16 = 0xEE18;
// pub const IMPORT_EVM_GASLIMIT: u16 = 0xEE19;
// pub const IMPORT_EVM_CHAINID: u16 = 0xEE1A;
// pub const IMPORT_EVM_BASEFEE: u16 = 0xEE1B;
// pub const IMPORT_EVM_SLOAD: u16 = 0xEE1C;
// pub const IMPORT_EVM_SSTORE: u16 = 0xEE1D;
// pub const IMPORT_EVM_LOG0: u16 = 0xEE1E;
// pub const IMPORT_EVM_LOG1: u16 = 0xEE1F;
// pub const IMPORT_EVM_LOG2: u16 = 0xEE20;
// pub const IMPORT_EVM_LOG3: u16 = 0xEE21;
// pub const IMPORT_EVM_LOG4: u16 = 0xEE22;
// pub const IMPORT_EVM_CREATE: u16 = 0xEE23;
// pub const IMPORT_EVM_CALL: u16 = 0xEE24;
// pub const IMPORT_EVM_CALLCODE: u16 = 0xEE25;
// pub const IMPORT_EVM_DELEGATECALL: u16 = 0xEE26;
// pub const IMPORT_EVM_CREATE2: u16 = 0xEE27;
// pub const IMPORT_EVM_STATICCALL: u16 = 0xEE28;
// pub const IMPORT_EVM_REVERT: u16 = 0xEE29;
// pub const IMPORT_EVM_SELFDESTRUCT: u16 = 0xEE2A;
