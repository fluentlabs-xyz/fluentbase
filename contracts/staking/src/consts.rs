//! Canonical ABI selectors, error IDs, protocol limits, and default values.

use fluentbase_sdk::{
    address,
    derive::{derive_keccak256_id, erc7201_slot},
    Address, FUEL_DENOM_RATE, U256,
};

pub const SIG_LEN_BYTES: usize = 4;

pub const STATUS_NOT_FOUND: u8 = 0;
pub const STATUS_ACTIVE: u8 = 1;
pub const STATUS_PENDING: u8 = 2;
pub const STATUS_JAIL: u8 = 3;

// ABI selectors are derived from their canonical signatures. The pinned hex
// values remain beside them to make ABI drift visible during review.

// 0xd86555fe
pub const SIG_INITIALIZE: u32 =
    derive_keccak256_id!(
        "initialize(address,address[],uint256[],bytes[],bytes[],bytes32[],uint16,address,uint32,uint32,uint32,uint256,uint256,uint64,address,address,uint256,address,address)"
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
// 0xfff952d5
pub const SIG_ADD_VALIDATOR: u32 =
    derive_keccak256_id!("addValidator(address,bytes,bytes,bytes32)");
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
// 0xd9b083ba
pub const SIG_GET_ACTIVE_VALIDATORS_LENGTH_AT: u32 =
    derive_keccak256_id!("getActiveValidatorsLengthAt(uint64)");
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
// 0x6cc69027
pub const SIG_DEFAULT_SLASH_REPORTER_BPS: u32 =
    derive_keccak256_id!("DEFAULT_SLASH_REPORTER_BPS()");
// 0x5d887462
pub const SIG_MAX_ACTIVE_VALIDATORS: u32 = derive_keccak256_id!("MAX_ACTIVE_VALIDATORS()");
// 0x2bc2fec4
pub const SIG_MAX_BLEND_STIPEND_PER_EPOCH: u32 =
    derive_keccak256_id!("MAX_BLEND_STIPEND_PER_EPOCH()");
// 0x0a3a6183
pub const SIG_MAX_SLASH_REPORTER_BPS: u32 = derive_keccak256_id!("MAX_SLASH_REPORTER_BPS()");
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
// 0xce534df5
pub const SIG_GET_SLASH_REPORTER_REWARD_BPS: u32 =
    derive_keccak256_id!("getSlashReporterRewardBps()");
// 0x58702003
pub const SIG_SET_SLASH_REPORTER_REWARD_BPS: u32 =
    derive_keccak256_id!("setSlashReporterRewardBps(uint32)");
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
// 0xe2cf72f9
pub const SIG_GET_EVIDENCE_DECODER: u32 = derive_keccak256_id!("getEvidenceDecoder()");
// 0x00857c90
pub const SIG_SET_EVIDENCE_DECODER: u32 = derive_keccak256_id!("setEvidenceDecoder(address)");
// 0xdb2366b4
pub const SIG_GET_LIVENESS_SLASHING: u32 = derive_keccak256_id!("getLivenessSlashing()");
// 0xbb32522a
pub const SIG_SET_LIVENESS_SLASHING: u32 = derive_keccak256_id!("setLivenessSlashing(address)");
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
// 0x8e948ac1
pub const SIG_GET_PRODUCTION_STATS: u32 =
    derive_keccak256_id!("getProductionStats(address,uint64)");
// 0xf06be669
pub const SIG_BLOCKS_IN_EPOCH: u32 = derive_keccak256_id!("blocksInEpoch(uint64)");
// 0x91c7d453
pub const SIG_PRODUCED_AT: u32 = derive_keccak256_id!("producedAt(uint64,uint32)");
// 0xaef690f9
pub const SIG_PENDING_EXCLUSIONS: u32 = derive_keccak256_id!("pendingExclusions()");
// 0x32066046
pub const SIG_READMIT_AT_EPOCH: u32 = derive_keccak256_id!("readmitAtEpoch(address)");
// 0x33de61d2
pub const SIG_LAST_PROCESSED_BLOCK: u32 = derive_keccak256_id!("lastProcessedBlock()");
// 0x8244a2c2
pub const SIG_RECORD_PRODUCTION: u32 = derive_keccak256_id!("recordProduction(uint64,uint8)");
// 0x92d321ab
pub const SIG_SETTLE_EPOCH_STIPEND_FROM: u32 =
    derive_keccak256_id!("settleEpochStipendFrom(uint64)");
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
// 0xc72e0d73
pub const SIG_GET_VALIDATOR_SELF_STAKE_LOCK: u32 =
    derive_keccak256_id!("getValidatorSelfStakeLock(address)");
// 0xc2fd58fc
pub const SIG_GET_PENDING_DELEGATOR_FEE: u32 =
    derive_keccak256_id!("getPendingDelegatorFee(address,address)");
// 0x426594b1
pub const SIG_CLAIM_DELEGATOR_FEE: u32 = derive_keccak256_id!("claimDelegatorFee(address)");
// 0xfe38ebef
pub const SIG_CLAIM_DELEGATOR_FEE_AT_EPOCH: u32 =
    derive_keccak256_id!("claimDelegatorFeeAtEpoch(address,uint64)");
// 0x5ef9e8c6
pub const SIG_CALC_AVAILABLE_FOR_REDELEGATE_AMOUNT: u32 =
    derive_keccak256_id!("calcAvailableForRedelegateAmount(address,address)");
// 0x8ecb3fc9
pub const SIG_REDELEGATE_DELEGATOR_FEE: u32 =
    derive_keccak256_id!("redelegateDelegatorFee(address)");
// 0xa631344a
pub const SIG_SETTLE_EPOCH_STIPEND: u32 = derive_keccak256_id!("settleEpochStipend(uint64)");
// 0x54c3e84b
pub const SIG_GET_EPOCH_REWARDS: u32 = derive_keccak256_id!("getEpochRewards(uint64)");
// 0xad36f42f
pub const SIG_GET_CONSENSUS_KEYS: u32 = derive_keccak256_id!("getConsensusKeys(address)");
// 0xd41c52eb
pub const SIG_GET_VALIDATORS_WITH_KEYS: u32 = derive_keccak256_id!("getValidatorsWithKeys()");
// 0xd96cbd7b
pub const SIG_GET_REGISTRY_WITH_KEYS: u32 = derive_keccak256_id!("getRegistryWithKeys()");
// 0x7cfba9f3
pub const SIG_GET_VALIDATORS_WITH_KEYS_AT: u32 =
    derive_keccak256_id!("getValidatorsWithKeysAt(uint64)");
// 0xc06a82de
pub const SIG_NEXT_EPOCH_TO_COMMIT: u32 = derive_keccak256_id!("nextEpochToCommit()");
// 0x8bd070e4
pub const SIG_COMMITTEE_SELECTION_EPOCH: u32 = derive_keccak256_id!("committeeSelectionEpoch()");
// 0x87401d8a
pub const SIG_COMMIT_EPOCH_COMMITTEE: u32 = derive_keccak256_id!("commitEpochCommittee(address[])");
// 0x2660899f
pub const SIG_GET_DKG_QUAL: u32 = derive_keccak256_id!("getDkgQual(uint64)");
// 0xd7f1733d
pub const SIG_RESOLVE_SIGNER: u32 = derive_keccak256_id!("resolveSigner(uint64,uint32)");
// 0x80b562de
pub const SIG_GET_EPOCH_COMMITTEE: u32 = derive_keccak256_id!("getEpochCommittee(uint64)");
// 0xe3e6cedc
pub const SIG_GET_EPOCH_COMMITTEE_LENGTH: u32 =
    derive_keccak256_id!("getEpochCommitteeLength(uint64)");
// 0xa4d160c1
pub const SIG_GET_EPOCH_COMMITTEE_WITH_STAKES: u32 =
    derive_keccak256_id!("getEpochCommitteeWithStakes(uint64)");
// 0xa5d2dd22
pub const SIG_BLS_COMPRESS_G2_UNCHECKED: u32 = derive_keccak256_id!("compressG2Unchecked(bytes)");
// 0x8bf26133
pub const SIG_BLS_VERIFY: u32 = derive_keccak256_id!("verify(bytes,bytes,bytes,bytes,bytes)");
// 0xa10954fe
pub const SIG_RESERVE_BALANCE: u32 = derive_keccak256_id!("reserveBalance()");
// 0x7f3bd56e
pub const SIG_RESERVE_DISBURSE: u32 = derive_keccak256_id!("disburse(address,uint256)");
// 0x32890bc0
pub const SIG_COMMIT_EQUIVOCATION_REPORT: u32 =
    derive_keccak256_id!("commitEquivocationReport(bytes32)");
// 0xc289d76e
pub const SIG_COMPUTE_EQUIVOCATION_REPORT_COMMITMENT: u32 =
    derive_keccak256_id!("computeEquivocationReportCommitment(address,uint8,bytes32,bytes32)");
// 0xa3aae5dd
pub const SIG_GET_EQUIVOCATION_REPORT_COMMITMENT: u32 =
    derive_keccak256_id!("getEquivocationReportCommitment(address)");
// 0x2bc5fb10
pub const SIG_SLASH_EQUIVOCATION_NOTARIZE: u32 =
    derive_keccak256_id!("slashEquivocationNotarize(bytes,bytes,bytes,bytes,address,bytes32)");
// 0xb034c58b
pub const SIG_SLASH_EQUIVOCATION_FINALIZE: u32 =
    derive_keccak256_id!("slashEquivocationFinalize(bytes,bytes,bytes,bytes,address,bytes32)");
// 0x337e1437
pub const SIG_SLASH_EQUIVOCATION_NULLIFY_FINALIZE: u32 = derive_keccak256_id!(
    "slashEquivocationNullifyFinalize(bytes,bytes,bytes,bytes,address,bytes32)"
);
// 0x27e7ff4b
pub const SIG_DECODE_CONFLICTING_NOTARIZE: u32 =
    derive_keccak256_id!("decodeConflictingNotarize(bytes)");
// 0xce4b0b3a
pub const SIG_DECODE_CONFLICTING_FINALIZE: u32 =
    derive_keccak256_id!("decodeConflictingFinalize(bytes)");
// 0x2d85f570
pub const SIG_DECODE_NULLIFY_FINALIZE: u32 = derive_keccak256_id!("decodeNullifyFinalize(bytes)");
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
pub const ERR_VALIDATOR_OWNER_ALREADY_IN_USE: u32 =
    derive_keccak256_id!("ValidatorOwnerAlreadyInUse(address)");
pub const ERR_NOT_PENDING_VALIDATOR: u32 = derive_keccak256_id!("NotPendingValidator(address)");
pub const ERR_NOT_ACTIVE_VALIDATOR: u32 = derive_keccak256_id!("NotActiveValidator()");
pub const ERR_ONLY_VALIDATOR_OWNER: u32 = derive_keccak256_id!("OnlyValidatorOwner(address)");
pub const ERR_VALIDATOR_OWNER_IMMUTABLE: u32 = derive_keccak256_id!("ValidatorOwnerImmutable()");
pub const ERR_ZERO_STAKING_TOKEN: u32 = derive_keccak256_id!("ZeroStakingToken()");
pub const ERR_INVALID_CHAIN_CONFIG: u32 = derive_keccak256_id!("InvalidChainConfig()");
pub const ERR_AMOUNT_TOO_LOW: u32 = derive_keccak256_id!("AmountTooLow(uint256)");
pub const ERR_INITIAL_STAKE_TOO_LOW: u32 = derive_keccak256_id!("InitialStakeTooLow(uint256)");
pub const ERR_OWNER_SELF_STAKE_BELOW_MINIMUM: u32 =
    derive_keccak256_id!("OwnerSelfStakeBelowMinimum()");
pub const ERR_INSUFFICIENT_BALANCE: u32 = derive_keccak256_id!("InsufficientBalance()");
pub const ERR_DELEGATION_QUEUE_EMPTY: u32 = derive_keccak256_id!("DelegationQueueEmpty()");
pub const ERR_DELEGATION_QUEUE_NOT_EMPTY: u32 =
    derive_keccak256_id!("DelegationQueueNotEmpty(uint256)");
pub const ERR_PENDING_DELEGATION: u32 = derive_keccak256_id!("PendingDelegation(uint64)");
pub const ERR_STAKING_TOKEN_CALL_FAILED: u32 = derive_keccak256_id!("StakingTokenCallFailed()");
pub const ERR_UNKNOWN_METHOD: u32 = derive_keccak256_id!("UnknownMethod()");
pub const ERR_ONLY_SYSTEM_CALL: u32 = derive_keccak256_id!("OnlySystemCall()");
pub const ERR_ONLY_SELF_CALL: u32 = derive_keccak256_id!("OnlySelfCall()");
pub const ERR_ZERO_VALUE: u32 = derive_keccak256_id!("ZeroValue(string)");
pub const ERR_MAX_ACTIVE_VALIDATORS_EXCEEDED: u32 =
    derive_keccak256_id!("MaxActiveValidatorsExceeded(uint32,uint32)");
pub const ERR_DPOS_ALREADY_ACTIVE: u32 = derive_keccak256_id!("DposAlreadyActive()");
pub const ERR_UNALIGNED_ACTIVATION_BLOCK: u32 = derive_keccak256_id!("UnalignedActivationBlock()");
pub const ERR_ACTIVATION_BLOCK_IN_PAST: u32 = derive_keccak256_id!("ActivationBlockInPast()");
pub const ERR_UNDELEGATE_WINDOW_TOO_SHORT: u32 =
    derive_keccak256_id!("UndelegateWindowTooShort(uint256,uint256)");
pub const ERR_SLASH_REPORTER_REWARD_BPS_TOO_HIGH: u32 =
    derive_keccak256_id!("SlashReporterRewardBpsTooHigh(uint32,uint32)");
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
pub const ERR_COMMITTEE_LENGTH_MISMATCH: u32 =
    derive_keccak256_id!("CommitteeLengthMismatch(uint256,uint256)");
pub const ERR_COMMITTEE_TOO_SMALL: u32 = derive_keccak256_id!("CommitteeTooSmall(uint256,uint256)");
pub const ERR_LEADER_STAKES_LENGTH_MISMATCH: u32 =
    derive_keccak256_id!("LeaderStakesLengthMismatch(uint64,uint256,uint256)");
pub const ERR_STIPEND_RATE_NOT_SNAPSHOTTED: u32 =
    derive_keccak256_id!("StipendRateNotSnapshotted(uint64)");
pub const ERR_EPOCH_NOT_YET_COMMITTABLE: u32 =
    derive_keccak256_id!("EpochNotYetCommittable(uint64,uint64)");
pub const ERR_COMMITTEE_MEMBER_KEYLESS: u32 =
    derive_keccak256_id!("CommitteeMemberKeyless(address)");
pub const ERR_COMMITTEE_MEMBER_NOT_IN_ACTIVE_SET: u32 =
    derive_keccak256_id!("CommitteeMemberNotInActiveSet(address)");
pub const ERR_COMMITTEE_NOT_STRICTLY_ASCENDING: u32 =
    derive_keccak256_id!("CommitteeNotStrictlyAscending(address)");
pub const ERR_ALREADY_SLASHED_FOR_EQUIVOCATION: u32 =
    derive_keccak256_id!("AlreadySlashedForEquivocation(address)");
pub const ERR_BLS_VERIFIER_NOT_CONFIGURED: u32 = derive_keccak256_id!("BlsVerifierNotConfigured()");
pub const ERR_INVALID_PROOF_OF_POSSESSION: u32 =
    derive_keccak256_id!("InvalidProofOfPossession(address)");
pub const ERR_INVALID_CONSENSUS_KEY_ENCODING: u32 =
    derive_keccak256_id!("InvalidConsensusKeyEncoding()");
pub const ERR_EQUIVOCATION_SIGNATURE_INVALID: u32 =
    derive_keccak256_id!("EquivocationSignatureInvalid()");
pub const ERR_EQUIVOCATION_KEY_MISMATCH: u32 = derive_keccak256_id!("EquivocationKeyMismatch()");
pub const ERR_EQUIVOCATION_EVIDENCE_EXPIRED: u32 =
    derive_keccak256_id!("EquivocationEvidenceExpired(uint64,uint64)");
pub const ERR_EVIDENCE_DECODER_NOT_CONFIGURED: u32 =
    derive_keccak256_id!("EvidenceDecoderNotConfigured()");
pub const ERR_ZERO_EQUIVOCATION_BENEFICIARY: u32 =
    derive_keccak256_id!("ZeroEquivocationBeneficiary()");
pub const ERR_ZERO_EQUIVOCATION_COMMITMENT: u32 =
    derive_keccak256_id!("ZeroEquivocationCommitment()");
pub const ERR_NO_EQUIVOCATION_COMMITMENT: u32 =
    derive_keccak256_id!("NoEquivocationCommitment(address)");
pub const ERR_EQUIVOCATION_COMMITMENT_MISMATCH: u32 =
    derive_keccak256_id!("EquivocationCommitmentMismatch(address,bytes32,bytes32)");
pub const ERR_EQUIVOCATION_COMMITMENT_NOT_MATURE: u32 =
    derive_keccak256_id!("EquivocationCommitmentNotMature(address,uint64,uint64)");
pub const ERR_INVALID_EQUIVOCATION_PROOF_KIND: u32 =
    derive_keccak256_id!("InvalidEquivocationProofKind(uint8)");

pub const BALANCE_COMPACT_PRECISION: U256 = U256::from_limbs([10_000_000_000, 0, 0, 0]);
pub const COMMISSION_RATE_MAX: u16 = 3_000;
pub const DEFAULT_EPOCH_BLOCK_INTERVAL: u64 = 200;
pub const DEFAULT_ACTIVE_VALIDATORS_LENGTH: u64 = 21;
pub const MAX_ACTIVE_VALIDATORS_LENGTH: u64 = 51;
pub const MIN_COMMITTEE_LENGTH: usize = 1;
pub const DEFAULT_UNDELEGATE_PERIOD: u64 = 7;
pub const WARMUP_DELAY: u64 = 2;
pub const MAX_EPOCHS_PER_CLAIM: u64 = 1_000;
/// Inherited from the Solidity, where it was sized against measured EVM gas for
/// one settled epoch inside a 12M caller bound. rWasm meters fuel, not gas, so
/// the bound this number encodes has not been measured on this runtime yet.
pub const MAX_SETTLE_CATCHUP: u64 = 4;
/// Exclusions stamped by one epoch close.
///
/// Deliberately low: a correlated loss of `f` seats is answered over at least
/// eight closes, which gives a healed cause time to clear the verdicts before
/// most stamps land.
pub const MAX_STAMPS_PER_CLOSE: usize = 2;
/// Fuel forwarded to the tolerant stipend leg.
///
/// The Solidity bound is 12M gas. Fuel is gas scaled by `FUEL_DENOM_RATE`, so
/// passing the gas figure straight into the fuel slot under-provisions the
/// frame twentyfold and turns the leg into a guaranteed `OutOfFuel`. Like
/// `MAX_SETTLE_CATCHUP`, the 12M itself is a measured EVM budget that has not
/// been re-measured against rWasm fuel.
pub const STIPEND_FUEL_CAP: u64 = 12_000_000 * FUEL_DENOM_RATE;
pub const EPOCH_COMMITTEE_RETENTION_MARGIN: u64 = 8;
pub const MAX_COMMITTEE_LOOKAHEAD_EPOCHS: u64 = 2;
pub const BLS_PUBKEY_UNCOMPRESSED_LENGTH: usize = 256;
pub const BLS_POP_UNCOMPRESSED_LENGTH: usize = 128;
pub const BLS_PUBKEY_LENGTH: usize = 96;

pub const EQUIVOCATION_PROOF_KIND_NOTARIZE: u8 = 0;
pub const EQUIVOCATION_PROOF_KIND_FINALIZE: u8 = 1;
pub const EQUIVOCATION_PROOF_KIND_NULLIFY_FINALIZE: u8 = 2;
pub const EQUIVOCATION_PROOF_KIND_COUNT: u8 = 3;

pub const DEFAULT_SLASH_REPORTER_REWARD_BPS: u32 = 3_000;
pub const MAX_SLASH_REPORTER_REWARD_BPS: u32 = 5_000;
pub const DEFAULT_MIN_VERDICT_DUE_BLOCKS: u32 = 100;
pub const DEFAULT_EXCLUSION_BACKOFF_CAP: u32 = 128;
/// A due-block floor above the epoch length makes `due >= floor` unsatisfiable
/// for every member at any stake, disabling the tier while it still reads as
/// enabled. Bounded absolutely so the check cannot become interval-dependent.
pub const MAX_MIN_VERDICT_DUE_BLOCKS: u32 = 1_000_000;
pub const MAX_BLEND_STIPEND_PER_EPOCH: U256 =
    U256::from_limbs([2_003_764_205_206_896_640, 54_210, 0, 0]);
pub const SYSTEM_CALLER: Address = address!("0xfffffffffffffffffffffffffffffffffffffffe");
pub const EQUIVOCATION_BURN_SINK: Address = address!("0x000000000000000000000000000000000000dead");
pub const DEFAULT_MIN_VALIDATOR_STAKE: U256 =
    U256::from_limbs([1_000_000_000_000_000_000, 0, 0, 0]);
pub const DEFAULT_MIN_STAKING_AMOUNT: U256 = U256::from_limbs([1_000_000_000_000_000_000, 0, 0, 0]);

pub const INITIALIZER_STORAGE_SLOT: U256 = erc7201_slot!("Fluent.storage.Initializer");
pub const CHAIN_CONFIG_STORAGE_SLOT: U256 = erc7201_slot!("Fluent.storage.ChainConfig");
pub const CONSENSUS_STORAGE_SLOT: U256 = erc7201_slot!("Fluent.storage.Consensus");
pub const STAKING_STORAGE_SLOT: U256 = erc7201_slot!("Fluent.storage.StakingStorage");
pub const PRODUCTION_LIVENESS_STORAGE_SLOT: U256 =
    erc7201_slot!("Fluent.storage.ProductionLiveness");
