use crate::{
    fuel_procedures::{
        CHARGE_FUEL_FUEL,
        CHARGE_FUEL_MANUALLY_FUEL,
        DEBUG_LOG_FUEL,
        EXEC_FUEL,
        EXIT_FUEL,
        FORWARD_OUTPUT_FUEL,
        FUEL_FUEL,
        INPUT_SIZE_FUEL,
        KECCAK256_FUEL,
        OUTPUT_SIZE_FUEL,
        PREIMAGE_COPY_FUEL,
        PREIMAGE_SIZE_FUEL,
        READ_INPUT_FUEL,
        READ_OUTPUT_FUEL,
        RESUME_FUEL,
        SECP256K1_RECOVER_FUEL,
        STATE_FUEL,
        WRITE_OUTPUT_FUEL,
    },
    sys_func_idx::SysFuncIdx::{
        CHARGE_FUEL_MANUALLY,
        DEBUG_LOG,
        EXEC,
        EXIT,
        FORWARD_OUTPUT,
        FUEL,
        INPUT_SIZE,
        KECCAK256,
        OUTPUT_SIZE,
        PREIMAGE_COPY,
        PREIMAGE_SIZE,
        READ_INPUT,
        READ_OUTPUT,
        SECP256K1_RECOVER,
        STATE,
        WRITE_OUTPUT,
    },
    SysFuncIdx,
    SysFuncIdx::CHARGE_FUEL,
};
use alloc::vec::Vec;
use rwasm::{
    core::{ImportLinker, ImportLinkerEntity, ValueType},
    engine::bytecode::Instruction,
    module::ImportName,
};
use ValueType::{I32, I64};

const MODULE: &str = "fluentbase_v1preview";

#[rustfmt::skip]
const SHARED_IMPORT_LINKER: [(&str, SysFuncIdx, &[ValueType], &[ValueType], &[Instruction]); 18] = [
    ("_keccak256", KECCAK256, &[I32; 3], &[], KECCAK256_FUEL),
    ("_exit", EXIT, &[I32; 1], &[], EXIT_FUEL),
    ("_state", STATE, &[], &[I32; 1], STATE_FUEL),
    ("_read", READ_INPUT, &[I32; 3], &[], READ_INPUT_FUEL),
    ("_input_size", INPUT_SIZE, &[], &[I32; 1], INPUT_SIZE_FUEL),
    ("_write", WRITE_OUTPUT, &[I32; 2], &[], WRITE_OUTPUT_FUEL),
    ("_output_size", OUTPUT_SIZE, &[], &[I32; 1], OUTPUT_SIZE_FUEL),
    ("_read_output", READ_OUTPUT, &[I32; 3], &[], READ_OUTPUT_FUEL),
    ("_exec", EXEC, &[I32; 5], &[I32; 1], EXEC_FUEL),
    ("_resume", EXEC, &[I32; 5], &[I32; 1], RESUME_FUEL),
    ("_forward_output", FORWARD_OUTPUT, &[I32; 2], &[], FORWARD_OUTPUT_FUEL),
    ("_charge_fuel_manually", CHARGE_FUEL_MANUALLY, &[I64; 2], &[I64; 1], CHARGE_FUEL_MANUALLY_FUEL),
    ("_charge_fuel", CHARGE_FUEL, &[I64; 1], &[], CHARGE_FUEL_FUEL),
    ("_fuel", FUEL, &[], &[I64; 1], FUEL_FUEL),
    ("_preimage_size", PREIMAGE_SIZE, &[I32; 1], &[I32; 1], PREIMAGE_SIZE_FUEL,),
    ("_preimage_copy", PREIMAGE_COPY, &[I32; 2], &[], PREIMAGE_COPY_FUEL,),
    ("_debug_log", DEBUG_LOG, &[I32; 2], &[], DEBUG_LOG_FUEL),
    ("_secp256k1_recover", SECP256K1_RECOVER, &[I32; 4], &[I32; 1], SECP256K1_RECOVER_FUEL),
];

pub fn create_import_linker() -> ImportLinker {
    ImportLinker::from(SHARED_IMPORT_LINKER.iter().map(
        |(name, func_idx, params, result, fuel_procedure)| {
            (
                ImportName::new(MODULE, *name),
                ImportLinkerEntity {
                    func_idx: *func_idx as u32,
                    fuel_procedure,
                    params,
                    result,
                },
            )
        },
    ))
}

pub fn get_import_linker_symbols() -> Vec<&'static str> {
    let mut symbols: Vec<&str> = SHARED_IMPORT_LINKER.iter().map(|value| value.0).collect();
    symbols.sort();
    symbols
}
