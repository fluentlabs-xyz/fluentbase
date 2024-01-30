use crate::{
    bindings::{
        _crypto_ecrecover,
        _crypto_keccak256,
        _crypto_poseidon,
        _crypto_poseidon2,
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
        _sys_halt,
        _sys_input_size,
        _sys_read,
        _sys_state,
        _sys_write,
        _zktrie_commit,
        _zktrie_field,
        _zktrie_open,
        _zktrie_rollback,
        _zktrie_root,
        _zktrie_update,
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
    fn sys_halt(exit_code: i32) {
        unsafe { _sys_halt(exit_code) }
    }

    #[inline(always)]
    fn sys_state() -> u32 {
        unsafe { _sys_state() }
    }

    #[inline(always)]
    fn crypto_keccak256(data: &[u8], output: &mut [u8]) {
        unsafe { _crypto_keccak256(data.as_ptr(), data.len() as i32, output.as_mut_ptr()) }
    }

    #[inline(always)]
    fn crypto_poseidon(fr32_data: &[u8], output: &mut [u8]) {
        unsafe {
            _crypto_poseidon(
                fr32_data.as_ptr(),
                fr32_data.len() as i32,
                output.as_mut_ptr(),
            )
        }
    }

    #[inline(always)]
    fn crypto_poseidon2(
        fa32_data: &Bytes32,
        fb32_data: &Bytes32,
        fd32_data: &Bytes32,
        output32: &mut [u8],
    ) -> bool {
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

    #[inline(always)]
    fn zktrie_open(root: &Bytes32) {
        unsafe { _zktrie_open(root.as_ptr()) }
    }

    #[inline(always)]
    fn zktrie_update(key: &Bytes32, flags: u32, values: &[Bytes32]) {
        unsafe { _zktrie_update(key.as_ptr(), flags, values.as_ptr(), values.len() as u32) }
    }

    #[inline(always)]
    fn zktrie_field(key: &Bytes32, field: u32, output: &mut [Bytes32]) {
        unsafe { _zktrie_field(key.as_ptr(), field, output.as_mut_ptr()) }
    }

    #[inline(always)]
    fn zktrie_root(output: &mut Bytes32) {
        unsafe { _zktrie_root(output.as_mut_ptr()) }
    }

    #[inline(always)]
    fn zktrie_rollback() {
        unsafe { _zktrie_rollback() }
    }

    #[inline(always)]
    fn zktrie_commit() {
        unsafe { _zktrie_commit() }
    }

    // #[inline(always)]
    // fn zktrie_store(key: &Bytes32, val: &Bytes32) {
    //     unsafe { _zktrie_store(key.as_ptr(), val.as_ptr()) }
    // }
    //
    // #[inline(always)]
    // fn zktrie_load(key: &Bytes32, val: &mut Bytes32) {
    //     unsafe { _statedb_get_storage(key.as_ptr(), val.as_mut_ptr()) }
    // }
}
