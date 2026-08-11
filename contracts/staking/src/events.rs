//! Solidity-compatible staking event definitions.

use alloc::vec::Vec;
use fluentbase_sdk::{derive::Event, Address, Bytes, B256, U256};

#[derive(Debug, Clone, PartialEq, Eq, Event)]
pub struct ValidatorAdded {
    #[indexed]
    pub validator: Address,
    #[indexed]
    pub owner: Address,
    pub status: u8,
    pub commission_rate: u16,
}

#[derive(Debug, Clone, PartialEq, Eq, Event)]
pub struct ValidatorModified {
    #[indexed]
    pub validator: Address,
    #[indexed]
    pub owner: Address,
    pub status: u8,
    pub commission_rate: u16,
}

#[derive(Debug, Clone, PartialEq, Eq, Event)]
pub struct Delegated {
    #[indexed]
    pub validator: Address,
    #[indexed]
    pub staker: Address,
    pub amount: U256,
    pub epoch: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Event)]
pub struct Undelegated {
    #[indexed]
    pub validator: Address,
    #[indexed]
    pub staker: Address,
    pub amount: U256,
    pub epoch: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Event)]
pub struct ActiveValidatorsLengthChanged {
    pub prev_value: u32,
    pub new_value: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Event)]
pub struct EpochBlockIntervalChanged {
    pub prev_value: u32,
    pub new_value: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Event)]
pub struct DposActivationBlockChanged {
    pub prev_value: u64,
    pub new_value: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Event)]
pub struct FelonyThresholdChanged {
    pub prev_value: u32,
    pub new_value: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Event)]
pub struct ValidatorJailEpochLengthChanged {
    pub prev_value: u32,
    pub new_value: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Event)]
pub struct SlashReporterRewardBpsChanged {
    pub prev_value: u32,
    pub new_value: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Event)]
pub struct SlashFundAddressChanged {
    pub prev_value: Address,
    pub new_value: Address,
}

#[derive(Debug, Clone, PartialEq, Eq, Event)]
pub struct ParticipationFloorBpsChanged {
    pub prev_value: u32,
    pub new_value: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Event)]
pub struct ParticipationJailDisabledChanged {
    pub prev_value: bool,
    pub new_value: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Event)]
pub struct BlendStipendPerEpochChanged {
    pub prev_value: U256,
    pub new_value: U256,
}

#[derive(Debug, Clone, PartialEq, Eq, Event)]
pub struct UndelegatePeriodChanged {
    pub prev_value: u32,
    pub new_value: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Event)]
pub struct MinValidatorStakeAmountChanged {
    pub prev_value: U256,
    pub new_value: U256,
}

#[derive(Debug, Clone, PartialEq, Eq, Event)]
pub struct MinStakingAmountChanged {
    pub prev_value: U256,
    pub new_value: U256,
}

#[derive(Debug, Clone, PartialEq, Eq, Event)]
pub struct BlsVerifierChanged {
    pub prev_value: Address,
    pub new_value: Address,
}

#[derive(Debug, Clone, PartialEq, Eq, Event)]
pub struct EvidenceDecoderChanged {
    pub prev_value: Address,
    pub new_value: Address,
}

#[derive(Debug, Clone, PartialEq, Eq, Event)]
pub struct LivenessSlashingChanged {
    pub prev_value: Address,
    pub new_value: Address,
}

#[derive(Debug, Clone, PartialEq, Eq, Event)]
pub struct BlendReserveChanged {
    pub prev_value: Address,
    pub new_value: Address,
}

#[derive(Event)]
pub struct ValidatorOwnerClaimed {
    #[indexed]
    pub validator: Address,
    pub amount: U256,
    pub epoch: u64,
}

#[derive(Event)]
pub struct Claimed {
    #[indexed]
    pub validator: Address,
    #[indexed]
    pub staker: Address,
    pub amount: U256,
    pub epoch: u64,
}

#[derive(Event)]
pub struct Redelegated {
    #[indexed]
    pub validator: Address,
    #[indexed]
    pub staker: Address,
    pub amount: U256,
    pub dust: U256,
    pub epoch: u64,
}

#[derive(Event)]
pub struct EpochBlendRewardsCommitted {
    #[indexed]
    pub epoch: u64,
    pub blend_amount: U256,
}

#[derive(Event)]
pub struct StipendSkipped {
    #[indexed]
    pub epoch: u64,
}

#[derive(Event)]
pub struct ConsensusKeysSet {
    #[indexed]
    pub validator: Address,
    pub bls_pubkey: Bytes,
    pub peer_pubkey: B256,
    pub activation_epoch: u64,
}

#[derive(Event)]
pub struct EpochCommitteeCommitted {
    #[indexed]
    pub epoch: u64,
    pub committee: Vec<Address>,
}

#[derive(Event)]
pub struct ValidatorReleased {
    #[indexed]
    pub validator: Address,
    pub epoch: u64,
}

#[derive(Event)]
pub struct ValidatorJailed {
    #[indexed]
    pub validator: Address,
    pub epoch: u64,
}

#[derive(Event)]
pub struct ValidatorSlashed {
    #[indexed]
    pub validator: Address,
    pub slashes: u32,
    pub epoch: u64,
}

#[derive(Event)]
pub struct LivenessJailSkippedHaltGuard {
    #[indexed]
    pub validator: Address,
    pub epoch: u64,
    pub active_set_size: U256,
    pub quorum_floor: U256,
}

#[derive(Event)]
pub struct EquivocationReportCommitted {
    #[indexed]
    pub beneficiary: Address,
    #[indexed]
    pub commitment: B256,
    pub block_number: u64,
}

#[derive(Event)]
pub struct EquivocationSlashed {
    #[indexed]
    pub validator: Address,
    pub epoch: u64,
    #[indexed]
    pub reporter: Address,
}

#[derive(Event)]
pub struct EquivocationStakeSeized {
    #[indexed]
    pub validator: Address,
    #[indexed]
    pub reporter: Address,
    pub reporter_reward: U256,
    pub remainder: U256,
    pub recipient: Address,
}
