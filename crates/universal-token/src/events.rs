use crate::common::fixed_bytes_from_u256;
use fluentbase_sdk::{derive::derive_keccak256, Address, SharedAPI, B256, U256};
use fluentbase_svm_common::common::evm_balance_from_lamports;
use solana_pubkey::Pubkey;

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

pub const EVENT_UT_TRANSFER: B256 = B256::new(derive_keccak256!("Transfer(address)"));
pub const EVENT_UT_TRANSFER_CHECKED: B256 =
    B256::new(derive_keccak256!("TransferChecked(address)"));

pub fn emit_ut_transfer<SDK: SharedAPI>(sdk: &mut SDK, from: &Pubkey, to: &Pubkey, amount: u64) {
    sdk.emit_log(
        &[
            EVENT_UT_TRANSFER,
            B256::left_padding_from(from.to_bytes().as_slice()),
            B256::left_padding_from(to.to_bytes().as_slice()),
        ],
        &fixed_bytes_from_u256(&evm_balance_from_lamports(amount)),
    );
}

pub fn emit_ut_transfer_checked<SDK: SharedAPI>(
    sdk: &mut SDK,
    from: &Address,
    to: &Address,
    amount: &U256,
) {
    sdk.emit_log(
        &[
            EVENT_UT_TRANSFER_CHECKED,
            B256::left_padding_from(from.as_slice()),
            B256::left_padding_from(to.as_slice()),
        ],
        &fixed_bytes_from_u256(amount),
    );
}
