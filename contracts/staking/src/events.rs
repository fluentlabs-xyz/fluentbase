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
    /// First epoch the new cap governs committee selection.
    pub effective_epoch: u64,
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
pub struct SlashFundAddressChanged {
    pub prev_value: Address,
    pub new_value: Address,
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
pub struct BlendReserveChanged {
    pub prev_value: Address,
    pub new_value: Address,
}

#[derive(Debug, Clone, PartialEq, Eq, Event)]
pub struct MinVerdictDueBlocksChanged {
    pub prev_value: u32,
    pub new_value: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Event)]
pub struct ExclusionBackoffCapChanged {
    pub prev_value: u32,
    pub new_value: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Event)]
pub struct ProductionLivenessDisabledChanged {
    pub prev_value: bool,
    pub new_value: bool,
}

#[derive(Event)]
pub struct ProductionExclusionApplied {
    #[indexed]
    pub validator: Address,
    pub bite_epoch: u64,
}

#[derive(Event)]
pub struct ProductionExclusionReleased {
    #[indexed]
    pub validator: Address,
    pub bite_epoch: u64,
}

/// The weight ring has wrapped past `epoch`, so it can neither be judged nor
/// paid.
///
/// Distinct from `PartialEpoch` on purpose. That one means blocks went missing,
/// which is the *cause* a ring miss would be a *consequence* of; conflating them
/// makes the alert unreadable.
///
/// This should be unreachable. The close reads at a lag of one while the ring
/// holds `WEIGHT_RING_EPOCHS`, so reaching it means the bound behind that
/// constant is wrong. Hence forfeit-and-announce rather than a graceful degraded
/// mode: a mode for a state that should not exist is a mode nobody will debug.
/// Reverting would be worse — the close is a pre-execution system call, so a
/// propagated error is a chain halt no transaction can repair — and a silent
/// zero worse than both, being indistinguishable from an epoch that legitimately
/// paid nothing.
///
/// Node-side, this rides `recordProduction`, whose logs ARE forwarded to
/// `emit_close_observability`; but that is a closed signature match, so the
/// event is mute until an arm is added for it.
#[derive(Event)]
pub struct EpochWeightsUnavailable {
    #[indexed]
    pub epoch: u64,
    pub members: u32,
}

/// Mandatory rather than diagnostic: the partial-epoch taint is derived from
/// the block count instead of stored, so one unrecorded block silently costs a
/// whole epoch its verdicts while the tier still reads as enabled.
#[derive(Event)]
pub struct PartialEpoch {
    #[indexed]
    pub epoch: u64,
    pub recorded: u32,
    pub expected: u32,
}

/// Emitted per failing member whether or not a stamp follows; a verdict without
/// a stamp is the normal case.
#[derive(Event)]
pub struct ProductionVerdictFailed {
    #[indexed]
    pub epoch: u64,
    #[indexed]
    pub validator: Address,
    pub produced: u32,
    pub due: U256,
}

/// More than `f` members failing for the first time in one epoch reads as an
/// environment rather than as individual faults. No stamps that close.
#[derive(Event)]
pub struct CorrelatedFailureEpoch {
    #[indexed]
    pub epoch: u64,
    pub new_failures: U256,
    pub tolerance: U256,
}

/// The liveness legs of this close have already committed, and so has the
/// accrual; only the payment was lost. The reward cursor did not advance, so the
/// next close retries contiguously and the epochs it skipped are still owed.
#[derive(Event)]
pub struct StipendLegSkipped {
    #[indexed]
    pub epoch: u64,
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

/// The selection for `epoch` produced fewer than `MIN_COMMITTEE_LENGTH`
/// eligible validators, so `epoch` re-seats the committee of `epoch - 1`
/// verbatim instead of reverting.
///
/// Emitted INSTEAD OF `EpochCommitteeCommitted`, never beside it: no committee
/// was derived, and a reader that treated the two as interchangeable would
/// record a fresh commit that did not happen.
///
/// This is the state the chain must be steered out of, not an error: `eligible`
/// says how far the visible population has fallen below the floor, and the seats
/// stay filled by validators the selection would no longer choose — some of them
/// possibly retired, tombstoned or unstaked. It clears by itself the moment
/// enough validators are registered, keyed and activated again, because the next
/// commit then derives a full set. Until then the seated set is frozen, so this
/// event firing epoch after epoch is the operator's cue to add validators.
///
/// `dkgQual[epoch]` is written `false` alongside it — the committee genuinely
/// did not change, which is what lets the beacon carry its key forward instead
/// of trying to re-mint one over a set it cannot assemble.
#[derive(Event)]
pub struct CommitteeCarriedOver {
    #[indexed]
    pub epoch: u64,
    /// How many validators the selection actually produced (`< MIN_COMMITTEE_LENGTH`).
    pub eligible: u32,
    /// How many seats the carried committee has.
    pub members: u32,
}

#[derive(Event)]
pub struct ValidatorJailed {
    #[indexed]
    pub validator: Address,
    pub epoch: u64,
}

#[derive(Event)]
pub struct EquivocationSlashed {
    #[indexed]
    pub validator: Address,
    /// The epoch the conflict happened in, not the epoch the penalty is stamped
    /// at — the two differ whenever a charge lands after its own epoch.
    pub epoch: u64,
}

#[derive(Event)]
pub struct EquivocationStakeSeized {
    #[indexed]
    pub validator: Address,
    /// What actually moved, not what was owed: a recipient that refuses the
    /// transfer leaves this zero rather than reverting the slash.
    pub seized: U256,
    pub recipient: Address,
}
