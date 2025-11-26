use crate::common::fixed_bytes_from_u256;
use crate::services::global_service::global_service;
use fluentbase_sdk::{derive::derive_keccak256, Address, B256, U256};

pub const EVENT_TRANSFER: B256 = B256::new(derive_keccak256!("Transfer(address,address,uint256)"));
pub const EVENT_APPROVAL: B256 = B256::new(derive_keccak256!("Approval(address,address,uint256)"));
pub const EVENT_PAUSED: B256 = B256::new(derive_keccak256!("Paused(address)"));
pub const EVENT_UNPAUSED: B256 = B256::new(derive_keccak256!("Unpaused(address)"));

pub fn emit_transfer_event(from: &Address, to: &Address, amount: &U256) {
    global_service(false).events_add(
        [
            EVENT_TRANSFER.0,
            B256::left_padding_from(from.as_slice()).0,
            B256::left_padding_from(to.as_slice()).0,
        ]
        .into(),
        fixed_bytes_from_u256(amount).to_vec(),
    );
}

pub fn emit_pause_event(pauser: &Address) {
    global_service(false).events_add([EVENT_PAUSED.0].into(), pauser.to_vec());
}

pub fn emit_unpause_event(pauser: &Address) {
    global_service(false).events_add([EVENT_UNPAUSED.0].into(), pauser.to_vec());
}

pub fn emit_approval_event(owner: &Address, spender: &Address, amount: &U256) {
    global_service(false).events_add(
        [
            EVENT_APPROVAL.0,
            B256::left_padding_from(owner.as_slice()).0,
            B256::left_padding_from(spender.as_slice()).0,
        ]
        .into(),
        fixed_bytes_from_u256(amount).to_vec(),
    );
}
