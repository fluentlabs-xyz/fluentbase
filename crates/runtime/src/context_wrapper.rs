use crate::{
    syscall_handler::{
        weierstrass::{
            Bls12381G1MapConfig, Bls12381G2MapConfig, Bn254G2DecompressConfig,
            Secp256r1VerifyConfig, SyscallWeierstrassAddAssign,
            SyscallWeierstrassCompressDecompressAssign, SyscallWeierstrassDoubleAssign,
            SyscallWeierstrassMapAssign, SyscallWeierstrassMulAssign,
            SyscallWeierstrassPairingAssign, SyscallWeierstrassRecoverAssign,
            SyscallWeierstrassVerifyAssign,
        },
        *,
    },
    RuntimeContext,
};
use fluentbase_types::{
    BytecodeOrHash, Bytes, BytesOrRef, ExitCode, NativeAPI, UnwrapExitCode, B256,
    BN254_G1_POINT_COMPRESSED_SIZE, BN254_G1_POINT_DECOMPRESSED_SIZE,
    BN254_G2_POINT_COMPRESSED_SIZE, BN254_G2_POINT_DECOMPRESSED_SIZE, G1_COMPRESSED_SIZE,
    G1_UNCOMPRESSED_SIZE, G2_COMPRESSED_SIZE, G2_UNCOMPRESSED_SIZE, GT_COMPRESSED_SIZE,
    PADDED_FP2_SIZE, PADDED_FP_SIZE, SCALAR_SIZE,
};
use sp1_curves::weierstrass::{
    bls12_381::Bls12381,
    bn254::{Bn254, Bn254BaseField},
};
use std::cell::RefCell;

#[derive(Default)]
pub struct RuntimeContextWrapper {
    pub ctx: RefCell<RuntimeContext>,
}

impl RuntimeContextWrapper {
    pub fn new(ctx: RuntimeContext) -> Self {
        Self {
            ctx: RefCell::new(ctx),
        }
    }
    pub fn into_inner(self) -> RuntimeContext {
        self.ctx.into_inner()
    }
}

impl NativeAPI for RuntimeContextWrapper {
    fn keccak256(data: &[u8]) -> B256 {
        SyscallKeccak256::fn_impl(data)
    }

    fn sha256(data: &[u8]) -> B256 {
        SyscallSha256::fn_impl(data)
    }

    fn blake3(data: &[u8]) -> B256 {
        SyscallBlake3::fn_impl(data)
    }
    fn poseidon(parameters: u32, endianness: u32, data: &[u8]) -> Result<B256, ExitCode> {
        SyscallPoseidon::fn_impl(parameters as u64, endianness as u64, data)
    }

    /////// Weierstrass curves ///////

    fn secp256k1_recover(digest: &B256, sig: &[u8; 64], rec_id: u8) -> Option<[u8; 65]> {
        SyscallWeierstrassRecoverAssign::<Secp256k1RecoverConfig>::fn_impl(digest, sig, rec_id).map(
            |v| {
                let mut result = [0u8; 65];
                let min = core::cmp::min(result.len(), v.len());
                result[..min].copy_from_slice(&v[..min]);
                result
            },
        )
    }

    fn curve256r1_verify(input: &[u8]) -> bool {
        SyscallWeierstrassVerifyAssign::<Secp256r1VerifyConfig>::fn_impl(input)
    }

    fn bls12_381_g1_add(p: &mut [u8; G1_UNCOMPRESSED_SIZE], q: &[u8; G1_UNCOMPRESSED_SIZE]) {
        if let Ok(result) = SyscallWeierstrassAddAssign::<Bls12381G1AddConfig>::fn_impl(p, q) {
            p.copy_from_slice(&result[..G1_UNCOMPRESSED_SIZE]);
        }
    }

    fn bls12_381_g1_msm(
        pairs: &[([u8; G1_UNCOMPRESSED_SIZE], [u8; SCALAR_SIZE])],
        out: &mut [u8; G1_UNCOMPRESSED_SIZE],
    ) {
        if pairs.is_empty() {
            out.fill(0);
            return;
        }

        // Convert pairs to the format expected by the MSM implementation
        let pairs_vec: Vec<(Vec<u8>, Vec<u8>)> = pairs
            .iter()
            .map(|(point, scalar)| (point.to_vec(), scalar.to_vec()))
            .collect();

        // Use the new MSM implementation
        let result = SyscallWeierstrassMsm::<Bls12381G1MulConfig>::fn_impl(&pairs_vec);

        // Copy result to output
        if !result.is_empty() {
            out.copy_from_slice(&result);
        } else {
            out.fill(0);
        }
    }

    fn bls12_381_g2_add(p: &mut [u8; G2_UNCOMPRESSED_SIZE], q: &[u8; G2_UNCOMPRESSED_SIZE]) {
        if let Ok(result) = SyscallWeierstrassAddAssign::<Bls12381G2AddConfig>::fn_impl(p, q) {
            if !result.is_empty() {
                p.copy_from_slice(&result[..G2_UNCOMPRESSED_SIZE]);
            }
        }
    }

    fn bls12_381_g2_msm(
        pairs: &[([u8; G2_UNCOMPRESSED_SIZE], [u8; SCALAR_SIZE])],
        out: &mut [u8; G2_UNCOMPRESSED_SIZE],
    ) {
        if pairs.is_empty() {
            out.fill(0);
            return;
        }

        // Convert pairs to the format expected by the MSM implementation
        let pairs_vec: Vec<(Vec<u8>, Vec<u8>)> = pairs
            .iter()
            .map(|(point, scalar)| (point.to_vec(), scalar.to_vec()))
            .collect();

        // Use the new MSM implementation
        let result = SyscallWeierstrassMsm::<Bls12381G2MulConfig>::fn_impl(&pairs_vec);
        if !result.is_empty() {
            out.copy_from_slice(&result);
        } else {
            out.fill(0);
        }
    }

    fn bls12_381_pairing(
        pairs: &[([u8; G1_COMPRESSED_SIZE], [u8; G2_COMPRESSED_SIZE])],
        out: &mut [u8; GT_COMPRESSED_SIZE],
    ) {
        let result = SyscallWeierstrassPairingAssign::<Bls12381>::fn_impl_bls12_381(&pairs);
        match result {
            Ok(v) => {
                let min = core::cmp::min(out.len(), v.len());
                out[..min].copy_from_slice(&v[..min]);
                if min < out.len() {
                    out[min..].fill(0);
                }
            }
            Err(_) => {
                out.fill(0);
            }
        }
    }

    fn bls12_381_map_fp_to_g1(p: &[u8; PADDED_FP_SIZE], out: &mut [u8; G1_UNCOMPRESSED_SIZE]) {
        let result = SyscallWeierstrassMapAssign::<Bls12381G1MapConfig>::fn_impl(p.as_slice());
        out.copy_from_slice(&result[..G1_UNCOMPRESSED_SIZE]);
    }

    fn bls12_381_map_fp2_to_g2(p: &[u8; PADDED_FP2_SIZE], out: &mut [u8; G2_UNCOMPRESSED_SIZE]) {
        let result = SyscallWeierstrassMapAssign::<Bls12381G2MapConfig>::fn_impl(p.as_slice());
        out.copy_from_slice(&result[..G2_UNCOMPRESSED_SIZE]);
    }

    fn bn254_add(
        p: &mut [u8; BN254_G1_POINT_DECOMPRESSED_SIZE],
        q: &[u8; BN254_G1_POINT_DECOMPRESSED_SIZE],
    ) -> Result<[u8; BN254_G1_POINT_DECOMPRESSED_SIZE], ExitCode> {
        match SyscallWeierstrassAddAssign::<Bn254G1AddConfig>::fn_impl(p, q) {
            Ok(result) => {
                if result.is_empty() {
                    return Err(ExitCode::MalformedBuiltinParams);
                }
                let min = core::cmp::min(p.len(), result.len());
                p[..min].copy_from_slice(&result[..min]);
                Ok(*p)
            }
            Err(e) => Err(e),
        }
    }

    fn bn254_double(p: &mut [u8; BN254_G1_POINT_DECOMPRESSED_SIZE]) {
        let result = SyscallWeierstrassDoubleAssign::<Bn254>::fn_impl(p);
        let min = core::cmp::min(p.len(), result.len());
        p[..min].copy_from_slice(&result[..min]);
    }

    fn bn254_mul(
        p: &mut [u8; BN254_G1_POINT_DECOMPRESSED_SIZE],
        q: &[u8; SCALAR_SIZE],
    ) -> Result<[u8; BN254_G1_POINT_DECOMPRESSED_SIZE], ExitCode> {
        let result = SyscallWeierstrassMulAssign::<Bn254G1MulConfig>::fn_impl(p, q)
            .map_err(|_| ExitCode::PrecompileError)?;
        let result_array: [u8; BN254_G1_POINT_DECOMPRESSED_SIZE] =
            result.try_into().map_err(|_| ExitCode::PrecompileError)?;
        Ok(result_array)
    }

    fn bn254_multi_pairing(
        pairs: &[(
            [u8; BN254_G1_POINT_DECOMPRESSED_SIZE],
            [u8; BN254_G2_POINT_DECOMPRESSED_SIZE],
        )],
    ) -> Result<[u8; SCALAR_SIZE], ExitCode> {
        let result = SyscallWeierstrassPairingAssign::<Bn254>::fn_impl_bn254(&pairs)
            .map_err(|_| ExitCode::PrecompileError)?;
        let result_array: [u8; SCALAR_SIZE] =
            result.try_into().map_err(|_| ExitCode::PrecompileError)?;
        Ok(result_array)
    }

    fn bn254_g1_compress(
        point: &[u8; BN254_G1_POINT_DECOMPRESSED_SIZE],
    ) -> Result<[u8; BN254_G1_POINT_COMPRESSED_SIZE], ExitCode> {
        let result = SyscallWeierstrassCompressDecompressAssign::<
            crate::syscall_handler::weierstrass::Bn254G1CompressConfig,
        >::fn_impl(point)?;
        result.try_into().map_err(|_| ExitCode::UnknownError)
    }

    fn bn254_g1_decompress(
        point: &[u8; BN254_G1_POINT_COMPRESSED_SIZE],
    ) -> Result<[u8; BN254_G1_POINT_DECOMPRESSED_SIZE], ExitCode> {
        let result = SyscallWeierstrassCompressDecompressAssign::<
            crate::syscall_handler::weierstrass::Bn254G1DecompressConfig,
        >::fn_impl(point)?;
        result.try_into().map_err(|_| ExitCode::UnknownError)
    }

    fn bn254_g2_compress(
        point: &[u8; BN254_G2_POINT_DECOMPRESSED_SIZE],
    ) -> Result<[u8; BN254_G2_POINT_COMPRESSED_SIZE], ExitCode> {
        let result = SyscallWeierstrassCompressDecompressAssign::<
            crate::syscall_handler::weierstrass::Bn254G2CompressConfig,
        >::fn_impl(point)?;
        result.try_into().map_err(|_| ExitCode::UnknownError)
    }

    fn bn254_g2_decompress(
        point: &[u8; BN254_G2_POINT_COMPRESSED_SIZE],
    ) -> Result<[u8; BN254_G2_POINT_DECOMPRESSED_SIZE], ExitCode> {
        let result =
            SyscallWeierstrassCompressDecompressAssign::<Bn254G2DecompressConfig>::fn_impl(point)?;
        result.try_into().map_err(|_| ExitCode::UnknownError)
    }

    fn bn254_fp_mul(p: &mut [u8; BN254_G1_POINT_DECOMPRESSED_SIZE], q: &[u8; SCALAR_SIZE]) {
        let result = SyscallFpOp::<Bn254BaseField, FieldMul>::fn_impl(p, q);
        let min = core::cmp::min(p.len(), result.len());
        p[..min].copy_from_slice(&result[..min]);
    }

    fn bn254_fp2_mul(p: &mut [u8; BN254_G2_POINT_COMPRESSED_SIZE], q: &[u8; SCALAR_SIZE]) {
        let result = SyscallFp2Mul::<Bn254BaseField>::fn_impl(p, q);
        let min = core::cmp::min(p.len(), result.len());
        p[..min].copy_from_slice(&result[..min]);
    }

    fn curve25519_ristretto_decompress_validate(p: &[u8; 32]) -> bool {
        SyscallCurve25519RistrettoDecompressValidate::fn_impl(p).map_or_else(|_| false, |_| true)
    }

    fn curve25519_ristretto_add(p: &mut [u8; 32], q: &[u8; 32]) -> bool {
        SyscallCurve25519RistrettoAdd::fn_impl(p, q).is_ok()
    }

    fn curve25519_ristretto_sub(p: &mut [u8; 32], q: &[u8; 32]) -> bool {
        SyscallCurve25519RistrettoSub::fn_impl(p, q).is_ok()
    }

    fn curve25519_ristretto_mul(p: &mut [u8; 32], q: &[u8; 32]) -> bool {
        SyscallCurve25519RistrettoMul::fn_impl(p, q).is_ok()
    }

    fn curve25519_ristretto_multiscalar_mul(
        pairs: &[([u8; 32], [u8; 32])],
        out: &mut [u8; 32],
    ) -> bool {
        let result = SyscallCurve25519RistrettoMultiscalarMul::fn_impl(pairs);
        match result {
            Ok(v) => {
                *out = v.compress().to_bytes();
            }
            Err(_) => return false,
        }
        true
    }

    fn curve25519_edwards_decompress_validate(p: &[u8; 32]) -> bool {
        SyscallCurve25519EdwardsDecompressValidate::fn_impl(p).map_or_else(|_| false, |_| true)
    }

    fn curve25519_edwards_add(p: &mut [u8; 32], q: &[u8; 32]) -> bool {
        SyscallCurve25519EdwardsAdd::fn_impl(p, q).is_ok()
    }

    fn curve25519_edwards_sub(p: &mut [u8; 32], q: &[u8; 32]) -> bool {
        SyscallCurve25519EdwardsSub::fn_impl(p, q).is_ok()
    }

    fn curve25519_edwards_mul(p: &mut [u8; 32], q: &[u8; 32]) -> bool {
        SyscallCurve25519EdwardsMul::fn_impl(p, q).is_ok()
    }

    fn curve25519_edwards_multiscalar_mul(
        pairs: &[([u8; 32], [u8; 32])],
        out: &mut [u8; 32],
    ) -> bool {
        let result = SyscallCurve25519EdwardsMultiscalarMul::fn_impl(pairs);
        match result {
            Ok(v) => {
                *out = v.compress().to_bytes();
            }
            Err(_) => return false,
        }
        true
    }

    fn big_mod_exp(base: &[u8], exponent: &[u8], modulus: &mut [u8]) -> Result<(), ExitCode> {
        SyscallMathBigModExp::fn_impl(base, exponent, modulus)
    }

    fn debug_log(message: &str) {
        SyscallDebugLog::fn_impl(message.as_bytes())
    }

    fn read(&self, target: &mut [u8], offset: u32) {
        let result =
            SyscallRead::fn_impl(&mut self.ctx.borrow_mut(), offset, target.len() as u32).unwrap();
        target.copy_from_slice(&result);
    }

    fn input_size(&self) -> u32 {
        SyscallInputSize::fn_impl(&self.ctx.borrow())
    }

    fn write(&self, value: &[u8]) {
        SyscallWrite::fn_impl(&mut self.ctx.borrow_mut(), value)
    }

    fn forward_output(&self, offset: u32, len: u32) {
        SyscallForwardOutput::fn_impl(&mut self.ctx.borrow_mut(), offset, len).unwrap_exit_code()
    }

    fn exit(&self, exit_code: ExitCode) -> ! {
        SyscallExit::fn_impl(&mut self.ctx.borrow_mut(), exit_code).unwrap_exit_code();
        unreachable!("exit code: {}", exit_code)
    }

    fn output_size(&self) -> u32 {
        SyscallOutputSize::fn_impl(&self.ctx.borrow())
    }

    fn read_output(&self, target: &mut [u8], offset: u32) {
        let result =
            SyscallReadOutput::fn_impl(&mut self.ctx.borrow_mut(), offset, target.len() as u32)
                .unwrap();
        target.copy_from_slice(&result);
    }

    fn state(&self) -> u32 {
        SyscallState::fn_impl(&self.ctx.borrow())
    }

    #[inline(always)]
    fn fuel(&self) -> u64 {
        SyscallFuel::fn_impl(&self.ctx.borrow())
    }

    fn charge_fuel_manually(&self, fuel_consumed: u64, fuel_refunded: i64) -> u64 {
        SyscallChargeFuelManually::fn_impl(&mut self.ctx.borrow_mut(), fuel_consumed, fuel_refunded)
            .unwrap()
    }

    fn charge_fuel(&self, fuel_consumed: u64) {
        SyscallChargeFuel::fn_impl(&mut self.ctx.borrow_mut(), fuel_consumed).unwrap();
    }

    fn exec(
        &self,
        code_hash: BytecodeOrHash,
        input: &[u8],
        fuel_limit: Option<u64>,
        state: u32,
    ) -> (u64, i64, i32) {
        let (fuel_consumed, fuel_refunded, exit_code) = SyscallExec::fn_impl(
            &mut self.ctx.borrow_mut(),
            code_hash,
            BytesOrRef::Ref(input),
            fuel_limit.unwrap_or(u64::MAX),
            state,
        );
        (fuel_consumed, fuel_refunded, exit_code)
    }

    fn resume(
        &self,
        call_id: u32,
        return_data: &[u8],
        exit_code: i32,
        fuel_consumed: u64,
        fuel_refunded: i64,
    ) -> (u64, i64, i32) {
        let (fuel_consumed, fuel_refunded, exit_code) = SyscallResume::fn_impl(
            &mut self.ctx.borrow_mut(),
            call_id,
            return_data,
            exit_code,
            fuel_consumed,
            fuel_refunded,
            0,
        );
        (fuel_consumed, fuel_refunded, exit_code)
    }

    fn preimage_size(&self, hash: &B256) -> u32 {
        SyscallPreimageSize::fn_impl(&self.ctx.borrow(), hash.as_slice()).unwrap()
    }

    fn preimage_copy(&self, hash: &B256, target: &mut [u8]) {
        let preimage = SyscallPreimageCopy::fn_impl(&self.ctx.borrow(), hash.as_slice()).unwrap();
        target.copy_from_slice(&preimage);
    }

    fn return_data(&self) -> Bytes {
        let ctx = self.ctx.borrow();
        ctx.execution_result.return_data.clone().into()
    }
}
