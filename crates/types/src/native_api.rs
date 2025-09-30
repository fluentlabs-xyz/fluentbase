use crate::{
    BytecodeOrHash, ExitCode, BN254_G1_POINT_COMPRESSED_SIZE, BN254_G1_POINT_DECOMPRESSED_SIZE,
    BN254_G2_POINT_COMPRESSED_SIZE, BN254_G2_POINT_DECOMPRESSED_SIZE, ED25519_COMPRESSED_SIZE,
    ED25519_DECOMPRESSED_SIZE,
};
use alloc::vec;
use alloy_primitives::{Bytes, B256};
use core::cell::RefCell;

/// A trait for providing shared API functionality.
pub trait NativeAPI {
    fn keccak256(data: &[u8]) -> B256;
    fn sha256(data: &[u8]) -> B256;
    fn blake3(data: &[u8]) -> B256;
    fn poseidon(parameters: u32, endianness: u32, data: &[u8]) -> Result<B256, ExitCode>;
    fn secp256k1_recover(digest: &B256, sig: &[u8; 64], rec_id: u8) -> Option<[u8; 65]>;
    fn curve256r1_verify(input: &[u8]) -> bool;

    fn ed25519_decompress(
        y: [u8; ED25519_COMPRESSED_SIZE],
        sign: u32,
    ) -> [u8; ED25519_DECOMPRESSED_SIZE];
    fn ed25519_add(
        p: [u8; ED25519_DECOMPRESSED_SIZE],
        q: [u8; ED25519_DECOMPRESSED_SIZE],
    ) -> [u8; ED25519_DECOMPRESSED_SIZE];

    fn bls12_381_g1_add(p: &mut [u8; 96], q: &[u8; 96]);
    fn bls12_381_g1_msm(pairs: &[([u8; 96], [u8; 32])], out: &mut [u8; 96]);
    fn bls12_381_g2_add(p: &mut [u8; 192], q: &[u8; 192]);
    fn bls12_381_g2_msm(pairs: &[([u8; 192], [u8; 32])], out: &mut [u8; 192]);
    fn bls12_381_pairing(pairs: &[([u8; 48], [u8; 96])], out: &mut [u8; 288]);
    fn bls12_381_map_fp_to_g1(p: &[u8; 64], out: &mut [u8; 96]);
    fn bls12_381_map_fp2_to_g2(p: &[u8; 128], out: &mut [u8; 192]);
    fn bn254_add(p: &mut [u8; 64], q: &[u8; 64]) -> Result<[u8; 64], ExitCode>;
    fn bn254_double(p: &mut [u8; 64]);
    fn bn254_mul(p: &mut [u8; 64], q: &[u8; 32]) -> Result<[u8; 64], ExitCode>;
    fn bn254_multi_pairing(elements: &[([u8; 64], [u8; 128])]) -> Result<[u8; 32], ExitCode>;
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
    fn exec(
        &self,
        code_hash: BytecodeOrHash,
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

pub trait InterruptAPI {
    fn interrupt(
        &self,
        code_hash: BytecodeOrHash,
        input: &[u8],
        fuel_limit: Option<u64>,
        state: u32,
    ) -> (u64, i64, i32);
}

impl<T: NativeAPI + ?Sized> InterruptAPI for T {
    // #[inline(always)]
    fn interrupt(
        &self,
        code_hash: BytecodeOrHash,
        input: &[u8],
        fuel_limit: Option<u64>,
        state: u32,
    ) -> (u64, i64, i32) {
        NativeAPI::exec(self, code_hash, input, fuel_limit, state)
    }
}

pub struct ExtractedInterruptionContext {
    pub code_hash: B256,
    pub input: Bytes,
    pub fuel_limit: Option<u64>,
    pub state: u32,
}

#[derive(Default)]
pub struct InterruptionExtractingAdapter {
    interruption: RefCell<Option<ExtractedInterruptionContext>>,
}

impl InterruptionExtractingAdapter {
    pub fn extract(self) -> ExtractedInterruptionContext {
        self.interruption.into_inner().unwrap()
    }
}

impl InterruptAPI for InterruptionExtractingAdapter {
    fn interrupt(
        &self,
        code_hash: BytecodeOrHash,
        input: &[u8],
        fuel_limit: Option<u64>,
        state: u32,
    ) -> (u64, i64, i32) {
        let context = ExtractedInterruptionContext {
            code_hash: code_hash.code_hash(),
            input: Bytes::copy_from_slice(input),
            fuel_limit,
            state,
        };
        _ = self.interruption.borrow_mut().insert(context);
        (0, 0, 0)
    }
}
