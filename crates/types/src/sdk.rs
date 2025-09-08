use crate::{
    evm::{write_evm_exit_message, write_evm_panic_message},
    syscall::SyscallResult,
    Address, Bytes, ContextReader, ExitCode, B256, BN254_G1_POINT_COMPRESSED_SIZE,
    BN254_G1_POINT_DECOMPRESSED_SIZE, BN254_G2_POINT_COMPRESSED_SIZE,
    BN254_G2_POINT_DECOMPRESSED_SIZE, FUEL_DENOM_RATE, U256,
};

pub type IsAccountOwnable = bool;
pub type IsColdAccess = bool;
pub type IsAccountEmpty = bool;

pub trait StorageAPI {
    fn write_storage(&mut self, slot: U256, value: U256) -> SyscallResult<()>;
    fn storage(&self, slot: &U256) -> SyscallResult<U256>;
}

pub trait MetadataAPI {
    fn metadata_write(
        &mut self,
        address: &Address,
        offset: u32,
        metadata: Bytes,
    ) -> SyscallResult<()>;
    fn metadata_size(
        &self,
        address: &Address,
    ) -> SyscallResult<(u32, IsAccountOwnable, IsColdAccess, IsAccountEmpty)>;
    fn metadata_create(&mut self, salt: &U256, metadata: Bytes) -> SyscallResult<()>;
    fn metadata_copy(&self, address: &Address, offset: u32, length: u32) -> SyscallResult<Bytes>;
}

pub trait MetadataStorageAPI {
    fn metadata_storage_read(&self, slot: &U256) -> SyscallResult<U256>;
    fn metadata_storage_write(&mut self, slot: &U256, value: U256) -> SyscallResult<()>;
}

pub trait SharedAPI: StorageAPI + MetadataAPI + MetadataStorageAPI {
    fn context(&self) -> impl ContextReader;
    fn keccak256(&self, data: &[u8]) -> B256;
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
    fn bls12_381_g1_msm(pairs: &[([u8; 64], [u8; 64])], out: &mut [u8; 64]);
    fn bls12_381_g2_add(p: &mut [u8; 192], q: &[u8; 192]) -> [u8; 192];
    fn bls12_381_g2_msm(pairs: &[([u8; 192], [u8; 32])], out: &mut [u8; 192]);
    fn bls12_381_pairing(pairs: &[([u8; 48], [u8; 96])], out: &mut [u8; 288]);
    fn bls12_381_map_fp_to_g1(p: &[u8; 64], out: &mut [u8; 64]);
    fn bls12_381_map_fp2_to_g2(p: &[u8; 64], out: &mut [u8; 64]);
    fn bn254_add(p: &mut [u8; 64], q: &[u8; 64]);
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
    fn bn254_double(p: &mut [u8; 64]);
    fn bn254_fp_mul(p: &mut [u8; 64], q: &[u8; 32]);
    fn bn254_fp2_mul(p: &mut [u8; 64], q: &[u8; 32]);

    fn big_mod_exp(base: &[u8], exponent: &[u8], modulus: &mut [u8]) -> Result<(), ExitCode>;

    fn read(&self, target: &mut [u8], offset: u32);
    fn input_size(&self) -> u32;

    fn input<'a>(&self) -> &'a [u8] {
        let input_size = self.input_size();
        let pointer = unsafe {
            alloc::alloc::alloc(core::alloc::Layout::from_size_align_unchecked(
                input_size as usize,
                8,
            ))
        };
        let mut buffer =
            unsafe { &mut *core::ptr::slice_from_raw_parts_mut(pointer, input_size as usize) };
        self.read(&mut buffer, 0);
        buffer
    }

    fn read_context(&self, target: &mut [u8], offset: u32);

    fn charge_fuel_manually(&self, fuel_consumed: u64, fuel_refunded: i64);

    fn sync_evm_gas(&self, gas_consumed: u64, gas_refunded: i64) {
        // TODO(dmitry123): "do we care about overflow here?"
        self.charge_fuel_manually(
            gas_consumed * FUEL_DENOM_RATE,
            gas_refunded * FUEL_DENOM_RATE as i64,
        );
    }

    fn fuel(&self) -> u64;

    fn write(&mut self, output: &[u8]);

    fn evm_exit(&mut self, exit_code: u32) -> ! {
        // write an EVM-compatible exit message (only if exit code is not zero)
        if exit_code != 0 {
            write_evm_exit_message(exit_code, |slice| {
                self.write(slice);
            });
            self.native_exit(ExitCode::Panic);
        } else {
            self.native_exit(ExitCode::Ok)
        }
    }

    fn native_exit(&self, exit_code: ExitCode) -> !;

    fn exit(&self) -> ! {
        self.native_exit(ExitCode::Ok)
    }

    fn panic(&self) -> ! {
        self.native_exit(ExitCode::Panic)
    }

    fn evm_panic(&mut self, panic_message: &str) -> ! {
        // write an EVM-compatible panic message
        write_evm_panic_message(panic_message, |slice| {
            self.write(slice);
        });
        // exit with panic exit code
        self.native_exit(ExitCode::Panic)
    }
    fn write_transient_storage(&mut self, slot: U256, value: U256) -> SyscallResult<()>;
    fn transient_storage(&self, slot: &U256) -> SyscallResult<U256>;

    fn emit_log(&mut self, topics: &[B256], data: &[u8]) -> SyscallResult<()>;

    fn self_balance(&self) -> SyscallResult<U256>;
    fn balance(&self, address: &Address) -> SyscallResult<U256>;

    fn block_hash(&self, block_number: u64) -> SyscallResult<B256>;
    fn code_size(&self, address: &Address) -> SyscallResult<u32>;
    fn code_hash(&self, address: &Address) -> SyscallResult<B256>;
    fn code_copy(
        &self,
        address: &Address,
        code_offset: u64,
        code_length: u64,
    ) -> SyscallResult<Bytes>;
    fn create(
        &mut self,
        salt: Option<U256>,
        value: &U256,
        init_code: &[u8],
    ) -> SyscallResult<Bytes>;
    fn call(
        &mut self,
        address: Address,
        value: U256,
        input: &[u8],
        fuel_limit: Option<u64>,
    ) -> SyscallResult<Bytes>;
    fn call_code(
        &mut self,
        address: Address,
        value: U256,
        input: &[u8],
        fuel_limit: Option<u64>,
    ) -> SyscallResult<Bytes>;
    fn delegate_call(
        &mut self,
        address: Address,
        input: &[u8],
        fuel_limit: Option<u64>,
    ) -> SyscallResult<Bytes>;
    fn static_call(
        &mut self,
        address: Address,
        input: &[u8],
        fuel_limit: Option<u64>,
    ) -> SyscallResult<Bytes>;
    fn destroy_account(&mut self, address: Address) -> SyscallResult<()>;
}
