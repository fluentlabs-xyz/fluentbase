use crate::common::{b256_from_address_try, u256_from_address, u256_from_bytes_slice_try};
use crate::consts::{ERR_INDEX_OUT_OF_BOUNDS, ERR_MALFORMED_INPUT};
use crate::helpers::bincode::{decode, encode};
use crate::services::storage_global::storage_service;
use crate::types::derived_key::{IKeyDeriver, KeyDeriver};
use crate::types::result_or_interruption::ResultOrInterruption;
use crate::{
    common::{address_from_u256, fixed_bytes_from_u256},
    unwrap, unwrap_opt,
};
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
            features: Vec::default(),
        }
    }
    pub fn try_decode_from_slice(
        value: &[u8],
    ) -> Result<(Self, usize), bincode::error::DecodeError> {
        decode(value)
    }
    pub fn try_encode(&self) -> Result<Vec<u8>, bincode::error::EncodeError> {
        encode(self)
    }
    pub fn try_encode_for_deploy(&self) -> Result<Vec<u8>, bincode::error::EncodeError> {
        let mut init_bytecode: Vec<u8> = UNIVERSAL_TOKEN_MAGIC_BYTES.to_vec();
        init_bytecode.extend(encode(self)?);
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

pub struct Settings {
    kd: Arc<KeyDeriver>,
    default_on_read: bool,
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
    pub fn new(slot: u64, default_on_read: bool) -> Self {
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
            default_on_read,
        }
    }

    pub fn default_on_read(&self) -> bool {
        self.default_on_read
    }

    fn total_supply_slot(&mut self) -> U256 {
        self.total_supply_slot
            .get_or_insert_with(|| self.kd.u256(&U256::from(1)))
            .clone()
    }

    fn minter_slot(&mut self) -> U256 {
        self.minter_slot
            .get_or_insert_with(|| self.kd.u256(&U256::from(2)))
            .clone()
    }

    fn pauser_slot(&mut self) -> U256 {
        self.pauser_slot
            .get_or_insert_with(|| self.kd.u256(&U256::from(3)))
            .clone()
    }

    fn symbol_slot(&mut self) -> U256 {
        self.symbol_slot
            .get_or_insert_with(|| self.kd.u256(&U256::from(4)))
            .clone()
    }

    fn name_slot(&mut self) -> U256 {
        self.name_slot
            .get_or_insert_with(|| self.kd.u256(&U256::from(5)))
            .clone()
    }

    fn decimals_slot(&mut self) -> U256 {
        self.decimals_slot
            .get_or_insert_with(|| self.kd.u256(&U256::from(6)))
            .clone()
    }

    fn flags_slot(&mut self) -> U256 {
        self.flags_slot
            .get_or_insert_with(|| self.kd.u256(&U256::from(7)))
            .clone()
    }

    pub fn total_supply_set(&mut self, value: &U256) {
        let s = self.total_supply_slot();
        storage_service(self.default_on_read).try_set(&s, value);
    }
    pub fn total_supply_get(&mut self) -> ResultOrInterruption<U256, u32> {
        let s = self.total_supply_slot();
        unwrap_opt!(storage_service(self.default_on_read).try_get(&s).cloned()).into()
    }
    pub fn minter_set(&mut self, value: &Address) {
        let s = self.minter_slot();
        let v = u256_from_address(&value);
        storage_service(self.default_on_read).try_set(&s, &v);
    }
    pub fn minter_get(&mut self) -> ResultOrInterruption<Address, u32> {
        let s = self.minter_slot();
        unwrap_opt!(storage_service(self.default_on_read)
            .try_get(&s)
            .map(|v| address_from_u256(v)))
        .into()
    }
    pub fn pauser_set(&mut self, value: &Address) {
        let s = self.pauser_slot();
        let v = u256_from_address(value);
        storage_service(self.default_on_read).try_set(&s, &v);
    }
    pub fn pauser_get(&mut self) -> ResultOrInterruption<Address, u32> {
        let s = self.pauser_slot();
        unwrap_opt!(storage_service(self.default_on_read)
            .try_get(&s)
            .map(|v| address_from_u256(v)))
        .into()
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
        let u256_repr = u256_from_bytes_slice_try(&byte_repr).unwrap();
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
            storage_service(self.default_on_read).try_set(&slot, &u256_repr);
        } else {
            return false;
        };
        true
    }
    #[inline(always)]
    fn short_str<'a>(&self, slot: &U256) -> ResultOrInterruption<Vec<u8>, u32> {
        let repr = unwrap_opt!(storage_service(self.default_on_read).try_get(slot).cloned());
        self.short_str_from_u256_repr(&repr).into()
    }
    pub fn symbol_set(&mut self, symbol: &[u8]) -> bool {
        let s = self.symbol_slot();
        self.short_str_set(&s, symbol)
    }
    pub fn symbol<'a>(&mut self) -> ResultOrInterruption<Vec<u8>, u32> {
        let v = self.symbol_slot();
        self.short_str(&v)
    }
    pub fn name_set(&mut self, symbol: &[u8]) -> bool {
        let s = self.name_slot();
        self.short_str_set(&s, symbol)
    }
    pub fn name<'a>(&mut self) -> ResultOrInterruption<Vec<u8>, u32> {
        let s = self.name_slot();
        self.short_str(&s)
    }
    pub fn decimals_set(&mut self, decimals: u8) -> bool {
        if decimals > Self::DECIMALS_MAX {
            return false;
        }
        let s = self.decimals_slot();
        storage_service(self.default_on_read).try_set(&s, &U256::from(decimals));
        true
    }
    pub fn decimals_get(&mut self) -> ResultOrInterruption<U256, u32> {
        let s = self.decimals_slot();
        unwrap_opt!(storage_service(self.default_on_read).try_get(&s).cloned()).into()
    }
    pub fn flags_get(&mut self) -> ResultOrInterruption<U256, u32> {
        let s = self.flags_slot();
        unwrap_opt!(storage_service(self.default_on_read).try_get(&s).cloned()).into()
    }
    pub fn flags_set(&mut self, flags: U256) {
        let s = self.flags_slot();
        storage_service(self.default_on_read).try_set(&s, &flags);
    }
}

pub struct Config {
    flags: Option<U256>,
    // settings: Settings,
    default_on_read: bool,
}

impl Config {
    pub fn new(default_on_read: bool) -> Self {
        Self {
            flags: None,
            default_on_read,
        }
    }

    pub fn get_or_init_flags(&mut self) -> ResultOrInterruption<U256, u32> {
        if None == self.flags {
            self.flags = Some(unwrap!(settings_service(self.default_on_read).flags_get()));
        }
        if let Some(v) = self.flags.as_mut() {
            return v.clone().into();
        }
        unreachable!();
    }

    fn get_or_init_flags_mut(&mut self) -> ResultOrInterruption<&mut U256, u32> {
        if None == self.flags {
            self.flags = Some(unwrap!(settings_service(self.default_on_read).flags_get()));
        }
        if let Some(v) = self.flags.as_mut() {
            return v.into();
        }
        unreachable!();
    }

    pub fn set_flag(&mut self, idx: usize, value: bool) -> ResultOrInterruption<(), u32> {
        if idx >= U256_LEN_BITS {
            return ERR_INDEX_OUT_OF_BOUNDS.into();
        }
        let flags: &mut U256 = unwrap!(self.get_or_init_flags_mut());
        flags.set_bit(idx, value);
        ().into()
    }

    pub fn save_flags(&self) -> bool {
        if let Some(flags) = self.flags {
            settings_service(self.default_on_read).flags_set(flags);
            return true;
        }
        false
    }

    fn flag_value(&mut self, idx: usize) -> ResultOrInterruption<bool, u32> {
        if idx >= U256_LEN_BITS {
            return ERR_INDEX_OUT_OF_BOUNDS.into();
        }
        let flags: U256 = unwrap!(self.get_or_init_flags());
        flags.bit(idx).into()
    }

    const MINTABLE_PLUGIN_FLAG_IDX: usize = 0;
    const PAUSABLE_PLUGIN_FLAG_IDX: usize = 1;
    const PAUSED_FLAG_IDX: usize = 2;

    #[inline(always)]
    pub fn enable_mintable_plugin(&mut self) {
        self.set_flag(Self::MINTABLE_PLUGIN_FLAG_IDX, true);
    }

    #[inline(always)]
    pub fn mintable_plugin_enabled(&mut self) -> ResultOrInterruption<bool, u32> {
        self.flag_value(Self::MINTABLE_PLUGIN_FLAG_IDX)
    }

    #[inline(always)]
    pub fn enable_pausable_plugin(&mut self) {
        self.set_flag(Self::PAUSABLE_PLUGIN_FLAG_IDX, true);
    }

    #[inline(always)]
    pub fn pausable_plugin_enabled(&mut self) -> ResultOrInterruption<bool, u32> {
        self.flag_value(Self::PAUSABLE_PLUGIN_FLAG_IDX)
    }

    #[inline(always)]
    pub fn paused(&mut self) -> ResultOrInterruption<bool, u32> {
        self.flag_value(Self::PAUSED_FLAG_IDX)
    }

    #[inline(always)]
    pub fn pause(&mut self) {
        self.set_flag(Self::PAUSED_FLAG_IDX, true);
    }

    #[inline(always)]
    pub fn unpause(&mut self) {
        self.set_flag(Self::PAUSED_FLAG_IDX, false);
    }
}

pub struct Balance {
    kd: KeyDeriver,
    default_on_read: bool,
}

impl Balance {
    pub fn new(slot: u64, default_on_read: bool) -> Self {
        let kd = KeyDeriver::new_specific_slot(slot);
        Self {
            kd,
            default_on_read,
        }
    }

    #[inline(always)]
    pub fn set(&self, address: &Address, value: &U256) {
        let key = self.kd.b256(&b256_from_address_try(address));
        storage_service(self.default_on_read).try_set(&key, value);
    }

    #[inline(always)]
    pub fn get(&self, address: &Address) -> ResultOrInterruption<U256, u32> {
        let key = self.kd.b256(&b256_from_address_try(address));
        unwrap_opt!(storage_service(self.default_on_read).try_get(&key).cloned()).into()
    }

    pub fn add(&self, address: &Address, amount: &U256) -> ResultOrInterruption<(), u32> {
        let current_balance: U256 = unwrap!(self.get(address));
        let new_balance = current_balance + amount;
        self.set(address, &new_balance);
        ().into()
    }

    pub fn subtract(&self, address: &Address, amount: &U256) -> ResultOrInterruption<bool, u32> {
        let current_balance: U256 = unwrap!(self.get(address));
        if &current_balance < amount {
            return false.into();
        }
        let new_balance = current_balance - amount;
        self.set(address, &new_balance);
        true.into()
    }

    pub fn send(
        &self,
        from: &Address,
        to: &Address,
        amount: &U256,
    ) -> ResultOrInterruption<bool, u32> {
        if !unwrap!(self.subtract(from, amount)) {
            return false.into();
        }
        self.add(to, amount).map(|_| true)
    }
}

pub struct Allowance {
    kd: KeyDeriver,
    default_on_read: bool,
}

impl Allowance {
    pub fn new(slot: u64, default_on_read: bool) -> Self {
        let kd = KeyDeriver::new_specific_slot(slot);
        Self {
            kd,
            default_on_read,
        }
    }

    fn key(&self, addr1: &Address, addr2: &Address) -> U256 {
        let mut s = Vec::with_capacity(Address::len_bytes() * 2);
        s.extend_from_slice(addr1.as_slice());
        s.extend_from_slice(addr2.as_slice());
        self.kd.slice(&s)
    }

    #[inline(always)]
    pub fn set(&self, owner: &Address, spender: &Address, value: &U256) {
        storage_service(self.default_on_read).try_set(&self.key(owner, spender), value);
    }

    #[inline(always)]
    pub fn update(&self, owner: &Address, spender: &Address, amount: &U256) {
        self.set(owner, spender, amount);
    }

    #[inline(always)]
    pub fn get(&self, owner: &Address, spender: &Address) -> ResultOrInterruption<U256, u32> {
        unwrap_opt!(storage_service(self.default_on_read)
            .try_get(&self.key(owner, spender))
            .cloned())
        .into()
    }
    pub fn subtract(
        &self,
        owner: &Address,
        spender: &Address,
        amount: &U256,
    ) -> ResultOrInterruption<bool, u32> {
        let allowance: U256 = unwrap!(self.get(owner, spender));
        if allowance < *amount {
            return false.into();
        }
        let new_allowance = allowance - amount;
        self.set(owner, spender, &new_allowance);
        true.into()
    }
}

pub static SETTINGS_SERVICE: spin::Once<spin::Mutex<Settings>> = spin::Once::new();
pub fn settings_service<'a>(default_on_read: bool) -> spin::MutexGuard<'a, Settings> {
    SETTINGS_SERVICE
        .call_once(|| spin::Mutex::new(Settings::new(1, default_on_read)))
        .lock()
}
pub static BALANCE_SERVICE: spin::Once<spin::Mutex<Balance>> = spin::Once::new();
pub fn balance_service<'a>(default_on_read: bool) -> spin::MutexGuard<'a, Balance> {
    BALANCE_SERVICE
        .call_once(|| spin::Mutex::new(Balance::new(2, default_on_read)))
        .lock()
}
pub static ALLOWANCE_SERVICE: spin::Once<spin::Mutex<Allowance>> = spin::Once::new();
pub fn allowance_service<'a>(default_on_read: bool) -> spin::MutexGuard<'a, Allowance> {
    ALLOWANCE_SERVICE
        .call_once(|| spin::Mutex::new(Allowance::new(3, default_on_read)))
        .lock()
}

pub fn init_services<'a>(
    default_on_read: bool,
) -> (
    spin::MutexGuard<'a, Settings>,
    spin::MutexGuard<'a, Balance>,
    spin::MutexGuard<'a, Allowance>,
) {
    // do not change slot values
    let s1 = settings_service(default_on_read);
    let s2 = balance_service(default_on_read);
    let s3 = allowance_service(default_on_read);
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
