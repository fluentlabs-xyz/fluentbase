#[allow(dead_code)]
use crate::{Bytes32, LowLevelAPI, LowLevelSDK};
use byteorder::{ByteOrder, LittleEndian};
use fluentbase_runtime::{
    instruction::{
        crypto_ecrecover::CryptoEcrecover,
        crypto_keccak256::CryptoKeccak256,
        crypto_poseidon::CryptoPoseidon,
        crypto_poseidon2::CryptoPoseidon2,
        preimage_copy::PreimageCopy,
        preimage_size::PreimageSize,
        rwasm_compile::RwasmCompile,
        sys_exec::SysExec,
        sys_halt::SysHalt,
        sys_input_size::SysInputSize,
        sys_output_size::SysOutputSize,
        sys_read::SysRead,
        sys_read_output::SysReadOutput,
        sys_state::SysState,
        sys_write::SysWrite,
    },
    RuntimeContext,
};
use std::ptr;

thread_local! {
    pub static CONTEXT: std::cell::Cell<RuntimeContext<'static, ()>> = std::cell::Cell::new(RuntimeContext::new(&[]));
}

fn with_context<F, R>(func: F) -> R
where
    F: Fn(&RuntimeContext<'static, ()>) -> R,
{
    CONTEXT.with(|ctx| {
        let ctx2 = ctx.take();
        let result = func(&ctx2);
        ctx.set(ctx2);
        result
    })
}

fn with_context_mut<F, R>(func: F) -> R
where
    F: Fn(&mut RuntimeContext<'static, ()>) -> R,
{
    CONTEXT.with(|ctx| {
        let mut ctx2 = ctx.take();
        let result = func(&mut ctx2);
        ctx.set(ctx2);
        result
    })
}

impl LowLevelAPI for LowLevelSDK {
    fn sys_read(target: &mut [u8], offset: u32) {
        let result =
            with_context(|ctx| SysRead::fn_impl(ctx, offset, target.len() as u32).unwrap());
        target.copy_from_slice(&result);
    }

    fn sys_input_size() -> u32 {
        with_context(|ctx| SysInputSize::fn_impl(ctx))
    }

    fn sys_write(value: &[u8]) {
        with_context_mut(|ctx| SysWrite::fn_impl(ctx, value))
    }

    fn sys_halt(exit_code: i32) {
        with_context_mut(|ctx| SysHalt::fn_impl(ctx, exit_code))
    }

    fn sys_output_size() -> u32 {
        with_context(|ctx| SysOutputSize::fn_impl(ctx))
    }

    fn sys_read_output(target: *mut u8, offset: u32, length: u32) {
        let result = with_context(|ctx| SysReadOutput::fn_impl(ctx, offset, length).unwrap());
        unsafe { ptr::copy(result.as_ptr(), target, length as usize) }
    }

    fn sys_exec(
        code_offset: *const u8,
        code_len: u32,
        input_offset: *const u8,
        input_len: u32,
        return_offset: *mut u8,
        return_len: u32,
        fuel_offset: *mut u32,
        state: u32,
    ) -> i32 {
        let bytecode =
            unsafe { &*ptr::slice_from_raw_parts(code_offset, code_len as usize) }.to_vec();
        let input =
            unsafe { &*ptr::slice_from_raw_parts(input_offset, input_len as usize) }.to_vec();
        let fuel = LittleEndian::read_u32(unsafe {
            &*ptr::slice_from_raw_parts(fuel_offset as *const u8, 4)
        });
        match with_context_mut(move |ctx| {
            SysExec::fn_impl(
                ctx,
                bytecode.clone(),
                input.clone(),
                return_len,
                fuel,
                state,
            )
        }) {
            Ok((result, remaining_fuel)) => {
                if return_len > 0 {
                    unsafe { ptr::copy(result.as_ptr(), return_offset, return_len as usize) }
                }
                LittleEndian::write_u32(
                    unsafe { &mut *ptr::slice_from_raw_parts_mut(fuel_offset as *mut u8, 4) },
                    remaining_fuel,
                );
                0
            }
            Err(err) => err.into_i32(),
        }
    }

    fn sys_state() -> u32 {
        with_context(|ctx| SysState::fn_impl(ctx))
    }

    fn crypto_keccak256(data_offset: *const u8, data_len: u32, output32_offset: *mut u8) {
        let result = CryptoKeccak256::fn_impl(unsafe {
            &*ptr::slice_from_raw_parts(data_offset, data_len as usize)
        });
        unsafe {
            ptr::copy(result.as_ptr(), output32_offset, 32);
        }
    }

    fn crypto_poseidon(data_offset: *const u8, data_len: u32, output32_offset: *mut u8) {
        let result = CryptoPoseidon::fn_impl(unsafe {
            &*ptr::slice_from_raw_parts(data_offset, data_len as usize)
        });
        unsafe {
            ptr::copy(result.as_ptr(), output32_offset, 32);
        }
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

    fn preimage_size(hash32: *const u8) -> u32 {
        with_context(|ctx| {
            PreimageSize::fn_impl(ctx, unsafe { &*ptr::slice_from_raw_parts(hash32, 32) })
        })
        .unwrap()
    }

    fn preimage_copy(hash32: *const u8, output_offset: *mut u8, output_len: u32) {
        let output = with_context(|ctx| {
            PreimageCopy::fn_impl(
                ctx,
                unsafe { &*ptr::slice_from_raw_parts(hash32, 32) },
                output_len,
            )
        })
        .unwrap();
        if output_len > 0 {
            unsafe {
                ptr::copy(output.as_ptr(), output_offset, output_len as usize);
            }
        }
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
        _is_delegate: bool,
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

    fn rwasm_create(
        _value32_offset: &[u8],
        _input_bytecode: &[u8],
        _salt32: &[u8],
        _deployed_contract_address20_output: &mut [u8],
        _is_create2: bool,
    ) -> i32 {
        unreachable!("rwasm methods are not available in this mode")
    }

    fn statedb_get_code(_key: &[u8], _output: &mut [u8], _code_offset: u32) {
        unreachable!("statedb methods are not available in this mode")
    }

    fn statedb_get_code_size(_key: &[u8]) -> u32 {
        unreachable!("statedb methods are not available in this mode")
    }

    fn statedb_get_code_hash(_key: &[u8], _out_hash32: &mut [u8]) -> () {
        unreachable!("statedb methods are not available in this mode")
    }

    fn statedb_get_balance(_address20: &[u8], _out_balance32: &mut [u8], _is_self: bool) -> () {
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

    fn zktrie_field(_key: *const u8, _field: u32, _output: *mut u8) {
        unreachable!("zktrie methods are not available in this mode")
    }

    fn zktrie_root(_output: &mut Bytes32) {
        unreachable!("zktrie methods are not available in this mode")
    }

    fn zktrie_checkpoint() -> u32 {
        unreachable!("zktrie methods are not available in this mode")
    }

    fn zktrie_rollback(_checkpoint: u32) {
        unreachable!("zktrie methods are not available in this mode")
    }

    fn zktrie_commit(_root32_offset: *mut u8) {
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
