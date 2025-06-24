use crate::{B256, ERR_INSUFFICIENT_BALANCE};
use alloc::vec::Vec;
use bincode::{
    config::{Configuration, Fixint, LittleEndian},
    Decode,
    Encode,
};
use core::ops::Range;
use fluentbase_sdk::{derive::solidity_storage, Address, SharedAPI, U256};
// use serde::{Deserialize, Serialize};

pub static BINCODE_CONFIG_DEFAULT: Configuration<LittleEndian, Fixint> = bincode::config::legacy();

// pub fn serialize_into<T: serde::Serialize>(
//     entity: &T,
//     dst: &mut [u8],
// ) -> Result<usize, bincode::error::EncodeError> {
//     let entity = Compat(entity);
//     bincode::encode_into_slice(entity, dst, BINCODE_CONFIG_DEFAULT.clone())
// }
//
// pub fn serialize_config<T: serde::Serialize, C: bincode::config::Config>(
//     entity: &T,
//     config: C,
// ) -> Result<Vec<u8>, bincode::error::EncodeError> {
//     let entity = Compat(entity);
//     Ok(bincode::encode_to_vec(entity, config)?)
// }
//
// pub fn serialize<T: serde::Serialize>(entity: &T) -> Result<Vec<u8>, bincode::error::EncodeError> {
//     serialize_config(entity, BINCODE_CONFIG_DEFAULT.clone())
// }

// pub fn deserialize<T: serde::de::DeserializeOwned>(
//     src: &[u8],
// ) -> Result<T, bincode::error::DecodeError> {
//     Ok(deserialize_config(src, BINCODE_CONFIG_DEFAULT.clone())?)
// }

// pub fn deserialize_config<T: serde::de::DeserializeOwned, C: bincode::config::Config>(
//     src: &[u8],
//     config: C,
// ) -> Result<T, bincode::error::DecodeError> {
//     let entity: Compat<T> = bincode::decode_from_slice(src, config)?.0;
//     Ok(entity.0)
// }

pub fn serialize_original<T: bincode::enc::Encode>(
    entity: &T,
) -> Result<Vec<u8>, bincode::error::EncodeError> {
    bincode::encode_to_vec(entity, BINCODE_CONFIG_DEFAULT.clone())
}

pub fn deserialize_original<T: bincode::de::Decode<()>>(
    src: &[u8],
) -> Result<(T, usize), bincode::error::DecodeError> {
    bincode::decode_from_slice(src, BINCODE_CONFIG_DEFAULT.clone())
}

pub const ADDRESS_LEN_BYTES: usize = Address::len_bytes();
pub const U256_LEN_BYTES: usize = size_of::<U256>();
pub const U256_LEN_BITS: usize = U256_LEN_BYTES * u8::BITS as usize;
pub const SIG_LEN_BYTES: usize = size_of::<u32>();

#[inline(always)]
pub fn u256_from_fixed_bytes(value: [u8; U256_LEN_BYTES]) -> U256 {
    U256::from_be_bytes(value)
}
#[inline(always)]
pub fn fixed_bytes_from_u256(value: &U256) -> [u8; U256_LEN_BYTES] {
    value.to_be_bytes::<U256_LEN_BYTES>()
}

#[inline(always)]
pub fn address_from_u256(value: &U256) -> Address {
    Address::from_slice(&fixed_bytes_from_u256(value)[U256_LEN_BYTES - ADDRESS_LEN_BYTES..])
}
pub fn u256_from_address(value: &Address) -> U256 {
    U256::from_be_slice(value.as_slice())
}

#[derive(Debug, PartialEq, Encode, Decode)]
pub enum Feature {
    Meta {
        name: Vec<u8>,
        symbol: Vec<u8>,
    },
    InitialTokenSupply {
        amount: [u8; U256_LEN_BYTES],
        owner: [u8; ADDRESS_LEN_BYTES],
        decimals: u8,
    },
    MintableFunctionality {
        minter: [u8; ADDRESS_LEN_BYTES],
    },
    Pausable {
        pauser: [u8; ADDRESS_LEN_BYTES],
    },
}

#[derive(Debug, PartialEq, Encode, Decode)]
pub struct InitialSettings {
    features: Vec<Feature>,
}

impl InitialSettings {
    pub fn try_from_slice(value: &[u8]) -> Result<(Self, usize), bincode::error::DecodeError> {
        deserialize_original(value)
    }
    pub fn is_valid(&self) -> bool {
        let mut has_initial_token_supply = false;
        for f in &self.features {
            match f {
                Feature::InitialTokenSupply { .. } => {
                    has_initial_token_supply = true;
                }
                _ => {}
            }
        }

        has_initial_token_supply
    }
    pub fn features(&self) -> &Vec<Feature> {
        &self.features
    }
}

#[inline(always)]
fn reconstruct_slice<'a>(ptr: usize, len: usize) -> &'a [u8] {
    unsafe { core::slice::from_raw_parts(ptr as *const u8, len) }
}

solidity_storage! {
    mapping(B256 => U256) Settings;
    mapping(Address => U256) Balance;
    mapping(Address => mapping(Address => U256)) Allowance;
}

impl Settings {
    const TOTAL_SUPPLY_SLOT: B256 = B256::with_last_byte(1);
    const MINTER_SLOT: B256 = B256::with_last_byte(2);
    const PAUSER_SLOT: B256 = B256::with_last_byte(3);
    const SYMBOL_SLOT: B256 = B256::with_last_byte(4);
    const NAME_SLOT: B256 = B256::with_last_byte(5);
    const DECIMALS_SLOT: B256 = B256::with_last_byte(6);
    const FLAGS_SLOT: B256 = B256::with_last_byte(7);
    pub const SHORT_STR_LEN_MIN: usize = 1;
    pub const SHORT_STR_LEN_LEN_BYTES: usize = 1;
    pub const SHORT_STR_BYTE_REPR_LEN_MIN: usize =
        Self::SHORT_STR_LEN_MIN + Self::SHORT_STR_LEN_LEN_BYTES;
    const SHORT_STR_LEN_MAX: usize = 31;
    const DECIMALS_MAX: usize = 36; // max val: 2**256=115792089237316195423570985008687907853269984665640564039457584007913129639936
    pub fn total_supply_set(sdk: &mut impl SharedAPI, value: U256) {
        Self::set(sdk, Self::TOTAL_SUPPLY_SLOT, value);
    }
    pub fn total_supply_get(sdk: &impl SharedAPI) -> U256 {
        Self::get(sdk, Self::TOTAL_SUPPLY_SLOT)
    }
    pub fn minter_set(sdk: &mut impl SharedAPI, value: &Address) {
        Self::set(sdk, Self::MINTER_SLOT, u256_from_address(&value));
    }
    pub fn minter_get(sdk: &impl SharedAPI) -> Address {
        address_from_u256(&Self::get(sdk, Self::MINTER_SLOT))
    }
    pub fn pauser_set(sdk: &mut impl SharedAPI, value: &Address) {
        Self::set(sdk, Self::PAUSER_SLOT, u256_from_address(value));
    }
    pub fn pauser_get(sdk: &impl SharedAPI) -> Address {
        address_from_u256(&Self::get(sdk, Self::PAUSER_SLOT))
    }
    #[inline(always)]
    fn short_str_to_u256_repr(short_str: &[u8]) -> Result<U256, ()> {
        let len = short_str.len();
        if len < Self::SHORT_STR_LEN_MIN || len > Self::SHORT_STR_LEN_MAX {
            return Err(());
        }
        let mut byte_repr = [0u8; U256_LEN_BYTES];
        byte_repr[0] = len as u8;
        byte_repr[1..len + 1].copy_from_slice(short_str);
        let u256_repr = U256::from_be_bytes::<U256_LEN_BYTES>(byte_repr);
        Ok(u256_repr)
    }
    #[inline(always)]
    fn short_str_from_u256_repr<'a>(repr: U256) -> &'a [u8] {
        let repr = repr.to_be_bytes::<U256_LEN_BYTES>();
        let len = repr[0] as usize;
        if len == 0 {
            return Default::default();
        }
        reconstruct_slice(repr.as_ptr() as usize + Self::SHORT_STR_LEN_LEN_BYTES, len)
    }
    #[inline(always)]
    pub fn short_str_try_from_slice_repr(repr: &[u8]) -> Result<&[u8], ()> {
        if repr.len() < Self::SHORT_STR_BYTE_REPR_LEN_MIN {
            return Err(());
        }
        let len = repr[0] as usize;
        if len > repr.len() - Self::SHORT_STR_LEN_LEN_BYTES || len > Self::SHORT_STR_LEN_MAX {
            return Err(());
        }
        Ok(reconstruct_slice(
            repr.as_ptr() as usize + Self::SHORT_STR_LEN_LEN_BYTES,
            len,
        ))
    }
    #[inline(always)]
    fn short_str_set(sdk: &mut impl SharedAPI, slot: B256, short_str: &[u8]) -> bool {
        if let Ok(u256_repr) = Self::short_str_to_u256_repr(&short_str) {
            Self::set(sdk, slot, u256_repr);
        } else {
            return false;
        };
        true
    }
    #[inline(always)]
    fn short_str<'a>(sdk: &impl SharedAPI, slot: B256) -> &'a [u8] {
        Self::short_str_from_u256_repr(Self::get(sdk, slot))
    }
    pub fn symbol_set(sdk: &mut impl SharedAPI, symbol: &[u8]) -> bool {
        Self::short_str_set(sdk, Self::SYMBOL_SLOT, symbol)
    }
    pub fn symbol<'a>(sdk: &impl SharedAPI) -> &'a [u8] {
        Self::short_str(sdk, Self::SYMBOL_SLOT)
    }
    pub fn name_set(sdk: &mut impl SharedAPI, symbol: &[u8]) -> bool {
        Self::short_str_set(sdk, Self::NAME_SLOT, symbol)
    }
    pub fn name<'a>(sdk: &impl SharedAPI) -> &'a [u8] {
        Self::short_str(sdk, Self::NAME_SLOT)
    }
    pub fn decimals_set(sdk: &mut impl SharedAPI, decimals: U256) -> bool {
        if decimals > U256::from(Self::DECIMALS_MAX) {
            return false;
        }
        Self::set(sdk, Self::DECIMALS_SLOT, decimals);
        true
    }
    pub fn decimals_get(sdk: &impl SharedAPI) -> U256 {
        Self::get(sdk, Self::DECIMALS_SLOT)
    }
    pub fn flags_get(sdk: &impl SharedAPI) -> U256 {
        Self::get(sdk, Self::FLAGS_SLOT)
    }
    pub fn flags_set(sdk: &mut impl SharedAPI, flags: U256) {
        Self::set(sdk, Self::FLAGS_SLOT, flags)
    }
}

pub struct Config {
    flags: Option<U256>,
}

impl Config {
    pub fn new() -> Self {
        Self { flags: None }
    }

    pub fn get_or_init_flags(&mut self, sdk: &impl SharedAPI) -> &U256 {
        if None == self.flags {
            self.flags = Some(Settings::flags_get(sdk));
        }
        self.flags.as_ref().expect("flags must be initialized")
    }

    fn get_or_init_flags_mut(&mut self, sdk: &impl SharedAPI) -> &mut U256 {
        if None == self.flags {
            self.flags = Some(Settings::flags_get(sdk));
        }
        self.flags.as_mut().expect("flags must be initialized")
    }

    pub fn set_flag(&mut self, sdk: &impl SharedAPI, idx: usize, value: bool) -> &U256 {
        assert!(idx < U256_LEN_BITS, "bit index out of bounds");
        let flags = self.get_or_init_flags_mut(sdk);
        flags.set_bit(idx, value);
        flags
    }

    pub fn save_flags(&self, sdk: &mut impl SharedAPI) {
        let flags = self.flags.expect("flags must be initialized before save");
        Settings::flags_set(sdk, flags)
    }

    fn flag_value(&mut self, sdk: &impl SharedAPI, idx: usize) -> bool {
        assert!(idx < U256_LEN_BITS, "bit index out of bounds");
        let flags = self.get_or_init_flags(sdk);
        flags.bit(idx)
    }

    fn bytes_range(&mut self, sdk: &impl SharedAPI, range: Range<usize>) -> &[u8] {
        let flags = self.get_or_init_flags(sdk);
        &flags.as_le_slice()[range]
    }

    fn bytes_range_mut(&mut self, sdk: &impl SharedAPI, range: Range<usize>) -> &mut [u8] {
        let flags = self.get_or_init_flags_mut(sdk);
        unsafe { &mut flags.as_le_slice_mut()[range] }
    }

    const MINTABLE_PLUGIN_FLAG_IDX: usize = 0;
    const PAUSABLE_PLUGIN_FLAG_IDX: usize = 1;
    const PAUSED_FLAG_IDX: usize = 2;

    #[inline(always)]
    pub fn enable_mintable_plugin(&mut self, sdk: &impl SharedAPI) -> &U256 {
        self.set_flag(sdk, Self::MINTABLE_PLUGIN_FLAG_IDX, true)
    }

    #[inline(always)]
    pub fn mintable_plugin_enabled(&mut self, sdk: &impl SharedAPI) -> bool {
        self.flag_value(sdk, Self::MINTABLE_PLUGIN_FLAG_IDX)
    }

    #[inline(always)]
    pub fn enable_pausable_plugin(&mut self, sdk: &impl SharedAPI) -> &U256 {
        self.set_flag(sdk, Self::PAUSABLE_PLUGIN_FLAG_IDX, true)
    }

    #[inline(always)]
    pub fn pausable_plugin_enabled(&mut self, sdk: &impl SharedAPI) -> bool {
        self.flag_value(sdk, Self::PAUSABLE_PLUGIN_FLAG_IDX)
    }

    #[inline(always)]
    pub fn paused(&mut self, sdk: &impl SharedAPI) -> bool {
        self.flag_value(sdk, Self::PAUSED_FLAG_IDX)
    }

    #[inline(always)]
    pub fn pause(&mut self, sdk: &impl SharedAPI) -> &U256 {
        self.set_flag(sdk, Self::PAUSED_FLAG_IDX, true)
    }

    #[inline(always)]
    pub fn unpause(&mut self, sdk: &impl SharedAPI) -> &U256 {
        self.set_flag(sdk, Self::PAUSED_FLAG_IDX, false)
    }
}

impl Balance {
    #[inline(always)]
    pub fn get_for(sdk: &impl SharedAPI, address: Address) -> U256 {
        Self::get(sdk, address)
    }
    pub fn add(sdk: &mut impl SharedAPI, address: Address, amount: U256) {
        let current_balance = Self::get(sdk, address);
        let new_balance = current_balance + amount;
        Self::set(sdk, address, new_balance);
    }

    pub fn subtract(sdk: &mut impl SharedAPI, address: Address, amount: U256) -> bool {
        let current_balance = Self::get(sdk, address);
        if current_balance < amount {
            return false;
        }
        let new_balance = current_balance - amount;
        Self::set(sdk, address, new_balance);
        true
    }

    pub fn send(sdk: &mut impl SharedAPI, from: Address, to: Address, amount: U256) {
        if !Balance::subtract(sdk, from, amount) {
            sdk.evm_exit(ERR_INSUFFICIENT_BALANCE);
        }
        Balance::add(sdk, to, amount);
    }
}

impl Allowance {
    #[inline(always)]
    pub fn update(sdk: &mut impl SharedAPI, owner: Address, spender: Address, amount: U256) {
        Self::set(sdk, owner, spender, amount);
    }
    #[inline(always)]
    pub fn get_current(sdk: &mut impl SharedAPI, owner: Address, spender: Address) -> U256 {
        Self::get(sdk, owner, spender)
    }
    pub fn subtract(
        sdk: &mut impl SharedAPI,
        owner: Address,
        spender: Address,
        amount: U256,
    ) -> bool {
        let current_allowance = Self::get(sdk, owner, spender);
        if current_allowance < amount {
            return false;
        }
        let new_allowance = current_allowance - amount;
        Self::set(sdk, owner, spender, new_allowance);
        true
    }
}

#[cfg(test)]
mod tests {
    use crate::storage::{
        address_from_u256,
        deserialize_original,
        fixed_bytes_from_u256,
        serialize_original,
        u256_from_address,
        Feature,
        InitialSettings,
        ADDRESS_LEN_BYTES,
    };
    use fluentbase_sdk::{address, Address, U256};

    #[test]
    fn operations_over_u256() {
        let mut value = U256::from(0);
        value.set_bit(0, true);
        value.set_bit(1, true);
        value.set_bit(8, true);
        assert_eq!(&value.as_le_slice()[0..3], &[3, 1, 0]);
        unsafe {
            value.as_le_slice_mut()[1] = 22;
        }
        assert_eq!(&value.as_le_slice()[0..3], &[3, 22, 0]);
    }

    #[test]
    fn ser_deser() {
        let settings = InitialSettings {
            features: vec![
                Feature::InitialTokenSupply {
                    amount: fixed_bytes_from_u256(&U256::from(2)),
                    owner: address!("0003000200500000400000040000002000800020").into(),
                    decimals: 12,
                },
                Feature::MintableFunctionality {
                    minter: address!("0303000200500020400000040000002000809020").into(),
                },
            ],
        };
        let addr = address!("0003000200500000400000040000002000800020");
        let addr_bytes: [u8; ADDRESS_LEN_BYTES] = addr.into();
        let addr_restored: Address = addr_bytes.into();
        assert_eq!(addr, addr_restored);
        // let settings_vec = serialize(&settings).unwrap();
        // let settings_restored: InitialSettings = deserialize(&settings_vec).unwrap();
        let settings_vec = serialize_original(&settings).unwrap();
        let (settings_restored, _) = deserialize_original(&settings_vec).unwrap();
        assert_eq!(settings, settings_restored);
    }

    #[test]
    fn address_u256() {
        let address = address!("0003000200500000400000040000002000800020");
        let u256 = u256_from_address(&address);
        let address_recovered = address_from_u256(&u256);
        assert_eq!(address_recovered, address);
    }
}
