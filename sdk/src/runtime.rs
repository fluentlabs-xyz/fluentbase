#[allow(dead_code)]
use crate::{Bytes32, LowLevelAPI, LowLevelSDK};
#[cfg(test)]
use alloc::vec::Vec;
use fluentbase_runtime::instruction::{
    crypto_ecrecover::CryptoEcrecover,
    crypto_keccak256::CryptoKeccak256,
    crypto_poseidon::CryptoPoseidon,
    crypto_poseidon2::CryptoPoseidon2,
    rwasm_compile::RwasmCompile,
};
#[cfg(test)]
use fluentbase_runtime::RuntimeContext;

#[cfg(test)]
thread_local! {
    pub static CONTEXT: std::cell::Cell<RuntimeContext<'static, ()>> = std::cell::Cell::new(RuntimeContext::new(&[]));
}

impl LowLevelAPI for LowLevelSDK {
    #[cfg(test)]
    fn sys_read(target: &mut [u8], offset: u32) {
        let input = CONTEXT.with(|ctx| {
            let ctx2 = ctx.take();
            let result = ctx2
                .read_input(offset, target.len() as u32)
                .unwrap()
                .to_vec();
            ctx.set(ctx2);
            result.to_vec()
        });
        target.copy_from_slice(&input);
    }

    #[cfg(not(test))]
    fn sys_read(_target: &mut [u8], _offset: u32) {
        unreachable!("sys methods are not available in this mode")
    }

    #[cfg(test)]
    fn sys_input_size() -> u32 {
        CONTEXT.with(|ctx| {
            let ctx2 = ctx.take();
            let result = ctx2.input_size();
            ctx.set(ctx2);
            result
        })
    }

    #[cfg(not(test))]
    fn sys_input_size() -> u32 {
        unreachable!("sys methods are not available in this mode")
    }

    #[cfg(test)]
    fn sys_write(value: &[u8]) {
        CONTEXT.with(|ctx| {
            let mut output = ctx.take();
            output.extend_return_data(value);
            ctx.set(output);
        });
    }

    #[cfg(not(test))]
    fn sys_write(_value: &[u8]) {
        unreachable!("sys methods are not available in this mode")
    }

    #[cfg(test)]
    fn sys_halt(exit_code: i32) {
        CONTEXT.with(|ctx| {
            let mut output = ctx.take();
            output.set_exit_code(exit_code);
            ctx.set(output);
        });
    }

    #[cfg(not(test))]
    fn sys_halt(_exit_code: i32) {
        unreachable!("sys methods are not available in this mode")
    }

    #[cfg(test)]
    fn sys_state() -> u32 {
        CONTEXT.with(|ctx| {
            let output = ctx.take();
            let result = output.state();
            ctx.set(output);
            result
        })
    }

    #[cfg(not(test))]
    fn sys_state() -> u32 {
        unreachable!("sys methods are not available in this mode")
    }

    fn crypto_keccak256(data: &[u8], output: &mut [u8]) {
        let result = CryptoKeccak256::fn_impl(data);
        output.copy_from_slice(&result);
    }

    fn crypto_poseidon(data: &[u8], output: &mut [u8]) {
        let result = CryptoPoseidon::fn_impl(data);
        output.copy_from_slice(&result);
    }

    fn crypto_poseidon2(
        fa_data: &[u8; 32],
        fb_data: &[u8; 32],
        fd_data: &[u8; 32],
        output: &mut [u8],
    ) -> bool {
        match CryptoPoseidon2::fn_impl(fa_data, fb_data, fd_data) {
            Ok(result) => {
                output.copy_from_slice(&result);
                true
            }
            Err(_) => false,
        }
    }

    fn crypto_ecrecover(digest: &[u8], sig: &[u8], output: &mut [u8], rec_id: u8) {
        let result = CryptoEcrecover::fn_impl(digest, sig, rec_id as u32);
        output.copy_from_slice(&result);
    }

    fn rwasm_compile(input: &[u8], output: &mut [u8]) -> i32 {
        match RwasmCompile::fn_impl(input, output.len() as u32) {
            Ok(result) => {
                output[0..result.len()].copy_from_slice(&result);
                0
            }
            Err(err_code) => err_code,
        }
    }

    fn rwasm_transact(
        _address: &[u8],
        _value: &[u8],
        _input: &[u8],
        _output: &mut [u8],
        _fuel: u32,
        _is_static: bool,
    ) -> i32 {
        unreachable!("rwasm methods are not available in this mode")
        // match RwasmTransact::fn_impl(address, value, input, output.len() as u32, fuel) {
        //     Ok(result) => {
        //         output[0..result.len()].copy_from_slice(&result);
        //         0
        //     }
        //     Err(err_code) => err_code,
        // }
    }

    fn statedb_get_code(_key: &[u8], _output: &mut [u8]) {
        unreachable!("statedb methods are not available in this mode")
    }

    fn statedb_get_code_size(_key: &[u8]) -> u32 {
        unreachable!("statedb methods are not available in this mode")
    }

    fn statedb_set_code(_key: &[u8], _code: &[u8]) {
        unreachable!("statedb methods are not available in this mode")
    }

    fn statedb_get_storage(_key: &[u8], _value: &mut [u8]) {
        unreachable!("statedb methods are not available in this mode")
    }

    fn statedb_update_storage(_key: &[u8], _value: &[u8]) {
        unreachable!("statedb methods are not available in this mode")
    }

    fn statedb_emit_log(_topics: &[Bytes32], _data: &[u8]) {
        unreachable!("statedb methods are not available in this mode")
    }

    fn zktrie_open(_root: &Bytes32) {
        unreachable!("zktrie methods are not available in this mode")
    }

    fn zktrie_update(_key: &Bytes32, _flags: u32, _values: &[Bytes32]) {
        unreachable!("zktrie methods are not available in this mode")
    }

    fn zktrie_field(_key: &Bytes32, _field: u32, _output: &mut [Bytes32]) {
        unreachable!("zktrie methods are not available in this mode")
    }

    fn zktrie_root(_output: &mut Bytes32) {
        unreachable!("zktrie methods are not available in this mode")
    }

    fn zktrie_rollback() {
        unreachable!("zktrie methods are not available in this mode")
    }

    fn zktrie_commit() {
        unreachable!("zktrie methods are not available in this mode")
    }

    fn zktrie_store(_key: &Bytes32, _val: &Bytes32) {
        unreachable!("zktrie methods are not available in this mode")
    }

    fn zktrie_load(_key: &Bytes32, _val: &mut Bytes32) {
        unreachable!("zktrie methods are not available in this mode")
    }
}

#[cfg(test)]
impl LowLevelSDK {
    pub fn with_test_input(input: Vec<u8>) {
        CONTEXT.with(|ctx| {
            let output = ctx.take();
            ctx.set(output.with_input(input));
        });
    }

    pub fn get_test_output() -> Vec<u8> {
        CONTEXT.with(|ctx| {
            let mut output = ctx.take();
            let result = output.output().clone();
            output.clean_output();
            ctx.set(output);
            result
        })
    }

    pub fn with_test_state(state: u32) {
        CONTEXT.with(|ctx| {
            let output = ctx.take();
            ctx.set(output.with_state(state));
        });
    }
}
