#[link(wasm_import_module = "fluentbase_v1preview")]
extern "C" {
    /// Functions that provide access to crypto elements, right now we support following:
    /// - Keccak256
    /// - Poseidon (two modes, message hash and two elements hash)
    /// - Ecrecover
    pub fn _keccak256(data_offset: *const u8, data_len: u32, output32_offset: *mut u8);
    pub fn _poseidon(data_offset: *const u8, data_len: u32, output32_offset: *mut u8);
    pub fn _poseidon_hash(
        fa32_offset: *const u8,
        fb32_offset: *const u8,
        fd32_offset: *const u8,
        output32_offset: *mut u8,
    );
    pub fn _ecrecover(
        digest32_offset: *const u8,
        sig64_offset: *const u8,
        output65_offset: *mut u8,
        rec_id: u32,
    );

    /// Basic system methods that are available for every app (shared and sovereign)
    pub fn _exit(code: i32) -> !;
    pub fn _write(offset: *const u8, length: u32);
    pub fn _input_size() -> u32;
    pub fn _read(target: *mut u8, offset: u32, length: u32);
    pub fn _output_size() -> u32;
    pub fn _read_output(target: *mut u8, offset: u32, length: u32);
    pub fn _forward_output(offset: u32, len: u32);
    pub fn _state() -> u32;

    /// Executes nested call with specified bytecode poseidon hash:
    /// - `hash32_ptr` - a 254-bit poseidon hash of a contract to be called
    /// - `address20_ptr` - a 160-bit callee address (must matches account's code hash)
    /// - `input_ptr` - pointer to the input (must be `ptr::null()` if len zero)
    /// - `input_len` - length of input (can be zero)
    /// - `context_ptr` - pointer to the context (must be `ptr::null()` for shared env)
    /// - `context_len` - length of the context
    /// - `return_ptr` - pointer to the return data (might be `ptr::null()`)
    /// - `return_len` - length of return data buffer (might be zero)
    /// - `fuel_ptr` - pointer to the fuel memory field (modifiable)
    pub fn _exec(
        hash32_ptr: *const u8,
        address20_ptr: *const u8,
        input_ptr: *const u8,
        input_len: u32,
        context_ptr: *const u8,
        context_len: u32,
        return_ptr: *mut u8,
        return_len: u32,
        fuel_ptr: *mut u32,
    ) -> i32;

    pub fn _charge_fuel(delta: u64) -> u64;

    /// Read context and write into specified target with offset and length.
    pub fn _read_context(target_ptr: *mut u8, offset: u32, length: u32);

    /// Journaled ZK Trie methods to work with blockchain state
    pub fn _checkpoint() -> u64;
    pub fn _get_leaf(
        key32_ptr: *const u8,
        field: u32,
        output32_ptr: *mut u8,
        committed: bool,
    ) -> bool;
    pub fn _update_leaf(
        key32_ptr: *const u8,
        flags: u32,
        vals32_offset: *const [u8; 32],
        vals32_len: u32,
    );
    pub fn _update_preimage(
        key32_ptr: *const u8,
        field: u32,
        preimage_ptr: *const u8,
        preimage_len: u32,
    ) -> bool;
    pub fn _compute_root(output32_ptr: *mut u8);
    pub fn _emit_log(
        address20_ptr: *const u8,
        topics32s_ptr: *const [u8; 32],
        topics32s_len: u32,
        data_ptr: *const u8,
        data_len: u32,
    );
    pub fn _commit(root32_ptr: *mut u8);
    pub fn _rollback(checkpoint: u64);
    pub fn _preimage_size(hash32_ptr: *const u8) -> u32;
    pub fn _preimage_copy(hash32_ptr: *const u8, preimage_ptr: *mut u8);

    pub fn _debug_log(msg_ptr: *const u8, msg_len: u32);
}
