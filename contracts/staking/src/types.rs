//! Internal representations of Solidity ABI commands and dependency results.

use alloc::vec::Vec;
use fluentbase_sdk::{
    byteorder::BE,
    codec::{Codec, FunctionArgs},
    Address, Bytes, B256, U256,
};

#[derive(Default, Debug, Codec)]
pub struct InitializeCommand {
    pub initial_owner: Address,
    pub validators: Vec<Address>,
    pub initial_stakes: Vec<U256>,
    pub bls_pubkeys_uncompressed: Vec<Bytes>,
    pub bls_pops_uncompressed: Vec<Bytes>,
    pub peer_pubkeys: Vec<B256>,
    pub commission_rate: u16,
    pub staking_token: Address,
    pub active_validators_length: u32,
    pub epoch_block_interval: u32,
    pub felony_threshold: u32,
    pub validator_jail_epoch_length: u32,
    pub undelegate_period: u32,
    pub min_validator_stake_amount: U256,
    pub min_staking_amount: U256,
    pub dpos_activation_block: u64,
    pub bls_verifier: Address,
    pub evidence_decoder: Address,
    pub min_undelegate_blocks: U256,
    pub liveness_slashing: Address,
    pub blend_reserve: Address,
}

impl FunctionArgs<BE, 32, true, false> for InitializeCommand {}

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
pub struct ValidatorDelegatorCommand {
    pub validator: Address,
    pub delegator: Address,
}

#[derive(Default, Debug, Codec)]
pub struct AddressAmountCommand {
    pub validator: Address,
    pub amount: U256,
}

#[derive(Default, Debug, Codec)]
pub struct ValidatorBlockCommand {
    pub validator: Address,
    pub block_number: U256,
}

#[derive(Default, Debug, Codec)]
pub struct ValidatorEpochCommand {
    pub validator: Address,
    pub before_epoch: u64,
}

#[derive(Default, Debug, Codec)]
pub struct AddValidatorCommand {
    pub validator: Address,
    pub bls_pubkey_uncompressed: Bytes,
    pub bls_pop_uncompressed: Bytes,
    pub peer_pubkey: B256,
}

impl FunctionArgs<BE, 32, true, false> for AddValidatorCommand {}

#[derive(Default, Debug, Codec)]
pub struct RegisterValidatorCommand {
    pub validator: Address,
    pub commission_rate: u16,
    pub initial_stake: U256,
    pub bls_pubkey_uncompressed: Bytes,
    pub bls_pop_uncompressed: Bytes,
    pub peer_pubkey: B256,
}

impl FunctionArgs<BE, 32, true, false> for RegisterValidatorCommand {}

#[derive(Default, Debug, Codec)]
pub struct U32Command {
    pub value: u32,
}

#[derive(Default, Debug, Codec)]
pub struct U64Command {
    pub value: u64,
}

#[derive(Default, Debug, Codec)]
pub struct U256Command {
    pub value: U256,
}

#[derive(Default, Debug, Codec)]
pub struct BoolCommand {
    pub value: bool,
}

#[derive(Default, Debug, Clone, PartialEq, Eq, Codec)]
pub struct ConsensusKeys {
    pub bls_pubkey: Bytes,
    pub peer_pubkey: B256,
    pub activation_epoch: u64,
}

#[derive(Default, Debug, Codec)]
pub struct EpochSignerCommand {
    pub epoch: u64,
    pub signer_idx: u32,
}

#[derive(Default, Debug, Codec)]
pub struct EquivocationCommand {
    pub evidence: Bytes,
    pub pk_uncompressed: Bytes,
    pub sig1_uncompressed: Bytes,
    pub sig2_uncompressed: Bytes,
    pub beneficiary: Address,
    pub salt: B256,
}

#[derive(Default, Debug, Codec)]
pub struct DecodedEvidence {
    pub epoch: u64,
    pub signer_idx: u32,
    pub kind1: u8,
    pub msg1: Bytes,
    pub sig1: Bytes,
    pub kind2: u8,
    pub msg2: Bytes,
    pub sig2: Bytes,
}
