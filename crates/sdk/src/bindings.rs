#[link(wasm_import_module = "fluentbase_v1preview")]
extern "C" {

    /// Low-level function that terminates the execution of the program and exits with the specified
    /// exit code.
    ///
    /// This function is typically used to perform an immediate and final exit of a program,
    /// bypassing Rust's standard teardown mechanisms.
    /// It effectively stops execution and prevents further operations, including cleanup or
    /// unwinding.
    ///
    /// # Parameters
    /// - `code` (i32): The non-positive exit code indicating the reason for termination.
    ///
    /// # Notes
    /// - This function is generally invoked in specialized environments, such as WebAssembly
    ///   runtimes, or through higher-level abstractions.
    /// - Consider alternatives in standard applications, such as returning control to the caller or
    ///   using Rust's standard exit mechanisms, for safer options.
    pub fn _exit(code: i32) -> !;

    // TODO(dmitry123): "rename to `_write_output`"
    pub fn _write(offset: *const u8, length: u32);

    /// Returns the size of the input data provided to the runtime environment.
    ///
    /// This function retrieves the size (in bytes) of the input payload that has
    /// been passed to the runtime.
    pub fn _input_size() -> u32;

    // TODO(dmitry123): "rename to `_read_input`"
    pub fn _read(target: *mut u8, offset: u32, length: u32);
    pub fn _output_size() -> u32;
    pub fn _read_output(target: *mut u8, offset: u32, length: u32);
    pub fn _forward_output(offset: u32, len: u32);
    pub fn _state() -> u32;

    /// Executes a nested call with specified bytecode poseidon hash.
    ///
    /// # Parameters
    /// - `hash32_ptr`: A pointer to a 254-bit poseidon hash of a contract to be called.
    /// - `input_ptr`: A pointer to the input data (const u8).
    /// - `input_len`: The length of the input data (u32).
    /// - `fuel_ptr`: A mutable pointer to a fuel value (u64), consumed fuel is stored in the same
    ///   pointer after execution.
    /// - `state`: A state value (u32), used internally to maintain function state.
    ///
    /// Fuel ptr can be set to zero if you want to delegate all remaining gas.
    /// In this case sender won't get the consumed gas result.
    ///
    /// # Returns
    /// - An `i32` value indicating the result of the execution, negative or zero result stands for
    ///   terminated execution, but positive code stands for interrupted execution (works only for
    ///   root execution level)
    pub fn _exec(
        hash32_ptr: *const u8,
        input_ptr: *const u8,
        input_len: u32,
        fuel_ptr: *mut u64,
        state: u32,
    ) -> i32;

    /// Resumes the execution of a previously suspended function call.
    ///
    /// This function is designed to handle the resumption of a function call
    /// that was previously paused.
    /// It takes several parameters that provide
    /// the necessary context and data for resuming the call.
    ///
    /// # Parameters
    ///
    /// * `call_id` - A unique identifier for the call that needs to be resumed.
    /// * `return_data_ptr` - A pointer to the return data that needs to be passed back to the
    ///   resuming function.
    /// This should point to a byte array.
    /// * `return_data_len` - The length of the return data in bytes.
    /// * `exit_code` - An integer code that represents the exit status of the resuming function.
    ///   Typically, this might be 0 for success or an error code for failure.
    /// * `fuel_ptr` - A mutable pointer to a 64-bit unsigned integer representing the fuel need to
    ///   be charged, also it puts a consumed fuel result into the same pointer
    pub fn _resume(
        call_id: u32,
        return_data_ptr: *const u8,
        return_data_len: u32,
        exit_code: i32,
        fuel_ptr: *mut u64,
    ) -> i32;

    pub fn _charge_fuel(delta: u64) -> u64;
    pub fn _fuel() -> u64;

    /// Journaled ZK Trie methods to work with blockchain state
    pub fn _preimage_size(hash32_ptr: *const u8) -> u32;
    pub fn _preimage_copy(hash32_ptr: *const u8, preimage_ptr: *mut u8);

    pub fn _debug_log(msg_ptr: *const u8, msg_len: u32);

    /// A raw FFI binding to the `_keccak256` function, which computes the Keccak-256 hash of the
    /// given input data.
    ///
    /// ### Parameters
    /// - `data_offset`: A pointer to the start of the input data for which the Keccak-256 hash
    ///   needs to be computed.
    /// - `data_len`: The length (in bytes) of the input data.
    /// - `output32_offset`: A pointer to a 32-byte buffer where the result (the Keccak-256 hash)
    ///   will be stored.
    ///
    /// ### Safety
    /// - This function is unsafe because it interacts with raw pointers and assumes the caller
    ///   ensures:
    ///   - `data_offset` points to valid memory containing at least `data_len` bytes.
    ///   - `output32_offset` points to valid writable memory of at least 32 bytes to store the
    ///     hash.
    /// - Improper use of this function may result in undefined behavior.
    ///
    /// ### Usage
    /// This method is typically intended for low-level operations, such as cryptographic
    /// computations, and is often wrapped in a safer abstraction to ensure the correct usage of
    /// memory.
    pub fn _keccak256(data_offset: *const u8, data_len: u32, output32_offset: *mut u8);
    pub fn _keccak256_permute(state_ptr: *const [u64; 25]);
    pub fn _poseidon(data_offset: *const u8, data_len: u32, output32_offset: *mut u8);
    pub fn _poseidon_hash(
        fa32_offset: *const u8,
        fb32_offset: *const u8,
        fd32_offset: *const u8,
        output32_offset: *mut u8,
    );
    pub fn _sha256_extend(w_ptr: *mut u8);
    pub fn _sha256_compress(w_ptr: *mut u8, h_ptr: *const u8);

    pub fn _ed25519_add(p_ptr: *mut u8, q_ptr: *const u8);
    pub fn _ed25519_decompress(slice_ptr: *mut u8, sign: u32);
    // TODO(dmitry123): "rename to `_secp256k1_recover`"
    pub fn _ecrecover(
        digest32_offset: *const u8,
        sig64_offset: *const u8,
        output65_offset: *mut u8,
        rec_id: u32,
    );
    pub fn _secp256k1_add(p_ptr: *mut u8, q_ptr: *const u8);
    pub fn _secp256k1_decompress(x_ptr: *mut u8, sign: u32);
    pub fn _secp256k1_double(p_ptr: *mut u8);

    pub fn _bls12381_decompress(arg1: u32, arg2: u32);
    pub fn _bls12381_add(arg1: u32, arg2: u32);
    pub fn _bls12381_double(p_ptr: u32);
    pub fn _bls12381_fp_add(arg1: u32, arg2: u32);
    pub fn _bls12381_fp_sub(arg1: u32, arg2: u32);
    pub fn _bls12381_fp_mul(arg1: u32, arg2: u32);
    pub fn _bls12381_fp2_add(arg1: u32, arg2: u32);
    pub fn _bls12381_fp2_sub(arg1: u32, arg2: u32);
    pub fn _bls12381_fp2_mul(arg1: u32, arg2: u32);

    pub fn _bn254_add(arg1: u32, arg2: u32);
    pub fn _bn254_double(p_ptr: u32);
    pub fn _bn254_fp_add(arg1: u32, arg2: u32);
    pub fn _bn254_fp_sub(arg1: u32, arg2: u32);
    pub fn _bn254_fp_mul(arg1: u32, arg2: u32);
    pub fn _bn254_fp2_add(arg1: u32, arg2: u32);
    pub fn _bn254_fp2_sub(arg1: u32, arg2: u32);
    pub fn _bn254_fp2_mul(arg1: u32, arg2: u32);

    pub fn _uint256_mul(x_ptr: u32, y_ptr: u32, m_ptr: u32);
}
