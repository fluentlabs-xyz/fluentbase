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
    pub fn _read_context(target_ptr: *mut u8, offset: u32, length: u32);

    /// Executes nested call with specified bytecode poseidon hash:
    /// - `hash32_ptr` - a 254-bit poseidon hash of a contract to be called
    /// - `input_ptr` - pointer to the input (must be `ptr::null()` if len zero)
    /// - `input_len` - length of input (can be zero)
    /// - `fuel` - an amount of fuel is allocated for the call
    /// - `state` - execution state (must be 0 for non-authorized calls)
    pub fn _exec(
        hash32_ptr: *const u8,
        input_ptr: *const u8,
        input_len: u32,
        fuel_limit: u64,
        state: u32,
    ) -> i32;
    pub fn _resume(
        call_id: u32,
        return_data_ptr: *const u8,
        return_data_len: u32,
        exit_code: i32,
    ) -> i32;

    pub fn _charge_fuel(delta: u64) -> u64;
    pub fn _fuel() -> u64;

    /// Journaled ZK Trie methods to work with blockchain state
    pub fn _preimage_size(hash32_ptr: *const u8) -> u32;
    pub fn _preimage_copy(hash32_ptr: *const u8, preimage_ptr: *mut u8);

    pub fn _debug_log(msg_ptr: *const u8, msg_len: u32);
}
