//! Canonical ABI selectors, error IDs, protocol limits, and default values.

use fluentbase_sdk::{
    address,
    derive::{derive_keccak256_id, erc7201_slot},
    uint, Address, U256,
};

pub const SIG_LEN_BYTES: usize = 4;

pub const STATUS_NOT_FOUND: u8 = 0;
pub const STATUS_ACTIVE: u8 = 1;
pub const STATUS_PENDING: u8 = 2;
pub const STATUS_JAIL: u8 = 3;

// ABI selectors are derived from their canonical signatures. The pinned hex
// values remain beside them to make ABI drift visible during review.

// 0xdfa8efb0
pub const SIG_INITIALIZE: u32 =
    derive_keccak256_id!(
        "initialize(address,address[],uint256[],bytes[],bytes[],bytes32[],uint16,address,uint32,uint32,uint32,uint256,uint256,uint64,address,uint256,address)"
    );
// 0x76671808
pub const SIG_CURRENT_EPOCH: u32 = derive_keccak256_id!("currentEpoch()");
// 0xaea0e78b
pub const SIG_NEXT_EPOCH: u32 = derive_keccak256_id!("nextEpoch()");
// 0xfacd743b
pub const SIG_IS_VALIDATOR: u32 = derive_keccak256_id!("isValidator(address)");
// 0x42ad55ac
pub const SIG_IS_VALIDATOR_ACTIVE: u32 = derive_keccak256_id!("isValidatorActive(address)");
// 0xa310624f
pub const SIG_GET_VALIDATOR_STATUS: u32 = derive_keccak256_id!("getValidatorStatus(address)");
// 0x30108c22
pub const SIG_GET_VALIDATOR_BY_OWNER: u32 = derive_keccak256_id!("getValidatorByOwner(address)");
// 0xb7ab4db5
pub const SIG_GET_VALIDATORS: u32 = derive_keccak256_id!("getValidators()");
// 0xb46e5520
pub const SIG_ACTIVATE_VALIDATOR: u32 = derive_keccak256_id!("activateValidator(address)");
// 0x1fe97684
pub const SIG_DISABLE_VALIDATOR: u32 = derive_keccak256_id!("disableValidator(address)");
// 0x14f8649f
pub const SIG_CHANGE_VALIDATOR_COMMISSION_RATE: u32 =
    derive_keccak256_id!("changeValidatorCommissionRate(address,uint16)");
// 0x0052c9e1
pub const SIG_CHANGE_VALIDATOR_OWNER: u32 =
    derive_keccak256_id!("changeValidatorOwner(address,address)");
// 0x9f9106d1
pub const SIG_GET_STAKING_TOKEN: u32 = derive_keccak256_id!("getStakingToken()");
// 0x32cc6f08
pub const SIG_GET_ACTIVE_VALIDATORS_LENGTH: u32 =
    derive_keccak256_id!("getActiveValidatorsLength()");
// 0x346c90a8
pub const SIG_GET_EPOCH_BLOCK_INTERVAL: u32 = derive_keccak256_id!("getEpochBlockInterval()");
// 0xa2a50528
pub const SIG_GET_DPOS_ACTIVATION_BLOCK: u32 = derive_keccak256_id!("getDposActivationBlock()");
// 0x5e7b72ad
pub const SIG_GET_UNDELEGATE_PERIOD: u32 = derive_keccak256_id!("getUndelegatePeriod()");
// 0x6f856847
pub const SIG_GET_MIN_VALIDATOR_STAKE_AMOUNT: u32 =
    derive_keccak256_id!("getMinValidatorStakeAmount()");
// 0xeea9a01b
pub const SIG_GET_MIN_STAKING_AMOUNT: u32 = derive_keccak256_id!("getMinStakingAmount()");
// 0xd951e186
pub const SIG_GET_VALIDATOR_DELEGATION: u32 =
    derive_keccak256_id!("getValidatorDelegation(address,address)");
// 0xe8810ea7
pub const SIG_GET_VALIDATOR_DELEGATED_STAKE_AT: u32 =
    derive_keccak256_id!("getValidatorDelegatedStakeAt(address,uint256)");
// 0x8d6067ed
pub const SIG_REGISTER_VALIDATOR: u32 =
    derive_keccak256_id!("registerValidator(address,uint16,uint256,bytes,bytes,bytes32)");
// 0x026e402b
pub const SIG_DELEGATE: u32 = derive_keccak256_id!("delegate(address,uint256)");
// 0x4d99dd16
pub const SIG_UNDELEGATE: u32 = derive_keccak256_id!("undelegate(address,uint256)");
// 0x5d887462
pub const SIG_MAX_ACTIVE_VALIDATORS: u32 = derive_keccak256_id!("MAX_ACTIVE_VALIDATORS()");
// 0x2bc2fec4
pub const SIG_MAX_BLEND_STIPEND_PER_EPOCH: u32 =
    derive_keccak256_id!("MAX_BLEND_STIPEND_PER_EPOCH()");
// 0x6fd3afb7
pub const SIG_DEFAULT_MIN_VERDICT_DUE_BLOCKS: u32 =
    derive_keccak256_id!("DEFAULT_MIN_VERDICT_DUE_BLOCKS()");
// 0xd4c30c1a
pub const SIG_DEFAULT_EXCLUSION_BACKOFF_CAP: u32 =
    derive_keccak256_id!("DEFAULT_EXCLUSION_BACKOFF_CAP()");
// 0x9b9a11ba
pub const SIG_MAX_MIN_VERDICT_DUE_BLOCKS: u32 =
    derive_keccak256_id!("MAX_MIN_VERDICT_DUE_BLOCKS()");
// 0x23b872dd
pub const SIG_ERC20_TRANSFER_FROM: u32 =
    derive_keccak256_id!("transferFrom(address,address,uint256)");
// 0xa9059cbb
pub const SIG_ERC20_TRANSFER: u32 = derive_keccak256_id!("transfer(address,uint256)");
// 0x70a08231
pub const SIG_ERC20_BALANCE_OF: u32 = derive_keccak256_id!("balanceOf(address)");
// 0xdd62ed3e
pub const SIG_ERC20_ALLOWANCE: u32 = derive_keccak256_id!("allowance(address,address)");
// 0xc910df38
pub const SIG_GET_SLASH_FUND_ADDRESS: u32 = derive_keccak256_id!("getSlashFundAddress()");
// 0xa79e7263
pub const SIG_SET_SLASH_FUND_ADDRESS: u32 = derive_keccak256_id!("setSlashFundAddress(address)");
// 0xc8f45d87
pub const SIG_GET_BLEND_STIPEND_PER_EPOCH: u32 = derive_keccak256_id!("getBlendStipendPerEpoch()");
// 0x2c91b879
pub const SIG_SET_BLEND_STIPEND_PER_EPOCH: u32 =
    derive_keccak256_id!("setBlendStipendPerEpoch(uint256)");
// 0xc227a412
pub const SIG_SET_ACTIVE_VALIDATORS_LENGTH: u32 =
    derive_keccak256_id!("setActiveValidatorsLength(uint32)");
// 0xaf70fa2c
pub const SIG_SET_EPOCH_BLOCK_INTERVAL: u32 = derive_keccak256_id!("setEpochBlockInterval(uint32)");
// 0xf517ca6a
pub const SIG_SET_DPOS_ACTIVATION_BLOCK: u32 =
    derive_keccak256_id!("setDposActivationBlock(uint64)");
// 0x41d8a080
pub const SIG_SET_UNDELEGATE_PERIOD: u32 = derive_keccak256_id!("setUndelegatePeriod(uint32)");
// 0xe1a2e863
pub const SIG_SET_MIN_VALIDATOR_STAKE_AMOUNT: u32 =
    derive_keccak256_id!("setMinValidatorStakeAmount(uint256)");
// 0x612d669e
pub const SIG_SET_MIN_STAKING_AMOUNT: u32 = derive_keccak256_id!("setMinStakingAmount(uint256)");
// 0xc6b904ad
pub const SIG_GET_BLS_VERIFIER: u32 = derive_keccak256_id!("getBlsVerifier()");
// 0x466ae541
pub const SIG_SET_BLS_VERIFIER: u32 = derive_keccak256_id!("setBlsVerifier(address)");
// 0x37dff538
pub const SIG_GET_BLEND_RESERVE: u32 = derive_keccak256_id!("getBlendReserve()");
// 0x7899ae8f
pub const SIG_SET_BLEND_RESERVE: u32 = derive_keccak256_id!("setBlendReserve(address)");
// 0xee3ad0e7
pub const SIG_GET_MIN_VERDICT_DUE_BLOCKS: u32 = derive_keccak256_id!("getMinVerdictDueBlocks()");
// 0x4fae9dea
pub const SIG_SET_MIN_VERDICT_DUE_BLOCKS: u32 =
    derive_keccak256_id!("setMinVerdictDueBlocks(uint32)");
// 0x6bed0322
pub const SIG_GET_EXCLUSION_BACKOFF_CAP: u32 = derive_keccak256_id!("getExclusionBackoffCap()");
// 0x3b543e1c
pub const SIG_SET_EXCLUSION_BACKOFF_CAP: u32 =
    derive_keccak256_id!("setExclusionBackoffCap(uint32)");
// 0x9a4c46bb
pub const SIG_GET_PRODUCTION_LIVENESS_DISABLED: u32 =
    derive_keccak256_id!("getProductionLivenessDisabled()");
// 0x8fc07556
pub const SIG_SET_PRODUCTION_LIVENESS_DISABLED: u32 =
    derive_keccak256_id!("setProductionLivenessDisabled(bool)");
// 0xf06be669
#[cfg(feature = "devnet-views")]
pub const SIG_BLOCKS_IN_EPOCH: u32 = derive_keccak256_id!("blocksInEpoch(uint64)");
// 0x91c7d453
#[cfg(feature = "devnet-views")]
pub const SIG_PRODUCED_AT: u32 = derive_keccak256_id!("producedAt(uint64,uint32)");
// 0xaef690f9
#[cfg(feature = "devnet-views")]
pub const SIG_PENDING_EXCLUSIONS: u32 = derive_keccak256_id!("pendingExclusions()");
// 0x33de61d2
#[cfg(feature = "devnet-views")]
pub const SIG_LAST_PROCESSED_BLOCK: u32 = derive_keccak256_id!("lastProcessedBlock()");
// 0x1752910e
pub const SIG_RECORD_PRODUCTION: u32 = derive_keccak256_id!("recordProduction(uint8)");
// 0x457179fd
pub const SIG_GET_VALIDATOR_FEE: u32 = derive_keccak256_id!("getValidatorFee(address)");
// 0xc6fb9065
pub const SIG_GET_PENDING_VALIDATOR_FEE: u32 =
    derive_keccak256_id!("getPendingValidatorFee(address)");
// 0xff4794fc
pub const SIG_CLAIM_VALIDATOR_FEE: u32 = derive_keccak256_id!("claimValidatorFee(address)");
// 0xadf2a79c
pub const SIG_CLAIM_VALIDATOR_FEE_AT_EPOCH: u32 =
    derive_keccak256_id!("claimValidatorFeeAtEpoch(address,uint64)");
// 0x52b7bea2
pub const SIG_GET_DELEGATOR_FEE: u32 = derive_keccak256_id!("getDelegatorFee(address,address)");
// 0xc2fd58fc
pub const SIG_GET_PENDING_DELEGATOR_FEE: u32 =
    derive_keccak256_id!("getPendingDelegatorFee(address,address)");
// 0x426594b1
pub const SIG_CLAIM_DELEGATOR_FEE: u32 = derive_keccak256_id!("claimDelegatorFee(address)");
// 0xfe38ebef
pub const SIG_CLAIM_DELEGATOR_FEE_AT_EPOCH: u32 =
    derive_keccak256_id!("claimDelegatorFeeAtEpoch(address,uint64)");
// 0xa789083d
pub const SIG_GET_DELEGATOR_PRINCIPAL: u32 =
    derive_keccak256_id!("getDelegatorPrincipal(address,address)");
// 0xe75f359c
pub const SIG_WITHDRAW_DELEGATOR_PRINCIPAL: u32 =
    derive_keccak256_id!("withdrawDelegatorPrincipal(address)");
// 0x5ef9e8c6
pub const SIG_CALC_AVAILABLE_FOR_REDELEGATE_AMOUNT: u32 =
    derive_keccak256_id!("calcAvailableForRedelegateAmount(address,address)");
// 0x8ecb3fc9
pub const SIG_REDELEGATE_DELEGATOR_FEE: u32 =
    derive_keccak256_id!("redelegateDelegatorFee(address)");
// 0x54c3e84b
pub const SIG_GET_EPOCH_REWARDS: u32 = derive_keccak256_id!("getEpochRewards(uint64)");
// 0xad36f42f
pub const SIG_GET_CONSENSUS_KEYS: u32 = derive_keccak256_id!("getConsensusKeys(address)");
// 0xd41c52eb
pub const SIG_GET_VALIDATORS_WITH_KEYS: u32 = derive_keccak256_id!("getValidatorsWithKeys()");
// 0xd96cbd7b
pub const SIG_GET_REGISTRY_WITH_KEYS: u32 = derive_keccak256_id!("getRegistryWithKeys()");
// 0xc06a82de
pub const SIG_NEXT_EPOCH_TO_COMMIT: u32 = derive_keccak256_id!("nextEpochToCommit()");
// 0xe505b249
pub const SIG_COMMIT_EPOCH_COMMITTEE: u32 = derive_keccak256_id!("commitEpochCommittee()");
// 0x2660899f
pub const SIG_GET_DKG_QUAL: u32 = derive_keccak256_id!("getDkgQual(uint64)");
// 0x80b562de
pub const SIG_GET_EPOCH_COMMITTEE: u32 = derive_keccak256_id!("getEpochCommittee(uint64)");
// 0xa4d160c1
pub const SIG_GET_EPOCH_COMMITTEE_WITH_STAKES: u32 =
    derive_keccak256_id!("getEpochCommitteeWithStakes(uint64)");
// 0xa5d2dd22
pub const SIG_BLS_COMPRESS_G2_UNCHECKED: u32 = derive_keccak256_id!("compressG2Unchecked(bytes)");
// 0x8bf26133
pub const SIG_BLS_VERIFY: u32 = derive_keccak256_id!("verify(bytes,bytes,bytes,bytes,bytes)");
// 0xdc6fb3f2
pub const SIG_SLASH_EQUIVOCATION: u32 = derive_keccak256_id!("slashEquivocation(uint64,uint32)");
// 0xe28d2f63
pub const SIG_SLASH_EQUIVOCATION_NOTARIZE: u32 =
    derive_keccak256_id!("slashEquivocationNotarize(bytes,bytes,bytes,bytes)");
// 0xadd07a3e
pub const SIG_SLASH_EQUIVOCATION_FINALIZE: u32 =
    derive_keccak256_id!("slashEquivocationFinalize(bytes,bytes,bytes,bytes)");
// 0xa10827e9
pub const SIG_SLASH_EQUIVOCATION_NULLIFY_FINALIZE: u32 =
    derive_keccak256_id!("slashEquivocationNullifyFinalize(bytes,bytes,bytes,bytes)");
// 0x8f498050
pub const SIG_BLS_COMPRESS_G1_UNCHECKED: u32 = derive_keccak256_id!("compressG1Unchecked(bytes)");

pub const ERR_ALREADY_INITIALIZED: u32 = derive_keccak256_id!("InvalidInitialization()");
pub const ERR_NOT_INITIALIZED: u32 = derive_keccak256_id!("NotInitialized()");
pub const ERR_ONLY_GOVERNANCE: u32 = derive_keccak256_id!("OnlyGovernance()");
pub const ERR_ZERO_OWNER: u32 = derive_keccak256_id!("ZeroOwner()");
pub const ERR_ZERO_VALIDATOR: u32 = derive_keccak256_id!("ZeroValidator()");
pub const ERR_MALFORMED_INPUT_LENGTH: u32 = derive_keccak256_id!("MalformedInputLength()");
pub const ERR_WRONG_AMOUNT_PRECISION: u32 = derive_keccak256_id!("WrongAmountPrecision()");
pub const ERR_BAD_COMMISSION_RATE: u32 = derive_keccak256_id!("BadCommissionRate(uint16)");
pub const ERR_VALIDATOR_ALREADY_EXISTS: u32 =
    derive_keccak256_id!("ValidatorAlreadyExists(address)");
pub const ERR_VALIDATOR_NOT_FOUND: u32 = derive_keccak256_id!("ValidatorNotFound(address)");
pub const ERR_VALIDATOR_TOMBSTONED: u32 = derive_keccak256_id!("ValidatorTombstoned(address)");
pub const ERR_VALIDATOR_OWNER_ALREADY_IN_USE: u32 =
    derive_keccak256_id!("ValidatorOwnerAlreadyInUse(address)");
pub const ERR_NOT_PENDING_VALIDATOR: u32 = derive_keccak256_id!("NotPendingValidator(address)");
pub const ERR_NOT_ACTIVE_VALIDATOR: u32 = derive_keccak256_id!("NotActiveValidator()");
pub const ERR_ONLY_VALIDATOR_OWNER: u32 = derive_keccak256_id!("OnlyValidatorOwner(address)");
pub const ERR_VALIDATOR_OWNER_IMMUTABLE: u32 = derive_keccak256_id!("ValidatorOwnerImmutable()");
pub const ERR_ZERO_STAKING_TOKEN: u32 = derive_keccak256_id!("ZeroStakingToken()");
pub const ERR_INVALID_CHAIN_CONFIG: u32 = derive_keccak256_id!("InvalidChainConfig()");
pub const ERR_AMOUNT_TOO_LOW: u32 = derive_keccak256_id!("AmountTooLow(uint256)");
pub const ERR_REMAINING_DELEGATION_TOO_LOW: u32 =
    derive_keccak256_id!("RemainingDelegationTooLow(uint256,uint256)");
pub const ERR_INITIAL_STAKE_TOO_LOW: u32 = derive_keccak256_id!("InitialStakeTooLow(uint256)");
pub const ERR_OWNER_SELF_STAKE_BELOW_MINIMUM: u32 =
    derive_keccak256_id!("OwnerSelfStakeBelowMinimum()");
pub const ERR_ZERO_OWNER_SELF_STAKE: u32 = derive_keccak256_id!("ZeroOwnerSelfStake()");
pub const ERR_INSUFFICIENT_BALANCE: u32 = derive_keccak256_id!("InsufficientBalance()");
pub const ERR_DELEGATION_QUEUE_EMPTY: u32 = derive_keccak256_id!("DelegationQueueEmpty()");
pub const ERR_DELEGATION_QUEUE_NOT_EMPTY: u32 =
    derive_keccak256_id!("DelegationQueueNotEmpty(uint256)");
pub const ERR_PENDING_DELEGATION: u32 = derive_keccak256_id!("PendingDelegation(uint64)");
pub const ERR_STAKING_TOKEN_CALL_FAILED: u32 = derive_keccak256_id!("StakingTokenCallFailed()");
pub const ERR_UNKNOWN_METHOD: u32 = derive_keccak256_id!("UnknownMethod()");
pub const ERR_ONLY_SYSTEM_CALL: u32 = derive_keccak256_id!("OnlySystemCall()");
pub const ERR_ZERO_VALUE: u32 = derive_keccak256_id!("ZeroValue(string)");
pub const ERR_ACTIVE_VALIDATORS_LENGTH_BELOW_COMMITTEE_FLOOR: u32 =
    derive_keccak256_id!("ActiveValidatorsLengthBelowCommitteeFloor(uint32,uint32)");
pub const ERR_MAX_ACTIVE_VALIDATORS_EXCEEDED: u32 =
    derive_keccak256_id!("MaxActiveValidatorsExceeded(uint32,uint32)");
pub const ERR_DPOS_ALREADY_ACTIVE: u32 = derive_keccak256_id!("DposAlreadyActive()");
pub const ERR_UNALIGNED_ACTIVATION_BLOCK: u32 = derive_keccak256_id!("UnalignedActivationBlock()");
pub const ERR_ACTIVATION_BLOCK_IN_PAST: u32 = derive_keccak256_id!("ActivationBlockInPast()");
pub const ERR_UNDELEGATE_WINDOW_TOO_SHORT: u32 =
    derive_keccak256_id!("UndelegateWindowTooShort(uint256,uint256)");
pub const ERR_BLEND_STIPEND_PER_EPOCH_TOO_HIGH: u32 =
    derive_keccak256_id!("BlendStipendPerEpochTooHigh(uint256,uint256)");
pub const ERR_MIN_VERDICT_DUE_BLOCKS_TOO_HIGH: u32 =
    derive_keccak256_id!("MinVerdictDueBlocksTooHigh(uint32,uint32)");
pub const ERR_INVALID_CLAIM_EPOCH: u32 = derive_keccak256_id!("InvalidClaimEpoch()");
pub const ERR_CONSENSUS_KEYS_ALREADY_SET: u32 =
    derive_keccak256_id!("ConsensusKeysAlreadySet(address)");
pub const ERR_PEER_PUBKEY_ALREADY_IN_USE: u32 =
    derive_keccak256_id!("PeerPubkeyAlreadyInUse(bytes32)");
pub const ERR_BLS_PUBKEY_ALREADY_IN_USE: u32 =
    derive_keccak256_id!("BlsPubkeyAlreadyInUse(bytes32)");
pub const ERR_CONSENSUS_KEYS_NOT_SET: u32 = derive_keccak256_id!("ConsensusKeysNotSet(address)");
pub const ERR_EPOCH_COMMITTEE_NOT_COMMITTED: u32 =
    derive_keccak256_id!("EpochCommitteeNotCommitted(uint64)");
pub const ERR_SIGNER_INDEX_OUT_OF_RANGE: u32 =
    derive_keccak256_id!("SignerIndexOutOfRange(uint64,uint32,uint256)");
pub const ERR_COMMITTEE_TOO_SMALL: u32 = derive_keccak256_id!("CommitteeTooSmall(uint256,uint256)");
/// A committee longer than one weight-ring frame holds.
///
/// An assertion, not a condition the contract expects to meet: the cap is
/// enforced at `initialize` and by `setActiveValidatorsLength`, and the
/// selection truncates to it, so `MAX_ACTIVE_VALIDATORS_LENGTH` already bounds
/// this two layers up. It is checked again here because the failure it prevents
/// is the one this storage design exists to make unrepresentable — an
/// over-long committee writes past its frame into the NEXT epoch's weights,
/// under this epoch's fresh stamp, and every read of both epochs then answers
/// confidently and wrongly. A halted chain is recoverable; that is not.
pub const ERR_COMMITTEE_EXCEEDS_WEIGHT_RING: u32 =
    derive_keccak256_id!("CommitteeExceedsWeightRing(uint256,uint256)");
pub const ERR_EPOCH_NOT_YET_COMMITTABLE: u32 =
    derive_keccak256_id!("EpochNotYetCommittable(uint64,uint64)");
pub const ERR_ALREADY_SLASHED_FOR_EQUIVOCATION: u32 =
    derive_keccak256_id!("AlreadySlashedForEquivocation(address)");
pub const ERR_BLS_VERIFIER_NOT_CONFIGURED: u32 = derive_keccak256_id!("BlsVerifierNotConfigured()");
pub const ERR_INVALID_PROOF_OF_POSSESSION: u32 =
    derive_keccak256_id!("InvalidProofOfPossession(address)");
pub const ERR_INVALID_CONSENSUS_KEY_ENCODING: u32 =
    derive_keccak256_id!("InvalidConsensusKeyEncoding()");
pub const ERR_EQUIVOCATION_SIGNATURE_INVALID: u32 =
    derive_keccak256_id!("EquivocationSignatureInvalid()");
pub const ERR_EQUIVOCATION_KEY_NOT_REGISTERED: u32 =
    derive_keccak256_id!("EquivocationKeyNotRegistered()");
pub const ERR_INVALID_EVIDENCE_ENCODING: u32 = derive_keccak256_id!("InvalidEvidenceEncoding()");
pub const ERR_EVIDENCE_SIGNER_MISMATCH: u32 =
    derive_keccak256_id!("EvidenceSignerMismatch(uint32,uint32)");
pub const ERR_EVIDENCE_ROUND_MISMATCH: u32 =
    derive_keccak256_id!("EvidenceRoundMismatch(uint64,uint64,uint64,uint64)");
pub const ERR_EVIDENCE_PROPOSALS_IDENTICAL: u32 =
    derive_keccak256_id!("EvidenceProposalsIdentical(uint64,uint64)");

/// Scale of every rate expressed in basis points: `10_000` bps == 100%.
///
/// `COMMISSION_RATE_MAX` is meaningless without it, and the site that applies a
/// rate divides by it.
pub const BPS_DENOMINATOR: u32 = 10_000;

/// Wei per compact stake unit: `10^10`.
///
/// Stake is stored as a `uint112` count of these units, so any amount that is
/// not a whole multiple of it cannot be represented and is rejected by
/// `math::compact_balance`.
pub const BALANCE_COMPACT_PRECISION: U256 = uint!(10_000_000_000_U256);

/// Highest commission a validator may charge its delegators: 3000 bps == 30%.
///
/// A policy figure, not a derived one: nothing in this crate breaks at a
/// different value. No derivation is recorded for the choice of 30%.
pub const COMMISSION_RATE_MAX: u16 = 3_000;

/// Reported as `prev_value` in the `EpochBlockIntervalChanged` init event.
///
/// Not a default: `apply_initial_config` always writes
/// `InitializeCommand::epoch_block_interval` to storage, and this constant is
/// only the "before" side of the event that records it.
pub const DEFAULT_EPOCH_BLOCK_INTERVAL: u64 = 200;

/// Reported as `prev_value` in the `ActiveValidatorsLengthChanged` init event.
///
/// Not a default — see [`DEFAULT_EPOCH_BLOCK_INTERVAL`].
pub const DEFAULT_ACTIVE_VALIDATORS_LENGTH: u64 = 21;

/// Hard ceiling on the configured active-set size, enforced at initialization
/// and by `setActiveValidatorsLength`.
///
/// It is what bounds every committee-wide loop in this crate — committee
/// commit, the liveness verdict pass, the stipend split — so it is the figure
/// that keeps those loops payable. No derivation is recorded for 51, and it has
/// not been measured against rWasm fuel on this runtime.
pub const MAX_ACTIVE_VALIDATORS_LENGTH: u64 = 51;

/// Epochs of frozen leader weights the ring buffer keeps.
///
/// Membership is retained forever; weights are not, because they change every
/// epoch while membership changes on an event. The ring bounds them by
/// construction — slot `E mod N` — so nothing has to be deleted and the cost
/// stops growing.
///
/// **The bound this has to satisfy is not the one first written down.** Epoch E
/// stays readable while `current − E < N − 2`: the close reads at a lag of one,
/// and the frame ahead is already claimed. Two arguments for a large N were
/// offered and both are false — a run of parked blocks does not lengthen the
/// close's lag (`last_processed_block` and `close_epoch` both run above the park
/// arm), and a commit-less epoch is not ring-write-less (the node's drain runs
/// unconditionally after the recorder). What actually consumes the margin is
/// produced-but-**unrecorded** blocks.
///
/// At 16 that is 13 epochs, roughly 1.12 M consecutive unrecorded blocks at an
/// 86,400-block epoch. The conclusion outlived both of its stated reasons, which
/// is recorded rather than tidied away.
pub const WEIGHT_RING_EPOCHS: u64 = 16;

/// Ring pair-slots one epoch occupies: two members' weights and their shared
/// epoch stamp pack into one 32-byte slot (`14 + 14 + 4`).
pub const PAIRS_MAX: usize = (MAX_ACTIVE_VALIDATORS_LENGTH as usize).div_ceil(2);

/// Total ring length. Sized against the cap rather than the launch
/// configuration, deliberately: `commitEpochCommittee` is a system call that
/// must not fail, so it is judged on its worst case.
pub const WEIGHT_RING_SLOTS: usize = WEIGHT_RING_EPOCHS as usize * PAIRS_MAX;

/// Smallest committee `commitEpochCommittee` will accept.
///
/// Four is the smallest `n` for which the Simplex fault tolerance
/// `math::fault_tolerance` returns `f >= 1`, so it is the smallest committee
/// that tolerates a single fault. The previous value of one was not a safety
/// floor at all: at `n = 1` the tolerance is zero, and the only state it ruled
/// out was an empty committee.
///
/// The chain is assumed always to have at least this many eligible validators —
/// the operator maintains a baseline set that can carry the network — so this
/// asserts that assumption rather than testing a condition the contract expects
/// to meet. Failing it stops the chain: the commit is a pre-execution system
/// call, so the revert is a block-execution error on every node, and it lands
/// before any transaction in the block runs — nothing can repair the state
/// afterwards.
///
/// The one way to break the assumption by configuration rather than by
/// circumstance is a committee cap below this floor, since the cap truncates the
/// selection. `setActiveValidatorsLength` refuses it — that is the unrecoverable
/// direction, because it lands on a running chain and the commit it breaks is a
/// pre-execution system call. `initialize` does not: a genesis with too small a
/// cap simply never leaves block zero, which is loud, immediate, and fixed by
/// relaunching.
pub const MIN_COMMITTEE_LENGTH: usize = 4;

/// Reported as `prev_value` in the `UndelegatePeriodChanged` init event.
///
/// Not a default — see [`DEFAULT_EPOCH_BLOCK_INTERVAL`].
pub const DEFAULT_UNDELEGATE_PERIOD: u64 = 7;

/// Epochs between a delegation being booked and it counting toward stake.
///
/// `delegate_to` books stake at `current_epoch + WARMUP_DELAY`, so a delegation
/// can never move a total that the current epoch's frozen committee — or any
/// committee already selected against an earlier epoch — was chosen from. Any
/// value above zero satisfies that; no derivation is recorded for exactly 2,
/// and it is not tied to `MAX_COMMITTEE_LOOKAHEAD_EPOCHS` despite matching it.
pub const WARMUP_DELAY: u64 = 2;

/// Epochs one reward claim may walk before it must be resumed by another call.
///
/// Bounds the per-call loop so a delegator that never claimed cannot build a
/// claim too large to execute. No derivation is recorded for 1000, and it has
/// not been measured against rWasm fuel on this runtime.
pub const MAX_EPOCHS_PER_CLAIM: u64 = 1_000;

/// Exclusions stamped by one epoch close.
///
/// Deliberately low: a correlated loss of `f` seats is answered over at least
/// eight closes, which gives a healed cause time to clear the verdicts before
/// most stamps land.
pub const MAX_STAMPS_PER_CLOSE: usize = 2;
/// How far ahead of the current epoch a committee may be committed, and — the
/// same number, because they are the same offset — how far back of the target
/// epoch its membership is selected from.
///
/// `commit_epoch_committee` is its only reader.
pub const MAX_COMMITTEE_LOOKAHEAD_EPOCHS: u64 = 2;

pub const BLS_PUBKEY_UNCOMPRESSED_LENGTH: usize = 256;
pub const BLS_POP_UNCOMPRESSED_LENGTH: usize = 128;
pub const BLS_PUBKEY_LENGTH: usize = 96;
/// 32-byte words a compressed BLS12-381 G2 key occupies in storage.
///
/// The storage array width, the split of the verifier's compressed output, and
/// the read that reassembles it all have to agree with this.
pub const BLS_PUBKEY_WORDS: usize = BLS_PUBKEY_LENGTH / U256::BYTES;
pub const BLS_SIGNATURE_LENGTH: usize = 48;
pub const PROPOSAL_PAYLOAD_LENGTH: usize = 32;

/// Message kinds as `consensus::namespace` reads them.
///
/// One nullify-finalize proof carries two different message kinds, so an
/// evidence shape cannot be reduced to a single kind.
pub const EVIDENCE_MESSAGE_KIND_NOTARIZE: u8 = 0;
pub const EVIDENCE_MESSAGE_KIND_NULLIFY: u8 = 1;
pub const EVIDENCE_MESSAGE_KIND_FINALIZE: u8 = 2;

/// A committee member passes an epoch's liveness verdict when it produced at
/// least `1 / MIN_PRODUCTION_SHARE_DENOMINATOR` of the blocks its stake weight
/// was due — half, today.
///
/// The floor in [`DEFAULT_MIN_VERDICT_DUE_BLOCKS`] is a confidence gate on
/// exactly this test and its derivation quotes this ratio, so the two move
/// together: a different share invalidates the sample size chosen there.
pub const MIN_PRODUCTION_SHARE_DENOMINATOR: u64 = 2;
/// Both the shipped floor and the highest one governance may set — the setter
/// bounds against this constant and `MAX_MIN_VERDICT_DUE_BLOCKS()` returns it.
///
/// One number does both jobs because one fact decides both. The floor is a
/// confidence gate on a statistical test: a member fails at `produced * 2 < due`,
/// which is only meaningful once the sample is large enough for the shortfall to
/// mean something. At a due of 100 that threshold already sits five standard
/// deviations out, so 100 is at once the value worth shipping and the value above
/// which raising it buys nothing. Governance may lower it — trading confidence
/// for reach — and may never raise it.
///
/// The floor is also a minimum stake share in disguise: a member is judged once
/// its share reaches `minVerdictDueBlocks / epochBlockInterval`. The bound is
/// absolute and nothing more; it does not promise that any given member is
/// judgeable, because which shares are met depends on the live stake
/// distribution, which governance does not set and which moves every epoch.
pub const DEFAULT_MIN_VERDICT_DUE_BLOCKS: u32 = 100;
/// Longest exclusion the liveness backoff ladder can reach, in epochs, as
/// shipped: an excluded validator waits `min(kick_count, cap)` epochs.
///
/// Governance may lower it. No derivation is recorded for 128.
pub const DEFAULT_EXCLUSION_BACKOFF_CAP: u32 = 128;

/// Ceiling on the per-epoch stipend governance may configure: `10^24` wei,
/// i.e. 1,000,000 BLEND per epoch. No derivation is recorded for the figure.
pub const MAX_BLEND_STIPEND_PER_EPOCH: U256 = uint!(1_000_000_000_000_000_000_000_000_U256);
pub const SYSTEM_CALLER: Address = address!("0xfffffffffffffffffffffffffffffffffffffffe");
pub const EQUIVOCATION_BURN_SINK: Address = address!("0x000000000000000000000000000000000000dead");

/// Reported as `prev_value` in the `MinValidatorStakeAmountChanged` init event:
/// `10^18` wei, i.e. 1 BLEND.
///
/// Not a default — see [`DEFAULT_EPOCH_BLOCK_INTERVAL`].
pub const DEFAULT_MIN_VALIDATOR_STAKE: U256 = uint!(1_000_000_000_000_000_000_U256);

/// Reported as `prev_value` in the `MinStakingAmountChanged` init event:
/// `10^18` wei, i.e. 1 BLEND.
///
/// Not a default — see [`DEFAULT_EPOCH_BLOCK_INTERVAL`].
pub const DEFAULT_MIN_STAKING_AMOUNT: U256 = uint!(1_000_000_000_000_000_000_U256);

pub const INITIALIZER_STORAGE_SLOT: U256 = erc7201_slot!("Fluent.storage.Initializer");
pub const CHAIN_CONFIG_STORAGE_SLOT: U256 = erc7201_slot!("Fluent.storage.ChainConfig");
pub const CONSENSUS_STORAGE_SLOT: U256 = erc7201_slot!("Fluent.storage.Consensus");
pub const STAKING_STORAGE_SLOT: U256 = erc7201_slot!("Fluent.storage.StakingStorage");
pub const PRODUCTION_LIVENESS_STORAGE_SLOT: U256 =
    erc7201_slot!("Fluent.storage.ProductionLiveness");
