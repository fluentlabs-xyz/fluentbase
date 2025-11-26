use crate::storage::{ADDRESS_LEN_BYTES, U256_LEN_BYTES};
use core::mem::transmute;
use fluentbase_sdk::{Address, SharedAPI, B256, U256};

#[inline(always)]
pub fn u256_from_slice_try(value: &[u8]) -> Option<U256> {
    U256::try_from_be_slice(value)
}

#[inline(always)]
pub fn u256_from_fixed_bytes(value: &[u8; U256_LEN_BYTES]) -> U256 {
    u256_from_slice_try(value).unwrap()
}

#[inline(always)]
pub fn u256_ref_from_fixed_bytes(value: &[u8; U256_LEN_BYTES]) -> &U256 {
    unsafe { transmute(value) }
}
#[inline(always)]
pub fn fixed_bytes_from_u256(value: &U256) -> [u8; U256_LEN_BYTES] {
    value.to_be_bytes::<U256_LEN_BYTES>()
}

#[inline(always)]
pub fn u256_from_address(value: &Address) -> U256 {
    U256::from_le_slice(value.as_slice())
}
#[inline(always)]
pub fn address_from_u256(value: &U256) -> Address {
    Address::from_slice(&value.to_le_bytes::<U256_LEN_BYTES>()[..ADDRESS_LEN_BYTES])
}

#[inline(always)]
pub fn b256_from_address_try(value: &Address) -> B256 {
    B256::right_padding_from(value.as_slice())
}
#[inline(always)]
pub fn address_from_b256(value: &B256) -> Address {
    Address::from_slice(&value.as_slice()[..ADDRESS_LEN_BYTES])
}
#[inline(always)]
pub fn sig_to_bytes(value: u32) -> [u8; size_of::<u32>()] {
    value.to_be_bytes()
}
#[inline(always)]
pub fn fixed_bytes_to_sig(value: [u8; size_of::<u32>()]) -> u32 {
    u32::from_be_bytes(value)
}
#[inline(always)]
pub fn bytes_to_sig(value: &[u8]) -> u32 {
    let value: [u8; size_of::<u32>()] = value.try_into().unwrap();
    fixed_bytes_to_sig(value)
}

#[cfg(test)]
mod tests {
    use crate::common::{address_from_u256, u256_from_address};
    use fluentbase_sdk::address;

    #[test]
    fn address_to_from_u256() {
        let address = address!("0003000200500000400000040000002000800020");
        let u256 = u256_from_address(&address);
        let address_recovered = address_from_u256(&u256);
        assert_eq!(address_recovered, address);
    }
}
