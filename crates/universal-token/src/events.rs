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

pub const EVENT_UT_TRANSFER: B256 =
    B256::new(derive_keccak256!("Transfer(address,address,uint256)"));
pub const EVENT_UT_TRANSFER_CHECKED: B256 = B256::new(derive_keccak256!(
    "TransferChecked(address,address,uint256)"
));
pub const EVENT_UT_APPROVE: B256 = B256::new(derive_keccak256!("Approve(address,address,uint256)"));
pub const EVENT_UT_APPROVE_CHECKED: B256 =
    B256::new(derive_keccak256!("ApproveChecked(address,address,uint256)"));
pub const EVENT_UT_REVOKE: B256 = B256::new(derive_keccak256!("Revoke(address,address,uint256)"));
pub const EVENT_UT_SET_AUTHORITY: B256 =
    B256::new(derive_keccak256!("SetAuthority(address,address,uint256)"));
pub const EVENT_UT_MINT_TO: B256 = B256::new(derive_keccak256!("MintTo(address,address,uint256)"));
pub const EVENT_UT_MINT_TO_CHECKED: B256 =
    B256::new(derive_keccak256!("MintToChecked(address,address,uint256)"));
pub const EVENT_UT_BURN: B256 = B256::new(derive_keccak256!("Burn(address,address,uint256)"));
pub const EVENT_UT_BURN_CHECKED: B256 =
    B256::new(derive_keccak256!("BurnChecked(address,address,uint256)"));
pub const EVENT_UT_CLOSE_ACCOUNT: B256 =
    B256::new(derive_keccak256!("CloseAccount(address,address,uint256)"));
pub const EVENT_UT_FREEZE_ACCOUNT: B256 =
    B256::new(derive_keccak256!("FreezeAccount(address,address,uint256)"));
pub const EVENT_UT_THAW_ACCOUNT: B256 =
    B256::new(derive_keccak256!("ThawAccount(address,address,uint256)"));

pub fn emit_ut_transfer<SDK: SharedAPI, const CHECKED: bool>(
    sdk: &mut SDK,
    from: &Pubkey,
    to: &Pubkey,
    amount: u64,
) {
    sdk.emit_log(
        &[
            if CHECKED {
                EVENT_UT_TRANSFER_CHECKED
            } else {
                EVENT_UT_TRANSFER
            },
            B256::left_padding_from(from.as_ref()),
            B256::left_padding_from(to.as_ref()),
        ],
        &fixed_bytes_from_u256(&evm_balance_from_lamports(amount)),
    );
}

pub fn emit_ut_approve<SDK: SharedAPI, const CHECKED: bool>(
    sdk: &mut SDK,
    source: &Pubkey,
    spender: &Pubkey,
    amount: u64,
) {
    sdk.emit_log(
        &[
            if CHECKED {
                EVENT_UT_APPROVE_CHECKED
            } else {
                EVENT_UT_APPROVE
            },
            B256::left_padding_from(source.as_ref()),
            B256::left_padding_from(spender.as_ref()),
        ],
        &fixed_bytes_from_u256(&evm_balance_from_lamports(amount)),
    );
}

pub fn emit_ut_revoke<SDK: SharedAPI>(sdk: &mut SDK, source: &Pubkey) {
    sdk.emit_log(
        &[EVENT_UT_REVOKE, B256::left_padding_from(source.as_ref())],
        &[],
    );
}

pub fn emit_ut_set_authority<SDK: SharedAPI>(
    sdk: &mut SDK,
    account: &Pubkey,
    new_authority: &Pubkey,
    authority_type: u8,
) {
    sdk.emit_log(
        &[
            EVENT_UT_SET_AUTHORITY,
            B256::left_padding_from(account.as_ref()),
            B256::left_padding_from(new_authority.as_ref()),
        ],
        &[authority_type],
    );
}

pub fn emit_ut_mint_to<SDK: SharedAPI, const CHECKED: bool>(
    sdk: &mut SDK,
    mint_account: &Pubkey,
    dst_account: &Pubkey,
    amount: u64,
) {
    sdk.emit_log(
        &[
            if CHECKED {
                EVENT_UT_MINT_TO_CHECKED
            } else {
                EVENT_UT_MINT_TO
            },
            B256::left_padding_from(mint_account.as_ref()),
            B256::left_padding_from(dst_account.as_ref()),
        ],
        &fixed_bytes_from_u256(&evm_balance_from_lamports(amount)),
    );
}

pub fn emit_ut_burn<SDK: SharedAPI, const CHECKED: bool>(
    sdk: &mut SDK,
    src_account: &Pubkey,
    mint_account: &Pubkey,
    amount: u64,
) {
    sdk.emit_log(
        &[
            if CHECKED {
                EVENT_UT_MINT_TO_CHECKED
            } else {
                EVENT_UT_MINT_TO
            },
            B256::left_padding_from(src_account.as_ref()),
            B256::left_padding_from(mint_account.as_ref()),
        ],
        &fixed_bytes_from_u256(&evm_balance_from_lamports(amount)),
    );
}

pub fn emit_ut_close_account<SDK: SharedAPI>(
    sdk: &mut SDK,
    token_account: &Pubkey,
    dst_account: &Pubkey,
    delegate: &Pubkey,
) {
    sdk.emit_log(
        &[
            EVENT_UT_CLOSE_ACCOUNT,
            B256::left_padding_from(token_account.as_ref()),
            B256::left_padding_from(dst_account.as_ref()),
            B256::left_padding_from(delegate.as_ref()),
        ],
        &[],
    );
}

pub fn emit_ut_freeze_account<SDK: SharedAPI, const FREEZE_OR_THAW: bool>(
    sdk: &mut SDK,
    token_account: &Pubkey,
    mint_account: &Pubkey,
    freeze_authority: &Pubkey,
) {
    sdk.emit_log(
        &[
            if FREEZE_OR_THAW {
                EVENT_UT_FREEZE_ACCOUNT
            } else {
                EVENT_UT_THAW_ACCOUNT
            },
            B256::left_padding_from(token_account.as_ref()),
            B256::left_padding_from(mint_account.as_ref()),
            B256::left_padding_from(freeze_authority.as_ref()),
        ],
        &[],
    );
}
