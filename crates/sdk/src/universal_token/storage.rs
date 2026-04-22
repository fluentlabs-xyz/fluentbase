use crate::{
    storage::MapKey,
    universal_token::{
        command::{
            AllowanceCommand, ApproveCommand, BalanceOfCommand, BurnCommand, MintCommand,
            TransferCommand, TransferFromCommand, UniversalTokenCommand, WithdrawCommand,
        },
        consts::{
            ALLOWANCE_STORAGE_SLOT, BALANCE_STORAGE_SLOT, CONTRACT_FROZEN_STORAGE_SLOT,
            DECIMALS_STORAGE_SLOT, MINTER_STORAGE_SLOT, NAME_STORAGE_SLOT, PAUSER_STORAGE_SLOT,
            SIG_ERC20_ALLOWANCE, SIG_ERC20_APPROVE, SIG_ERC20_BALANCE, SIG_ERC20_BALANCE_OF,
            SIG_ERC20_BURN, SIG_ERC20_DECIMALS, SIG_ERC20_DEPOSIT, SIG_ERC20_MINT, SIG_ERC20_NAME,
            SIG_ERC20_PAUSE, SIG_ERC20_SYMBOL, SIG_ERC20_TOTAL_SUPPLY, SIG_ERC20_TRANSFER,
            SIG_ERC20_TRANSFER_FROM, SIG_ERC20_UNPAUSE, SIG_ERC20_WITHDRAW, SYMBOL_STORAGE_SLOT,
            TOTAL_SUPPLY_STORAGE_SLOT, WRAPPED_STORAGE_SLOT,
        },
    },
};
use alloc::vec::Vec;
use fluentbase_codec::{Codec, SolidityABI};
use fluentbase_types::{bytes::BytesMut, Address, Bytes, B256, U256, UNIVERSAL_TOKEN_MAGIC_BYTES};

pub const SIG_LEN_BYTES: usize = 4;

#[derive(Default, Debug, PartialEq, Eq, Clone, Copy, Codec)]
#[repr(transparent)]
pub struct TokenNameOrSymbol {
    bytes: B256,
}

impl From<&str> for TokenNameOrSymbol {
    fn from(value: &str) -> Self {
        Self::from_str(value)
    }
}

impl TokenNameOrSymbol {
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(value: &str) -> Self {
        debug_assert!(value.len() <= B256::len_bytes());
        let mut bytes = B256::ZERO;
        let len = core::cmp::min(B256::len_bytes(), value.len());
        bytes[..len].copy_from_slice(value.as_bytes());
        Self { bytes }
    }

    pub fn as_str(&self) -> Option<&str> {
        let length = self
            .bytes
            .iter()
            .position(|c| *c == 0u8)
            .unwrap_or(B256::len_bytes());
        str::from_utf8(&self.bytes[0..length]).ok()
    }
}

#[derive(Default, Debug, PartialEq, Codec)]
pub struct LegacyInitialSettings {
    pub token_name: [u8; 32],
    pub token_symbol: [u8; 32],
    pub decimals: u8,
    pub initial_supply: U256,
    pub minter: Address,
    pub pauser: Address,
}

impl LegacyInitialSettings {
    pub fn decode_with_prefix(buf: &[u8]) -> Option<Self> {
        if buf.len() < 4 {
            return None;
        }
        let (sig, buf) = buf.split_at(4);
        if sig != UNIVERSAL_TOKEN_MAGIC_BYTES {
            return None;
        }
        let result: Self = SolidityABI::decode(&buf, 0).ok()?;
        Some(result)
    }
}

/// Initial settings payload sizes including magic prefix.
pub const INITIAL_SETTINGS_V1_SIZE: usize = 4 + 6 * 32;
pub const INITIAL_SETTINGS_V2_SIZE: usize = 4 + 7 * 32;

#[derive(Default, Debug, PartialEq, Codec)]
struct InitialSettingsV1 {
    pub token_name: TokenNameOrSymbol,
    pub token_symbol: TokenNameOrSymbol,
    pub decimals: u8,
    pub initial_supply: U256,
    pub minter: Address,
    pub pauser: Address,
}

#[derive(Default, Debug, PartialEq, Codec)]
struct InitialSettingsV2 {
    pub token_name: TokenNameOrSymbol,
    pub token_symbol: TokenNameOrSymbol,
    pub decimals: u8,
    pub initial_supply: U256,
    pub minter: Address,
    pub pauser: Address,
    pub wrapped: bool,
}

#[derive(Default, Debug, PartialEq)]
pub struct InitialSettings {
    pub token_name: TokenNameOrSymbol,
    pub token_symbol: TokenNameOrSymbol,
    pub decimals: u8,
    pub initial_supply: U256,
    pub minter: Address,
    pub pauser: Address,
    /// Enables wrapped-token extension (`deposit()` / `withdraw(uint256)`).
    pub wrapped: Option<bool>,
}

impl InitialSettings {
    pub fn encode_with_prefix(&self) -> Bytes {
        let mut bytes = BytesMut::new();
        if let Some(wrapped) = self.wrapped {
            let settings = InitialSettingsV2 {
                token_name: self.token_name,
                token_symbol: self.token_symbol,
                decimals: self.decimals,
                initial_supply: self.initial_supply,
                minter: self.minter,
                pauser: self.pauser,
                wrapped,
            };
            SolidityABI::encode(&settings, &mut bytes, 0).unwrap();
        } else {
            let settings = InitialSettingsV1 {
                token_name: self.token_name,
                token_symbol: self.token_symbol,
                decimals: self.decimals,
                initial_supply: self.initial_supply,
                minter: self.minter,
                pauser: self.pauser,
            };
            SolidityABI::encode(&settings, &mut bytes, 0).unwrap();
        }
        let result = bytes.freeze();
        // TODO(d1r1): Optimize allocation
        let mut output = Vec::with_capacity(UNIVERSAL_TOKEN_MAGIC_BYTES.len() + result.len());
        output.extend_from_slice(&UNIVERSAL_TOKEN_MAGIC_BYTES[..]);
        output.extend_from_slice(&result);
        output.into()
    }

    pub fn decode_with_prefix(buf: &[u8]) -> Option<Self> {
        if buf.len() < 4 {
            return None;
        }
        let (sig, payload) = buf.split_at(4);
        if sig != UNIVERSAL_TOKEN_MAGIC_BYTES {
            return None;
        }

        match buf.len() {
            INITIAL_SETTINGS_V1_SIZE => {
                let settings: InitialSettingsV1 = SolidityABI::decode(&payload, 0).ok()?;
                Some(Self {
                    token_name: settings.token_name,
                    token_symbol: settings.token_symbol,
                    decimals: settings.decimals,
                    initial_supply: settings.initial_supply,
                    minter: settings.minter,
                    pauser: settings.pauser,
                    wrapped: None,
                })
            }
            INITIAL_SETTINGS_V2_SIZE => {
                let settings: InitialSettingsV2 = SolidityABI::decode(&payload, 0).ok()?;
                Some(Self {
                    token_name: settings.token_name,
                    token_symbol: settings.token_symbol,
                    decimals: settings.decimals,
                    initial_supply: settings.initial_supply,
                    minter: settings.minter,
                    pauser: settings.pauser,
                    wrapped: Some(settings.wrapped),
                })
            }
            _ if buf.len() > INITIAL_SETTINGS_V1_SIZE => {
                // Legacy format uses a different layout and larger payload.
                let settings = LegacyInitialSettings::decode_with_prefix(buf)?;
                Some(Self {
                    token_name: TokenNameOrSymbol {
                        bytes: settings.token_name.into(),
                    },
                    token_symbol: TokenNameOrSymbol {
                        bytes: settings.token_symbol.into(),
                    },
                    decimals: settings.decimals,
                    initial_supply: settings.initial_supply,
                    minter: settings.minter,
                    pauser: settings.pauser,
                    wrapped: None,
                })
            }
            _ => None,
        }
    }
}

pub fn erc20_compute_deploy_storage_keys(input: &[u8], caller: &Address) -> Option<Vec<U256>> {
    if input.len() < SIG_LEN_BYTES {
        return None;
    }
    let mut result = Vec::with_capacity(8);
    let InitialSettings {
        minter,
        pauser,
        initial_supply,
        wrapped,
        ..
    } = InitialSettings::decode_with_prefix(input)?;
    result.push(DECIMALS_STORAGE_SLOT);
    result.push(NAME_STORAGE_SLOT);
    result.push(SYMBOL_STORAGE_SLOT);
    result.push(TOTAL_SUPPLY_STORAGE_SLOT);
    if !initial_supply.is_zero() {
        let storage_slot = caller.compute_slot(BALANCE_STORAGE_SLOT);
        result.push(storage_slot);
    }
    if !minter.is_zero() {
        result.push(MINTER_STORAGE_SLOT);
    }
    if !pauser.is_zero() {
        result.push(PAUSER_STORAGE_SLOT);
    }
    // Push wrapped flag only if we use V2 settings
    if wrapped.is_some() {
        result.push(WRAPPED_STORAGE_SLOT);
    }
    Some(result)
}

pub fn erc20_compute_main_storage_keys(input: &[u8], caller: &Address) -> Option<Vec<U256>> {
    if input.len() < SIG_LEN_BYTES {
        return None;
    }
    let mut result = Vec::with_capacity(8);
    let (sig, input) = input.split_at(SIG_LEN_BYTES);
    let sig = u32::from_be_bytes(sig.try_into().unwrap());
    match sig {
        SIG_ERC20_TOTAL_SUPPLY => result.push(TOTAL_SUPPLY_STORAGE_SLOT),
        SIG_ERC20_TRANSFER => {
            result.push(CONTRACT_FROZEN_STORAGE_SLOT);
            let storage_slot = caller.compute_slot(BALANCE_STORAGE_SLOT);
            result.push(storage_slot);
            let TransferCommand { to, .. } = TransferCommand::try_decode(input).ok()?;
            let storage_slot = to.compute_slot(BALANCE_STORAGE_SLOT);
            result.push(storage_slot);
        }
        SIG_ERC20_TRANSFER_FROM => {
            result.push(CONTRACT_FROZEN_STORAGE_SLOT);
            let TransferFromCommand { from, to, .. } =
                TransferFromCommand::try_decode(input).ok()?;
            result.push(from.compute_slot(BALANCE_STORAGE_SLOT));
            result.push(to.compute_slot(BALANCE_STORAGE_SLOT));
            let allowance_slot = from.compute_slot(ALLOWANCE_STORAGE_SLOT);
            result.push(caller.compute_slot(allowance_slot));
        }
        SIG_ERC20_BALANCE => {
            let storage_slot = caller.compute_slot(BALANCE_STORAGE_SLOT);
            result.push(storage_slot);
        }
        SIG_ERC20_BALANCE_OF => {
            let BalanceOfCommand { owner } = BalanceOfCommand::try_decode(input).ok()?;
            result.push(owner.compute_slot(BALANCE_STORAGE_SLOT));
        }
        SIG_ERC20_SYMBOL => result.push(SYMBOL_STORAGE_SLOT),
        SIG_ERC20_NAME => result.push(NAME_STORAGE_SLOT),
        SIG_ERC20_DECIMALS => result.push(DECIMALS_STORAGE_SLOT),
        SIG_ERC20_ALLOWANCE => {
            let AllowanceCommand { owner, spender } = AllowanceCommand::try_decode(input).ok()?;
            let allowance_slot = owner.compute_slot(ALLOWANCE_STORAGE_SLOT);
            result.push(spender.compute_slot(allowance_slot));
        }
        SIG_ERC20_APPROVE => {
            let ApproveCommand { spender, .. } = ApproveCommand::try_decode(input).ok()?;
            let allowance_slot = caller.compute_slot(ALLOWANCE_STORAGE_SLOT);
            result.push(spender.compute_slot(allowance_slot));
        }
        SIG_ERC20_MINT => {
            result.push(CONTRACT_FROZEN_STORAGE_SLOT);
            let MintCommand { to, .. } = MintCommand::try_decode(input).ok()?;
            result.push(to.compute_slot(BALANCE_STORAGE_SLOT));
            result.push(MINTER_STORAGE_SLOT);
            result.push(TOTAL_SUPPLY_STORAGE_SLOT);
        }
        SIG_ERC20_BURN => {
            result.push(CONTRACT_FROZEN_STORAGE_SLOT);
            let BurnCommand { from, .. } = BurnCommand::try_decode(input).ok()?;
            result.push(from.compute_slot(BALANCE_STORAGE_SLOT));
            result.push(MINTER_STORAGE_SLOT);
            result.push(TOTAL_SUPPLY_STORAGE_SLOT);
        }
        SIG_ERC20_PAUSE => {
            result.push(CONTRACT_FROZEN_STORAGE_SLOT);
            result.push(PAUSER_STORAGE_SLOT);
        }
        SIG_ERC20_UNPAUSE => {
            result.push(CONTRACT_FROZEN_STORAGE_SLOT);
            result.push(PAUSER_STORAGE_SLOT);
        }
        SIG_ERC20_DEPOSIT => {
            result.push(WRAPPED_STORAGE_SLOT);
            result.push(CONTRACT_FROZEN_STORAGE_SLOT);
            result.push(caller.compute_slot(BALANCE_STORAGE_SLOT));
            result.push(TOTAL_SUPPLY_STORAGE_SLOT);
        }
        SIG_ERC20_WITHDRAW => {
            result.push(WRAPPED_STORAGE_SLOT);
            result.push(CONTRACT_FROZEN_STORAGE_SLOT);
            let WithdrawCommand { .. } = WithdrawCommand::try_decode(input).ok()?;
            result.push(caller.compute_slot(BALANCE_STORAGE_SLOT));
            result.push(TOTAL_SUPPLY_STORAGE_SLOT);
        }
        _ => {}
    }
    Some(result)
}

pub fn erc20_compute_storage_keys(
    input: &[u8],
    caller: &Address,
    is_create: bool,
) -> Option<Vec<U256>> {
    if is_create {
        erc20_compute_deploy_storage_keys(input, caller)
    } else {
        erc20_compute_main_storage_keys(input, caller)
    }
}

#[cfg(test)]
mod tests {
    use crate::universal_token::storage::{
        InitialSettings, TokenNameOrSymbol, INITIAL_SETTINGS_V1_SIZE, INITIAL_SETTINGS_V2_SIZE,
    };
    use fluentbase_types::{address, Address, U256};

    #[test]
    fn test_ops_u256_overflow() {
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
    fn test_ser_der() {
        let settings = InitialSettings {
            token_name: TokenNameOrSymbol::from_str("Hello"),
            token_symbol: TokenNameOrSymbol::from_str("World"),
            decimals: 12,
            initial_supply: U256::from(2),
            minter: address!("0303000200500020400000040000002000809020"),
            pauser: Address::ZERO,
            wrapped: None,
        };
        let addr = address!("0003000200500000400000040000002000800020");
        let addr_bytes: [u8; Address::len_bytes()] = addr.into();
        let addr_restored: Address = addr_bytes.into();
        assert_eq!(addr, addr_restored);
        let settings_vec = settings.encode_with_prefix();
        assert_eq!(settings_vec.len(), INITIAL_SETTINGS_V1_SIZE);
        let settings_restored = InitialSettings::decode_with_prefix(settings_vec.as_ref()).unwrap();
        assert_eq!(settings, settings_restored);
    }

    #[test]
    fn test_ser_der_wrapped_settings() {
        let settings = InitialSettings {
            token_name: TokenNameOrSymbol::from_str("WETH"),
            token_symbol: TokenNameOrSymbol::from_str("WETH"),
            decimals: 18,
            initial_supply: U256::ZERO,
            minter: Address::ZERO,
            pauser: Address::ZERO,
            wrapped: Some(true),
        };

        let settings_vec = settings.encode_with_prefix();
        assert_eq!(settings_vec.len(), INITIAL_SETTINGS_V2_SIZE);
        let settings_restored = InitialSettings::decode_with_prefix(settings_vec.as_ref()).unwrap();
        assert_eq!(settings, settings_restored);
    }
}
