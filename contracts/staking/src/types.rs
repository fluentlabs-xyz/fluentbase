use alloc::vec::Vec;
use fluentbase_sdk::{codec::Codec, Address, U256};

#[derive(Default, Debug, Codec)]
pub struct InitializeCommand {
    pub initial_owner: Address,
    pub validators: Vec<Address>,
    pub initial_stakes: Vec<U256>,
    pub commission_rate: u16,
}

#[derive(Default, Debug, Codec)]
pub struct AddressCommand {
    pub value: Address,
}

#[derive(Default, Debug, Codec)]
pub struct AddressU16Command {
    pub validator: Address,
    pub value: u16,
}

#[derive(Default, Debug, Codec)]
pub struct TwoAddressesCommand {
    pub validator: Address,
    pub value: Address,
}

#[derive(Default, Debug, Codec)]
pub struct ConfigureCommand {
    pub staking_token: Address,
    pub active_validators_length: u64,
    pub epoch_block_interval: u64,
    pub min_validator_stake_amount: U256,
    pub min_staking_amount: U256,
}
