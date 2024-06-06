#[link(wasm_import_module = "fluentbase_v1alpha")]
extern "C" {
    /// Functions that provide access to crypto elements, right now we support following:
    /// - Keccak256
    /// - Poseidon (two modes, message hash and two elements hash)
    /// - Ecrecover
    pub fn _crypto_keccak256(data_offset: *const u8, data_len: u32, output32_offset: *mut u8);
    pub fn _crypto_poseidon(data_offset: *const u8, data_len: u32, output32_offset: *mut u8);
    pub fn _crypto_poseidon_hash(
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

    /// Executes nested call with specified bytecode poseidon hash:
    /// - `code_hash32_ptr` - a 254-bit poseidon hash of a contract to be called
    /// - `input_ptr` - pointer to the input (must be `ptr::null()` if len zero)
    /// - `input_len` - length of input (can be zero)
    /// - `context_ptr` - pointer to the context (must be `ptr::null()` for shared env)
    /// - `context_len` - length of the context
    /// - `return_ptr` - pointer to the return data (might be `ptr::null()`)
    /// - `return_len` - length of return data buffer (might be zero)
    /// - `fuel_ptr` - pointer to the fuel memory field (modifiable)
    /// - `state` - state of the call, for example deployment or main call
    pub fn _sys_exec(
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

    pub fn _sys_fuel(delta: u64) -> u64;

    /// This function modifies context register for nested calls, we suppose to have two registers:
    /// 1. CO - context offset
    /// 2. CL - context length
    /// Nested calls can read context only relatively using this CO/CL constraints.
    pub fn _sys_rewrite_context(context_ptr: *const u8, context_len: u32);

    /// Read context and write into specified target with offset and length. Registers CO & CL must
    /// be initialized using `_sys_rewrite_context` function in advance.
    pub fn _sys_context(target_ptr: *mut u8, offset: u32, length: u32);

    /// Journaled ZK Trie methods to work with blockchain state
    pub fn _jzkt_open(root32_ptr: *const u8);
    pub fn _jzkt_checkpoint() -> u64;
    pub fn _jzkt_get(
        key32_ptr: *const u8,
        field: u32,
        output32_ptr: *mut u8,
        committed: bool,
    ) -> bool;
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
        address20_ptr: *const u8,
        topics32s_ptr: *const [u8; 32],
        topics32s_len: u32,
        data_ptr: *const u8,
        data_len: u32,
    );
    pub fn _jzkt_commit(root32_ptr: *mut u8);
    pub fn _jzkt_rollback(checkpoint: u64);
    pub fn _jzkt_preimage_size(hash32_ptr: *const u8) -> u32;
    pub fn _jzkt_preimage_copy(hash32_ptr: *const u8, preimage_ptr: *mut u8);

    pub fn _wasm_to_rwasm_size(input_ptr: *const u8, input_len: u32) -> i32;
    pub fn _wasm_to_rwasm(
        input_ptr: *const u8,
        input_len: u32,
        output_ptr: *mut u8,
        output_len: u32,
    ) -> i32;
    pub fn _debug_log(msg_ptr: *const u8, msg_len: u32);
}
