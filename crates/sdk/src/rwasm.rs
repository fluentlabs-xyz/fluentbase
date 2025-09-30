pub use crate::{bindings::*, B256};
use fluentbase_types::{
    BytecodeOrHash, ExitCode, NativeAPI, BN254_G1_POINT_COMPRESSED_SIZE,
    BN254_G1_POINT_DECOMPRESSED_SIZE, BN254_G2_POINT_COMPRESSED_SIZE,
    BN254_G2_POINT_DECOMPRESSED_SIZE, EDWARDS_COMPRESSED_SIZE, EDWARDS_DECOMPRESSED_SIZE,
    TOWER_FP_BLS12381_SIZE, TOWER_FP_BN256_SIZE,
};

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
    fn sha256(data: &[u8]) -> B256 {
        unsafe {
            let mut res = B256::ZERO;
            _sha256(
                data.as_ptr(),
                data.len() as u32,
                res.as_mut_slice().as_mut_ptr(),
            );
            res
        }
    }

    #[inline(always)]
    fn blake3(data: &[u8]) -> B256 {
        unsafe {
            let mut res = B256::ZERO;
            _blake3(
                data.as_ptr(),
                data.len() as u32,
                res.as_mut_slice().as_mut_ptr(),
            );
            res
        }
    }
    #[inline(always)]
    fn poseidon(parameters: u32, endianness: u32, data: &[u8]) -> Result<B256, ExitCode> {
        unsafe {
            let mut res = B256::ZERO;
            if _poseidon(
                parameters,
                endianness,
                data.as_ptr() as *const u8,
                data.len() as u32,
                res.as_mut_ptr(),
            ) != 0
            {
                return Err(ExitCode::MalformedBuiltinParams);
            };
            Ok(res)
        }
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
    fn curve256r1_verify(input: &[u8]) -> bool {
        unsafe {
            let mut output = [0u8; 32];
            let ok = _curve256r1_verify(input.as_ptr(), input.len() as u32, output.as_mut_ptr());
            ok == 0
        }
    }
    #[inline(always)]
    fn ed25519_decompress(
        y: [u8; EDWARDS_COMPRESSED_SIZE],
        sign: u32,
    ) -> [u8; EDWARDS_DECOMPRESSED_SIZE] {
        let mut res = [0u8; EDWARDS_DECOMPRESSED_SIZE];
        res[EDWARDS_COMPRESSED_SIZE..].copy_from_slice(&y);
        unsafe { _ed25519_decompress(res.as_mut_ptr(), sign) };
        res
    }
    #[inline(always)]
    fn ed25519_add(
        mut p: [u8; EDWARDS_DECOMPRESSED_SIZE],
        q: [u8; EDWARDS_DECOMPRESSED_SIZE],
    ) -> [u8; EDWARDS_DECOMPRESSED_SIZE] {
        unsafe { _ed25519_add(p.as_mut_ptr(), q.as_ptr()) };
        p
    }

    #[inline(always)]
    fn tower_fp1_bn254_add(
        mut x: [u8; TOWER_FP_BN256_SIZE],
        y: [u8; TOWER_FP_BN256_SIZE],
    ) -> [u8; TOWER_FP_BN256_SIZE] {
        unsafe { _tower_fp1_bn254_add(x.as_mut_ptr(), y.as_ptr()) };
        x
    }
    #[inline(always)]
    fn tower_fp1_bn254_sub(
        x: [u8; TOWER_FP_BN256_SIZE],
        y: [u8; TOWER_FP_BN256_SIZE],
    ) -> [u8; TOWER_FP_BN256_SIZE] {
        unsafe { _tower_fp1_bn254_sub(x.as_mut_ptr(), y.as_ptr()) };
        x
    }
    #[inline(always)]
    fn tower_fp1_bn254_mul(
        x: [u8; TOWER_FP_BN256_SIZE],
        y: [u8; TOWER_FP_BN256_SIZE],
    ) -> [u8; TOWER_FP_BN256_SIZE] {
        unsafe { _tower_fp1_bn254_mul(x.as_mut_ptr(), y.as_ptr()) };
        x
    }
    #[inline(always)]
    fn tower_fp1_bls12381_add(
        _x: [u8; TOWER_FP_BLS12381_SIZE],
        _y: [u8; TOWER_FP_BLS12381_SIZE],
    ) -> [u8; TOWER_FP_BLS12381_SIZE] {
        unsafe { _tower_fp2_bls12381_add(x.as_mut_ptr(), y.as_ptr()) };
        x
    }
    #[inline(always)]
    fn tower_fp1_bls12381_sub(
        _x: [u8; TOWER_FP_BLS12381_SIZE],
        _y: [u8; TOWER_FP_BLS12381_SIZE],
    ) -> [u8; TOWER_FP_BLS12381_SIZE] {
        unsafe { _tower_fp2_bls12381_sub(x.as_mut_ptr(), y.as_ptr()) };
        x
    }
    #[inline(always)]
    fn tower_fp1_bls12381_mul(
        _x: [u8; TOWER_FP_BLS12381_SIZE],
        _y: [u8; TOWER_FP_BLS12381_SIZE],
    ) -> [u8; TOWER_FP_BLS12381_SIZE] {
        unsafe { _tower_fp2_bls12381_mul(x.as_mut_ptr(), y.as_ptr()) };
        x
    }
    #[inline(always)]
    fn tower_fp2_bn254_add(
        x: [u8; TOWER_FP_BN256_SIZE],
        y: [u8; TOWER_FP_BN256_SIZE],
    ) -> [u8; TOWER_FP_BN256_SIZE] {
        unsafe { _tower_fp2_bn254_add(x.as_mut_ptr(), y.as_ptr()) };
        x
    }
    #[inline(always)]
    fn tower_fp2_bn254_sub(
        x: [u8; TOWER_FP_BN256_SIZE],
        y: [u8; TOWER_FP_BN256_SIZE],
    ) -> [u8; TOWER_FP_BN256_SIZE] {
        unsafe { _tower_fp2_bn254_sub(x.as_mut_ptr(), y.as_ptr()) };
        x
    }
    #[inline(always)]
    fn tower_fp2_bn254_mul(
        x: [u8; TOWER_FP_BN256_SIZE],
        y: [u8; TOWER_FP_BN256_SIZE],
    ) -> [u8; TOWER_FP_BN256_SIZE] {
        unsafe { _tower_fp2_bn254_mul(x.as_mut_ptr(), y.as_ptr()) };
        x
    }
    #[inline(always)]
    fn tower_fp2_bls12381_add(
        _x: [u8; TOWER_FP_BLS12381_SIZE],
        _y: [u8; TOWER_FP_BLS12381_SIZE],
    ) -> [u8; TOWER_FP_BLS12381_SIZE] {
        unsafe { _tower_fp2_bls12381_add(x.as_mut_ptr(), y.as_ptr()) };
        x
    }
    #[inline(always)]
    fn tower_fp2_bls12381_sub(
        _x: [u8; TOWER_FP_BLS12381_SIZE],
        _y: [u8; TOWER_FP_BLS12381_SIZE],
    ) -> [u8; TOWER_FP_BLS12381_SIZE] {
        unsafe { _tower_fp2_bls12381_sub(x.as_mut_ptr(), y.as_ptr()) };
        x
    }
    #[inline(always)]
    fn tower_fp2_bls12381_mul(
        _x: [u8; TOWER_FP_BLS12381_SIZE],
        _y: [u8; TOWER_FP_BLS12381_SIZE],
    ) -> [u8; TOWER_FP_BLS12381_SIZE] {
        unsafe { _tower_fp2_bls12381_mul(x.as_mut_ptr(), y.as_ptr()) };
        x
    }

    #[inline(always)]
    fn bn254_add(p: &mut [u8; 64], q: &[u8; 64]) -> Result<[u8; 64], ExitCode> {
        unsafe {
            _bn254_add(p.as_ptr() as u32, q.as_ptr() as u32);
        }
        Ok(*p)
    }

    #[inline(always)]
    fn bn254_mul(p: &mut [u8; 64], q: &[u8; 32]) -> Result<[u8; 64], ExitCode> {
        unsafe {
            _bn254_mul(p.as_ptr() as u32, q.as_ptr() as u32);
        }
        Ok(*p)
    }

    #[inline(always)]
    fn bn254_multi_pairing(elements: &[([u8; 64], [u8; 128])]) -> Result<[u8; 32], ExitCode> {
        let mut result = [0u8; 32];
        unsafe {
            _bn254_multi_pairing(
                elements.as_ptr() as *const u8,
                elements.len() as u32,
                result.as_mut_ptr(),
            );
        }
        Ok(result)
    }

    #[inline(always)]
    fn bn254_g1_compress(
        point: &[u8; BN254_G1_POINT_DECOMPRESSED_SIZE],
    ) -> Result<[u8; BN254_G1_POINT_COMPRESSED_SIZE], ExitCode> {
        let mut result_point = [0u8; BN254_G1_POINT_COMPRESSED_SIZE];
        unsafe {
            if _bn254_g1_compress(point.as_ptr() as *const u8, result_point.as_mut_ptr()) != 0 {
                return Err(ExitCode::MalformedBuiltinParams);
            };
        }
        Ok(result_point)
    }

    #[inline(always)]
    fn bn254_g1_decompress(
        point: &[u8; BN254_G1_POINT_COMPRESSED_SIZE],
    ) -> Result<[u8; BN254_G1_POINT_DECOMPRESSED_SIZE], ExitCode> {
        let mut result_point = [0u8; BN254_G1_POINT_DECOMPRESSED_SIZE];
        unsafe {
            _bn254_g1_decompress(point.as_ptr() as *const u8, result_point.as_mut_ptr());
        }
        Ok(result_point)
    }

    #[inline(always)]
    fn bn254_g2_compress(
        point: &[u8; BN254_G2_POINT_DECOMPRESSED_SIZE],
    ) -> Result<[u8; BN254_G2_POINT_COMPRESSED_SIZE], ExitCode> {
        let mut result_point = [0u8; BN254_G2_POINT_COMPRESSED_SIZE];
        unsafe {
            _bn254_g2_compress(point.as_ptr() as *const u8, result_point.as_mut_ptr());
        }
        Ok(result_point)
    }

    #[inline(always)]
    fn bn254_g2_decompress(
        point: &[u8; BN254_G2_POINT_COMPRESSED_SIZE],
    ) -> Result<[u8; BN254_G2_POINT_DECOMPRESSED_SIZE], ExitCode> {
        let mut result_point = [0u8; BN254_G2_POINT_DECOMPRESSED_SIZE];
        unsafe {
            _bn254_g2_decompress(point.as_ptr() as *const u8, result_point.as_mut_ptr());
        }
        Ok(result_point)
    }

    #[inline(always)]
    fn bn254_double(p: &mut [u8; 64]) {
        unsafe {
            _bn254_double(p.as_ptr() as u32);
        }
    }

    #[inline(always)]
    fn bn254_fp_mul(p: &mut [u8; 64], q: &[u8; 32]) {
        unsafe {
            _bn254_fp_mul(p.as_ptr() as u32, q.as_ptr() as u32);
        }
    }

    #[inline(always)]
    fn bn254_fp2_mul(p: &mut [u8; 64], q: &[u8; 32]) {
        unsafe {
            _bn254_fp2_mul(p.as_ptr() as u32, q.as_ptr() as u32);
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
    fn exec(
        &self,
        code_hash: BytecodeOrHash,
        input: &[u8],
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
    fn preimage_size(&self, hash: &B256) -> u32 {
        unsafe { _preimage_size(hash.as_ptr()) }
    }

    #[inline(always)]
    fn preimage_copy(&self, hash: &B256, target: &mut [u8]) {
        unsafe { _preimage_copy(hash.as_ptr(), target.as_mut_ptr()) }
    }

    // BLS12-381 implementations
    #[inline(always)]
    fn bls12_381_g1_add(p: &mut [u8; 96], q: &[u8; 96]) {
        unsafe { _bls12_381_g1_add(p.as_mut_ptr(), q.as_ptr()) }
    }

    #[inline(always)]
    fn bls12_381_g1_msm(pairs: &[([u8; 96], [u8; 32])], out: &mut [u8; 96]) {
        unsafe {
            _bls12_381_g1_msm(
                pairs.as_ptr() as *const u8,
                pairs.len() as u32,
                out.as_mut_ptr(),
            )
        }
    }

    #[inline(always)]
    fn bls12_381_g2_add(p: &mut [u8; 192], q: &[u8; 192]) {
        unsafe { _bls12_381_g2_add(p.as_mut_ptr(), q.as_ptr()) }
    }

    #[inline(always)]
    fn bls12_381_g2_msm(pairs: &[([u8; 192], [u8; 32])], out: &mut [u8; 192]) {
        unsafe {
            _bls12_381_g2_msm(
                pairs.as_ptr() as *const u8,
                pairs.len() as u32,
                out.as_mut_ptr(),
            )
        }
    }

    #[inline(always)]
    fn bls12_381_pairing(pairs: &[([u8; 48], [u8; 96])], out: &mut [u8; 288]) {
        unsafe {
            _bls12_381_pairing(
                pairs.as_ptr() as *const u8,
                pairs.len() as u32,
                out.as_mut_ptr(),
            )
        }
    }

    #[inline(always)]
    fn bls12_381_map_fp_to_g1(p: &[u8; 64], out: &mut [u8; 96]) {
        unsafe { _bls12_381_map_fp_to_g1(p.as_ptr(), out.as_mut_ptr()) }
    }

    #[inline(always)]
    fn bls12_381_map_fp2_to_g2(p: &[u8; 128], out: &mut [u8; 192]) {
        unsafe { _bls12_381_map_fp2_to_g2(p.as_ptr(), out.as_mut_ptr()) }
    }
}
