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
    use crate::{BytecodeOrHash, ExitCode};
    use alloy_primitives::{address, b256, keccak256};

    struct TestContext;

    impl NativeAPI for TestContext {
        fn keccak256(data: &[u8]) -> B256 {
            keccak256(data)
        }

        fn sha256(_data: &[u8]) -> B256 {
            todo!()
        }

        fn secp256k1_recover(_digest: &B256, _sig: &[u8; 64], _rec_id: u8) -> Option<[u8; 65]> {
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

        fn exec<I: Into<BytecodeOrHash>>(
            &self,
            _code_hash: I,
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
