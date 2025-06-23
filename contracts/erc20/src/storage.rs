use fluentbase_sdk::{address, derive::solidity_storage, Address, SharedAPI, U256};

pub const U256_SIZE_BYTES: usize = size_of::<U256>();
pub const SIG_SIZE_BYTES: usize = size_of::<u32>();

solidity_storage! {
    mapping(Address => U256) Settings;
    mapping(Address => U256) Balance;
    mapping(Address => mapping(Address => U256)) Allowance;
}

impl Settings {
    const TOTAL_SUPPLY_ADDRESS: Address = address!("455243323020746F74616C20737570706C790000"); // hex: ERC20 total supply
    const SYMBOL_ADDRESS: Address = address!("73796D626F6C0000000000000000000000000000"); // hex: symbol
    pub const SHORT_STR_LEN_MIN: usize = 1;
    pub const SHORT_STR_BYTE_REPR_LEN_MIN: usize = Self::SHORT_STR_LEN_MIN + 1;
    const SHORT_STR_LEN_MAX: usize = 31;
    const NAME_ADDRESS: Address = address!("6E616D6500000000000000000000000000000000"); // hex: name
    const DECIMALS_ADDRESS: Address = address!("646563696D616C73000000000000000000000000"); // hex: decimals
    const DECIMALS_MAX: usize = 36; // 115792089237316195423570985008687907853269984665640564039457584007913129639936
    pub fn total_supply_set(sdk: &mut impl SharedAPI, amount: U256) {
        Self::set(sdk, Self::TOTAL_SUPPLY_ADDRESS, amount);
    }
    pub fn total_supply_get(sdk: &impl SharedAPI) -> U256 {
        Self::get(sdk, Self::TOTAL_SUPPLY_ADDRESS)
    }
    #[inline(always)]
    fn short_str_to_u256_repr(short_str: &[u8]) -> Result<U256, ()> {
        let len = short_str.len();
        if len < Self::SHORT_STR_LEN_MIN || len > Self::SHORT_STR_LEN_MAX {
            return Err(());
        }
        let mut byte_repr = [0u8; U256_SIZE_BYTES];
        byte_repr[0] = len as u8;
        byte_repr[1..len + 1].copy_from_slice(short_str);
        let u256_repr = U256::from_be_bytes::<U256_SIZE_BYTES>(byte_repr);
        Ok(u256_repr)
    }
    #[inline(always)]
    fn short_str_from_u256_repr<'a>(repr: U256) -> &'a [u8] {
        let repr = repr.to_be_bytes::<U256_SIZE_BYTES>();
        let len = repr[0] as usize;
        if len == 0 {
            return Default::default();
        }
        unsafe { core::slice::from_raw_parts((repr.as_ptr() as usize + 1) as *const u8, len) }
    }
    #[inline(always)]
    pub fn short_str_try_from_slice_repr(repr: &[u8]) -> Result<&[u8], ()> {
        if repr.len() < Self::SHORT_STR_BYTE_REPR_LEN_MIN {
            return Err(());
        }
        let len = repr[0] as usize;
        if len > repr.len() - 1 || len > Self::SHORT_STR_LEN_MAX {
            return Err(());
        }
        let result =
            unsafe { core::slice::from_raw_parts((repr.as_ptr() as usize + 1) as *const u8, len) };
        Ok(result)
    }
    #[inline(always)]
    fn short_str_set(sdk: &mut impl SharedAPI, address: Address, short_str: &[u8]) -> bool {
        if let Ok(u256_repr) = Self::short_str_to_u256_repr(&short_str) {
            Self::set(sdk, address, u256_repr);
        } else {
            return false;
        };
        true
    }
    #[inline(always)]
    fn short_str<'a>(sdk: &impl SharedAPI, storage_address: Address) -> &'a [u8] {
        Self::short_str_from_u256_repr(Self::get(sdk, storage_address))
    }
    pub fn symbol_set(sdk: &mut impl SharedAPI, symbol: &[u8]) -> bool {
        Self::short_str_set(sdk, Self::SYMBOL_ADDRESS, symbol)
    }
    pub fn symbol<'a>(sdk: &impl SharedAPI) -> &'a [u8] {
        Self::short_str(sdk, Self::SYMBOL_ADDRESS)
    }
    pub fn name_set(sdk: &mut impl SharedAPI, symbol: &[u8]) -> bool {
        Self::short_str_set(sdk, Self::NAME_ADDRESS, symbol)
    }
    pub fn name<'a>(sdk: &impl SharedAPI) -> &'a [u8] {
        Self::short_str(sdk, Self::NAME_ADDRESS)
    }
    pub fn decimals_set(sdk: &mut impl SharedAPI, decimals: U256) -> bool {
        if decimals > U256::from(Self::DECIMALS_MAX) {
            return false;
        }
        Self::set(sdk, Self::DECIMALS_ADDRESS, decimals);
        true
    }
    pub fn decimals_get(sdk: &impl SharedAPI) -> U256 {
        Self::get(sdk, Self::DECIMALS_ADDRESS)
    }
}

impl Balance {
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
}

impl Allowance {
    pub fn update(sdk: &mut impl SharedAPI, owner: Address, spender: Address, amount: U256) {
        Self::set(sdk, owner, spender, amount);
    }
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
