#[allow(dead_code)]
use crate::{Bytes32, LowLevelAPI, LowLevelSDK};
#[cfg(test)]
use alloc::vec::Vec;
use fluentbase_runtime::instruction::{
    crypto_ecrecover::SysEcrecover,
    crypto_keccak256::SysKeccak256,
    crypto_poseidon::SysPoseidon,
    crypto_poseidon2::SysPoseidon2,
    rwasm_compile::SysCompile,
    rwasm_transact::SysExec,
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
    fn sys_read(target: &mut [u8], offset: u32) {
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
    fn sys_write(value: &[u8]) {
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
    fn sys_halt(exit_code: i32) {
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
        let result = SysKeccak256::fn_impl(data);
        output.copy_from_slice(&result);
    }

    fn crypto_poseidon(data: &[u8], output: &mut [u8]) {
        let result = SysPoseidon::fn_impl(data);
        output.copy_from_slice(&result);
    }

    fn crypto_poseidon2(
        fa_data: &[u8; 32],
        fb_data: &[u8; 32],
        fd_data: &[u8; 32],
        output: &mut [u8],
    ) -> bool {
        match SysPoseidon2::fn_impl(fa_data, fb_data, fd_data) {
            Ok(result) => {
                output.copy_from_slice(&result);
                true
            }
            Err(_) => false,
        }
    }

    fn crypto_ecrecover(digest: &[u8], sig: &[u8], output: &mut [u8], rec_id: u8) {
        let result = SysEcrecover::fn_impl(digest, sig, rec_id as u32);
        output.copy_from_slice(&result);
    }

    fn rwasm_compile(input: &[u8], output: &mut [u8]) -> i32 {
        match SysCompile::fn_impl(input, output.len() as u32) {
            Ok(result) => {
                output[0..result.len()].copy_from_slice(&result);
                0
            }
            Err(err_code) => err_code,
        }
    }

    fn rwasm_transact(code: &[u8], input: &[u8], output: &mut [u8], state: u32, fuel: u32) -> i32 {
        match SysExec::fn_impl(code, input, state, fuel, output.len() as u32) {
            Ok(result) => {
                output[0..result.len()].copy_from_slice(&result);
                0
            }
            Err(err_code) => err_code,
        }
    }

    fn zktrie_open(root: &Bytes32) {
        todo!()
    }

    fn zktrie_update(key: &Bytes32, flags: u32, values: &[Bytes32]) {
        todo!()
    }

    fn zktrie_field(key: &Bytes32, output: &mut [Bytes32]) {
        todo!()
    }

    fn zktrie_root(output: &mut Bytes32) {
        todo!()
    }

    fn zktrie_rollback() {
        todo!()
    }

    fn zktrie_commit() {
        todo!()
    }

    fn zktrie_store(key: &Bytes32, val: &Bytes32) {
        todo!()
    }

    fn zktrie_load(key: &Bytes32, val: &mut Bytes32) {
        todo!()
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
