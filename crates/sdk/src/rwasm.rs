use crate::{
    bindings::{
        _crypto_ecrecover, _crypto_keccak256, _crypto_poseidon, _crypto_poseidon2,
        _jzkt_checkpoint, _jzkt_commit, _jzkt_compute_root, _jzkt_emit_log, _jzkt_get, _jzkt_open,
        _jzkt_preimage_copy, _jzkt_preimage_size, _jzkt_remove, _jzkt_rollback, _jzkt_update,
        _jzkt_update_preimage, _sys_exec_hash, _sys_forward_output, _sys_halt, _sys_input_size,
        _sys_output_size, _sys_read, _sys_read_output, _sys_state, _sys_write,
    },
    LowLevelAPI, LowLevelSDK,
};

impl LowLevelAPI for LowLevelSDK {
    #[inline(always)]
    fn sys_read(target: &mut [u8], offset: u32) {
        unsafe { _sys_read(target.as_mut_ptr(), offset, target.len() as u32) }
    }

    #[inline(always)]
    fn sys_input_size() -> u32 {
        unsafe { _sys_input_size() }
    }

    #[inline(always)]
    fn sys_write(value: &[u8]) {
        unsafe { _sys_write(value.as_ptr(), value.len() as u32) }
    }

    #[inline(always)]
    fn sys_forward_output(offset: u32, len: u32) {
        unsafe { _sys_forward_output(offset, len) }
    }

    #[inline(always)]
    fn sys_halt(exit_code: i32) {
        unsafe { _sys_halt(exit_code) }
    }

    #[inline(always)]
    fn sys_output_size() -> u32 {
        unsafe { _sys_output_size() }
    }

    #[inline(always)]
    fn sys_read_output(target: *mut u8, offset: u32, length: u32) {
        unsafe { _sys_read_output(target, offset, length) }
    }

    #[inline(always)]
    fn sys_state() -> u32 {
        unsafe { _sys_state() }
    }

    #[inline(always)]
    fn sys_exec_hash(
        code_hash32_offset: *const u8,
        input_offset: *const u8,
        input_len: u32,
        return_offset: *mut u8,
        return_len: u32,
        fuel_offset: *const u32,
        state: u32,
    ) -> i32 {
        unsafe {
            _sys_exec_hash(
                code_hash32_offset,
                input_offset,
                input_len,
                return_offset,
                return_len,
                fuel_offset,
                state,
            )
        }
    }

    #[inline(always)]
    fn crypto_keccak256(data_offset: *const u8, data_len: u32, output32_offset: *mut u8) {
        unsafe { _crypto_keccak256(data_offset, data_len, output32_offset) }
    }

    #[inline(always)]
    fn crypto_poseidon(data_offset: *const u8, data_len: u32, output32_offset: *mut u8) {
        unsafe { _crypto_poseidon(data_offset, data_len, output32_offset) }
    }

    #[inline(always)]
    fn crypto_poseidon2(
        fa32_ptr: *const u8,
        fb32_ptr: *const u8,
        fd32_ptr: *const u8,
        output32_ptr: *mut u8,
    ) {
        unsafe { _crypto_poseidon2(fa32_ptr, fb32_ptr, fd32_ptr, output32_ptr) }
    }

    #[inline(always)]
    fn crypto_ecrecover(
        digest32_ptr: *const u8,
        sig64_ptr: *const u8,
        output65_ptr: *mut u8,
        rec_id: u8,
    ) {
        unsafe { _crypto_ecrecover(digest32_ptr, sig64_ptr, output65_ptr, rec_id as u32) }
    }

    #[inline(always)]
    fn jzkt_open(root32_ptr: *const u8) {
        unsafe { _jzkt_open(root32_ptr) }
    }
    #[inline(always)]
    fn jzkt_checkpoint() -> u64 {
        unsafe { _jzkt_checkpoint() }
    }
    #[inline(always)]
    fn jzkt_get(key32_offset: *const u8, field: u32, output32_offset: *mut u8) -> bool {
        unsafe { _jzkt_get(key32_offset, field, output32_offset) }
    }
    #[inline(always)]
    fn jzkt_update(key32_ptr: *const u8, flags: u32, vals32_ptr: *const [u8; 32], vals32_len: u32) {
        unsafe {
            _jzkt_update(key32_ptr, flags, vals32_ptr, vals32_len);
        }
    }
    #[inline(always)]
    fn jzkt_update_preimage(
        key32_ptr: *const u8,
        field: u32,
        preimage_ptr: *const u8,
        preimage_len: u32,
    ) -> bool {
        unsafe { _jzkt_update_preimage(key32_ptr, field, preimage_ptr, preimage_len) }
    }
    #[inline(always)]
    fn jzkt_remove(key32_offset: *const u8) {
        unsafe { _jzkt_remove(key32_offset) }
    }
    #[inline(always)]
    fn jzkt_compute_root(output32_offset: *mut u8) {
        unsafe { _jzkt_compute_root(output32_offset) }
    }
    #[inline(always)]
    fn jzkt_emit_log(
        address20_ptr: *const u8,
        topics32s_ptr: *const [u8; 32],
        topics32s_len: u32,
        data_ptr: *const u8,
        data_len: u32,
    ) {
        unsafe {
            _jzkt_emit_log(
                address20_ptr,
                topics32s_ptr,
                topics32s_len,
                data_ptr,
                data_len,
            )
        }
    }
    #[inline(always)]
    fn jzkt_commit(root32_offset: *mut u8) {
        unsafe { _jzkt_commit(root32_offset) }
    }
    #[inline(always)]
    fn jzkt_rollback(checkpoint: u64) {
        unsafe { _jzkt_rollback(checkpoint) }
    }
    #[inline(always)]
    fn jzkt_preimage_size(hash32_ptr: *const u8) -> u32 {
        unsafe { _jzkt_preimage_size(hash32_ptr) }
    }
    #[inline(always)]
    fn jzkt_preimage_copy(hash32_ptr: *const u8, preimage_ptr: *mut u8) {
        unsafe { _jzkt_preimage_copy(hash32_ptr, preimage_ptr) }
    }
}
