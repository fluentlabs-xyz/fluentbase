use rwasm::core::{ImportLinker, ImportLinkerEntity, ValueType};

macro_rules! import_func {
    ($name:literal, $sys_func_idx:ident, $params:expr, $result:expr) => {
        (
            "fluentbase_v1preview",
            $name,
            ImportLinkerEntity {
                func_idx: $crate::SysFuncIdx::$sys_func_idx as u32,
                block_fuel: 0,
                params: &$params,
                result: &$result,
            },
        )
    };
}

const SHARED_IMPORT_LINKER: [(&'static str, &'static str, ImportLinkerEntity); 17] = [
    import_func!("_keccak256", KECCAK256, [ValueType::I32; 3], []),
    import_func!(
        "_secp256k1_recover",
        SECP256K1_RECOVER,
        [ValueType::I32; 4],
        [ValueType::I32; 1]
    ),
    import_func!("_exit", EXIT, [ValueType::I32; 1], []),
    import_func!("_state", STATE, [], [ValueType::I32; 1]),
    import_func!("_read", READ_INPUT, [ValueType::I32; 3], []),
    import_func!("_input_size", INPUT_SIZE, [], [ValueType::I32; 1]),
    import_func!("_write", WRITE_OUTPUT, [ValueType::I32; 2], []),
    import_func!("_output_size", OUTPUT_SIZE, [], [ValueType::I32; 1]),
    import_func!("_read_output", READ_OUTPUT, [ValueType::I32; 3], []),
    import_func!("_exec", EXEC, [ValueType::I32; 5], [ValueType::I32; 1]),
    import_func!("_resume", EXEC, [ValueType::I32; 5], [ValueType::I32; 1]),
    import_func!("_forward_output", FORWARD_OUTPUT, [ValueType::I32; 2], []),
    import_func!(
        "_charge_fuel",
        CHARGE_FUEL,
        [ValueType::I64; 2],
        [ValueType::I64; 1]
    ),
    import_func!("_fuel", FUEL, [], [ValueType::I64; 1]),
    import_func!(
        "_preimage_size",
        PREIMAGE_SIZE,
        [ValueType::I32; 1],
        [ValueType::I32; 1]
    ),
    import_func!("_preimage_copy", PREIMAGE_COPY, [ValueType::I32; 2], []),
    import_func!("_debug_log", DEBUG_LOG, [ValueType::I32; 2], []),
];

pub fn create_import_linker() -> ImportLinker {
    ImportLinker::from(SHARED_IMPORT_LINKER)
}
