use crate::{
    command::{
        AllowanceCommand, ApproveCommand, BalanceOfCommand, BurnCommand, MintCommand,
        TransferCommand, TransferFromCommand, UniversalTokenCommand,
    },
    consts::{
        ALLOWANCE_STORAGE_SLOT, BALANCE_STORAGE_SLOT, CONTRACT_FROZEN_STORAGE_SLOT,
        DECIMALS_STORAGE_SLOT, MINTER_STORAGE_SLOT, NAME_STORAGE_SLOT, PAUSER_STORAGE_SLOT,
        SIG_ERC20_ALLOWANCE, SIG_ERC20_APPROVE, SIG_ERC20_BALANCE, SIG_ERC20_BALANCE_OF,
        SIG_ERC20_BURN, SIG_ERC20_DECIMALS, SIG_ERC20_MINT, SIG_ERC20_NAME, SIG_ERC20_PAUSE,
        SIG_ERC20_SYMBOL, SIG_ERC20_TOTAL_SUPPLY, SIG_ERC20_TRANSFER, SIG_ERC20_TRANSFER_FROM,
        SIG_ERC20_UNPAUSE, SYMBOL_STORAGE_SLOT, TOTAL_SUPPLY_STORAGE_SLOT,
    },
};
use alloc::vec::Vec;
use fluentbase_sdk::{
    bytes::BytesMut,
    codec::{Codec, SolidityABI},
    storage::MapKey,
    Address, Bytes, U256, UNIVERSAL_TOKEN_MAGIC_BYTES,
};

pub const ADDRESS_LEN_BYTES: usize = Address::len_bytes();
pub const U256_LEN_BYTES: usize = U256::BYTES;
pub const SIG_LEN_BYTES: usize = 4;
pub const DECIMALS_DEFAULT: u8 = 2;

#[derive(Default, Debug, PartialEq, Codec)]
#[repr(transparent)]
pub struct TokenNameOrSymbol {
    bytes: [u8; U256::BYTES],
}

impl From<&str> for TokenNameOrSymbol {
    fn from(value: &str) -> Self {
        Self::from_str(value)
    }
}

impl TokenNameOrSymbol {
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(value: &str) -> Self {
        debug_assert!(value.len() <= U256::BYTES);
        let mut bytes = [0u8; U256::BYTES];
        let len = core::cmp::min(U256::BYTES, value.len());
        bytes[..len].copy_from_slice(value.as_bytes());
        Self { bytes }
    }

    pub fn as_str(&self) -> Option<&str> {
        let length = self
            .bytes
            .iter()
            .position(|c| *c == 0u8)
            .unwrap_or(U256::BYTES);
        str::from_utf8(&self.bytes[0..length]).ok()
    }
}

#[derive(Default, Debug, PartialEq, Codec)]
pub struct InitialSettings {
    pub token_name: TokenNameOrSymbol,
    pub token_symbol: TokenNameOrSymbol,
    pub decimals: u8,
    pub initial_supply: U256,
    pub minter: Address,
    pub pauser: Address,
}

impl InitialSettings {
    pub fn encode_with_prefix(&self) -> Bytes {
        let mut bytes = BytesMut::new();
        SolidityABI::encode(self, &mut bytes, 0).unwrap();
        let result = bytes.freeze();
        // TODO(dmitry123): Optimize allocation
        let mut output = Vec::with_capacity(UNIVERSAL_TOKEN_MAGIC_BYTES.len() + result.len());
        output.extend_from_slice(&UNIVERSAL_TOKEN_MAGIC_BYTES[..]);
        output.extend_from_slice(&result);
        output.into()
    }

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

pub fn erc20_compute_deploy_storage_keys(input: &[u8], caller: &Address) -> Option<Vec<U256>> {
    if input.len() < SIG_LEN_BYTES {
        return None;
    }
    let mut result = Vec::with_capacity(7);
    let Some(InitialSettings {
        minter,
        pauser,
        initial_supply,
        ..
    }) = InitialSettings::decode_with_prefix(input)
    else {
        // If input is incorrect then no storage keys required
        return None;
    };
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
    Some(result)
}

pub fn erc20_compute_main_storage_keys(input: &[u8], caller: &Address) -> Option<Vec<U256>> {
    if input.len() < SIG_LEN_BYTES {
        return None;
    }
    let mut result = Vec::with_capacity(7);
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
    use crate::storage::{InitialSettings, ADDRESS_LEN_BYTES};
    use fluentbase_sdk::{address, Address, U256};

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
            token_name: Default::default(),
            token_symbol: Default::default(),
            decimals: 12,
            initial_supply: U256::from(2),
            minter: address!("0303000200500020400000040000002000809020"),
            pauser: Address::ZERO,
        };
        let addr = address!("0003000200500000400000040000002000800020");
        let addr_bytes: [u8; ADDRESS_LEN_BYTES] = addr.into();
        let addr_restored: Address = addr_bytes.into();
        assert_eq!(addr, addr_restored);
        let settings_vec = settings.encode_with_prefix();
        let settings_restored = InitialSettings::decode_with_prefix(settings_vec.as_ref()).unwrap();
        assert_eq!(settings, settings_restored);
    }
}
