use crate::{
    bindings::{
        _charge_fuel,
        _checkpoint,
        _commit,
        _compute_root,
        _context_call,
        _debug_log,
        _ecrecover,
        _emit_log,
        _exec,
        _exit,
        _forward_output,
        _get_leaf,
        _input_size,
        _keccak256,
        _output_size,
        _poseidon,
        _poseidon_hash,
        _preimage_copy,
        _preimage_size,
        _read,
        _read_context,
        _read_output,
        _rollback,
        _state,
        _update_leaf,
        _update_preimage,
        _write,
    },
    sdk::{SharedAPI, SovereignAPI},
    LowLevelSDK,
};

impl SharedAPI for LowLevelSDK {
    #[inline(always)]
    fn read(target: &mut [u8], offset: u32) {
        unsafe { _read(target.as_mut_ptr(), offset, target.len() as u32) }
    }

    #[inline(always)]
    fn input_size() -> u32 {
        unsafe { _input_size() }
    }

    #[inline(always)]
    fn write(value: &[u8]) {
        unsafe { _write(value.as_ptr(), value.len() as u32) }
    }

    #[inline(always)]
    fn forward_output(offset: u32, len: u32) {
        unsafe { _forward_output(offset, len) }
    }

    #[inline(always)]
    fn exit(exit_code: i32) -> ! {
        unsafe { _exit(exit_code) }
    }

    #[inline(always)]
    fn output_size() -> u32 {
        unsafe { _output_size() }
    }

    #[inline(always)]
    fn read_output(target: *mut u8, offset: u32, length: u32) {
        unsafe { _read_output(target, offset, length) }
    }

    #[inline(always)]
    fn state() -> u32 {
        unsafe { _state() }
    }

    #[inline(always)]
    fn exec(
        code_hash32_ptr: *const u8,
        input_ptr: *const u8,
        input_len: u32,
        return_ptr: *mut u8,
        return_len: u32,
        fuel_ptr: *mut u32,
    ) -> i32 {
        unsafe {
            _exec(
                code_hash32_ptr,
                input_ptr,
                input_len,
                return_ptr,
                return_len,
                fuel_ptr,
            )
        }
    }

    #[inline(always)]
    fn charge_fuel(delta: u64) -> u64 {
        unsafe { _charge_fuel(delta) }
    }

    #[inline(always)]
    fn read_context(target_ptr: *mut u8, offset: u32, length: u32) {
        unsafe { _read_context(target_ptr, offset, length) }
    }

    #[inline(always)]
    fn keccak256(data_ptr: *const u8, data_len: u32, output32_ptr: *mut u8) {
        unsafe { _keccak256(data_ptr, data_len, output32_ptr) }
    }

    #[inline(always)]
    fn poseidon(data_ptr: *const u8, data_len: u32, output32_ptr: *mut u8) {
        unsafe { _poseidon(data_ptr, data_len, output32_ptr) }
    }

    #[inline(always)]
    fn poseidon_hash(
        fa32_ptr: *const u8,
        fb32_ptr: *const u8,
        fd32_ptr: *const u8,
        output32_ptr: *mut u8,
    ) {
        unsafe { _poseidon_hash(fa32_ptr, fb32_ptr, fd32_ptr, output32_ptr) }
    }

    #[inline(always)]
    fn ecrecover(digest32_ptr: *const u8, sig64_ptr: *const u8, output65_ptr: *mut u8, rec_id: u8) {
        unsafe { _ecrecover(digest32_ptr, sig64_ptr, output65_ptr, rec_id as u32) }
    }
}

impl SovereignAPI for LowLevelSDK {
    #[inline(always)]
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
    ) -> i32 {
        unsafe {
            _context_call(
                code_hash32_ptr,
                input_ptr,
                input_len,
                context_ptr,
                context_len,
                return_ptr,
                return_len,
                fuel_ptr,
                state,
            )
        }
    }

    #[inline(always)]
    fn checkpoint() -> u64 {
        unsafe { _checkpoint() }
    }

    #[inline(always)]
    fn get_leaf(key32_ptr: *const u8, field: u32, output32_ptr: *mut u8, committed: bool) -> bool {
        unsafe { _get_leaf(key32_ptr, field, output32_ptr, committed) }
    }

    #[inline(always)]
    fn update_leaf(key32_ptr: *const u8, flags: u32, vals32_ptr: *const [u8; 32], vals32_len: u32) {
        unsafe {
            _update_leaf(key32_ptr, flags, vals32_ptr, vals32_len);
        }
    }

    #[inline(always)]
    fn update_preimage(
        key32_ptr: *const u8,
        field: u32,
        preimage_ptr: *const u8,
        preimage_len: u32,
    ) -> bool {
        unsafe { _update_preimage(key32_ptr, field, preimage_ptr, preimage_len) }
    }

    #[inline(always)]
    fn compute_root(output32_ptr: *mut u8) {
        unsafe { _compute_root(output32_ptr) }
    }

    #[inline(always)]
    fn emit_log(
        address20_ptr: *const u8,
        topics32s_ptr: *const [u8; 32],
        topics32s_len: u32,
        data_ptr: *const u8,
        data_len: u32,
    ) {
        unsafe {
            _emit_log(
                address20_ptr,
                topics32s_ptr,
                topics32s_len,
                data_ptr,
                data_len,
            )
        }
    }
    #[inline(always)]
    fn commit(root32_ptr: *mut u8) {
        unsafe { _commit(root32_ptr) }
    }

    #[inline(always)]
    fn rollback(checkpoint: u64) {
        unsafe { _rollback(checkpoint) }
    }

    #[inline(always)]
    fn preimage_size(hash32_ptr: *const u8) -> u32 {
        unsafe { _preimage_size(hash32_ptr) }
    }

    #[inline(always)]
    fn preimage_copy(hash32_ptr: *const u8, preimage_ptr: *mut u8) {
        unsafe { _preimage_copy(hash32_ptr, preimage_ptr) }
    }

    #[inline(always)]
    fn debug_log(msg_ptr: *const u8, msg_len: u32) {
        unsafe { _debug_log(msg_ptr, msg_len) }
    }
}
