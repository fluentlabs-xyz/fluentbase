use crate::{
    BytecodeOrHash, ExitCode, BN254_G1_POINT_COMPRESSED_SIZE, BN254_G1_POINT_DECOMPRESSED_SIZE,
    BN254_G2_POINT_COMPRESSED_SIZE, BN254_G2_POINT_DECOMPRESSED_SIZE,
};
use alloc::vec;
use alloy_primitives::{Bytes, B256};

/// A trait for providing shared API functionality.
pub trait NativeAPI {
    fn keccak256(data: &[u8]) -> B256;
    fn sha256(data: &[u8]) -> B256;
    fn blake3(data: &[u8]) -> B256;
    fn poseidon(parameters: u32, endianness: u32, data: &[u8]) -> Result<B256, ExitCode>;
    fn secp256k1_recover(digest: &B256, sig: &[u8; 64], rec_id: u8) -> Option<[u8; 65]>;
    fn curve25519_edwards_decompress_validate(p: &[u8; 32]) -> bool;
    fn curve25519_edwards_add(p: &mut [u8; 32], q: &[u8; 32]) -> bool;
    fn curve25519_edwards_sub(p: &mut [u8; 32], q: &[u8; 32]) -> bool;
    fn curve25519_edwards_mul(p: &mut [u8; 32], q: &[u8; 32]) -> bool;
    fn curve25519_edwards_multiscalar_mul(
        pairs: &[([u8; 32], [u8; 32])],
        out: &mut [u8; 32],
    ) -> bool;
    fn curve25519_ristretto_decompress_validate(p: &[u8; 32]) -> bool;
    fn curve25519_ristretto_add(p: &mut [u8; 32], q: &[u8; 32]) -> bool;
    fn curve25519_ristretto_sub(p: &mut [u8; 32], q: &[u8; 32]) -> bool;
    fn curve25519_ristretto_mul(p: &mut [u8; 32], q: &[u8; 32]) -> bool;
    fn curve25519_ristretto_multiscalar_mul(
        pairs: &[([u8; 32], [u8; 32])],
        out: &mut [u8; 32],
    ) -> bool;
    fn bls12_381_g1_add(p: &mut [u8; 96], q: &[u8; 96]);
    fn bls12_381_g1_msm(pairs: &[([u8; 96], [u8; 32])], out: &mut [u8; 96]);
    fn bls12_381_g2_add(p: &mut [u8; 192], q: &[u8; 192]) -> [u8; 192];
    fn bls12_381_g2_msm(pairs: &[([u8; 192], [u8; 32])], out: &mut [u8; 192]);
    fn bls12_381_pairing(pairs: &[([u8; 48], [u8; 96])], out: &mut [u8; 288]);
    fn bls12_381_map_fp_to_g1(p: &[u8; 64], out: &mut [u8; 64]);
    fn bls12_381_map_fp2_to_g2(p: &[u8; 64], out: &mut [u8; 64]);
    fn bn254_add(p: &mut [u8; 64], q: &[u8; 64]);
    fn bn254_double(p: &mut [u8; 64]);
    fn bn254_mul(p: &mut [u8; 64], q: &[u8; 32]);
    fn bn254_multi_pairing(elements: &[([u8; 64], [u8; 128])]) -> [u8; 32];
    fn bn254_g1_compress(
        point: &[u8; BN254_G1_POINT_DECOMPRESSED_SIZE],
    ) -> Result<[u8; BN254_G1_POINT_COMPRESSED_SIZE], ExitCode>;
    fn bn254_g1_decompress(
        point: &[u8; BN254_G1_POINT_COMPRESSED_SIZE],
    ) -> Result<[u8; BN254_G1_POINT_DECOMPRESSED_SIZE], ExitCode>;
    fn bn254_g2_compress(
        point: &[u8; BN254_G2_POINT_DECOMPRESSED_SIZE],
    ) -> Result<[u8; BN254_G2_POINT_COMPRESSED_SIZE], ExitCode>;
    fn bn254_g2_decompress(
        point: &[u8; BN254_G2_POINT_COMPRESSED_SIZE],
    ) -> Result<[u8; BN254_G2_POINT_DECOMPRESSED_SIZE], ExitCode>;
    fn bn254_fp_mul(p: &mut [u8; 64], q: &[u8; 32]);
    fn bn254_fp2_mul(p: &mut [u8; 64], q: &[u8; 32]);

    fn big_mod_exp(base: &[u8], exponent: &[u8], modulus: &mut [u8]) -> Result<(), ExitCode>;

    fn debug_log(message: &str);

    fn read(&self, target: &mut [u8], offset: u32);
    fn input_size(&self) -> u32;
    fn write(&self, value: &[u8]);
    fn forward_output(&self, offset: u32, len: u32);
    fn exit(&self, exit_code: ExitCode) -> !;
    fn output_size(&self) -> u32;
    fn read_output(&self, target: &mut [u8], offset: u32);
    fn state(&self) -> u32;
    fn fuel(&self) -> u64;
    fn charge_fuel_manually(&self, fuel_consumed: u64, fuel_refunded: i64) -> u64;
    fn charge_fuel(&self, fuel_consumed: u64);
    fn exec<I: Into<BytecodeOrHash>>(
        &self,
        code_hash: I,
        input: &[u8],
        fuel_limit: Option<u64>,
        state: u32,
    ) -> (u64, i64, i32);
    fn resume(
        &self,
        call_id: u32,
        return_data: &[u8],
        exit_code: i32,
        fuel_consumed: u64,
        fuel_refunded: i64,
    ) -> (u64, i64, i32);

    #[deprecated(note = "don't use")]
    fn preimage_size(&self, hash: &B256) -> u32;
    #[deprecated(note = "don't use")]
    fn preimage_copy(&self, hash: &B256, target: &mut [u8]);

    fn input(&self) -> Bytes {
        let input_size = self.input_size();
        let mut buffer = vec![0u8; input_size as usize];
        self.read(&mut buffer, 0);
        buffer.into()
    }

    fn return_data(&self) -> Bytes {
        let output_size = self.output_size();
        let mut buffer = vec![0u8; output_size as usize];
        self.read_output(&mut buffer, 0);
        buffer.into()
    }
}

#[macro_export]
macro_rules! bn254_add_common_impl {
    ($p: ident, $q: ident, $action_p_eq_q: block, $action_rest: block) => {
        if *$p == [0u8; 64] {
            if *$q != [0u8; 64] {
                *$p = *$q;
            }
            return;
        } else if *$q == [0u8; 64] {
            return;
        } else if *$p == *$q {
            $action_p_eq_q
        } else {
            $action_rest
        }
    };
}
