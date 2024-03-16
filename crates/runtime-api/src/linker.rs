macro_rules! import_func {
    ($name:literal, $sys_func_idx:ident) => {
        (
            "fluentbase_v1alpha",
            $name,
            $crate::types::SysFuncIdx::$sys_func_idx as u32,
            0,
        )
    };
}

const SHARED_IMPORT_LINKER: [(&'static str, &'static str, u32, u32); 20] = [
    import_func!("_crypto_keccak256", CRYPTO_KECCAK256),
    import_func!("_crypto_poseidon", CRYPTO_KECCAK256),
    import_func!("_crypto_poseidon2", CRYPTO_POSEIDON2),
    import_func!("_crypto_ecrecover", CRYPTO_ECRECOVER),
    import_func!("_sys_halt", SYS_HALT),
    import_func!("_sys_write", SYS_WRITE),
    import_func!("_sys_input_size", SYS_INPUT_SIZE),
    import_func!("_sys_read", SYS_READ),
    import_func!("_sys_output_size", SYS_OUTPUT_SIZE),
    import_func!("_sys_read_output", SYS_READ_OUTPUT),
    import_func!("_sys_forward_output", SYS_FORWARD_OUTPUT),
    import_func!("_sys_state", SYS_STATE),
    import_func!("_sys_exec", SYS_EXEC),
    // import_func!("_jzkt_open", JZKT_OPEN),
    // import_func!("_jzkt_checkpoint", JZKT_CHECKPOINT),
    import_func!("_jzkt_get", JZKT_GET),
    // import_func!("_jzkt_update", JZKT_UPDATE),
    // import_func!("_jzkt_update_preimage", JZKT_UPDATE_PREIMAGE),
    // import_func!("_jzkt_remove", JZKT_REMOVE),
    import_func!("_jzkt_compute_root", JZKT_COMPUTE_ROOT),
    import_func!("_jzkt_emit_log", JZKT_EMIT_LOG),
    // import_func!("_jzkt_commit", JZKT_COMMIT),
    // import_func!("_jzkt_rollback", JZKT_ROLLBACK),
    import_func!("_jzkt_store", JZKT_STORE),
    import_func!("_jzkt_load", JZKT_LOAD),
    import_func!("_jzkt_preimage_size", JZKT_PREIMAGE_SIZE),
    import_func!("_jzkt_preimage_copy", JZKT_PREIMAGE_COPY),
];

pub fn create_shared_import_linker<F: From<[(&'static str, &'static str, u32, u32); 20]>>() -> F {
    F::from(SHARED_IMPORT_LINKER)
}

const SOVEREIGN_IMPORT_LINKER: [(&'static str, &'static str, u32, u32); 27] = [
    import_func!("_crypto_keccak256", CRYPTO_KECCAK256),
    import_func!("_crypto_poseidon", CRYPTO_KECCAK256),
    import_func!("_crypto_poseidon2", CRYPTO_POSEIDON2),
    import_func!("_crypto_ecrecover", CRYPTO_ECRECOVER),
    import_func!("_sys_halt", SYS_HALT),
    import_func!("_sys_write", SYS_WRITE),
    import_func!("_sys_input_size", SYS_INPUT_SIZE),
    import_func!("_sys_read", SYS_READ),
    import_func!("_sys_output_size", SYS_OUTPUT_SIZE),
    import_func!("_sys_read_output", SYS_READ_OUTPUT),
    import_func!("_sys_forward_output", SYS_FORWARD_OUTPUT),
    import_func!("_sys_state", SYS_STATE),
    import_func!("_sys_exec", SYS_EXEC),
    import_func!("_jzkt_open", JZKT_OPEN),
    import_func!("_jzkt_checkpoint", JZKT_CHECKPOINT),
    import_func!("_jzkt_get", JZKT_GET),
    import_func!("_jzkt_update", JZKT_UPDATE),
    import_func!("_jzkt_update_preimage", JZKT_UPDATE_PREIMAGE),
    import_func!("_jzkt_remove", JZKT_REMOVE),
    import_func!("_jzkt_compute_root", JZKT_COMPUTE_ROOT),
    import_func!("_jzkt_emit_log", JZKT_EMIT_LOG),
    import_func!("_jzkt_commit", JZKT_COMMIT),
    import_func!("_jzkt_rollback", JZKT_ROLLBACK),
    import_func!("_jzkt_store", JZKT_STORE),
    import_func!("_jzkt_load", JZKT_LOAD),
    import_func!("_jzkt_preimage_size", JZKT_PREIMAGE_SIZE),
    import_func!("_jzkt_preimage_copy", JZKT_PREIMAGE_COPY),
];

pub fn create_sovereign_import_linker<F: From<[(&'static str, &'static str, u32, u32); 27]>>() -> F
{
    F::from(SOVEREIGN_IMPORT_LINKER)
}
