//! Canonical ABI selectors, error IDs, protocol limits, and default values.

use fluentbase_sdk::{
    address,
    derive::{derive_keccak256_id, erc7201_slot},
    staking_protocol, uint, Address, U256,
};
use fluentbase_staking_abi::{self as abi, SolCall, SolError};

/// A selector from the shared ABI crate, in the `u32` form the dispatcher
/// compares against.
///
/// The rule, the same one `fluentbase-staking-abi`'s header states and
/// `agreement_check.py` G3 enforces: a handler that MORE THAN ONE place outside
/// this crate has to encode — the node, the genesis bootstrap, the e2e stands,
/// the Python harness — gets its selector this way, and a rename in the shared
/// crate is then a compile error here and in every caller at the same time.
///
/// A handler with a single caller outside this crate keeps `derive_keccak256_id!`
/// below: one declaration on each side of one pairing is not a duplicate anyone
/// can drift, and moving it would only widen the shared crate for nothing.
const fn sig<C: SolCall>() -> u32 {
    u32::from_be_bytes(C::SELECTOR)
}

const fn err<E: SolError>() -> u32 {
    u32::from_be_bytes(E::SELECTOR)
}

pub const SIG_LEN_BYTES: usize = 4;

pub const STATUS_NOT_FOUND: u8 = 0;
pub const STATUS_ACTIVE: u8 = 1;
pub const STATUS_PENDING: u8 = 2;
pub const STATUS_JAIL: u8 = 3;

// ABI selectors are derived from their canonical signatures. The pinned hex
// values remain beside them to make ABI drift visible during review.

// 0xfecaf0f1
pub const SIG_INITIALIZE: u32 = sig::<abi::initializeCall>();
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
// 0x9f9106d1
pub const SIG_GET_STAKING_TOKEN: u32 = derive_keccak256_id!("getStakingToken()");
// 0x32cc6f08
pub const SIG_GET_ACTIVE_VALIDATORS_LENGTH: u32 = sig::<abi::getActiveValidatorsLengthCall>();
// 0x346c90a8
pub const SIG_GET_EPOCH_BLOCK_INTERVAL: u32 = sig::<abi::getEpochBlockIntervalCall>();
// 0xa2a50528
pub const SIG_GET_DPOS_ACTIVATION_BLOCK: u32 = sig::<abi::getDposActivationBlockCall>();
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
pub const SIG_REGISTER_VALIDATOR: u32 = sig::<abi::registerValidatorCall>();
// 0x026e402b
pub const SIG_DELEGATE: u32 = sig::<abi::delegateCall>();
// 0x4d99dd16
pub const SIG_UNDELEGATE: u32 = sig::<abi::undelegateCall>();
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
pub const SIG_SET_BLEND_STIPEND_PER_EPOCH: u32 = sig::<abi::setBlendStipendPerEpochCall>();
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
// 0x37dff538
pub const SIG_GET_BLEND_RESERVE: u32 = derive_keccak256_id!("getBlendReserve()");
// 0x7899ae8f
pub const SIG_SET_BLEND_RESERVE: u32 = sig::<abi::setBlendReserveCall>();
pub const SIG_APPLY_BLEND_RESERVE: u32 = sig::<abi::applyBlendReserveCall>();
pub const SIG_APPLY_SLASH_FUND_ADDRESS: u32 = sig::<abi::applySlashFundAddressCall>();
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
    sig::<abi::setProductionLivenessDisabledCall>();
// 0xf06be669
#[cfg(feature = "devnet-views")]
pub const SIG_BLOCKS_IN_EPOCH: u32 = sig::<abi::blocksInEpochCall>();
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
pub const SIG_RECORD_PRODUCTION: u32 = sig::<abi::recordProductionCall>();
// 0x457179fd
pub const SIG_GET_VALIDATOR_FEE: u32 = derive_keccak256_id!("getValidatorFee(address)");
// 0xff4794fc
pub const SIG_CLAIM_VALIDATOR_FEE: u32 = sig::<abi::claimValidatorFeeCall>();
// 0x52b7bea2
pub const SIG_GET_DELEGATOR_FEE: u32 = derive_keccak256_id!("getDelegatorFee(address,address)");
// 0x426594b1
pub const SIG_CLAIM_DELEGATOR_FEE: u32 = derive_keccak256_id!("claimDelegatorFee(address)");
// 0xa789083d
pub const SIG_GET_DELEGATOR_PRINCIPAL: u32 =
    derive_keccak256_id!("getDelegatorPrincipal(address,address)");
// 0xe75f359c
pub const SIG_WITHDRAW_DELEGATOR_PRINCIPAL: u32 =
    derive_keccak256_id!("withdrawDelegatorPrincipal(address)");
// 0x8ecb3fc9
pub const SIG_REDELEGATE_DELEGATOR_FEE: u32 =
    derive_keccak256_id!("redelegateDelegatorFee(address)");
// 0x54c3e84b
pub const SIG_GET_EPOCH_REWARDS: u32 = sig::<abi::getEpochRewardsCall>();
// 0xad36f42f
pub const SIG_GET_CONSENSUS_KEYS: u32 = sig::<abi::getConsensusKeysCall>();
// 0xd96cbd7b
pub const SIG_GET_REGISTRY_WITH_KEYS: u32 = sig::<abi::getRegistryWithKeysCall>();
// 0xc06a82de
pub const SIG_NEXT_EPOCH_TO_COMMIT: u32 = sig::<abi::nextEpochToCommitCall>();
// 0xe505b249
pub const SIG_COMMIT_EPOCH_COMMITTEE: u32 = sig::<abi::commitEpochCommitteeCall>();
// 0x2660899f
pub const SIG_GET_DKG_QUAL: u32 = sig::<abi::getDkgQualCall>();
// 0x80b562de
pub const SIG_GET_EPOCH_COMMITTEE: u32 = derive_keccak256_id!("getEpochCommittee(uint64)");
// 0xa4d160c1
pub const SIG_GET_EPOCH_COMMITTEE_WITH_STAKES: u32 = sig::<abi::getEpochCommitteeWithStakesCall>();
// 0xdc6fb3f2
pub const SIG_SLASH_EQUIVOCATION: u32 = sig::<abi::slashEquivocationCall>();
// 0xe28d2f63
pub const SIG_SLASH_EQUIVOCATION_NOTARIZE: u32 = sig::<abi::slashEquivocationNotarizeCall>();
// 0xadd07a3e
pub const SIG_SLASH_EQUIVOCATION_FINALIZE: u32 = sig::<abi::slashEquivocationFinalizeCall>();
// 0xa10827e9
pub const SIG_SLASH_EQUIVOCATION_NULLIFY_FINALIZE: u32 =
    sig::<abi::slashEquivocationNullifyFinalizeCall>();

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
/// selection truncates to it, so `MAX_COMMITTEE_SIZE` already bounds
/// this two layers up. It is checked again here because the failure it prevents
/// is the one this storage design exists to make unrepresentable — an
/// over-long committee writes past its frame into the NEXT epoch's weights,
/// under this epoch's fresh stamp, and every read of both epochs then answers
/// confidently and wrongly. A halted chain is recoverable; that is not.
pub const ERR_COMMITTEE_EXCEEDS_WEIGHT_RING: u32 =
    derive_keccak256_id!("CommitteeExceedsWeightRing(uint256,uint256)");
pub const ERR_EPOCH_NOT_YET_COMMITTABLE: u32 =
    derive_keccak256_id!("EpochNotYetCommittable(uint64,uint64)");
pub const ERR_ALREADY_SLASHED_FOR_EQUIVOCATION: u32 = err::<abi::AlreadySlashedForEquivocation>();
// The five errors the inlined BLS verifier raises. Their names and selectors
// are the ones the external `BLS12381Verifier` predeploy used, so a caller that
// decoded a revert from it decodes the same revert now.
pub const ERR_BLS_INFINITY_POINT: u32 = derive_keccak256_id!("InfinityPoint()");
pub const ERR_BLS_NAMESPACE_TOO_LONG: u32 = derive_keccak256_id!("NamespaceTooLong()");
pub const ERR_BLS_DST_TOO_LONG: u32 = derive_keccak256_id!("DstTooLong()");
pub const ERR_BLS_PRECOMPILE_FAILED: u32 = derive_keccak256_id!("PrecompileFailed()");
pub const ERR_BLS_INVALID_POINT_LENGTH: u32 = derive_keccak256_id!("InvalidPointLength()");
pub const ERR_INVALID_PROOF_OF_POSSESSION: u32 =
    derive_keccak256_id!("InvalidProofOfPossession(address)");
pub const ERR_INVALID_CONSENSUS_KEY_ENCODING: u32 =
    derive_keccak256_id!("InvalidConsensusKeyEncoding()");
pub const ERR_EQUIVOCATION_SIGNATURE_INVALID: u32 =
    derive_keccak256_id!("EquivocationSignatureInvalid()");
pub const ERR_EQUIVOCATION_KEY_NOT_REGISTERED: u32 =
    derive_keccak256_id!("EquivocationKeyNotRegistered()");
pub const ERR_INVALID_EVIDENCE_ENCODING: u32 = derive_keccak256_id!("InvalidEvidenceEncoding()");
/// An address timelock asked to land before its term elapsed: `(current epoch,
/// the epoch it becomes effective at)`.
pub const ERR_TIMELOCK_NOT_ELAPSED: u32 = derive_keccak256_id!("TimelockNotElapsed(uint64,uint64)");
/// An address timelock asked to land with nothing declared.
pub const ERR_NO_PENDING_CHANGE: u32 = derive_keccak256_id!("NoPendingChange()");
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
/// `math::compact_balance`. The node divides by the same figure to recover the
/// weight its leader elector ranks on, which is why the number lives in
/// `fluentbase_types::staking_protocol` and not here.
pub const BALANCE_COMPACT_PRECISION: U256 = staking_protocol::BALANCE_COMPACT_PRECISION_U256;

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
///
/// The node bounds its certificate-bitmap codec and its one-byte leader index
/// by the same number, so it is declared once, shared, and imported here under
/// the shared name.
pub use staking_protocol::MAX_COMMITTEE_SIZE;

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
///
/// The node's committee module pins its read window against this number, so it
/// is declared once, shared, and imported here under the shared name.
pub use staking_protocol::WEIGHT_RING_EPOCHS;

/// Ring pair-slots one epoch occupies: two members' weights and their shared
/// epoch stamp pack into one 32-byte slot (`14 + 14 + 4`).
pub const PAIRS_MAX: usize = (MAX_COMMITTEE_SIZE as usize).div_ceil(2);

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
/// selection. Both places that set the cap refuse it:
/// `setActiveValidatorsLength` because it lands on a running chain and the
/// commit it breaks is a pre-execution system call, and `initialize` because a
/// genesis with too small a cap never leaves block zero and nothing else would
/// say which value was wrong.
///
/// The node asserts the same floor when it decodes a committee, so the number is
/// shared rather than mirrored.
pub use staking_protocol::MIN_COMMITTEE_LENGTH;

/// Reported as `prev_value` in the `UndelegatePeriodChanged` init event.
///
/// Not a default — see [`DEFAULT_EPOCH_BLOCK_INTERVAL`].
pub const DEFAULT_UNDELEGATE_PERIOD: u64 = 7;

/// Epochs a declared address setting waits before it can be applied.
///
/// The two address setters — `setSlashFundAddress` and `setBlendReserve` — name
/// the seizure recipient and the stipend source, and a governance key that could
/// move either in one block could point both at itself. Splitting each into
/// declare-then-apply puts this many epochs of public notice between the two,
/// during which the declaration is visible as an ordinary receipt log.
///
/// Epochs, not blocks and not seconds: this contract reads no clock
/// (`block_timestamp()` appears nowhere in it) and every other deadline it keeps
/// is in epochs, so a second unit would be a second thing to reason about. At the
/// devnet interval of 200 blocks this is 1,400 blocks; at a production epoch it
/// is a week of them. No derivation is recorded for exactly 7 — it is the figure
/// the decision fixed (`.dpos-study/DECISIONS.md` §3, 2026-09-04).
///
/// `setBlendStipendPerEpoch` is deliberately NOT under it, decided at the same
/// time and for a stated reason; do not extend the scheme to it without
/// reopening that decision.
pub const ADDRESS_SETTER_TIMELOCK_EPOCHS: u64 = 7;

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
/// `commit_epoch_committee` applies it to membership; `selection_epoch_for`
/// applies the same offset to the reward split, so a seat's stipend is divided
/// by the vintage its weight was frozen from. Changing this number moves both —
/// and it moves the node's ahead-commit horizon too, which is why it is shared.
pub use staking_protocol::MAX_COMMITTEE_LOOKAHEAD_EPOCHS;

/// Key and signature widths on the wire, shared with the node's `fluentbase-bls`
/// (which knows them as `PUBKEY_EIP2537_BYTES` / `SIGNATURE_EIP2537_BYTES` /
/// `PUBKEY_BYTES` / `SIGNATURE_BYTES`). A proof of possession is a G1 signature,
/// so it has the signature width.
pub use staking_protocol::{
    BLS_PUBKEY_LENGTH, BLS_PUBKEY_UNCOMPRESSED_LENGTH,
    BLS_SIGNATURE_UNCOMPRESSED_LENGTH as BLS_POP_UNCOMPRESSED_LENGTH,
};
/// 32-byte words a compressed BLS12-381 G2 key occupies in storage.
///
/// The storage array width, the split of the verifier's compressed output, and
/// the read that reassembles it all have to agree with this.
pub const BLS_PUBKEY_WORDS: usize = BLS_PUBKEY_LENGTH / U256::BYTES;
pub use staking_protocol::{BLS_SIGNATURE_LENGTH, PROPOSAL_PAYLOAD_LENGTH};

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
/// bounds against this constant.
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

/// Epochs without a production failure that retire a validator's backoff ladder.
///
/// Measured from the last failure to the epoch currently being judged, NOT as a
/// count of clean epochs served: `last_failed_epoch_p1` only moves when the
/// validator is judged, so epochs spent outside the committee count toward the
/// run. A ladder that only ever climbed made one bad week permanent, and the
/// exclusion length is already capped by `DEFAULT_EXCLUSION_BACKOFF_CAP` — this
/// is the other direction, and the two are independent.
///
/// No derivation is recorded for 30.
pub const KICK_LADDER_RESET_EPOCHS: u64 = 30;

/// Ceiling on the per-epoch stipend governance may configure: `10^24` wei,
/// i.e. 1,000,000 BLEND per epoch. No derivation is recorded for the figure.
pub const MAX_BLEND_STIPEND_PER_EPOCH: U256 = uint!(1_000_000_000_000_000_000_000_000_U256);
/// The address every pre-execution system call arrives from.
///
/// The node reaches for the same constant (`fluentbase_types::SYSTEM_ADDRESS`)
/// when it issues those calls; this crate used to repeat the literal even though
/// it already imports `GENESIS_GOVERNANCE` from that same crate.
pub use fluentbase_sdk::SYSTEM_ADDRESS as SYSTEM_CALLER;
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
