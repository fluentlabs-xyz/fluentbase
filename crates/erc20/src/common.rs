use crate::{
    consts::ERR_CONVERSION,
    storage::{ADDRESS_LEN_BYTES, U256_LEN_BYTES},
};
use fluentbase_sdk::{Address, SharedAPI, U256};

const ENDIANNESS_BE: bool = true;

#[inline(always)]
pub fn u256_from_bytes_slice(sdk: &mut impl SharedAPI, value: &[u8]) -> U256 {
    if let Some(v) = u256_from_bytes_slice_try(value) {
        return v;
    }
    sdk.evm_exit(ERR_CONVERSION);
}

#[inline(always)]
pub fn u256_from_bytes_slice_try(value: &[u8]) -> Option<U256> {
    if ENDIANNESS_BE {
        U256::try_from_be_slice(value)
    } else {
        U256::try_from_le_slice(value)
    }
}

#[inline(always)]
pub fn u256_from_fixed_bytes(sdk: &mut impl SharedAPI, value: &[u8; U256_LEN_BYTES]) -> U256 {
    u256_from_bytes_slice(sdk, value)
}
#[inline(always)]
pub fn fixed_bytes_from_u256(value: &U256) -> [u8; U256_LEN_BYTES] {
    if ENDIANNESS_BE {
        value.to_be_bytes::<U256_LEN_BYTES>()
    } else {
        value.to_le_bytes::<U256_LEN_BYTES>()
    }
}

#[inline(always)]
pub fn u256_from_address(sdk: &mut impl SharedAPI, value: &Address) -> U256 {
    if let Some(v) = u256_from_address_try(value) {
        return v;
    }
    sdk.evm_exit(ERR_CONVERSION);
}

#[inline(always)]
pub fn u256_from_address_try(value: &Address) -> Option<U256> {
    U256::try_from_le_slice(value.as_slice())
}
#[inline(always)]
pub fn address_from_u256(value: &U256) -> Address {
    Address::from_slice(&value.to_le_bytes::<U256_LEN_BYTES>()[..ADDRESS_LEN_BYTES])
}
#[inline(always)]
pub fn sig_to_bytes(value: u32) -> [u8; size_of::<u32>()] {
    if ENDIANNESS_BE {
        value.to_be_bytes()
    } else {
        value.to_le_bytes()
    }
}
#[inline(always)]
pub fn fixed_bytes_to_sig(value: [u8; size_of::<u32>()]) -> u32 {
    if ENDIANNESS_BE {
        u32::from_be_bytes(value)
    } else {
        u32::from_le_bytes(value)
    }
}
#[inline(always)]
pub fn bytes_to_sig(value: &[u8]) -> u32 {
    let value: [u8; size_of::<u32>()] = value.try_into().unwrap();
    fixed_bytes_to_sig(value)
}

#[cfg(test)]
mod tests {
    use crate::common::{address_from_u256, u256_from_address_try};
    use fluentbase_sdk::address;

    #[test]
    fn address_to_u256_and_back() {
        let address = address!("0003000200500000400000040000002000800020");
        let u256 = u256_from_address_try(&address).unwrap();
        let address_recovered = address_from_u256(&u256);
        assert_eq!(address_recovered, address);
    }
}
