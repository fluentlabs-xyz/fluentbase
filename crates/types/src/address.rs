use crate::{native_api::NativeAPI, Address, B256, U256};

#[inline(always)]
pub fn calc_create_address<API: NativeAPI>(deployer: &Address, nonce: u64) -> Address {
    use alloy_rlp::{Encodable, EMPTY_LIST_CODE, EMPTY_STRING_CODE};
    const MAX_LEN: usize = 1 + (1 + 20) + 9;
    let len = 22 + nonce.length();
    debug_assert!(len <= MAX_LEN);
    let mut out = [0u8; MAX_LEN];
    out[0] = EMPTY_LIST_CODE + len as u8 - 1;
    out[1] = EMPTY_STRING_CODE + 20;
    out[2..22].copy_from_slice(deployer.as_slice());
    Encodable::encode(&nonce, &mut &mut out[22..]);
    let out = &out[..len];
    Address::from_word(API::keccak256(&out))
}

#[inline(always)]
pub fn calc_create2_address<API: NativeAPI>(
    deployer: &Address,
    salt: &U256,
    init_code_hash: &B256,
) -> Address {
    let mut bytes = [0; 85];
    bytes[0] = 0xff;
    bytes[1..21].copy_from_slice(deployer.as_slice());
    bytes[21..53].copy_from_slice(&salt.to_be_bytes::<32>());
    bytes[53..85].copy_from_slice(init_code_hash.as_slice());
    let hash = API::keccak256(&bytes);
    Address::from_word(hash)
}

#[inline(always)]
pub fn calc_create4_address(owner: &Address, salt: &U256, hash_func: fn(&[u8]) -> B256) -> Address {
    let mut bytes = [0; 53];
    bytes[0] = 0x44;
    bytes[1..21].copy_from_slice(owner.as_slice());
    bytes[21..53].copy_from_slice(&salt.to_be_bytes::<32>());
    let hash = hash_func(&bytes);
    Address::from_word(hash)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        BytecodeOrHash, ExitCode, BN254_G1_POINT_COMPRESSED_SIZE, BN254_G1_POINT_DECOMPRESSED_SIZE,
        BN254_G2_POINT_COMPRESSED_SIZE, BN254_G2_POINT_DECOMPRESSED_SIZE, ED25519_COMPRESSED_SIZE,
        ED25519_DECOMPRESSED_SIZE,
    };
    use alloy_primitives::{address, b256, keccak256};

    struct TestContext;

    impl NativeAPI for TestContext {
        fn keccak256(data: &[u8]) -> B256 {
            keccak256(data)
        }

        fn sha256(_data: &[u8]) -> B256 {
            todo!()
        }

        fn curve256r1_verify(_input: &[u8]) -> bool {
            todo!()
        }

        fn blake3(_data: &[u8]) -> B256 {
            todo!()
        }

        fn poseidon(_parameters: u32, _endianness: u32, _data: &[u8]) -> Result<B256, ExitCode> {
            todo!()
        }

        fn secp256k1_recover(_digest: &B256, _sig: &[u8; 64], _rec_id: u8) -> Option<[u8; 65]> {
            todo!()
        }

        fn ed25519_decompress(
            _y: [u8; ED25519_COMPRESSED_SIZE],
            _sign: u32,
        ) -> [u8; ED25519_DECOMPRESSED_SIZE] {
            todo!()
        }

        fn ed25519_add(
            _p: [u8; ED25519_DECOMPRESSED_SIZE],
            _q: [u8; ED25519_DECOMPRESSED_SIZE],
        ) -> [u8; ED25519_DECOMPRESSED_SIZE] {
            todo!()
        }

        fn bls12_381_g1_add(_p: &mut [u8; 96], _q: &[u8; 96]) {
            todo!()
        }

        fn bls12_381_g1_msm(_pairs: &[([u8; 96], [u8; 32])], _out: &mut [u8; 96]) {
            todo!()
        }

        fn bls12_381_g2_add(_p: &mut [u8; 192], _q: &[u8; 192]) {
            todo!()
        }

        fn bls12_381_g2_msm(_pairs: &[([u8; 192], [u8; 32])], _out: &mut [u8; 192]) {
            todo!()
        }

        fn bls12_381_pairing(_pairs: &[([u8; 48], [u8; 96])], _out: &mut [u8; 288]) {
            todo!()
        }

        fn bls12_381_map_fp_to_g1(_p: &[u8; 64], _out: &mut [u8; 96]) {
            todo!()
        }

        fn bls12_381_map_fp2_to_g2(_p: &[u8; 128], _out: &mut [u8; 192]) {
            todo!()
        }

        fn bn254_add(_p: &mut [u8; 64], _q: &[u8; 64]) -> Result<[u8; 64], ExitCode> {
            todo!()
        }

        fn bn254_double(_p: &mut [u8; 64]) {
            todo!()
        }

        fn bn254_mul(_p: &mut [u8; 64], _q: &[u8; 32]) -> Result<[u8; 64], ExitCode> {
            todo!()
        }

        fn bn254_multi_pairing(_elements: &[([u8; 64], [u8; 128])]) -> Result<[u8; 32], ExitCode> {
            todo!()
        }

        fn bn254_g1_compress(
            _point: &[u8; BN254_G1_POINT_DECOMPRESSED_SIZE],
        ) -> Result<[u8; BN254_G1_POINT_COMPRESSED_SIZE], ExitCode> {
            todo!()
        }

        fn bn254_g1_decompress(
            _point: &[u8; BN254_G1_POINT_COMPRESSED_SIZE],
        ) -> Result<[u8; BN254_G1_POINT_DECOMPRESSED_SIZE], ExitCode> {
            todo!()
        }

        fn bn254_g2_compress(
            _point: &[u8; BN254_G2_POINT_DECOMPRESSED_SIZE],
        ) -> Result<[u8; BN254_G2_POINT_COMPRESSED_SIZE], ExitCode> {
            todo!()
        }

        fn bn254_g2_decompress(
            _point: &[u8; BN254_G2_POINT_COMPRESSED_SIZE],
        ) -> Result<[u8; BN254_G2_POINT_DECOMPRESSED_SIZE], ExitCode> {
            todo!()
        }

        fn bn254_fp_mul(_p: &mut [u8; 64], _q: &[u8; 32]) {
            todo!()
        }

        fn bn254_fp2_mul(_p: &mut [u8; 64], _q: &[u8; 32]) {
            todo!()
        }

        fn big_mod_exp(
            _base: &[u8],
            _exponent: &[u8],
            _modulus: &mut [u8],
        ) -> Result<(), ExitCode> {
            todo!()
        }

        fn debug_log(_message: &str) {
            todo!()
        }

        fn read(&self, _target: &mut [u8], _offset: u32) {
            todo!()
        }

        fn input_size(&self) -> u32 {
            todo!()
        }

        fn write(&self, _value: &[u8]) {
            todo!()
        }

        fn forward_output(&self, _offset: u32, _len: u32) {
            todo!()
        }

        fn exit(&self, _exit_code: ExitCode) -> ! {
            todo!()
        }

        fn output_size(&self) -> u32 {
            todo!()
        }

        fn read_output(&self, _target: &mut [u8], _offset: u32) {
            todo!()
        }

        fn state(&self) -> u32 {
            todo!()
        }

        fn fuel(&self) -> u64 {
            todo!()
        }

        fn charge_fuel_manually(&self, _fuel_consumed: u64, _fuel_refunded: i64) -> u64 {
            todo!()
        }

        fn charge_fuel(&self, _fuel_consumed: u64) {
            todo!()
        }

        fn exec(
            &self,
            _code_hash: BytecodeOrHash,
            _input: &[u8],
            _fuel_limit: Option<u64>,
            _state: u32,
        ) -> (u64, i64, i32) {
            todo!()
        }

        fn resume(
            &self,
            _call_id: u32,
            _return_data: &[u8],
            _exit_code: i32,
            _fuel_consumed: u64,
            _fuel_refunded: i64,
        ) -> (u64, i64, i32) {
            todo!()
        }

        fn preimage_size(&self, _hash: &B256) -> u32 {
            todo!()
        }

        fn preimage_copy(&self, _hash: &B256, _target: &mut [u8]) {
            todo!()
        }
    }

    #[test]
    fn test_create_address() {
        for (address, nonce) in [
            (address!("0000000000000000000000000000000000000000"), 0),
            (
                address!("0000000000000000000000000000000000000000"),
                u32::MIN,
            ),
            (
                address!("0000000000000000000000000000000000000000"),
                u32::MAX,
            ),
            (address!("2340820934820934820934809238402983400000"), 0),
            (
                address!("2340820934820934820934809238402983400000"),
                u32::MIN,
            ),
            (
                address!("2340820934820934820934809238402983400000"),
                u32::MAX,
            ),
        ] {
            assert_eq!(
                calc_create_address::<TestContext>(&address, nonce as u64),
                address.create(nonce as u64)
            );
        }
    }

    #[test]
    fn test_create2_address() {
        let address = Address::ZERO;
        for (salt, hash) in [(
            b256!("0000000000000000000000000000000000000000000000000000000000000001"),
            b256!("0000000000000000000000000000000000000000000000000000000000000002"),
        )] {
            assert_eq!(
                calc_create2_address::<TestContext>(&address, &salt.into(), &hash),
                address.create2(salt, hash)
            );
        }
    }
}
