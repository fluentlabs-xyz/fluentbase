use alloc::collections::BTreeMap;
use alloc::vec::Vec;

use lazy_static::lazy_static;

use crate::{
    common::ValueType, linker::LinkerError, module::ImportName, AsContextMut, Caller, Func, FuncType, Linker, Store,
};

pub trait ImportHandler {
    // sys calls
    fn sys_halt(&mut self, _exit_code: u32) {}
    fn sys_write(&mut self, _offset: u32, _length: u32) {}
    fn sys_read(&mut self, _offset: u32, _length: u32) {}
    // evm calls
    fn evm_return(&mut self, _offset: u32, _length: u32) {}
}

#[derive(Default, Debug)]
pub struct DefaultImportHandler {
    input: Vec<u8>,
    exit_code: u32,
    output: Vec<u8>,
}

impl ImportHandler for DefaultImportHandler {
    fn sys_halt(&mut self, exit_code: u32) {
        self.exit_code = exit_code;
    }

    fn sys_write(&mut self, _offset: u32, _length: u32) {}
    fn sys_read(&mut self, _offset: u32, _length: u32) {}

    fn evm_return(&mut self, _offset: u32, _length: u32) {}
}

impl DefaultImportHandler {
    pub fn new(input: Vec<u8>) -> Self {
        Self {
            input,
            ..Default::default()
        }
    }

    pub fn exit_code(&self) -> u32 {
        self.exit_code
    }

    pub fn output(&self) -> &Vec<u8> {
        &self.output
    }
}

// SYS host functions (starts with 0xAA00)
pub const IMPORT_SYS_HALT: u32 = 0xAA01;
pub const IMPORT_SYS_WRITE: u32 = 0xAA02;
pub const IMPORT_SYS_READ: u32 = 0xAA03;

// EVM-compatible host functions (starts with 0xEE00)
pub const IMPORT_EVM_STOP: u32 = 0xEE01;
pub const IMPORT_EVM_RETURN: u32 = 0xEE02;
pub const IMPORT_EVM_KECCAK256: u32 = 0xEE03;
pub const IMPORT_EVM_ADDRESS: u32 = 0xEE04;
pub const IMPORT_EVM_BALANCE: u32 = 0xEE05;
pub const IMPORT_EVM_ORIGIN: u32 = 0xEE06;
pub const IMPORT_EVM_CALLER: u32 = 0xEE07;
pub const IMPORT_EVM_CALLVALUE: u32 = 0xEE08;
pub const IMPORT_EVM_CALLDATALOAD: u32 = 0xEE09;
pub const IMPORT_EVM_CALLDATASIZE: u32 = 0xEE0A;
pub const IMPORT_EVM_CALLDATACOPY: u32 = 0xEE0B;
pub const IMPORT_EVM_CODESIZE: u32 = 0xEE0C;
pub const IMPORT_EVM_CODECOPY: u32 = 0xEE0D;
pub const IMPORT_EVM_GASPRICE: u32 = 0xEE0E;
pub const IMPORT_EVM_EXTCODESIZE: u32 = 0xEE0F;
pub const IMPORT_EVM_EXTCODECOPY: u32 = 0xEE10;
pub const IMPORT_EVM_EXTCODEHASH: u32 = 0xEE11;
pub const IMPORT_EVM_RETURNDATASIZE: u32 = 0xEE12;
pub const IMPORT_EVM_RETURNDATACOPY: u32 = 0xEE13;
pub const IMPORT_EVM_BLOCKHASH: u32 = 0xEE14;
pub const IMPORT_EVM_COINBASE: u32 = 0xEE15;
pub const IMPORT_EVM_TIMESTAMP: u32 = 0xEE16;
pub const IMPORT_EVM_NUMBER: u32 = 0xEE17;
pub const IMPORT_EVM_DIFFICULTY: u32 = 0xEE18;
pub const IMPORT_EVM_GASLIMIT: u32 = 0xEE19;
pub const IMPORT_EVM_CHAINID: u32 = 0xEE1A;
pub const IMPORT_EVM_BASEFEE: u32 = 0xEE1B;
pub const IMPORT_EVM_SLOAD: u32 = 0xEE1C;
pub const IMPORT_EVM_SSTORE: u32 = 0xEE1D;
pub const IMPORT_EVM_LOG0: u32 = 0xEE1E;
pub const IMPORT_EVM_LOG1: u32 = 0xEE1F;
pub const IMPORT_EVM_LOG2: u32 = 0xEE20;
pub const IMPORT_EVM_LOG3: u32 = 0xEE21;
pub const IMPORT_EVM_LOG4: u32 = 0xEE22;
pub const IMPORT_EVM_CREATE: u32 = 0xEE23;
pub const IMPORT_EVM_CALL: u32 = 0xEE24;
pub const IMPORT_EVM_CALLCODE: u32 = 0xEE25;
pub const IMPORT_EVM_DELEGATECALL: u32 = 0xEE26;
pub const IMPORT_EVM_CREATE2: u32 = 0xEE27;
pub const IMPORT_EVM_STATICCALL: u32 = 0xEE28;
pub const IMPORT_EVM_REVERT: u32 = 0xEE29;
pub const IMPORT_EVM_SELFDESTRUCT: u32 = 0xEE2A;

#[derive(Debug, Clone)]
pub struct ImportFunc {
    import_name: ImportName,
    index: u32,
    func_type: FuncType,
}

impl ImportFunc {
    pub fn new(import_name: ImportName, index: u32, func_type: FuncType) -> Self {
        Self {
            import_name,
            index,
            func_type,
        }
    }

    pub fn new_env<P, R>(fn_name: &'static str, index: u32, input: P, output: R) -> Self
    where
        P: IntoIterator<Item = ValueType>,
        R: IntoIterator<Item = ValueType>,
    {
        let func_type = FuncType::new::<P, R>(input, output);
        Self::new(ImportName::new("env", fn_name), index, func_type)
    }

    pub fn import_name(&self) -> &ImportName {
        &self.import_name
    }

    pub fn index(&self) -> u32 {
        self.index
    }

    pub fn func_type(&self) -> &FuncType {
        &self.func_type
    }
}

pub struct ImportLinker {
    func_by_index: BTreeMap<u32, ImportFunc>,
    func_by_name: BTreeMap<ImportName, u32>,
}

impl Default for ImportLinker {
    fn default() -> Self {
        let mut result = Self {
            func_by_index: Default::default(),
            func_by_name: Default::default(),
        };
        result.insert_function(ImportFunc::new_env(
            "_sys_halt",
            IMPORT_SYS_HALT,
            [ValueType::I32; 1],
            [],
        ));
        result.insert_function(ImportFunc::new_env(
            "_sys_read",
            IMPORT_SYS_READ,
            [ValueType::I32; 2],
            [],
        ));
        result.insert_function(ImportFunc::new_env(
            "_sys_write",
            IMPORT_SYS_WRITE,
            [ValueType::I32; 2],
            [],
        ));
        result
    }
}

impl ImportLinker {
    pub fn insert_function(&mut self, import_func: ImportFunc) {
        assert!(!self.func_by_index.contains_key(&import_func.index), "already persist");
        assert!(
            !self.func_by_name.contains_key(&import_func.import_name),
            "already persist"
        );
        self.func_by_index.insert(import_func.index, import_func.clone());
        self.func_by_name.insert(import_func.import_name, import_func.index);
    }

    pub fn attach_linker<D>(&mut self, linker: &mut Linker<D>, store: &mut Store<D>) -> Result<(), LinkerError>
    where
        D: ImportHandler,
    {
        macro_rules! link_call {
            ($fn_name:ident($arg1:ident: $type1:ident, $arg2:ident: $type2:ident)) => {
                linker.define(
                    "env",
                    concat!("_", stringify!($fn_name)),
                    Func::wrap(
                        store.as_context_mut(),
                        |mut caller: Caller<'_, D>, $arg1: $type1, $arg2: $type2| {
                            caller.data_mut().$fn_name($arg1, $arg2);
                        },
                    ),
                )?;
            };
            ($fn_name:ident($arg1:ident: $type1:ident)) => {
                linker.define(
                    "env",
                    concat!("_", stringify!($fn_name)),
                    Func::wrap(
                        store.as_context_mut(),
                        |mut caller: Caller<'_, D>, $arg1: $type1| {
                            caller.data_mut().$fn_name($arg1);
                        },
                    ),
                )?;
            };
        }
        link_call!(sys_halt(exit_code: u32));
        link_call!(sys_write(offset: u32, length: u32));
        link_call!(sys_read(offset: u32, length: u32));
        Ok(())
    }

    pub fn resolve_by_index(&self, index: u32) -> Option<&ImportFunc> {
        self.func_by_index.get(&index)
    }

    pub fn index_mapping(&self) -> BTreeMap<ImportName, u32> {
        self.func_by_name.clone()
    }
}

lazy_static! {

    static ref WAZM_CIRCUITS: BTreeMap<ImportName, u32> = BTreeMap::from([
        // SYS calls
        (ImportName::new("env", "_sys_halt"), IMPORT_SYS_HALT),
        // (ImportName::new("env", "_sys_write"), IMPORT_SYS_WRITE),
        // (ImportName::new("env", "_sys_read"), IMPORT_SYS_READ),
        // // EVM calls
        // (ImportName::new("env", "_evm_stop"), IMPORT_EVM_STOP),
        // (ImportName::new("env", "_evm_return"), IMPORT_EVM_RETURN),
        // (ImportName::new("env", "_evm_keccak256"), IMPORT_EVM_KECCAK256),
        // (ImportName::new("env", "_evm_address"), IMPORT_EVM_ADDRESS),
        // (ImportName::new("env", "_evm_balance"), IMPORT_EVM_BALANCE),
        // (ImportName::new("env", "_evm_origin"), IMPORT_EVM_ORIGIN),
        // (ImportName::new("env", "_evm_caller"), IMPORT_EVM_CALLER),
        // (ImportName::new("env", "_evm_callvalue"), IMPORT_EVM_CALLVALUE),
        // (ImportName::new("env", "_evm_calldataload"), IMPORT_EVM_CALLDATALOAD),
        // (ImportName::new("env", "_evm_calldatasize"), IMPORT_EVM_CALLDATASIZE),
        // (ImportName::new("env", "_evm_calldatacopy"), IMPORT_EVM_CALLDATACOPY),
        // (ImportName::new("env", "_evm_codesize"), IMPORT_EVM_CODESIZE),
        // (ImportName::new("env", "_evm_codecopy"), IMPORT_EVM_CODECOPY),
        // (ImportName::new("env", "_evm_gasprice"), IMPORT_EVM_GASPRICE),
        // (ImportName::new("env", "_evm_extcodesize"), IMPORT_EVM_EXTCODESIZE),
        // (ImportName::new("env", "_evm_extcodecopy"), IMPORT_EVM_EXTCODECOPY),
        // (ImportName::new("env", "_evm_extcodehash"), IMPORT_EVM_EXTCODEHASH),
        // (ImportName::new("env", "_evm_returndatasize"), IMPORT_EVM_RETURNDATASIZE),
        // (ImportName::new("env", "_evm_returndatacopy"), IMPORT_EVM_RETURNDATACOPY),
        // (ImportName::new("env", "_evm_blockhash"), IMPORT_EVM_BLOCKHASH),
        // (ImportName::new("env", "_evm_coinbase"), IMPORT_EVM_COINBASE),
        // (ImportName::new("env", "_evm_timestamp"), IMPORT_EVM_TIMESTAMP),
        // (ImportName::new("env", "_evm_number"), IMPORT_EVM_NUMBER),
        // (ImportName::new("env", "_evm_difficulty"), IMPORT_EVM_DIFFICULTY),
        // (ImportName::new("env", "_evm_gaslimit"), IMPORT_EVM_GASLIMIT),
        // (ImportName::new("env", "_evm_chainid"), IMPORT_EVM_CHAINID),
        // (ImportName::new("env", "_evm_basefee"), IMPORT_EVM_BASEFEE),
        // (ImportName::new("env", "_evm_sload"), IMPORT_EVM_SLOAD),
        // (ImportName::new("env", "_evm_sstore"), IMPORT_EVM_SSTORE),
        // (ImportName::new("env", "_evm_log0"), IMPORT_EVM_LOG0),
        // (ImportName::new("env", "_evm_log1"), IMPORT_EVM_LOG1),
        // (ImportName::new("env", "_evm_log2"), IMPORT_EVM_LOG2),
        // (ImportName::new("env", "_evm_log3"), IMPORT_EVM_LOG3),
        // (ImportName::new("env", "_evm_log4"), IMPORT_EVM_LOG4),
        // (ImportName::new("env", "_evm_create"), IMPORT_EVM_CREATE),
        // (ImportName::new("env", "_evm_call"), IMPORT_EVM_CALL),
        // (ImportName::new("env", "_evm_callcode"), IMPORT_EVM_CALLCODE),
        // (ImportName::new("env", "_evm_delegatecall"), IMPORT_EVM_DELEGATECALL),
        // (ImportName::new("env", "_evm_create2"), IMPORT_EVM_CREATE2),
        // (ImportName::new("env", "_evm_staticcall"), IMPORT_EVM_STATICCALL),
        // (ImportName::new("env", "_evm_revert"), IMPORT_EVM_REVERT),
        // (ImportName::new("env", "_evm_selfdestruct"), IMPORT_EVM_SELFDESTRUCT),
    ]);
}
