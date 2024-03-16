#[link(wasm_import_module = "fluentbase_v1alpha")]
extern "C" {
    /// Functions that provide access to crypto elements, right now we support following:
    /// - Keccak256
    /// - Poseidon (two modes, message hash and two elements hash)
    /// - Ecrecover
    pub fn _crypto_keccak256(data_offset: *const u8, data_len: u32, output32_offset: *mut u8);
    pub fn _crypto_poseidon(data_offset: *const u8, data_len: u32, output32_offset: *mut u8);
    pub fn _crypto_poseidon2(
        fa32_offset: *const u8,
        fb32_offset: *const u8,
        fd32_offset: *const u8,
        output32_offset: *mut u8,
    );
    pub fn _crypto_ecrecover(
        digest32_offset: *const u8,
        sig64_offset: *const u8,
        output65_offset: *mut u8,
        rec_id: u32,
    );

    /// Basic system methods that are available for every app (shared and sovereign)
    pub fn _sys_halt(code: i32) -> !;
    pub fn _sys_write(offset: *const u8, length: u32);
    pub fn _sys_input_size() -> u32;
    pub fn _sys_read(target: *mut u8, offset: u32, length: u32);
    pub fn _sys_output_size() -> u32;
    pub fn _sys_read_output(target: *mut u8, offset: u32, length: u32);
    pub fn _sys_forward_output(offset: u32, len: u32);
    pub fn _sys_state() -> u32;
    pub fn _sys_exec(
        code_offset: *const u8,
        code_len: u32,
        input_offset: *const u8,
        input_len: u32,
        return_offset: *mut u8,
        return_len: u32,
        fuel_offset: *const u32,
        state: u32,
    ) -> i32;

    /// Journaled ZK Trie methods to work with blockchain state
    pub fn _jzkt_open(root32_ptr: *const u8);
    pub fn _jzkt_checkpoint() -> (u32, u32);
    pub fn _jzkt_get(key32_ptr: *const u8, field: u32, output32_ptr: *mut u8) -> bool;
    pub fn _jzkt_update(
        key32_ptr: *const u8,
        flags: u32,
        vals32_offset: *const [u8; 32],
        vals32_len: u32,
    );
    pub fn _jzkt_update_preimage(
        key32_ptr: *const u8,
        field: u32,
        preimage_ptr: *const u8,
        preimage_len: u32,
    ) -> bool;
    pub fn _jzkt_remove(key32_ptr: *const u8);
    pub fn _jzkt_compute_root(output32_ptr: *mut u8);
    pub fn _jzkt_emit_log(
        key32_ptr: *const u8,
        topics32s_ptr: *const [u8; 32],
        topics32s_len: u32,
        data_ptr: *const u8,
        data_len: u32,
    );
    pub fn _jzkt_commit(root32_ptr: *mut u8);
    pub fn _jzkt_rollback(checkpoint0: u32, checkpoint1: u32);
    pub fn _jzkt_store(slot32_ptr: *const u8, value32_ptr: *const u8);
    pub fn _jzkt_load(slot32_ptr: *const u8, value32_ptr: *mut u8) -> i32;
    pub fn _jzkt_preimage_size(hash32_ptr: *const u8) -> u32;
    pub fn _jzkt_preimage_copy(hash32_ptr: *const u8, preimage_ptr: *mut u8);
}
