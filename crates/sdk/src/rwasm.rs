pub use crate::{
    bindings::{
        _charge_fuel,
        _debug_log,
        _ecrecover,
        _exec,
        _exit,
        _forward_output,
        _fuel,
        _input_size,
        _keccak256,
        _output_size,
        _poseidon,
        _poseidon_hash,
        _preimage_copy,
        _preimage_size,
        _read,
        _read_output,
        _resume,
        _state,
        _write,
    },
    B256,
};
use fluentbase_types::{ContextFreeNativeAPI, NativeAPI, F254};

#[derive(Default)]
pub struct RwasmContext;

impl ContextFreeNativeAPI for RwasmContext {
    #[inline(always)]
    fn keccak256(data: &[u8]) -> B256 {
        unsafe {
            let mut res = B256::ZERO;
            _keccak256(
                data.as_ptr(),
                data.len() as u32,
                res.as_mut_slice().as_mut_ptr(),
            );
            res
        }
    }

    #[inline(always)]
    fn sha256(_data: &[u8]) -> B256 {
        todo!("not implemented")
    }

    #[inline(always)]
    fn poseidon(data: &[u8]) -> F254 {
        unsafe {
            let mut res = B256::ZERO;
            _poseidon(
                data.as_ptr(),
                data.len() as u32,
                res.as_mut_slice().as_mut_ptr(),
            );
            res
        }
    }

    #[inline(always)]
    fn poseidon_hash(fa: &F254, fb: &F254, fd: &F254) -> F254 {
        let mut res = B256::ZERO;
        unsafe {
            _poseidon_hash(
                fa.as_ptr(),
                fb.as_ptr(),
                fd.as_ptr(),
                res.as_mut_slice().as_mut_ptr(),
            )
        }
        res
    }

    #[inline(always)]
    fn ec_recover(digest: &B256, sig: &[u8; 64], rec_id: u8) -> [u8; 65] {
        unsafe {
            let mut res: [u8; 65] = [0u8; 65];
            _ecrecover(
                digest.0.as_ptr(),
                sig.as_ptr(),
                res.as_mut_ptr(),
                rec_id as u32,
            );
            res
        }
    }

    #[inline(always)]
    fn debug_log(message: &str) {
        unsafe { _debug_log(message.as_ptr(), message.len() as u32) }
    }
}

impl NativeAPI for RwasmContext {
    #[inline(always)]
    fn read(&self, target: &mut [u8], offset: u32) {
        unsafe { _read(target.as_mut_ptr(), offset, target.len() as u32) }
    }

    #[inline(always)]
    fn input_size(&self) -> u32 {
        unsafe { _input_size() }
    }

    #[inline(always)]
    fn write(&self, value: &[u8]) {
        unsafe { _write(value.as_ptr(), value.len() as u32) }
    }

    #[inline(always)]
    fn forward_output(&self, offset: u32, len: u32) {
        unsafe { _forward_output(offset, len) }
    }

    #[inline(always)]
    fn exit(&self, exit_code: i32) -> ! {
        unsafe { _exit(exit_code) }
    }

    #[inline(always)]
    fn output_size(&self) -> u32 {
        unsafe { _output_size() }
    }

    #[inline(always)]
    fn read_output(&self, target: &mut [u8], offset: u32) {
        unsafe { _read_output(target.as_mut_ptr(), offset, target.len() as u32) }
    }

    #[inline(always)]
    fn state(&self) -> u32 {
        unsafe { _state() }
    }

    #[inline(always)]
    fn fuel(&self) -> u64 {
        unsafe { _fuel() }
    }

    #[inline(always)]
    fn charge_fuel(&self, value: u64) -> u64 {
        unsafe { _charge_fuel(value) }
    }

    #[inline(always)]
    fn exec(&self, code_hash: &F254, input: &[u8], mut fuel_limit: u64, state: u32) -> (u64, i32) {
        unsafe {
            let exit_code = _exec(
                code_hash.as_ptr(),
                input.as_ptr(),
                input.len() as u32,
                &mut fuel_limit as *mut u64,
                state,
            );
            // fuel limit now contains consumed fuel
            (fuel_limit, exit_code)
        }
    }

    #[inline(always)]
    fn resume(
        &self,
        call_id: u32,
        return_data: &[u8],
        exit_code: i32,
        mut fuel_used: u64,
    ) -> (u64, i32) {
        unsafe {
            let exit_code = _resume(
                call_id,
                return_data.as_ptr(),
                return_data.len() as u32,
                exit_code,
                &mut fuel_used as *mut u64,
            );
            (fuel_used, exit_code)
        }
    }

    #[inline(always)]
    fn preimage_size(&self, hash: &B256) -> u32 {
        unsafe { _preimage_size(hash.as_ptr()) }
    }

    #[inline(always)]
    fn preimage_copy(&self, hash: &B256, target: &mut [u8]) {
        unsafe { _preimage_copy(hash.as_ptr(), target.as_mut_ptr()) }
    }
}
