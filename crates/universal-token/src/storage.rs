use crate::common::{address_from_u256, fixed_bytes_from_u256};
use crate::common::{b256_from_address_try, u256_from_address, u256_from_slice_try};
use crate::consts::{
    ERR_INDEX_OUT_OF_BOUNDS, ERR_INSUFFICIENT_ALLOWANCE, ERR_INSUFFICIENT_BALANCE,
    ERR_MALFORMED_INPUT, ERR_OVERFLOW,
};
use crate::helpers::bincode::{decode, encode};
use crate::services::global_service::global_service;
use crate::types::derived_key::{IKeyDeriver, KeyDeriver};
use alloc::sync::Arc;
use alloc::vec::Vec;
use bincode::{Decode, Encode};
use fluentbase_sdk::{Address, SharedAPI, U256, UNIVERSAL_TOKEN_MAGIC_BYTES};

pub const ADDRESS_LEN_BYTES: usize = Address::len_bytes();
pub const U256_LEN_BYTES: usize = size_of::<U256>();
pub const U256_LEN_BITS: usize = U256_LEN_BYTES * u8::BITS as usize;
pub const SIG_LEN_BYTES: usize = size_of::<u32>();
pub const DECIMALS_DEFAULT: u8 = 2;

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
            features: Default::default(),
        }
    }
    pub fn try_decode_from_slice(
        value: &[u8],
    ) -> Result<(Self, usize), bincode::error::DecodeError> {
        decode(value)
    }
    pub fn encode_for_deploy(&self) -> Vec<u8> {
        let mut init_bytecode: Vec<u8> = UNIVERSAL_TOKEN_MAGIC_BYTES.to_vec();
        init_bytecode.extend(encode(self).unwrap());
        init_bytecode
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

pub struct Settings {
    kd: Arc<KeyDeriver>,
    total_supply_slot: Option<U256>,
    minter_slot: Option<U256>,
    pauser_slot: Option<U256>,
    symbol_slot: Option<U256>,
    name_slot: Option<U256>,
    decimals_slot: Option<U256>,
    flags_slot: Option<U256>,
}

impl Settings {
    pub const SHORT_STR_LEN_MIN: usize = 1;
    pub const SHORT_STR_LEN_LEN_BYTES: usize = 1;
    pub const SHORT_STR_BYTE_REPR_LEN_MIN: usize =
        Self::SHORT_STR_LEN_MIN + Self::SHORT_STR_LEN_LEN_BYTES;
    const SHORT_STR_LEN_MAX: usize = 31;
    const DECIMALS_MAX: u8 = 36;
    pub fn new(slot: u64) -> Self {
        let kd = Arc::new(KeyDeriver::new_specific_slot(slot));

        Self {
            total_supply_slot: None,
            minter_slot: None,
            pauser_slot: None,
            symbol_slot: None,
            name_slot: None,
            decimals_slot: None,
            flags_slot: None,
            kd,
        }
    }

    pub fn total_supply_slot(&mut self) -> U256 {
        self.total_supply_slot
            .get_or_insert_with(|| self.kd.u256(&U256::from(1)))
            .clone()
    }

    pub fn minter_slot(&mut self) -> U256 {
        self.minter_slot
            .get_or_insert_with(|| self.kd.u256(&U256::from(2)))
            .clone()
    }

    pub fn pauser_slot(&mut self) -> U256 {
        self.pauser_slot
            .get_or_insert_with(|| self.kd.u256(&U256::from(3)))
            .clone()
    }

    pub fn symbol_slot(&mut self) -> U256 {
        self.symbol_slot
            .get_or_insert_with(|| self.kd.u256(&U256::from(4)))
            .clone()
    }

    pub fn name_slot(&mut self) -> U256 {
        self.name_slot
            .get_or_insert_with(|| self.kd.u256(&U256::from(5)))
            .clone()
    }

    pub fn decimals_slot(&mut self) -> U256 {
        self.decimals_slot
            .get_or_insert_with(|| self.kd.u256(&U256::from(6)))
            .clone()
    }

    pub fn flags_slot(&mut self) -> U256 {
        self.flags_slot
            .get_or_insert_with(|| self.kd.u256(&U256::from(7)))
            .clone()
    }

    pub fn total_supply_set(&mut self, value: &U256) {
        let s = self.total_supply_slot();
        global_service().set_value(&s, value);
    }
    pub fn total_supply_get(&mut self) -> Option<U256> {
        let s = self.total_supply_slot();
        global_service().try_get_value(&s).cloned()
    }
    pub fn minter_set(&mut self, value: &Address) {
        let s = self.minter_slot();
        let v = u256_from_address(&value);
        global_service().set_value(&s, &v);
    }
    pub fn minter_get(&mut self) -> Option<Address> {
        let s = self.minter_slot();
        global_service()
            .try_get_value(&s)
            .map(|v| address_from_u256(v))
    }
    pub fn pauser_set(&mut self, value: &Address) {
        let s = self.pauser_slot();
        let v = u256_from_address(value);
        global_service().set_value(&s, &v);
    }
    pub fn pauser_get(&mut self) -> Option<Address> {
        let s = self.pauser_slot();
        global_service()
            .try_get_value(&s)
            .map(|v| address_from_u256(v))
    }
    #[inline(always)]
    fn short_str_to_u256_repr(&self, short_str: &[u8]) -> Result<U256, u32> {
        let len = short_str.len();
        if len < Self::SHORT_STR_LEN_MIN || len > Self::SHORT_STR_LEN_MAX {
            return Err(ERR_MALFORMED_INPUT);
        }
        let mut byte_repr = [0u8; U256_LEN_BYTES];
        byte_repr[0] = len as u8;
        byte_repr[1..1 + len].copy_from_slice(short_str);
        let u256_repr = u256_from_slice_try(&byte_repr).unwrap();
        Ok(u256_repr)
    }
    #[inline(always)]
    fn short_str_from_u256_repr<'a>(&self, repr: &U256) -> Vec<u8> {
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
    fn short_str_set(&self, slot: &U256, short_str: &[u8]) -> bool {
        if let Ok(u256_repr) = self.short_str_to_u256_repr(&short_str) {
            global_service().set_value(&slot, &u256_repr);
        } else {
            return false;
        };
        true
    }
    #[inline(always)]
    fn short_str<'a>(&self, slot: &U256) -> Option<Vec<u8>> {
        let s = global_service();
        let repr = s.try_get_value(slot)?;
        self.short_str_from_u256_repr(repr).into()
    }
    pub fn symbol_set(&mut self, symbol: &[u8]) -> bool {
        let s = self.symbol_slot();
        self.short_str_set(&s, symbol)
    }
    pub fn symbol<'a>(&mut self) -> Option<Vec<u8>> {
        let v = self.symbol_slot();
        self.short_str(&v)
    }
    pub fn name_set(&mut self, symbol: &[u8]) -> bool {
        let s = self.name_slot();
        self.short_str_set(&s, symbol)
    }
    pub fn name<'a>(&mut self) -> Option<Vec<u8>> {
        let s = self.name_slot();
        self.short_str(&s)
    }
    pub fn decimals_set(&mut self, decimals: u8) -> bool {
        if decimals > Self::DECIMALS_MAX {
            return false;
        }
        let s = self.decimals_slot();
        global_service().set_value(&s, &U256::from(decimals));
        true
    }
    pub fn decimals_get(&mut self) -> Option<U256> {
        let s = self.decimals_slot();
        global_service().try_get_value(&s).cloned()
    }
    pub fn flags_get(&mut self) -> Option<U256> {
        let s = self.flags_slot();
        global_service().try_get_value(&s).cloned()
    }
    pub fn flags_set(&mut self, flags: U256) {
        let s = self.flags_slot();
        global_service().set_value(&s, &flags);
    }
}

pub struct Config {
    flags: Option<U256>,
}

impl Config {
    pub fn new() -> Self {
        Self { flags: None }
    }

    pub fn get_or_init_flags(&mut self) -> &Option<U256> {
        if self.flags.is_none() {
            self.flags = Some(settings_service().flags_get().expect("flags value exists"));
        }
        &self.flags
    }

    fn get_or_init_flags_mut(&mut self) -> &mut U256 {
        if self.flags.is_none() {
            self.flags = Some(settings_service().flags_get().expect("flags value exists"));
        }
        self.flags.as_mut().unwrap()
    }

    pub fn set_flag(&mut self, idx: usize, value: bool) -> Result<(), u32> {
        if idx >= U256_LEN_BITS {
            return Err(ERR_INDEX_OUT_OF_BOUNDS);
        }
        let flags: &mut U256 = self.get_or_init_flags_mut();
        flags.set_bit(idx, value);
        Ok(())
    }

    pub fn save_flags(&self) -> bool {
        if let Some(flags) = self.flags {
            settings_service().flags_set(flags);
            return true;
        }
        false
    }

    fn flag_value(&mut self, idx: usize) -> Result<bool, u32> {
        if idx >= U256_LEN_BITS {
            return Err(ERR_INDEX_OUT_OF_BOUNDS);
        }
        let flags = self.get_or_init_flags().expect("flags value exists");
        Ok(flags.bit(idx))
    }

    const MINTABLE_PLUGIN_FLAG_IDX: usize = 0;
    const PAUSABLE_PLUGIN_FLAG_IDX: usize = 1;
    const PAUSED_FLAG_IDX: usize = 2;

    #[inline(always)]
    pub fn enable_mintable_plugin(&mut self) {
        self.set_flag(Self::MINTABLE_PLUGIN_FLAG_IDX, true).unwrap();
    }

    #[inline(always)]
    pub fn mintable_plugin_enabled(&mut self) -> Result<bool, u32> {
        self.flag_value(Self::MINTABLE_PLUGIN_FLAG_IDX)
    }

    #[inline(always)]
    pub fn enable_pausable_plugin(&mut self) {
        self.set_flag(Self::PAUSABLE_PLUGIN_FLAG_IDX, true).unwrap();
    }

    #[inline(always)]
    pub fn pausable_plugin_enabled(&mut self) -> Result<bool, u32> {
        self.flag_value(Self::PAUSABLE_PLUGIN_FLAG_IDX)
    }

    #[inline(always)]
    pub fn paused(&mut self) -> Result<bool, u32> {
        self.flag_value(Self::PAUSED_FLAG_IDX)
    }

    #[inline(always)]
    pub fn pause(&mut self) {
        self.set_flag(Self::PAUSED_FLAG_IDX, true).unwrap();
    }

    #[inline(always)]
    pub fn unpause(&mut self) {
        self.set_flag(Self::PAUSED_FLAG_IDX, false).unwrap();
    }
}

pub struct Balance {
    kd: KeyDeriver,
}

impl Balance {
    pub fn new(slot: u64) -> Self {
        let kd = KeyDeriver::new_specific_slot(slot);
        Self { kd }
    }

    #[inline(always)]
    pub fn set(&self, address: &Address, value: &U256) {
        let key = self.kd.b256(&b256_from_address_try(address));
        global_service().set_value(&key, value);
    }

    #[inline(always)]
    pub fn key(&self, address: &Address) -> U256 {
        self.kd.b256(&b256_from_address_try(address))
    }

    #[inline(always)]
    pub fn get(&self, address: &Address) -> U256 {
        let key = self.key(address);
        global_service()
            .try_get_value(&key)
            .cloned()
            .expect("balance exists")
    }

    pub fn add(&self, address: &Address, amount: &U256) -> Result<(), u32> {
        let current_balance = self.get(address);
        let new_balance = current_balance.overflowing_add(*amount);
        if new_balance.1 {
            return Err(ERR_OVERFLOW);
        }
        self.set(address, &new_balance.0);
        Ok(())
    }

    pub fn subtract(&self, address: &Address, amount: &U256) -> Result<(), u32> {
        let current_balance = self.get(address);
        let new_balance = current_balance.overflowing_sub(*amount);
        if new_balance.1 {
            return Err(ERR_INSUFFICIENT_BALANCE);
        }
        self.set(address, &new_balance.0);
        Ok(())
    }

    pub fn send(&self, from: &Address, to: &Address, amount: &U256) -> Result<(), u32> {
        self.subtract(from, amount)?;
        self.add(to, amount)
    }
}

pub struct Allowance {
    kd: KeyDeriver,
}

impl Allowance {
    pub fn new(slot: u64) -> Self {
        let kd = KeyDeriver::new_specific_slot(slot);
        Self { kd }
    }

    pub fn key(&self, a1: &Address, a2: &Address) -> U256 {
        let mut s = Vec::with_capacity(Address::len_bytes() * 2);
        s.extend_from_slice(a1.as_slice());
        s.extend_from_slice(a2.as_slice());
        self.kd.slice(&s)
    }

    #[inline(always)]
    pub fn set(&self, owner: &Address, spender: &Address, value: &U256) {
        global_service().set_value(&self.key(owner, spender), value);
    }

    #[inline(always)]
    pub fn update(&self, owner: &Address, spender: &Address, amount: &U256) {
        self.set(owner, spender, amount);
    }

    #[inline(always)]
    pub fn get(&self, owner: &Address, spender: &Address) -> U256 {
        global_service()
            .try_get_value(&self.key(owner, spender))
            .cloned()
            .expect("storage value exists")
    }
    pub fn subtract(&self, owner: &Address, spender: &Address, amount: &U256) -> Result<(), u32> {
        let allowance = self.get(owner, spender);
        if &allowance < amount {
            return Err(ERR_INSUFFICIENT_ALLOWANCE);
        }
        let new_allowance = allowance - amount;
        self.set(owner, spender, &new_allowance);
        Ok(())
    }
}

pub static SETTINGS_SERVICE: spin::Once<spin::Mutex<Settings>> = spin::Once::new();
pub fn settings_service<'a>() -> spin::MutexGuard<'a, Settings> {
    SETTINGS_SERVICE
        .call_once(|| spin::Mutex::new(Settings::new(1)))
        .lock()
}
pub static BALANCE_SERVICE: spin::Once<spin::Mutex<Balance>> = spin::Once::new();
pub fn balance_service<'a>() -> spin::MutexGuard<'a, Balance> {
    BALANCE_SERVICE
        .call_once(|| spin::Mutex::new(Balance::new(2)))
        .lock()
}
pub static ALLOWANCE_SERVICE: spin::Once<spin::Mutex<Allowance>> = spin::Once::new();
pub fn allowance_service<'a>() -> spin::MutexGuard<'a, Allowance> {
    ALLOWANCE_SERVICE
        .call_once(|| spin::Mutex::new(Allowance::new(3)))
        .lock()
}

pub fn init_services<'a>() -> (
    spin::MutexGuard<'a, Settings>,
    spin::MutexGuard<'a, Balance>,
    spin::MutexGuard<'a, Allowance>,
) {
    // do not change slot values
    let s1 = settings_service();
    let s2 = balance_service();
    let s3 = allowance_service();
    (s1, s2, s3)
}

#[cfg(test)]
mod tests {
    use crate::helpers::bincode::{decode, encode};
    use crate::{
        common::fixed_bytes_from_u256,
        storage::{Feature, InitialSettings, ADDRESS_LEN_BYTES},
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
        let settings_vec = encode(&settings).unwrap();
        let (settings_restored, _) = decode(&settings_vec).unwrap();
        assert_eq!(settings, settings_restored);
    }
}
