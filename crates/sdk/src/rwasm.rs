pub use crate::{
    bindings::{
        _charge_fuel,
        _charge_fuel_manually,
        _debug_log,
        _exec,
        _exit,
        _forward_output,
        _fuel,
        _input_size,
        _keccak256,
        _output_size,
        _preimage_copy,
        _preimage_size,
        _read,
        _read_output,
        _resume,
        _secp256k1_recover,
        _state,
        _write,
    },
    B256,
};
use fluentbase_types::{native_api::NativeAPI, BytecodeOrHash, ExitCode};

#[derive(Default)]
pub struct RwasmContext;

impl NativeAPI for RwasmContext {
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
    fn secp256k1_recover(digest: &B256, sig: &[u8; 64], rec_id: u8) -> Option<[u8; 65]> {
        unsafe {
            let mut res: [u8; 65] = [0u8; 65];
            let ok = _secp256k1_recover(
                digest.0.as_ptr(),
                sig.as_ptr(),
                res.as_mut_ptr(),
                rec_id as u32,
            );
            if ok == 0 {
                Some(res)
            } else {
                None
            }
        }
    }

    #[inline(always)]
    fn debug_log(message: &str) {
        unsafe { _debug_log(message.as_ptr(), message.len() as u32) }
    }

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
    fn exit(&self, exit_code: ExitCode) -> ! {
        unsafe { _exit(exit_code.into_i32()) }
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
    fn charge_fuel_manually(&self, fuel_consumed: u64, fuel_refunded: i64) -> u64 {
        unsafe { _charge_fuel_manually(fuel_consumed, fuel_refunded) }
    }

    #[inline(always)]
    fn charge_fuel(&self, fuel_consumed: u64) {
        unsafe { _charge_fuel(fuel_consumed) }
    }

    #[inline(always)]
    fn exec<I: Into<BytecodeOrHash>>(
        &self,
        code_hash: I,
        input: &[u8],
        fuel_limit: Option<u64>,
        state: u32,
    ) -> (u64, i64, i32) {
        let code_hash: BytecodeOrHash = code_hash.into();
        unsafe {
            let mut fuel_info: [i64; 2] = [fuel_limit.unwrap_or(u64::MAX) as i64, 0];
            let exit_code = _exec(
                code_hash.hash().as_ptr(),
                input.as_ptr(),
                input.len() as u32,
                &mut fuel_info as *mut [i64; 2],
                state,
            );
            (fuel_info[0] as u64, fuel_info[1], exit_code)
        }
    }

    #[inline(always)]
    fn resume(
        &self,
        call_id: u32,
        return_data: &[u8],
        exit_code: i32,
        fuel_consumed: u64,
        fuel_refunded: i64,
    ) -> (u64, i64, i32) {
        unsafe {
            let mut fuel_info: [i64; 2] = [fuel_consumed as i64, fuel_refunded];
            let exit_code = _resume(
                call_id,
                return_data.as_ptr(),
                return_data.len() as u32,
                exit_code,
                &mut fuel_info as *mut [i64; 2],
            );
            (fuel_info[0] as u64, fuel_info[1], exit_code)
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
