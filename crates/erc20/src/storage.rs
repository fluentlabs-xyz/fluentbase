use crate::{
    common::{address_from_u256, fixed_bytes_from_u256, u256_from_address, u256_from_fixed_bytes},
    consts::{ERR_INDEX_OUT_OF_BOUNDS, ERR_INSUFFICIENT_BALANCE, ERR_UNINIT},
    helpers::{deserialize, serialize},
};
use alloc::vec::Vec;
use bincode::{Decode, Encode};
use core::ops::Range;
use fluentbase_sdk::{derive::solidity_storage, Address, SharedAPI, B256, ERC20_MAGIC_BYTES, U256};

pub const ADDRESS_LEN_BYTES: usize = Address::len_bytes();
pub const U256_LEN_BYTES: usize = size_of::<U256>();
pub const U256_LEN_BITS: usize = U256_LEN_BYTES * u8::BITS as usize;
pub const SIG_LEN_BYTES: usize = size_of::<u32>();
pub const DECIMALS_DEFAULT: u8 = 18;

#[derive(Debug, PartialEq, Encode, Decode)]
pub enum Feature {
    Meta {
        name: Vec<u8>,
        symbol: Vec<u8>,
    },
    InitialSupply {
        amount: [u8; U256_LEN_BYTES],
        owner: [u8; ADDRESS_LEN_BYTES],
        decimals: u8,
    },
    Mintable {
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
    pub fn new() -> Self {
        Self {
            features: Vec::default(),
        }
    }
    pub fn try_decode_from_slice(
        value: &[u8],
    ) -> Result<(Self, usize), bincode::error::DecodeError> {
        deserialize(value)
    }
    pub fn try_encode(&self) -> Result<Vec<u8>, bincode::error::EncodeError> {
        serialize(self)
    }
    pub fn try_encode_for_deploy(&self) -> Result<Vec<u8>, bincode::error::EncodeError> {
        let mut init_bytecode: Vec<u8> = ERC20_MAGIC_BYTES.to_vec();
        init_bytecode.extend(serialize(self)?);
        Ok(init_bytecode)
    }
    pub fn add_feature(&mut self, feature: Feature) {
        self.features.push(feature);
    }
    pub fn is_valid(&self) -> bool {
        let mut has_initial_token_supply = false;
        for f in &self.features {
            match f {
                Feature::InitialSupply { .. } => {
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
        let v = u256_from_address(sdk, &value);
        Self::set(sdk, Self::MINTER_SLOT, v);
    }
    pub fn minter_get(sdk: &impl SharedAPI) -> Address {
        address_from_u256(&Self::get(sdk, Self::MINTER_SLOT))
    }
    pub fn pauser_set(sdk: &mut impl SharedAPI, value: &Address) {
        let v = u256_from_address(sdk, value);
        Self::set(sdk, Self::PAUSER_SLOT, v);
    }
    pub fn pauser_get(sdk: &impl SharedAPI) -> Address {
        address_from_u256(&Self::get(sdk, Self::PAUSER_SLOT))
    }
    #[inline(always)]
    fn short_str_to_u256_repr(sdk: &mut impl SharedAPI, short_str: &[u8]) -> Result<U256, ()> {
        let len = short_str.len();
        if len < Self::SHORT_STR_LEN_MIN || len > Self::SHORT_STR_LEN_MAX {
            return Err(());
        }
        let mut byte_repr = [0u8; U256_LEN_BYTES];
        byte_repr[0] = len as u8;
        byte_repr[1..len + 1].copy_from_slice(short_str);
        let u256_repr = u256_from_fixed_bytes(sdk, &byte_repr);
        Ok(u256_repr)
    }
    #[inline(always)]
    fn short_str_from_u256_repr<'a>(repr: &U256) -> Vec<u8> {
        let repr = fixed_bytes_from_u256(&repr);
        let len = repr[0] as usize;
        if len == 0 {
            return Default::default();
        }
        repr[Self::SHORT_STR_LEN_LEN_BYTES..Self::SHORT_STR_LEN_LEN_BYTES + len]
            .try_into()
            .unwrap()
    }
    #[inline(always)]
    fn short_str_set(sdk: &mut impl SharedAPI, slot: B256, short_str: &[u8]) -> bool {
        if let Ok(u256_repr) = Self::short_str_to_u256_repr(sdk, &short_str) {
            Self::set(sdk, slot, u256_repr);
        } else {
            return false;
        };
        true
    }
    #[inline(always)]
    fn short_str<'a>(sdk: &impl SharedAPI, slot: B256) -> Vec<u8> {
        let repr = Self::get(sdk, slot);
        Self::short_str_from_u256_repr(&repr)
    }
    pub fn symbol_set(sdk: &mut impl SharedAPI, symbol: &[u8]) -> bool {
        Self::short_str_set(sdk, Self::SYMBOL_SLOT, symbol)
    }
    pub fn symbol<'a>(sdk: &impl SharedAPI) -> Vec<u8> {
        Self::short_str(sdk, Self::SYMBOL_SLOT)
    }
    pub fn name_set(sdk: &mut impl SharedAPI, symbol: &[u8]) -> bool {
        Self::short_str_set(sdk, Self::NAME_SLOT, symbol)
    }
    pub fn name<'a>(sdk: &impl SharedAPI) -> Vec<u8> {
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

    pub fn get_or_init_flags(&mut self, sdk: &mut impl SharedAPI) -> &U256 {
        if None == self.flags {
            self.flags = Some(Settings::flags_get(sdk));
        }
        if let Some(v) = self.flags.as_ref() {
            return v;
        }
        sdk.evm_exit(ERR_UNINIT);
    }

    fn get_or_init_flags_mut(&mut self, sdk: &mut impl SharedAPI) -> &mut U256 {
        if None == self.flags {
            self.flags = Some(Settings::flags_get(sdk));
        }
        if let Some(v) = self.flags.as_mut() {
            return v;
        }
        sdk.evm_exit(ERR_UNINIT);
    }

    pub fn set_flag(&mut self, sdk: &mut impl SharedAPI, idx: usize, value: bool) -> &U256 {
        if idx >= U256_LEN_BITS {
            sdk.evm_exit(ERR_INDEX_OUT_OF_BOUNDS);
        }
        let flags = self.get_or_init_flags_mut(sdk);
        flags.set_bit(idx, value);
        flags
    }

    pub fn save_flags(&self, sdk: &mut impl SharedAPI) -> bool {
        if let Some(flags) = self.flags {
            Settings::flags_set(sdk, flags);
            return true;
        }
        false
    }

    fn flag_value(&mut self, sdk: &mut impl SharedAPI, idx: usize) -> bool {
        if idx >= U256_LEN_BITS {
            sdk.evm_exit(ERR_INDEX_OUT_OF_BOUNDS);
        }
        let flags = self.get_or_init_flags(sdk);
        flags.bit(idx)
    }

    #[allow(unused)]
    fn bytes_range(&mut self, sdk: &mut impl SharedAPI, r: Range<usize>) -> &[u8] {
        let flags = self.get_or_init_flags(sdk);
        &flags.as_le_slice()[r]
    }

    #[allow(unused)]
    fn bytes_range_mut(&mut self, sdk: &mut impl SharedAPI, r: Range<usize>) -> &mut [u8] {
        let flags = self.get_or_init_flags_mut(sdk);
        unsafe { &mut flags.as_le_slice_mut()[r] }
    }

    const MINTABLE_PLUGIN_FLAG_IDX: usize = 0;
    const PAUSABLE_PLUGIN_FLAG_IDX: usize = 1;
    const PAUSED_FLAG_IDX: usize = 2;

    #[inline(always)]
    pub fn enable_mintable_plugin(&mut self, sdk: &mut impl SharedAPI) -> &U256 {
        self.set_flag(sdk, Self::MINTABLE_PLUGIN_FLAG_IDX, true)
    }

    #[inline(always)]
    pub fn mintable_plugin_enabled(&mut self, sdk: &mut impl SharedAPI) -> bool {
        self.flag_value(sdk, Self::MINTABLE_PLUGIN_FLAG_IDX)
    }

    #[inline(always)]
    pub fn enable_pausable_plugin(&mut self, sdk: &mut impl SharedAPI) -> &U256 {
        self.set_flag(sdk, Self::PAUSABLE_PLUGIN_FLAG_IDX, true)
    }

    #[inline(always)]
    pub fn pausable_plugin_enabled(&mut self, sdk: &mut impl SharedAPI) -> bool {
        self.flag_value(sdk, Self::PAUSABLE_PLUGIN_FLAG_IDX)
    }

    #[inline(always)]
    pub fn paused(&mut self, sdk: &mut impl SharedAPI) -> bool {
        self.flag_value(sdk, Self::PAUSED_FLAG_IDX)
    }

    #[inline(always)]
    pub fn pause(&mut self, sdk: &mut impl SharedAPI) -> &U256 {
        self.set_flag(sdk, Self::PAUSED_FLAG_IDX, true)
    }

    #[inline(always)]
    pub fn unpause(&mut self, sdk: &mut impl SharedAPI) -> &U256 {
        self.set_flag(sdk, Self::PAUSED_FLAG_IDX, false)
    }
}

impl Balance {
    #[inline(always)]
    pub fn get_for(sdk: &impl SharedAPI, address: Address) -> U256 {
        Balance::get(sdk, address)
    }
    pub fn add(sdk: &mut impl SharedAPI, address: Address, amount: U256) {
        let current_balance = Balance::get(sdk, address);
        let new_balance = current_balance + amount;
        Balance::set(sdk, address, new_balance);
    }

    pub fn subtract(sdk: &mut impl SharedAPI, address: Address, amount: U256) -> bool {
        let current_balance = Balance::get(sdk, address);
        if current_balance < amount {
            return false;
        }
        let new_balance = current_balance - amount;
        Balance::set(sdk, address, new_balance);
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
    use crate::{
        common::{fixed_bytes_from_u256, u256_from_address_try},
        storage::{
            address_from_u256,
            deserialize,
            serialize,
            u256_from_address,
            Feature,
            InitialSettings,
            ADDRESS_LEN_BYTES,
        },
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
                Feature::InitialSupply {
                    amount: fixed_bytes_from_u256(&U256::from(2)),
                    owner: address!("0003000200500000400000040000002000800020").into(),
                    decimals: 12,
                },
                Feature::Mintable {
                    minter: address!("0303000200500020400000040000002000809020").into(),
                },
            ],
        };
        let addr = address!("0003000200500000400000040000002000800020");
        let addr_bytes: [u8; ADDRESS_LEN_BYTES] = addr.into();
        let addr_restored: Address = addr_bytes.into();
        assert_eq!(addr, addr_restored);
        let settings_vec = serialize(&settings).unwrap();
        let (settings_restored, _) = deserialize(&settings_vec).unwrap();
        assert_eq!(settings, settings_restored);
    }

    #[test]
    fn address_to_u256_and_back() {
        let address = address!("0003000200500000400000040000002000800020");
        let u256 = u256_from_address_try(&address).unwrap();
        let address_recovered = address_from_u256(&u256);
        assert_eq!(address_recovered, address);
    }
}
