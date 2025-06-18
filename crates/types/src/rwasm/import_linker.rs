use crate::{rwasm::block_fuel::MINIMAL_BASE_FUEL_COST, SysFuncIdx};
use rwasm::{instruction_set, ImportLinker, ImportName, ValType};

pub fn create_import_linker() -> ImportLinker {
    // TODO(dmitry123): "optimize it, don't clone or use Arc/Rc"
    let mut import_linker = ImportLinker::default();
    macro_rules! import_function {
        ($func_name:literal, $sys_func_idx:ident, $params:expr, $results:expr, $fuel_expr:ident) => {
            import_linker.insert_function(
                ImportName::new("fluentbase_v1preview", $func_name),
                SysFuncIdx::$sys_func_idx as u32,
                instruction_set! {
                    .op_consume_fuel(MINIMAL_BASE_FUEL_COST as u32)
                },
                $params,
                $results,
            );
        };
    }
    import_function!(
        "_keccak256",
        KECCAK256,
        &[ValType::I32; 3],
        &[],
        KECCAK256_FUEL
    );
    import_function!("_exit", EXIT, &[ValType::I32; 1], &[], EXIT_FUEL);
    import_function!("_state", STATE, &[], &[ValType::I32; 1], STATE_FUEL);
    import_function!(
        "_read",
        READ_INPUT,
        &[ValType::I32; 3],
        &[],
        READ_INPUT_FUEL
    );
    import_function!(
        "_input_size",
        INPUT_SIZE,
        &[],
        &[ValType::I32; 1],
        INPUT_SIZE_FUEL
    );
    import_function!(
        "_write",
        WRITE_OUTPUT,
        &[ValType::I32; 2],
        &[],
        WRITE_OUTPUT_FUEL
    );
    import_function!(
        "_output_size",
        OUTPUT_SIZE,
        &[],
        &[ValType::I32; 1],
        OUTPUT_SIZE_FUEL
    );
    import_function!(
        "_read_output",
        READ_OUTPUT,
        &[ValType::I32; 3],
        &[],
        READ_OUTPUT_FUEL
    );
    import_function!(
        "_exec",
        EXEC,
        &[ValType::I32; 5],
        &[ValType::I32; 1],
        EXEC_FUEL
    );
    import_function!(
        "_resume",
        EXEC,
        &[ValType::I32; 5],
        &[ValType::I32; 1],
        RESUME_FUEL
    );
    import_function!(
        "_forward_output",
        FORWARD_OUTPUT,
        &[ValType::I32; 2],
        &[],
        FORWARD_OUTPUT_FUEL
    );
    import_function!(
        "_charge_fuel_manually",
        CHARGE_FUEL_MANUALLY,
        &[ValType::I64; 2],
        &[ValType::I64; 1],
        CHARGE_FUEL_MANUALLY_FUEL
    );
    import_function!(
        "_charge_fuel",
        CHARGE_FUEL,
        &[ValType::I64; 1],
        &[],
        CHARGE_FUEL_FUEL
    );
    import_function!("_fuel", FUEL, &[], &[ValType::I64; 1], FUEL_FUEL);
    import_function!(
        "_preimage_size",
        PREIMAGE_SIZE,
        &[ValType::I32; 1],
        &[ValType::I32; 1],
        PREIMAGE_SIZE_FUEL
    );
    import_function!(
        "_preimage_copy",
        PREIMAGE_COPY,
        &[ValType::I32; 2],
        &[],
        PREIMAGE_COPY_FUEL
    );
    import_function!(
        "_debug_log",
        DEBUG_LOG,
        &[ValType::I32; 2],
        &[],
        DEBUG_LOG_FUEL
    );
    import_function!(
        "_secp256k1_recover",
        SECP256K1_RECOVER,
        &[ValType::I32; 4],
        &[ValType::I32; 1],
        SECP256K1_RECOVER_FUEL
    );
    import_linker
}
