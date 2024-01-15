use crate::{
    bindings::{
        _crypto_ecrecover,
        _crypto_keccak256,
        _crypto_poseidon,
        _crypto_poseidon2,
        _rwasm_compile,
        _rwasm_transact,
        _sys_halt,
        _sys_input_size,
        _sys_read,
        _sys_state,
        _sys_write,
        _zktrie_open,
        _zktrie_root,
    },
    Bytes32,
    LowLevelAPI,
    LowLevelSDK,
};

impl LowLevelAPI for LowLevelSDK {
    fn sys_read(target: &mut [u8], offset: u32) {
        unsafe { _sys_read(target.as_mut_ptr(), offset, target.len() as u32) }
    }

    fn sys_input_size() -> u32 {
        unsafe { _sys_input_size() }
    }

    fn sys_write(value: &[u8]) {
        unsafe { _sys_write(value.as_ptr(), value.len() as u32) }
    }

    fn sys_halt(exit_code: i32) {
        unsafe { _sys_halt(exit_code) }
    }

    fn sys_state() -> u32 {
        unsafe { _sys_state() }
    }

    fn crypto_keccak256(data: &[u8], output: &mut [u8]) {
        unsafe { _crypto_keccak256(data.as_ptr(), data.len() as i32, output.as_mut_ptr()) }
    }

    fn crypto_poseidon(fr32_data: &[u8], output: &mut [u8]) {
        unsafe {
            _crypto_poseidon(
                fr32_data.as_ptr(),
                fr32_data.len() as i32,
                output.as_mut_ptr(),
            )
        }
    }

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

    fn rwasm_transact(
        bytecode: &[u8],
        input: &[u8],
        output: &mut [u8],
        state: u32,
        fuel_limit: u32,
    ) -> i32 {
        unsafe {
            _rwasm_transact(
                bytecode.as_ptr(),
                bytecode.len() as u32,
                input.as_ptr(),
                input.len() as u32,
                output.as_mut_ptr(),
                output.len() as u32,
                state,
                fuel_limit,
            )
        }
    }

    fn zktrie_open(root: &Bytes32) -> u32 {
        unsafe { _zktrie_open(root.as_ptr()) }
    }

    fn zktrie_update(trie: u32, key: &Bytes32, flags: u32, values: &[Bytes32]) {
        // unsafe {
        //     _zktrie_update(
        //         trie,
        //         key.as_ptr(),
        //         flags,
        //         values.as_ptr().as_ptr(),
        //         values.len() as u32,
        //     )
        // }
    }

    fn zktrie_get(trie: u32, key: &Bytes32, output: &mut [Bytes32]) {
        // unsafe { _zktrie_get(trie, key.as_ptr(), output.as_mut_ptr().as_mut_ptr()) }
    }

    fn zktrie_root(trie: u32, output: &mut Bytes32) {
        unsafe { _zktrie_root(trie, output.as_mut_ptr()) }
    }
}
