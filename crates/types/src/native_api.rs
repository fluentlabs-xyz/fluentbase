use crate::{BytecodeOrHash, ExitCode};
use alloc::vec;
use alloy_primitives::{Bytes, B256};

/// A trait for providing shared API functionality.
pub trait NativeAPI {
    fn keccak256(data: &[u8]) -> B256;
    fn sha256(data: &[u8]) -> B256;
    fn secp256k1_recover(digest: &B256, sig: &[u8; 64], rec_id: u8) -> Option<[u8; 65]>;
    fn bn254_add(p: &mut [u8; 64], q: &[u8; 64]);
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
