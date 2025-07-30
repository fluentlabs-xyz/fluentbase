use crate::{emit_fuel_procedure, SysFuncIdx};
use alloc::rc::Rc;
use rwasm::{ImportLinker, ImportName, ValType};

pub fn create_import_linker() -> Rc<ImportLinker> {
    let mut import_linker = ImportLinker::default();
    macro_rules! import_function {
        ($func_name:literal, $sys_func_idx:ident, $params:expr, $results:expr) => {
            import_linker.insert_function(
                ImportName::new("fluentbase_v1preview", $func_name),
                SysFuncIdx::$sys_func_idx as u32,
                emit_fuel_procedure(SysFuncIdx::$sys_func_idx),
                $params,
                $results,
            );
        };
    }
    import_function!("_keccak256", KECCAK256, &[ValType::I32; 3], &[]);
    import_function!("_exit", EXIT, &[ValType::I32; 1], &[]);
    import_function!("_state", STATE, &[], &[ValType::I32; 1]);
    import_function!("_read", READ_INPUT, &[ValType::I32; 3], &[]);
    import_function!("_input_size", INPUT_SIZE, &[], &[ValType::I32; 1]);
    import_function!("_write", WRITE_OUTPUT, &[ValType::I32; 2], &[]);
    import_function!("_output_size", OUTPUT_SIZE, &[], &[ValType::I32; 1]);
    import_function!("_read_output", READ_OUTPUT, &[ValType::I32; 3], &[]);
    import_function!("_exec", EXEC, &[ValType::I32; 5], &[ValType::I32; 1]);
    import_function!("_resume", RESUME, &[ValType::I32; 5], &[ValType::I32; 1]);
    import_function!("_forward_output", FORWARD_OUTPUT, &[ValType::I32; 2], &[]);
    import_function!(
        "_charge_fuel_manually",
        CHARGE_FUEL_MANUALLY,
        &[ValType::I64; 2],
        &[ValType::I64; 1]
    );
    import_function!("_charge_fuel", CHARGE_FUEL, &[ValType::I64; 1], &[]);
    import_function!("_fuel", FUEL, &[], &[ValType::I64; 1]);
    import_function!(
        "_preimage_size",
        PREIMAGE_SIZE,
        &[ValType::I32; 1],
        &[ValType::I32; 1]
    );
    import_function!("_preimage_copy", PREIMAGE_COPY, &[ValType::I32; 2], &[]);
    import_function!("_debug_log", DEBUG_LOG, &[ValType::I32; 2], &[]);
    import_function!(
        "_secp256k1_recover",
        SECP256K1_RECOVER,
        &[ValType::I32; 4],
        &[ValType::I32; 1]
    );
    import_function!("_bn254_add", BN254_ADD, &[ValType::I32; 2], &[]);
    import_function!("_bn254_double", BN254_DOUBLE, &[ValType::I32; 1], &[]);
    import_function!("_bn254_mul", BN254_MUL, &[ValType::I32; 2], &[]);
    import_function!(
        "_bn254_multi_pairing",
        BN254_MULTI_PAIRING,
        &[ValType::I32; 3],
        &[]
    );
    import_function!("_bn254_fp_mul", BN254_FP_MUL, &[ValType::I32; 2], &[]);
    import_function!("_bn254_fp2_mul", BN254_FP2_MUL, &[ValType::I32; 2], &[]);
    Rc::new(import_linker)
}
