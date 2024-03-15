use crate::{
    bindings::{
        _crypto_ecrecover,
        _crypto_keccak256,
        _crypto_poseidon,
        _crypto_poseidon2,
        _jzkt_checkpoint,
        _jzkt_commit,
        _jzkt_compute_root,
        _jzkt_emit_log,
        _jzkt_get,
        _jzkt_load,
        _jzkt_open,
        _jzkt_preimage_copy,
        _jzkt_preimage_size,
        _jzkt_remove,
        _jzkt_rollback,
        _jzkt_store,
        _jzkt_update,
        _jzkt_update_preimage,
        _rwasm_compile,
        _rwasm_create,
        _rwasm_transact,
        _statedb_emit_log,
        _statedb_get_balance,
        _statedb_get_code,
        _statedb_get_code_hash,
        _statedb_get_code_size,
        _statedb_get_storage,
        _statedb_set_code,
        _statedb_update_storage,
        _sys_exec,
        _sys_forward_output,
        _sys_halt,
        _sys_input_size,
        _sys_output_size,
        _sys_read,
        _sys_read_output,
        _sys_state,
        _sys_write,
    },
    Bytes32,
    LowLevelAPI,
    LowLevelSDK,
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
    fn sys_exec(
        code_offset: *const u8,
        code_len: u32,
        input_offset: *const u8,
        input_len: u32,
        return_offset: *mut u8,
        return_len: u32,
        fuel_offset: *const u32,
        state: u32,
    ) -> i32 {
        unsafe {
            _sys_exec(
                code_offset,
                code_len,
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
        fa32_data: &Bytes32,
        fb32_data: &Bytes32,
        fd32_data: &Bytes32,
        output32: &mut [u8],
    ) {
        unsafe {
            _crypto_poseidon2(
                fa32_data.as_ptr(),
                fb32_data.as_ptr(),
                fd32_data.as_ptr(),
                output32.as_mut_ptr(),
            )
        }
    }

    #[inline(always)]
    fn crypto_ecrecover(digest: &[u8], sig: &[u8], output: &mut [u8], rec_id: u8) {
        unsafe {
            _crypto_ecrecover(
                digest.as_ptr(),
                sig.as_ptr(),
                output.as_mut_ptr(),
                rec_id as u32,
            )
        }
    }

    #[inline(always)]
    fn jzkt_open(root32_ptr: *const u8) {
        unsafe { _jzkt_open(root32_ptr) }
    }
    #[inline(always)]
    fn jzkt_checkpoint() -> (u32, u32) {
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
        key32_ptr: *const u8,
        topics32s_ptr: *const [u8; 32],
        topics32s_len: u32,
        data_ptr: *const u8,
        data_len: u32,
    ) {
        unsafe { _jzkt_emit_log(key32_ptr, topics32s_ptr, topics32s_len, data_ptr, data_len) }
    }
    #[inline(always)]
    fn jzkt_commit(root32_offset: *mut u8) {
        unsafe { _jzkt_commit(root32_offset) }
    }
    #[inline(always)]
    fn jzkt_rollback(checkpoint0: u32, checkpoint1: u32) {
        unsafe { _jzkt_rollback(checkpoint0, checkpoint1) }
    }
    #[inline(always)]
    fn jzkt_store(slot32_ptr: *const u8, value32_ptr: *const u8) {
        unsafe { _jzkt_store(slot32_ptr, value32_ptr) }
    }
    #[inline(always)]
    fn jzkt_load(slot32_ptr: *const u8, value32_ptr: *mut u8) -> i32 {
        unsafe { _jzkt_load(slot32_ptr, value32_ptr) }
    }
    #[inline(always)]
    fn jzkt_preimage_size(hash32_ptr: *const u8) -> u32 {
        unsafe { _jzkt_preimage_size(hash32_ptr) }
    }
    #[inline(always)]
    fn jzkt_preimage_copy(hash32_ptr: *const u8, preimage_ptr: *mut u8) {
        unsafe { _jzkt_preimage_copy(hash32_ptr, preimage_ptr) }
    }

    #[inline(always)]
    fn rwasm_compile(input: &[u8], output: &mut [u8]) -> i32 {
        unsafe {
            _rwasm_compile(
                input.as_ptr(),
                input.len() as u32,
                output.as_mut_ptr(),
                output.len() as u32,
            )
        }
    }

    #[inline(always)]
    fn rwasm_transact(
        address: &[u8],
        value: &[u8],
        input: &[u8],
        output: &mut [u8],
        fuel: u32,
        is_delegate: bool,
        is_static: bool,
    ) -> i32 {
        unsafe {
            _rwasm_transact(
                address.as_ptr(),
                value.as_ptr(),
                input.as_ptr(),
                input.len() as u32,
                output.as_mut_ptr(),
                output.len() as u32,
                fuel,
                is_delegate as u32,
                is_static as u32,
            )
        }
    }

    #[inline(always)]
    fn rwasm_create(
        value32: &[u8],
        input_bytecode: &[u8],
        salt32: &[u8],
        deployed_contract_address20_output: &mut [u8],
        is_create2: bool,
    ) -> i32 {
        unsafe {
            _rwasm_create(
                value32.as_ptr(),
                input_bytecode.as_ptr(),
                input_bytecode.len() as u32,
                salt32.as_ptr(),
                deployed_contract_address20_output.as_mut_ptr(),
                is_create2 as u32,
            )
        }
    }

    #[inline(always)]
    fn statedb_get_code(key: &[u8], output: &mut [u8], code_offset: u32) {
        unsafe {
            _statedb_get_code(
                key.as_ptr(),
                output.as_mut_ptr(),
                code_offset,
                output.len() as u32,
            )
        }
    }

    #[inline(always)]
    fn statedb_get_code_size(key: &[u8]) -> u32 {
        unsafe { _statedb_get_code_size(key.as_ptr()) }
    }

    #[inline(always)]
    fn statedb_get_code_hash(key: &[u8], out_hash32: &mut [u8]) -> () {
        unsafe { _statedb_get_code_hash(key.as_ptr(), out_hash32.as_mut_ptr()) }
    }

    #[inline(always)]
    fn statedb_get_balance(address20: &[u8], out_balance32: &mut [u8], is_self: bool) -> () {
        unsafe {
            _statedb_get_balance(
                address20.as_ptr(),
                out_balance32.as_mut_ptr(),
                is_self as u32,
            )
        }
    }

    #[inline(always)]
    fn statedb_set_code(key: &[u8], code: &[u8]) {
        unsafe { _statedb_set_code(key.as_ptr(), code.as_ptr(), code.len() as u32) }
    }

    #[inline(always)]
    fn statedb_get_storage(key: &[u8], value: &mut [u8]) {
        unsafe { _statedb_get_storage(key.as_ptr(), value.as_mut_ptr()) }
    }

    #[inline(always)]
    fn statedb_update_storage(key: &[u8], value: &[u8]) {
        unsafe { _statedb_update_storage(key.as_ptr(), value.as_ptr()) }
    }

    #[inline(always)]
    fn statedb_emit_log(topics: &[Bytes32], data: &[u8]) {
        unsafe {
            _statedb_emit_log(
                topics.as_ptr(),
                topics.len() as u32,
                data.as_ptr(),
                data.len() as u32,
            )
        }
    }
}
