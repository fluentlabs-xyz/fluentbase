use crate::{
    syscall_handler::{
        ecc_add, syscall_ed25519_decompress_impl, syscall_edwards_add_impl,
        syscall_hashing_keccak256_permute_impl, syscall_hashing_sha256_compress_impl,
        syscall_hashing_sha256_extend_impl, Bls12381G1AddConfig, Bls12381G1MapConfig,
        Bls12381G1MulConfig, Bls12381G2AddConfig, Bls12381G2MapConfig, Bls12381G2MulConfig,
        Bn254G1AddConfig, Bn254G1MulConfig, Bn254G2DecompressConfig, SyscallEccCompressDecompress,
        SyscallEccDouble, SyscallEccMapping, SyscallEccMsm, SyscallEccMul, SyscallEccPairing,
    },
    RuntimeContextWrapper,
};
use fluentbase_types::{
    CryptoAPI, ExitCode, UnwrapExitCode, BN254_G1_POINT_COMPRESSED_SIZE,
    BN254_G1_POINT_DECOMPRESSED_SIZE, BN254_G2_POINT_COMPRESSED_SIZE,
    BN254_G2_POINT_DECOMPRESSED_SIZE, EDWARDS_COMPRESSED_SIZE, EDWARDS_DECOMPRESSED_SIZE,
    G1_COMPRESSED_SIZE, G1_UNCOMPRESSED_SIZE, G2_COMPRESSED_SIZE, G2_UNCOMPRESSED_SIZE,
    GT_COMPRESSED_SIZE, PADDED_FP2_SIZE, PADDED_FP_SIZE, SCALAR_SIZE, TOWER_FP_BLS12381_SIZE,
    TOWER_FP_BN256_SIZE,
};
use sp1_curves::weierstrass::{bls12_381::Bls12381, bn254::Bn254};

impl CryptoAPI for RuntimeContextWrapper {
    fn keccak256_permute(state: &mut [u64; 25]) {
        syscall_hashing_keccak256_permute_impl(state);
    }

    fn sha256_extend(w: &mut [u32; 64]) {
        syscall_hashing_sha256_extend_impl(w);
    }

    fn sha256_compress(state: &mut [u32; 8], w: &[u32; 64]) {
        syscall_hashing_sha256_compress_impl(state, w);
    }

    fn ed25519_decompress(
        y: [u8; EDWARDS_COMPRESSED_SIZE],
        sign: u32,
    ) -> [u8; EDWARDS_DECOMPRESSED_SIZE] {
        syscall_ed25519_decompress_impl(y, sign).unwrap_exit_code()
    }

    fn ed25519_add(
        p: [u8; EDWARDS_DECOMPRESSED_SIZE],
        q: [u8; EDWARDS_DECOMPRESSED_SIZE],
    ) -> [u8; EDWARDS_DECOMPRESSED_SIZE] {
        syscall_edwards_add_impl(p, q).unwrap()
    }

    fn tower_fp1_bn254_add(
        _x: [u8; TOWER_FP_BN256_SIZE],
        _y: [u8; TOWER_FP_BN256_SIZE],
    ) -> [u8; TOWER_FP_BN256_SIZE] {
        todo!()
    }

    fn tower_fp1_bn254_sub(
        _x: [u8; TOWER_FP_BN256_SIZE],
        _y: [u8; TOWER_FP_BN256_SIZE],
    ) -> [u8; TOWER_FP_BN256_SIZE] {
        todo!()
    }

    fn tower_fp1_bn254_mul(
        _x: [u8; TOWER_FP_BN256_SIZE],
        _y: [u8; TOWER_FP_BN256_SIZE],
    ) -> [u8; TOWER_FP_BN256_SIZE] {
        todo!()
    }

    fn tower_fp1_bls12381_add(
        _x: [u8; TOWER_FP_BLS12381_SIZE],
        _y: [u8; TOWER_FP_BLS12381_SIZE],
    ) -> [u8; TOWER_FP_BLS12381_SIZE] {
        todo!()
    }

    fn tower_fp1_bls12381_sub(
        _x: [u8; TOWER_FP_BLS12381_SIZE],
        _y: [u8; TOWER_FP_BLS12381_SIZE],
    ) -> [u8; TOWER_FP_BLS12381_SIZE] {
        todo!()
    }

    fn tower_fp1_bls12381_mul(
        _x: [u8; TOWER_FP_BLS12381_SIZE],
        _y: [u8; TOWER_FP_BLS12381_SIZE],
    ) -> [u8; TOWER_FP_BLS12381_SIZE] {
        todo!()
    }

    fn tower_fp2_bn254_add(
        _x: [u8; TOWER_FP_BN256_SIZE],
        _y: [u8; TOWER_FP_BN256_SIZE],
    ) -> [u8; TOWER_FP_BN256_SIZE] {
        todo!()
    }

    fn tower_fp2_bn254_sub(
        _x: [u8; TOWER_FP_BN256_SIZE],
        _y: [u8; TOWER_FP_BN256_SIZE],
    ) -> [u8; TOWER_FP_BN256_SIZE] {
        todo!()
    }

    fn tower_fp2_bn254_mul(
        _x: [u8; TOWER_FP_BN256_SIZE],
        _y: [u8; TOWER_FP_BN256_SIZE],
    ) -> [u8; TOWER_FP_BN256_SIZE] {
        todo!()
    }
    fn tower_fp2_bls12381_add(
        _x: [u8; TOWER_FP_BLS12381_SIZE],
        _y: [u8; TOWER_FP_BLS12381_SIZE],
    ) -> [u8; TOWER_FP_BLS12381_SIZE] {
        todo!()
    }

    fn tower_fp2_bls12381_sub(
        _x: [u8; TOWER_FP_BLS12381_SIZE],
        _y: [u8; TOWER_FP_BLS12381_SIZE],
    ) -> [u8; TOWER_FP_BLS12381_SIZE] {
        todo!()
    }

    fn tower_fp2_bls12381_mul(
        _x: [u8; TOWER_FP_BLS12381_SIZE],
        _y: [u8; TOWER_FP_BLS12381_SIZE],
    ) -> [u8; TOWER_FP_BLS12381_SIZE] {
        todo!()
    }

    fn bls12_381_g1_add(p: &mut [u8; G1_UNCOMPRESSED_SIZE], q: &[u8; G1_UNCOMPRESSED_SIZE]) {
        if let Ok(result) = ecc_add::ecc_add_impl::<Bls12381G1AddConfig>(p, q) {
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
        let result = SyscallEccMsm::<Bls12381G1MulConfig>::fn_impl(&pairs_vec);

        // Copy result to output
        if !result.is_empty() {
            out.copy_from_slice(&result);
        } else {
            out.fill(0);
        }
    }

    fn bls12_381_g2_add(p: &mut [u8; G2_UNCOMPRESSED_SIZE], q: &[u8; G2_UNCOMPRESSED_SIZE]) {
        if let Ok(result) = ecc_add::ecc_add_impl::<Bls12381G2AddConfig>(p, q) {
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
        let result = SyscallEccMsm::<Bls12381G2MulConfig>::fn_impl(&pairs_vec);
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
        let result = SyscallEccPairing::<Bls12381>::fn_impl_bls12_381(&pairs);
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

    fn bls12_381_map_g1(p: &[u8; PADDED_FP_SIZE], out: &mut [u8; G1_UNCOMPRESSED_SIZE]) {
        let result = SyscallEccMapping::<Bls12381G1MapConfig>::fn_impl(p.as_slice());
        out.copy_from_slice(&result[..G1_UNCOMPRESSED_SIZE]);
    }

    fn bls12_381_map_g2(p: &[u8; PADDED_FP2_SIZE], out: &mut [u8; G2_UNCOMPRESSED_SIZE]) {
        let result = SyscallEccMapping::<Bls12381G2MapConfig>::fn_impl(p.as_slice());
        out.copy_from_slice(&result[..G2_UNCOMPRESSED_SIZE]);
    }

    fn bn254_add(
        p: &mut [u8; BN254_G1_POINT_DECOMPRESSED_SIZE],
        q: &[u8; BN254_G1_POINT_DECOMPRESSED_SIZE],
    ) -> [u8; BN254_G1_POINT_DECOMPRESSED_SIZE] {
        ecc_add::ecc_add_impl::<Bn254G1AddConfig>(p, q)
            .unwrap_exit_code()
            .try_into()
            .unwrap()
    }

    fn bn254_double(p: &mut [u8; BN254_G1_POINT_DECOMPRESSED_SIZE]) {
        let result = SyscallEccDouble::<Bn254>::fn_impl(p);
        let min = core::cmp::min(p.len(), result.len());
        p[..min].copy_from_slice(&result[..min]);
    }

    fn bn254_mul(
        p: &mut [u8; BN254_G1_POINT_DECOMPRESSED_SIZE],
        q: &[u8; SCALAR_SIZE],
    ) -> Result<[u8; BN254_G1_POINT_DECOMPRESSED_SIZE], ExitCode> {
        let result = SyscallEccMul::<Bn254G1MulConfig>::fn_impl(p, q)
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
        let result = SyscallEccPairing::<Bn254>::fn_impl_bn254(&pairs)
            .map_err(|_| ExitCode::PrecompileError)?;
        let result_array: [u8; SCALAR_SIZE] =
            result.try_into().map_err(|_| ExitCode::PrecompileError)?;
        Ok(result_array)
    }

    fn bn254_g1_compress(
        point: &[u8; BN254_G1_POINT_DECOMPRESSED_SIZE],
    ) -> Result<[u8; BN254_G1_POINT_COMPRESSED_SIZE], ExitCode> {
        let result =
            SyscallEccCompressDecompress::<crate::syscall_handler::Bn254G1CompressConfig>::fn_impl(
                point,
            )?;
        result.try_into().map_err(|_| ExitCode::UnknownError)
    }

    fn bn254_g1_decompress(
        point: &[u8; BN254_G1_POINT_COMPRESSED_SIZE],
    ) -> Result<[u8; BN254_G1_POINT_DECOMPRESSED_SIZE], ExitCode> {
        let result = SyscallEccCompressDecompress::<
            crate::syscall_handler::Bn254G1DecompressConfig,
        >::fn_impl(point)?;
        result.try_into().map_err(|_| ExitCode::UnknownError)
    }

    fn bn254_g2_compress(
        point: &[u8; BN254_G2_POINT_DECOMPRESSED_SIZE],
    ) -> Result<[u8; BN254_G2_POINT_COMPRESSED_SIZE], ExitCode> {
        let result =
            SyscallEccCompressDecompress::<crate::syscall_handler::Bn254G2CompressConfig>::fn_impl(
                point,
            )?;
        result.try_into().map_err(|_| ExitCode::UnknownError)
    }

    fn bn254_g2_decompress(
        point: &[u8; BN254_G2_POINT_COMPRESSED_SIZE],
    ) -> Result<[u8; BN254_G2_POINT_DECOMPRESSED_SIZE], ExitCode> {
        let result = SyscallEccCompressDecompress::<Bn254G2DecompressConfig>::fn_impl(point)?;
        result.try_into().map_err(|_| ExitCode::UnknownError)
    }
}
