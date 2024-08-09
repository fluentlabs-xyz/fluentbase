use crate::{
    alloc_slice,
    bindings::{
        _charge_fuel,
        _debug_log,
        _ecrecover,
        _exec,
        _exit,
        _forward_output,
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
        _resume,
        _state,
        _write,
    },
    Address,
    Bytes,
    B256,
    U256,
};
use alloc::{vec, vec::Vec};
use byteorder::{ByteOrder, LittleEndian};
use fluentbase_types::{
    calc_storage_key,
    Account,
    AccountCheckpoint,
    AccountStatus,
    Bytes32,
    ExitCode,
    Fuel,
    NativeAPI,
    SovereignAPI,
    F254,
    JZKT_ACCOUNT_BALANCE_FIELD,
    JZKT_ACCOUNT_COMPRESSION_FLAGS,
    JZKT_ACCOUNT_NONCE_FIELD,
    JZKT_ACCOUNT_RWASM_CODE_HASH_FIELD,
    JZKT_ACCOUNT_RWASM_CODE_SIZE_FIELD,
    JZKT_ACCOUNT_SOURCE_CODE_HASH_FIELD,
    JZKT_ACCOUNT_SOURCE_CODE_SIZE_FIELD,
    JZKT_STORAGE_COMPRESSION_FLAGS,
};

#[derive(Default)]
pub struct RwasmContext;

impl NativeAPI for RwasmContext {
    #[inline(always)]
    fn keccak256(&self, data: &[u8]) -> B256 {
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
    fn poseidon(&self, data: &[u8]) -> F254 {
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
    fn poseidon_hash(&self, fa: &F254, fb: &F254, fd: &F254) -> F254 {
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
    fn ec_recover(&self, digest: &B256, sig: &[u8; 64], rec_id: u8) -> [u8; 65] {
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
    fn debug_log(&self, message: &str) {
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
    fn read_context(&self, target: &mut [u8], offset: u32) {
        unsafe { _read_context(target.as_mut_ptr(), offset, target.len() as u32) }
    }

    #[inline(always)]
    fn charge_fuel(&self, value: u64) -> u64 {
        unsafe { _charge_fuel(value) }
    }

    fn exec(
        &self,
        code_hash: &F254,
        address: &Address,
        input: &[u8],
        fuel: &mut Fuel,
        state: u32,
    ) -> i32 {
        let mut fuel32 = fuel.remaining() as u32;
        let exit_code = unsafe {
            _exec(
                code_hash.as_ptr(),
                address.as_ptr(),
                input.as_ptr(),
                input.len() as u32,
                core::ptr::null(),
                0,
                core::ptr::null_mut(),
                0,
                &mut fuel32 as *mut u32,
                state,
            )
        };
        let fuel_spent = fuel.remaining() as u32 - fuel32;
        fuel.charge(fuel_spent as u64);
        exit_code
    }

    #[inline(always)]
    fn resume(&self, call_id: u32, exit_code: i32) -> i32 {
        unsafe { _resume(call_id, exit_code) }
    }
}
