use crate::common::fixed_bytes_from_u256;
use fluentbase_sdk::{
    derive::{derive_evm_error, derive_keccak256, derive_keccak256_id},
    Address,
    SharedAPI,
    B256,
    U256,
};

pub const ERR_MALFORMED_INPUT: u32 = derive_evm_error!("MalformedInput()");
pub const ERR_INSUFFICIENT_BALANCE: u32 = derive_evm_error!("InsufficientBalance()");
pub const ERR_INSUFFICIENT_ALLOWANCE: u32 = derive_evm_error!("InsufficientAllowance()");
pub const ERR_INDEX_OUT_OF_BOUNDS: u32 = derive_evm_error!("IndexOutOfBounds()");
pub const ERR_DECODE: u32 = derive_evm_error!("DecodeError()");
pub const ERR_INVALID_META_NAME: u32 = derive_evm_error!("InvalidMetaName()");
pub const ERR_INVALID_META_SYMBOL: u32 = derive_evm_error!("InvalidMetaSymbol()");
pub const ERR_MINTABLE_PLUGIN_NOT_ACTIVE: u32 = derive_evm_error!("MintablePluginNotActive()");
pub const ERR_PAUSABLE_PLUGIN_NOT_ACTIVE: u32 = derive_evm_error!("PausablePluginNotActive()");
pub const ERR_ALREADY_PAUSED: u32 = derive_evm_error!("AlreadyPaused()");
pub const ERR_ALREADY_UNPAUSED: u32 = derive_evm_error!("AlreadyUnpaused()");
pub const ERR_INVALID_MINTER: u32 = derive_evm_error!("InvalidMinter()");
pub const ERR_INVALID_PAUSER: u32 = derive_evm_error!("InvalidPauser()");
pub const ERR_INVALID_RECIPIENT: u32 = derive_evm_error!("InvalidRecipient()");
pub const ERR_OVERFLOW: u32 = derive_evm_error!("Overflow()");
pub const ERR_VALIDATION: u32 = derive_evm_error!("Validation()");
pub const ERR_UNINIT: u32 = derive_evm_error!("UninitError()");
pub const ERR_CONVERSION: u32 = derive_evm_error!("ConversionError()");

pub const SIG_SYMBOL: u32 = derive_keccak256_id!("symbol()");
pub const SIG_NAME: u32 = derive_keccak256_id!("name()");
pub const SIG_DECIMALS: u32 = derive_keccak256_id!("decimals()");
pub const SIG_TOTAL_SUPPLY: u32 = derive_keccak256_id!("totalSupply()");
pub const SIG_BALANCE_OF: u32 = derive_keccak256_id!("balanceOf(address)");
pub const SIG_TRANSFER: u32 = derive_keccak256_id!("transfer(address,uint256)");
pub const SIG_TRANSFER_FROM: u32 = derive_keccak256_id!("transferFrom(address,address,uint256)");
pub const SIG_ALLOWANCE: u32 = derive_keccak256_id!("allowance(address)");
pub const SIG_APPROVE: u32 = derive_keccak256_id!("approve(address,uint256)");
pub const SIG_MINT: u32 = derive_keccak256_id!("mint(address,uint256)");
pub const SIG_PAUSE: u32 = derive_keccak256_id!("pause()");
pub const SIG_UNPAUSE: u32 = derive_keccak256_id!("unpause()");

pub const EVENT_TRANSFER: B256 = B256::new(derive_keccak256!("Transfer(address,address,uint256)"));
pub const EVENT_APPROVAL: B256 = B256::new(derive_keccak256!("Approval(address,address,uint256)"));
pub const EVENT_PAUSED: B256 = B256::new(derive_keccak256!("Paused(address)"));
pub const EVENT_UNPAUSED: B256 = B256::new(derive_keccak256!("Unpaused(address)"));

pub fn emit_transfer_event(sdk: &mut impl SharedAPI, from: &Address, to: &Address, amount: &U256) {
    sdk.emit_log(
        &[
            EVENT_TRANSFER,
            B256::left_padding_from(from.as_slice()),
            B256::left_padding_from(to.as_slice()),
        ],
        &fixed_bytes_from_u256(amount),
    );
}

pub fn emit_pause_event(sdk: &mut impl SharedAPI, pauser: &Address) {
    sdk.emit_log(&[EVENT_PAUSED], pauser.as_slice());
}

pub fn emit_unpause_event(sdk: &mut impl SharedAPI, pauser: &Address) {
    sdk.emit_log(&[EVENT_UNPAUSED], pauser.as_slice());
}

pub fn emit_approval_event(
    sdk: &mut impl SharedAPI,
    owner: &Address,
    spender: &Address,
    amount: &U256,
) {
    sdk.emit_log(
        &[
            EVENT_APPROVAL,
            B256::left_padding_from(owner.as_slice()),
            B256::left_padding_from(spender.as_slice()),
        ],
        &fixed_bytes_from_u256(amount),
    );
}
