macro_rules! import_func {
    ($name:literal, $sys_func_idx:ident) => {
        (
            "fluentbase_v1preview",
            $name,
            $crate::types::SysFuncIdx::$sys_func_idx as u32,
            0,
        )
    };
}

const SHARED_IMPORT_LINKER: [(&'static str, &'static str, u32, u32); 20] = [
    import_func!("_keccak256", KECCAK256),
    import_func!("_poseidon", KECCAK256),
    import_func!("_poseidon_hash", POSEIDON_HASH),
    import_func!("_ecrecover", ECRECOVER),
    import_func!("_exit", EXIT),
    import_func!("_state", STATE),
    import_func!("_read", READ),
    import_func!("_input_size", INPUT_SIZE),
    import_func!("_write", WRITE),
    import_func!("_output_size", OUTPUT_SIZE),
    import_func!("_read_output", READ_OUTPUT),
    import_func!("_exec", EXEC),
    import_func!("_resume", EXEC),
    import_func!("_forward_output", FORWARD_OUTPUT),
    import_func!("_charge_fuel", CHARGE_FUEL),
    import_func!("_fuel", FUEL),
    import_func!("_read_context", READ_CONTEXT),
    import_func!("_preimage_size", PREIMAGE_SIZE),
    import_func!("_preimage_copy", PREIMAGE_COPY),
    import_func!("_debug_log", DEBUG_LOG),
];

pub fn create_shared_import_linker<
    F: From<[(&'static str, &'static str, u32, u32); SHARED_IMPORT_LINKER.len()]>,
>() -> F {
    F::from(SHARED_IMPORT_LINKER)
}

const SOVEREIGN_IMPORT_LINKER: [(&'static str, &'static str, u32, u32); 20] = [
    import_func!("_keccak256", KECCAK256),
    import_func!("_poseidon", KECCAK256),
    import_func!("_poseidon_hash", POSEIDON_HASH),
    import_func!("_ecrecover", ECRECOVER),
    import_func!("_exit", EXIT),
    import_func!("_state", STATE),
    import_func!("_read", READ),
    import_func!("_input_size", INPUT_SIZE),
    import_func!("_write", WRITE),
    import_func!("_output_size", OUTPUT_SIZE),
    import_func!("_read_output", READ_OUTPUT),
    import_func!("_exec", EXEC),
    import_func!("_resume", EXEC),
    import_func!("_forward_output", FORWARD_OUTPUT),
    import_func!("_charge_fuel", CHARGE_FUEL),
    import_func!("_fuel", FUEL),
    import_func!("_read_context", READ_CONTEXT),
    import_func!("_preimage_size", PREIMAGE_SIZE),
    import_func!("_preimage_copy", PREIMAGE_COPY),
    import_func!("_debug_log", DEBUG_LOG),
];

pub fn create_sovereign_import_linker<
    F: From<[(&'static str, &'static str, u32, u32); SOVEREIGN_IMPORT_LINKER.len()]>,
>() -> F {
    F::from(SOVEREIGN_IMPORT_LINKER)
}
