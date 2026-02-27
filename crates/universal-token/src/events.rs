use fluentbase_sdk::{derive::derive_keccak256, Address, ExitCode, SharedAPI, B256, U256};

pub const EVENT_TRANSFER: B256 = B256::new(derive_keccak256!("Transfer(address,address,uint256)"));
pub const EVENT_APPROVAL: B256 = B256::new(derive_keccak256!("Approval(address,address,uint256)"));
pub const EVENT_PAUSED: B256 = B256::new(derive_keccak256!("Paused(address)"));
pub const EVENT_UNPAUSED: B256 = B256::new(derive_keccak256!("Unpaused(address)"));

pub fn emit_transfer_event<SDK: SharedAPI>(
    sdk: &mut SDK,
    from: &Address,
    to: &Address,
    amount: &U256,
) -> Result<(), ExitCode> {
    sdk.emit_log(
        &[
            EVENT_TRANSFER,
            B256::left_padding_from(from.as_slice()),
            B256::left_padding_from(to.as_slice()),
        ],
        amount.to_be_bytes::<{ U256::BYTES }>(),
    )
    .ok()
}

pub fn emit_pause_event<SDK: SharedAPI>(sdk: &mut SDK, pauser: &Address) -> Result<(), ExitCode> {
    let pauser_padded = B256::left_padding_from(pauser.as_slice());
    sdk.emit_log(&[EVENT_PAUSED], pauser_padded).ok()
}

pub fn emit_unpause_event<SDK: SharedAPI>(sdk: &mut SDK, pauser: &Address) -> Result<(), ExitCode> {
    let pauser_padded = B256::left_padding_from(pauser.as_slice());
    sdk.emit_log(&[EVENT_UNPAUSED], pauser_padded).ok()
}

pub fn emit_approval_event<SDK: SharedAPI>(
    sdk: &mut SDK,
    owner: &Address,
    spender: &Address,
    amount: &U256,
) -> Result<(), ExitCode> {
    sdk.emit_log(
        &[
            EVENT_APPROVAL,
            B256::left_padding_from(owner.as_slice()),
            B256::left_padding_from(spender.as_slice()),
        ],
        amount.to_be_bytes::<{ U256::BYTES }>(),
    )
    .ok()
}
