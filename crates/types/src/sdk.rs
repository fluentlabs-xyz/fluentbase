pub trait SharedAPI {
    fn keccak256(data_ptr: *const u8, data_len: u32, output32_ptr: *mut u8);
    fn poseidon(data_ptr: *const u8, data_len: u32, output32_ptr: *mut u8);
    fn poseidon_hash(
        fa32_ptr: *const u8,
        fb32_ptr: *const u8,
        fd32_ptr: *const u8,
        output32_ptr: *mut u8,
    );
    fn ecrecover(digest32_ptr: *const u8, sig65_ptr: *const u8, output65_ptr: *mut u8, rec_id: u8);

    fn read(target: &mut [u8], offset: u32);
    fn input_size() -> u32;
    fn write(value: &[u8]);
    fn forward_output(offset: u32, len: u32);
    fn exit(exit_code: i32) -> !;
    fn output_size() -> u32;
    fn read_output(target: *mut u8, offset: u32, length: u32);
    fn state() -> u32;
    fn charge_fuel(delta: u64) -> u64;
    fn read_context(target_ptr: *mut u8, offset: u32, length: u32);

    fn exec(
        code_hash32_ptr: *const u8,
        input_ptr: *const u8,
        input_len: u32,
        return_ptr: *mut u8,
        return_len: u32,
        fuel_ptr: *mut u32,
    ) -> i32;
}

pub trait SovereignAPI: SharedAPI {
    fn context_call(
        code_hash32_ptr: *const u8,
        input_ptr: *const u8,
        input_len: u32,
        context_ptr: *const u8,
        context_len: u32,
        return_ptr: *mut u8,
        return_len: u32,
        fuel_ptr: *mut u32,
        state: u32,
    ) -> i32;

    fn checkpoint() -> u64;
    fn get_leaf(key32_ptr: *const u8, field: u32, output32_ptr: *mut u8, committed: bool) -> bool;
    fn update_leaf(key32_ptr: *const u8, flags: u32, vals32_ptr: *const [u8; 32], vals32_len: u32);
    fn update_preimage(
        key32_ptr: *const u8,
        field: u32,
        preimage_ptr: *const u8,
        preimage_len: u32,
    ) -> bool;
    fn compute_root(output32_ptr: *mut u8);
    fn emit_log(
        address20_ptr: *const u8,
        topics32s_ptr: *const [u8; 32],
        topics32s_len: u32,
        data_ptr: *const u8,
        data_len: u32,
    );
    fn commit(root32_ptr: *mut u8);
    fn rollback(checkpoint: u64);
    fn preimage_size(hash32_ptr: *const u8) -> u32;
    fn preimage_copy(hash32_ptr: *const u8, preimage_ptr: *mut u8);

    fn debug_log(msg_ptr: *const u8, msg_len: u32);
}
