#[link(wasm_import_module = "fluentbase_v1alpha")]
extern "C" {
    pub(crate) fn _crypto_keccak256(
        data_offset: *const u8,
        data_len: u32,
        output32_offset: *mut u8,
    );
    pub(crate) fn _crypto_poseidon(data_offset: *const u8, data_len: u32, output32_offset: *mut u8);
    pub(crate) fn _crypto_poseidon2(
        fa32_offset: *const u8,
        fb32_offset: *const u8,
        fd32_offset: *const u8,
        output32_offset: *mut u8,
    ) -> bool;
    pub(crate) fn _crypto_ecrecover(
        digest32_offset: *const u8,
        sig64_offset: *const u8,
        output65_offset: *mut u8,
        rec_id: u32,
    );

    pub(crate) fn _sys_halt(code: i32) -> !;
    pub(crate) fn _sys_write(offset: *const u8, length: u32);
    pub(crate) fn _sys_input_size() -> u32;
    pub(crate) fn _sys_read(target: *mut u8, offset: u32, length: u32);
    pub(crate) fn _sys_output_size() -> u32;
    pub(crate) fn _sys_read_output(target: *mut u8, offset: u32, length: u32);
    pub(crate) fn _sys_state() -> u32;
    pub(crate) fn _sys_exec(
        code_offset: *const u8,
        code_len: u32,
        input_offset: *const u8,
        input_len: u32,
        return_offset: *mut u8,
        return_len: u32,
        fuel: u32,
    ) -> i32;

    pub(crate) fn _preimage_size(hash32: *const u8) -> u32;
    pub(crate) fn _preimage_copy(hash32: *const u8, output_offset: *mut u8, output_len: u32);

    // pub(crate) fn _mpt_open();
    // pub(crate) fn _mpt_update(
    //     key_offset: *const u8,
    //     key_len: i32,
    //     value_offset: *const u8,
    //     value_len: i32,
    // );
    // pub(crate) fn _mpt_get(key_offset: *const u8, key_len: u32, output_offset: *mut u8) -> i32;
    // pub(crate) fn _mpt_get_root(output_offset: *mut u8) -> i32;

    pub(crate) fn _rwasm_transact(
        address20_offset: *const u8,
        value32_offset: *const u8,
        input_offset: *const u8,
        input_length: u32,
        return_offset: *mut u8,
        return_length: u32,
        fuel: u32,
        is_delegate: u32,
        is_static: u32,
    ) -> i32;
    pub(crate) fn _rwasm_compile(
        input_ptr: *const u8,
        input_len: u32,
        output_ptr: *mut u8,
        output_len: u32,
    ) -> i32;
    pub(crate) fn _rwasm_create(
        value32_offset: *const u8,
        input_bytecode_offset: *const u8,
        input_bytecode_length: u32,
        salt32_offset: *const u8,
        deployed_contract_address20_output_offset: *mut u8,
        is_create2: u32,
    ) -> i32;

    pub(crate) fn _statedb_get_code(
        key20_offset: *const u8,
        output_offset: *mut u8,
        code_offset: u32,
        out_len: u32,
    );
    pub(crate) fn _statedb_get_code_size(key20_offset: *const u8) -> u32;
    pub(crate) fn _statedb_get_code_hash(
        key20_offset: *const u8,
        output_hash32_offset: *mut u8,
    ) -> ();
    pub(crate) fn _statedb_set_code(key20_offset: *const u8, code_offset: *const u8, code_len: u32);
    pub(crate) fn _statedb_get_balance(
        address20_offset: *const u8,
        out_balance32_offset: *mut u8,
        is_self: u32,
    );
    pub(crate) fn _statedb_get_storage(key32_offset: *const u8, val32_offset: *mut u8);
    pub(crate) fn _statedb_update_storage(key32_offset: *const u8, val32_offset: *const u8);
    pub(crate) fn _statedb_emit_log(
        topics32_offset: *const [u8; 32],
        topics32_length: u32,
        data_offset: *const u8,
        data_len: u32,
    );

    pub(crate) fn _zktrie_open(root32_ptr: *const u8) -> ();
    pub(crate) fn _zktrie_update(
        key32_offset: *const u8,
        flags: u32,
        vals32_offset: *const [u8; 32],
        vals32_len: u32,
    );
    pub(crate) fn _zktrie_field(key32_offset: *const u8, field: u32, output32_offset: *mut u8);
    pub(crate) fn _zktrie_root(output32_offset: *mut u8);
    pub(crate) fn _zktrie_checkpoint() -> u32;
    pub(crate) fn _zktrie_rollback(checkpoint: u32);
    pub(crate) fn _zktrie_commit(root32_offset: *mut u8);
}
