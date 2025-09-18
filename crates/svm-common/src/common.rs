use core::array::TryFromSliceError;
use fluentbase_types::{Address, U256};
use solana_pubkey::{Pubkey, PUBKEY_BYTES, SVM_ADDRESS_PREFIX};

pub fn pubkey_from_u256(value: &U256) -> Pubkey {
    Pubkey::new_from_array(value.to_le_bytes())
}

pub fn pubkey_to_u256(value: &Pubkey) -> U256 {
    U256::from_le_bytes(value.to_bytes())
}

#[inline(always)]
pub fn is_evm_pubkey(pk: &Pubkey) -> bool {
    pk.as_ref().starts_with(&SVM_ADDRESS_PREFIX)
}

pub fn pubkey_from_evm_address<const SVM_PREFIX: bool>(value: &Address) -> Pubkey {
    let mut pk = [0u8; PUBKEY_BYTES];
    if SVM_PREFIX {
        pk[0..SVM_ADDRESS_PREFIX.len()].copy_from_slice(&SVM_ADDRESS_PREFIX);
    }
    pk[SVM_ADDRESS_PREFIX.len()..].copy_from_slice(value.as_slice());
    Pubkey::new_from_array(pk)
}

pub fn evm_address_from_pubkey<const VALIDATE_PREFIX: bool>(pk: &Pubkey) -> Result<Address, ()> {
    if VALIDATE_PREFIX && !is_evm_pubkey(pk) {
        return Err(());
    }
    Ok(Address::from_slice(
        &pk.as_ref()[SVM_ADDRESS_PREFIX.len()..],
    ))
}

pub const SIZE_OF_U64: usize = size_of::<u64>();
pub const ONE_GWEI: u64 = 1_000_000_000;
pub fn lamports_from_evm_balance(value: U256) -> u64 {
    let value = value / U256::from(ONE_GWEI);
    let bytes: [u8; SIZE_OF_U64] = value.to_be_bytes::<{ U256::BYTES }>().as_ref()
        [U256::BYTES - SIZE_OF_U64..U256::BYTES]
        .try_into()
        .unwrap();
    u64::from_be_bytes(bytes)
}

pub fn evm_balance_from_lamports(value: u64) -> U256 {
    let mut bytes = [0u8; U256::BYTES];
    bytes[U256::BYTES - SIZE_OF_U64..U256::BYTES].copy_from_slice(&value.to_be_bytes());
    U256::from_be_bytes(bytes) * U256::from(ONE_GWEI)
}

pub fn lamports_try_from_slice(input: &[u8]) -> Result<u64, TryFromSliceError> {
    let amount_bytes: [u8; size_of::<u64>()] = input[..size_of::<u64>()].try_into()?;
    Ok(u64::from_be_bytes(amount_bytes))
}

pub fn lamports_to_bytes(lamports: u64) -> [u8; size_of::<u64>()] {
    lamports.to_be_bytes()
}

pub fn pubkey_try_from_slice(input: &[u8]) -> Result<Pubkey, TryFromSliceError> {
    let bytes: [u8; PUBKEY_BYTES] = input[..PUBKEY_BYTES].try_into()?;
    Ok(Pubkey::new_from_array(bytes))
}
