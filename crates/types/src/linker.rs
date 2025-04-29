use crate::{
    sys_func_idx::{
        SysFuncIdx,
        SysFuncIdx::{
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
    },
    SysFuncIdx::CHARGE_FUEL,
};
use alloc::vec::Vec;
use rwasm::{
    core::{ImportLinker, ImportLinkerEntity, UntypedValue, ValueType},
    engine::bytecode::{FuncIdx, Instruction, LocalDepth},
    module::ImportName,
};
use ValueType::{I32, I64};

const SHARED_IMPORT_LINKER: [(
    &'static str,
    SysFuncIdx,
    &'static [ValueType],
    &'static [ValueType],
    &'static [Instruction],
); 18] = [
    (
        "_keccak256",
        KECCAK256,
        &[I32; 3],
        &[],
        &[
            Instruction::LocalGet(LocalDepth::from_u32(1)), // second argument is the input size
            Instruction::I32Const(UntypedValue::from_bits(0)),
            Instruction::I32Mul,
            Instruction::Call(FuncIdx::from_u32(CHARGE_FUEL as u32)),
        ],
    ),
    ("_exit", EXIT, &[I32; 1], &[], &[]),
    ("_state", STATE, &[], &[I32; 1], &[]),
    ("_read", READ_INPUT, &[I32; 3], &[], &[]),
    ("_input_size", INPUT_SIZE, &[], &[I32; 1], &[]),
    ("_write", WRITE_OUTPUT, &[I32; 2], &[], &[]),
    ("_output_size", OUTPUT_SIZE, &[], &[I32; 1], &[]),
    ("_read_output", READ_OUTPUT, &[I32; 3], &[], &[]),
    ("_exec", EXEC, &[I32; 5], &[I32; 1], &[]),
    ("_resume", EXEC, &[I32; 5], &[I32; 1], &[]),
    ("_forward_output", FORWARD_OUTPUT, &[I32; 2], &[], &[]),
    (
        "_charge_fuel_manually",
        CHARGE_FUEL_MANUALLY,
        &[I64; 2],
        &[I64; 1],
        &[],
    ),
    ("_charge_fuel", CHARGE_FUEL, &[I64; 1], &[], &[]),
    ("_fuel", FUEL, &[], &[I64; 1], &[]),
    ("_preimage_size", PREIMAGE_SIZE, &[I32; 1], &[I32; 1], &[]),
    ("_preimage_copy", PREIMAGE_COPY, &[I32; 2], &[], &[]),
    ("_debug_log", DEBUG_LOG, &[I32; 2], &[], &[]),
    (
        "_secp256k1_recover",
        SECP256K1_RECOVER,
        &[I32; 4],
        &[I32; 1],
        &[],
    ),
];

const MODULE: &'static str = "fluentbase_v1preview";

pub fn create_import_linker() -> ImportLinker {
    let entities: Vec<(ImportName, ImportLinkerEntity)> = SHARED_IMPORT_LINKER
        .iter()
        .map(|(name, func_idx, params, result, fuel_procedure)| {
            (
                ImportName::new(MODULE, *name),
                ImportLinkerEntity {
                    func_idx: (*func_idx).into(),
                    fuel_procedure: *fuel_procedure,
                    params: *params,
                    result: *result,
                },
            )
        })
        .collect();
    ImportLinker::from(entities)
}

pub fn get_import_linker_symbols() -> Vec<&'static str> {
    let mut symbols: Vec<&str> = SHARED_IMPORT_LINKER
        .iter()
        .map(|(name, _, _, _, _)| *name)
        .collect();
    symbols.sort();
    symbols
}
