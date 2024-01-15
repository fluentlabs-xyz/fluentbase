use crate::translator::{host::Host, translator::Translator};
use fluentbase_rwasm::{module::ImportName, rwasm::InstructionSet};

#[derive(Clone)]
pub(super) enum SystemFunc {
    CryptoKeccak256,
    EvmSstore,
    EvmSload,
    SysHalt,
    SysWrite,
}

impl SystemFunc {
    fn to_str(&self) -> &str {
        match self {
            SystemFunc::CryptoKeccak256 => "_crypto_keccak256",
            SystemFunc::EvmSstore => "_evm_sstore",
            SystemFunc::EvmSload => "_evm_sload",
            SystemFunc::SysHalt => "_sys_halt",
            SystemFunc::SysWrite => "_sys_write",
        }
    }
    fn set_from_str(&mut self, fn_name: &str) {
        match fn_name {
            "_crypto_keccak256" => *self = SystemFunc::CryptoKeccak256,
            "_evm_sstore" => *self = SystemFunc::EvmSstore,
            "_evm_sload" => *self = SystemFunc::EvmSload,
            "_sys_halt" => *self = SystemFunc::SysHalt,
            "_sys_write" => *self = SystemFunc::SysWrite,
            _ => panic!("unknown func name '{}'", fn_name),
        }
    }
    fn from_str(fn_name: &str) -> SystemFunc {
        match fn_name {
            "_crypto_keccak256" => SystemFunc::CryptoKeccak256,
            "_evm_sstore" => SystemFunc::EvmSstore,
            "_evm_sload" => SystemFunc::EvmSload,
            "_sys_halt" => SystemFunc::SysHalt,
            "_sys_write" => SystemFunc::SysWrite,
            _ => panic!("unknown func name '{}'", fn_name),
        }
    }
}

pub(super) fn wasm_call(
    translator: &mut Translator,
    is: &mut InstructionSet,
    fn_name: SystemFunc,
) -> u64 {
    let mut ops_count = is.len() as u64;
    let import_fn_idx =
        translator.get_import_linker().index_mapping()[&ImportName::new("env", fn_name.to_str())].0;
    is.op_call(import_fn_idx);

    ops_count = is.len() as u64 - ops_count;
    ops_count
}

pub(super) fn preprocess_op_params(translator: &mut Translator<'_>, host: &mut dyn Host) {
    let opcode = translator.opcode_prev();
    let instruction_set = host.instruction_set();
    let meta = translator
        .subroutine_data(opcode)
        .expect(&format!("no meta found for 0x{:x?} opcode", opcode));
    let prev_funcs_len = meta.begin_offset as u32;
    let is_len = instruction_set.len();
    let return_offset = is_len - prev_funcs_len;
    instruction_set.op_i32_const(return_offset);
}

pub(super) fn replace_current_opcode_with_call_to_subroutine(
    translator: &mut Translator<'_>,
    host: &mut dyn Host,
) {
    preprocess_op_params(translator, host);

    let is = host.instruction_set();
    let op = translator.opcode_prev();
    let sd = translator
        .subroutine_data(op)
        .expect(format!("subroutine data not found for opcode 0x{:x?}", op).as_str());

    let is_len = is.len();
    let se = sd.begin_offset as i32 - is_len as i32 + 2 + sd.rel_entry_offset as i32;
    is.op_br(se);
}
