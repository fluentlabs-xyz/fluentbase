pub trait LowLevelAPI {
    fn crypto_keccak256(data_offset: *const u8, data_len: u32, output32_offset: *mut u8);
    fn crypto_poseidon(data_offset: *const u8, data_len: u32, output32_offset: *mut u8);
    fn crypto_poseidon2(
        fa32_ptr: *const u8,
        fb32_ptr: *const u8,
        fd32_ptr: *const u8,
        output32_ptr: *mut u8,
    );
    fn crypto_ecrecover(
        digest32_ptr: *const u8,
        sig65_ptr: *const u8,
        output65_ptr: *mut u8,
        rec_id: u8,
    );

    fn sys_read(target: &mut [u8], offset: u32);
    fn sys_input_size() -> u32;
    fn sys_write(value: &[u8]);
    fn sys_forward_output(offset: u32, len: u32);
    fn sys_halt(exit_code: i32);
    fn sys_output_size() -> u32;
    fn sys_read_output(target: *mut u8, offset: u32, length: u32);
    fn sys_state() -> u32;

    fn sys_exec_hash(
        code_hash32_offset: *const u8,
        input_offset: *const u8,
        input_len: u32,
        return_offset: *mut u8,
        return_len: u32,
        fuel_offset: *const u32,
        state: u32,
    ) -> i32;

    fn jzkt_open(root32_ptr: *const u8);
    fn jzkt_checkpoint() -> u64;
    fn jzkt_get(key32_offset: *const u8, field: u32, output32_offset: *mut u8) -> bool;
    fn jzkt_update(key32_ptr: *const u8, flags: u32, vals32_ptr: *const [u8; 32], vals32_len: u32);
    fn jzkt_update_preimage(
        key32_ptr: *const u8,
        field: u32,
        preimage_ptr: *const u8,
        preimage_len: u32,
    ) -> bool;
    fn jzkt_remove(key32_offset: *const u8);
    fn jzkt_compute_root(output32_offset: *mut u8);
    fn jzkt_emit_log(
        key32_ptr: *const u8,
        topics32s_ptr: *const [u8; 32],
        topics32s_len: u32,
        data_ptr: *const u8,
        data_len: u32,
    );
    fn jzkt_commit(root32_offset: *mut u8);
    fn jzkt_rollback(checkpoint: u64);
    fn jzkt_preimage_size(hash32_ptr: *const u8) -> u32;
    fn jzkt_preimage_copy(hash32_ptr: *const u8, preimage_ptr: *mut u8);
}
