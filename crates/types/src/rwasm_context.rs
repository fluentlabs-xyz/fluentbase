use crate::{
    BytecodeOrHash, CryptoAPI, ExitCode, NativeAPI, BLS12381_FP_SIZE, BLS12381_G1_COMPRESSED_SIZE,
    BLS12381_G1_RAW_AFFINE_SIZE, BN254_FP_SIZE, BN254_G1_RAW_AFFINE_SIZE,
    ED25519_POINT_COMPRESSED_SIZE, ED25519_POINT_DECOMPRESSED_SIZE, SECP256K1_G1_COMPRESSED_SIZE,
    SECP256K1_G1_RAW_AFFINE_SIZE, SECP256R1_G1_COMPRESSED_SIZE, SECP256R1_G1_RAW_AFFINE_SIZE,
};
use alloc::borrow::Cow;
use core::convert::Into;

pub mod bindings;
use bindings::*;

#[derive(Default)]
pub struct RwasmContext;

impl NativeAPI for RwasmContext {
    #[inline(always)]
    fn exit(&self, exit_code: ExitCode) -> ! {
        unsafe { _exit(exit_code.into_i32()) }
    }

    #[inline(always)]
    fn state(&self) -> u32 {
        unsafe { _state() }
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
    fn output_size(&self) -> u32 {
        unsafe { _output_size() }
    }

    #[inline(always)]
    fn read_output(&self, target: &mut [u8], offset: u32) {
        unsafe { _read_output(target.as_mut_ptr(), offset, target.len() as u32) }
    }

    #[inline(always)]
    fn exec(
        &self,
        code_hash: BytecodeOrHash,
        input: Cow<'_, [u8]>,
        fuel_limit: Option<u64>,
        state: u32,
    ) -> (u64, i64, i32) {
        let code_hash: BytecodeOrHash = code_hash.into();
        unsafe {
            let mut fuel_info: [i64; 2] = [fuel_limit.unwrap_or(u64::MAX) as i64, 0];
            let exit_code = _exec(
                code_hash.code_hash().as_ptr(),
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
    fn forward_output(&self, offset: u32, len: u32) {
        unsafe { _forward_output(offset, len) }
    }

    #[inline(always)]
    fn fuel(&self) -> u64 {
        unsafe { _fuel() }
    }

    #[inline(always)]
    fn debug_log(message: &str) {
        unsafe { _debug_log(message.as_ptr(), message.len() as u32) }
    }

    #[inline(always)]
    fn charge_fuel(&self, fuel_consumed: u64) {
        unsafe { _charge_fuel(fuel_consumed) }
    }

    #[inline(always)]
    fn enter_unconstrained(&self) {
        unsafe { _enter_unconstrained() }
    }

    #[inline(always)]
    fn exit_unconstrained(&self) {
        unsafe { _exit_unconstrained() }
    }

    #[inline(always)]
    fn write_fd(&self, fd: u32, slice: &[u8]) {
        unsafe { _write_fd(fd, slice.as_ptr(), slice.len() as u32) }
    }
}

#[rustfmt::skip]
impl CryptoAPI for RwasmContext {
    #[inline(always)]
    fn keccak256_permute(state: &mut [u64; 25]) {
        unsafe { _keccak256_permute(state.as_mut_ptr() as *mut [u64; 25]) }
    }
    #[inline(always)]
    fn sha256_extend(w: &mut [u32; 64]) {
        unsafe { _sha256_extend(w.as_mut_ptr() as *mut [u32; 64]) }
    }
    #[inline(always)]
    fn sha256_compress(state: &mut [u32; 8], w: &[u32; 64]) {
        unsafe { _sha256_compress(state.as_mut_ptr() as *mut [u32; 8], w.as_ptr() as *mut [u32; 64]) }
    }

    #[inline(always)]
    fn ed25519_decompress(y: [u8; ED25519_POINT_COMPRESSED_SIZE], sign: u32) -> [u8; ED25519_POINT_DECOMPRESSED_SIZE] {
        let mut result = [0u8; ED25519_POINT_DECOMPRESSED_SIZE];
        result[..ED25519_POINT_COMPRESSED_SIZE].copy_from_slice(&y);
        unsafe { _ed25519_decompress(result.as_mut_ptr(), sign) };
        result
    }
    #[inline(always)]
    fn ed25519_add(mut p: [u8; ED25519_POINT_DECOMPRESSED_SIZE], q: [u8; ED25519_POINT_DECOMPRESSED_SIZE]) -> [u8; ED25519_POINT_DECOMPRESSED_SIZE] {
        unsafe { _ed25519_add(p.as_mut_ptr(), q.as_ptr()) };
        p
    }

    #[inline(always)]
    fn tower_fp1_bn254_add(mut x: [u8; BN254_FP_SIZE], y: [u8; BN254_FP_SIZE]) -> [u8; BN254_FP_SIZE] {
        unsafe { _tower_fp1_bn254_add(x.as_mut_ptr(), y.as_ptr()) };
        x
    }
    #[inline(always)]
    fn tower_fp1_bn254_sub(mut x: [u8; BN254_FP_SIZE], y: [u8; BN254_FP_SIZE]) -> [u8; BN254_FP_SIZE] {
        unsafe { _tower_fp1_bn254_sub(x.as_mut_ptr(), y.as_ptr()) };
        x
    }
    #[inline(always)]
    fn tower_fp1_bn254_mul(mut x: [u8; BN254_FP_SIZE], y: [u8; BN254_FP_SIZE]) -> [u8; BN254_FP_SIZE] {
        unsafe { _tower_fp1_bn254_mul(x.as_mut_ptr(), y.as_ptr()) };
        x
    }
    #[inline(always)]
    fn tower_fp1_bls12381_add(mut x: [u8; BLS12381_FP_SIZE], y: [u8; BLS12381_FP_SIZE]) -> [u8; BLS12381_FP_SIZE] {
        unsafe { _tower_fp1_bls12381_add(x.as_mut_ptr(), y.as_ptr()) };
        x
    }
    #[inline(always)]
    fn tower_fp1_bls12381_sub(mut x: [u8; BLS12381_FP_SIZE], y: [u8; BLS12381_FP_SIZE]) -> [u8; BLS12381_FP_SIZE] {
        unsafe { _tower_fp1_bls12381_sub(x.as_mut_ptr(), y.as_ptr()) };
        x
    }
    #[inline(always)]
    fn tower_fp1_bls12381_mul(mut x: [u8; BLS12381_FP_SIZE], y: [u8; BLS12381_FP_SIZE]) -> [u8; BLS12381_FP_SIZE] {
        unsafe { _tower_fp1_bls12381_mul(x.as_mut_ptr(), y.as_ptr()) };
        x
    }
    #[inline(always)]
    fn tower_fp2_bn254_add(mut a_c0: [u8; BN254_FP_SIZE], mut a_c1: [u8; BN254_FP_SIZE], b_c0: [u8; BN254_FP_SIZE], b_c1: [u8; BN254_FP_SIZE]) -> ([u8; BN254_FP_SIZE], [u8; BN254_FP_SIZE]) {
        unsafe { _tower_fp2_bn254_add(a_c0.as_mut_ptr(), a_c1.as_mut_ptr(), b_c0.as_ptr(), b_c1.as_ptr()) };
        (a_c0, a_c1)
    }
    #[inline(always)]
    fn tower_fp2_bn254_sub(mut a_c0: [u8; BN254_FP_SIZE], mut a_c1: [u8; BN254_FP_SIZE], b_c0: [u8; BN254_FP_SIZE], b_c1: [u8; BN254_FP_SIZE]) -> ([u8; BN254_FP_SIZE], [u8; BN254_FP_SIZE]) {
        unsafe { _tower_fp2_bn254_sub(a_c0.as_mut_ptr(), a_c1.as_mut_ptr(), b_c0.as_ptr(), b_c1.as_ptr()) };
        (a_c0, a_c1)
    }
    #[inline(always)]
    fn tower_fp2_bn254_mul(mut a_c0: [u8; BN254_FP_SIZE], mut a_c1: [u8; BN254_FP_SIZE], b_c0: [u8; BN254_FP_SIZE], b_c1: [u8; BN254_FP_SIZE]) -> ([u8; BN254_FP_SIZE], [u8; BN254_FP_SIZE]) {
        unsafe { _tower_fp2_bn254_mul(a_c0.as_mut_ptr(), a_c1.as_mut_ptr(), b_c0.as_ptr(), b_c1.as_ptr()) };
        (a_c0, a_c1)
    }
    #[inline(always)]
    fn tower_fp2_bls12381_add(mut a_c0: [u8; BLS12381_FP_SIZE], mut a_c1: [u8; BLS12381_FP_SIZE], b_c0: [u8; BLS12381_FP_SIZE], b_c1: [u8; BLS12381_FP_SIZE]) -> ([u8; BLS12381_FP_SIZE], [u8; BLS12381_FP_SIZE]) {
        unsafe { _tower_fp2_bls12381_add(a_c0.as_mut_ptr(), a_c1.as_mut_ptr(), b_c0.as_ptr(), b_c1.as_ptr()) };
        (a_c0, a_c1)
    }
    #[inline(always)]
    fn tower_fp2_bls12381_sub(mut a_c0: [u8; BLS12381_FP_SIZE], mut a_c1: [u8; BLS12381_FP_SIZE], b_c0: [u8; BLS12381_FP_SIZE], b_c1: [u8; BLS12381_FP_SIZE]) -> ([u8; BLS12381_FP_SIZE], [u8; BLS12381_FP_SIZE]) {
        unsafe { _tower_fp2_bls12381_sub(a_c0.as_mut_ptr(), a_c1.as_mut_ptr(), b_c0.as_ptr(), b_c1.as_ptr()) };
        (a_c0, a_c1)
    }
    #[inline(always)]
    fn tower_fp2_bls12381_mul(mut a_c0: [u8; BLS12381_FP_SIZE], mut a_c1: [u8; BLS12381_FP_SIZE], b_c0: [u8; BLS12381_FP_SIZE], b_c1: [u8; BLS12381_FP_SIZE]) -> ([u8; BLS12381_FP_SIZE], [u8; BLS12381_FP_SIZE]) {
        unsafe { _tower_fp2_bls12381_mul(a_c0.as_mut_ptr(), a_c1.as_mut_ptr(), b_c0.as_ptr(), b_c1.as_ptr()) };
        (a_c0, a_c1)
    }

    #[inline(always)]
    fn secp256k1_add(mut p: [u8; SECP256K1_G1_RAW_AFFINE_SIZE], q: [u8; SECP256K1_G1_RAW_AFFINE_SIZE]) -> [u8; SECP256K1_G1_RAW_AFFINE_SIZE] {
        unsafe { _secp256k1_add(p.as_mut_ptr(), q.as_ptr()) };
        p
    }
    #[inline(always)]
    fn secp256k1_decompress(x: [u8; SECP256K1_G1_COMPRESSED_SIZE], sign: u32) -> [u8; SECP256K1_G1_RAW_AFFINE_SIZE] {
        let mut result = [0u8; SECP256K1_G1_RAW_AFFINE_SIZE];
        result[..SECP256K1_G1_COMPRESSED_SIZE].copy_from_slice(x.as_slice());
        result.reverse();
        // The input is YX (LE)
        unsafe { _secp256k1_decompress(result.as_mut_ptr(), sign) };
        result.reverse();
        // The output result is XY (BE)
        result
    }
    #[inline(always)]
    fn secp256k1_double(mut p: [u8; SECP256K1_G1_RAW_AFFINE_SIZE]) -> [u8; SECP256K1_G1_RAW_AFFINE_SIZE] {
        unsafe { _secp256k1_double(p.as_mut_ptr()) };
        p
    }

    #[inline(always)]
    fn secp256r1_add(mut p: [u8; SECP256R1_G1_RAW_AFFINE_SIZE], q: [u8; SECP256R1_G1_RAW_AFFINE_SIZE]) -> [u8; SECP256R1_G1_RAW_AFFINE_SIZE] {
        unsafe { _secp256r1_add(p.as_mut_ptr(), q.as_ptr()) };
        p
    }
    #[inline(always)]
    fn secp256r1_decompress(x: [u8; SECP256R1_G1_COMPRESSED_SIZE], sign: u32) -> [u8; SECP256R1_G1_RAW_AFFINE_SIZE] {
        let mut result = [0u8; SECP256R1_G1_RAW_AFFINE_SIZE];
        result[..SECP256R1_G1_COMPRESSED_SIZE].copy_from_slice(x.as_slice());
        result.reverse();
        // The input is YX (LE)
        unsafe { _secp256r1_decompress(result.as_mut_ptr(), sign) };
        result.reverse();
        // The output result is XY (BE)
        result
    }
    #[inline(always)]
    fn secp256r1_double(mut p: [u8; SECP256R1_G1_RAW_AFFINE_SIZE]) -> [u8; SECP256R1_G1_RAW_AFFINE_SIZE] {
        unsafe { _secp256r1_double(p.as_mut_ptr()) };
        p
    }

    #[inline(always)]
    fn bls12381_add(mut p: [u8; BLS12381_G1_RAW_AFFINE_SIZE], q: [u8; BLS12381_G1_RAW_AFFINE_SIZE]) -> [u8; BLS12381_G1_RAW_AFFINE_SIZE] {
        unsafe { _bls12381_add(p.as_mut_ptr(), q.as_ptr()) };
        p
    }
    #[inline(always)]
    fn bls12381_decompress(x: [u8; BLS12381_G1_COMPRESSED_SIZE], sign: u32) -> [u8; BLS12381_G1_RAW_AFFINE_SIZE] {
        let mut result = [0u8; BLS12381_G1_RAW_AFFINE_SIZE];
        result[..BLS12381_G1_COMPRESSED_SIZE].copy_from_slice(x.as_slice());
        result.reverse();
        // The input is YX (LE)
        unsafe { _bls12381_decompress(result.as_mut_ptr(), sign) };
        result.reverse();
        // The output result is XY (BE)
        result
    }
    #[inline(always)]
    fn bls12381_double(mut p: [u8; BLS12381_G1_RAW_AFFINE_SIZE]) -> [u8; BLS12381_G1_RAW_AFFINE_SIZE] {
        unsafe { _bls12381_double(p.as_mut_ptr()) };
        p
    }

    #[inline(always)]
    fn bn254_add(mut p: [u8; BN254_G1_RAW_AFFINE_SIZE], q: [u8; BN254_G1_RAW_AFFINE_SIZE]) -> [u8; BN254_G1_RAW_AFFINE_SIZE] {
        unsafe { _bn254_add(p.as_mut_ptr(), q.as_ptr()) };
        p
    }
    #[inline(always)]
    fn bn254_double(mut p: [u8; BN254_G1_RAW_AFFINE_SIZE]) -> [u8; BN254_G1_RAW_AFFINE_SIZE] {
        unsafe { _bn254_double(p.as_mut_ptr()) };
        p
    }

    #[inline(always)]
    fn uint256_mul_mod(x: &[u8; 32], y: &[u8; 32], m: &[u8; 32]) -> [u8; 32] {
        let mut x = x.clone();
        unsafe { _uint256_mul_mod(x.as_mut_ptr(), y.as_ptr(), m.as_ptr()) };
        x
    }
    #[inline(always)]
    fn uint256_x2048_mul(a: &[u8; 32], b: &[u8; 256]) -> ([u8; 256], [u8; 32]) {
        let (mut lo, mut hi) = ([0u8; 256], [0u8; 32]);
        unsafe { _uint256_x2048_mul(a.as_ptr(), b.as_ptr(), lo.as_mut_ptr(), hi.as_mut_ptr()) };
        (lo, hi)
    }
}
