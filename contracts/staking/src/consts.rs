//! Canonical ABI selectors, error IDs, protocol limits, and default values.

use fluentbase_sdk::{
    address,
    derive::{derive_keccak256_id, erc7201_slot},
    Address, U256,
};

pub const SIG_LEN_BYTES: usize = 4;

// ABI selectors are derived from their canonical signatures. The pinned hex
// values remain beside them to make ABI drift visible during review.

// 0xdca9ac1b
pub const SIG_INITIALIZE: u32 =
    derive_keccak256_id!(
        "initialize(address,address[],uint256[],uint16,address,uint32,uint32,uint32,uint32,uint32,uint256,uint256,uint64,address,address,uint256,address,address)"
    );
// 0x76671808
pub const SIG_CURRENT_EPOCH: u32 = derive_keccak256_id!("currentEpoch()");
// 0xaea0e78b
pub const SIG_NEXT_EPOCH: u32 = derive_keccak256_id!("nextEpoch()");
// 0x8da5cb5b
pub const SIG_OWNER: u32 = derive_keccak256_id!("owner()");
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
// 0x4d238c8e
pub const SIG_ADD_VALIDATOR: u32 = derive_keccak256_id!("addValidator(address)");
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
// 0xdd0fb5df
pub const SIG_REGISTER_VALIDATOR: u32 =
    derive_keccak256_id!("registerValidator(address,uint16,uint256)");
// 0x026e402b
pub const SIG_DELEGATE: u32 = derive_keccak256_id!("delegate(address,uint256)");
// 0x4d99dd16
pub const SIG_UNDELEGATE: u32 = derive_keccak256_id!("undelegate(address,uint256)");
// 0x2c1d88e8
pub const SIG_DEFAULT_PARTICIPATION_FLOOR_BPS: u32 =
    derive_keccak256_id!("DEFAULT_PARTICIPATION_FLOOR_BPS()");
// 0x6cc69027
pub const SIG_DEFAULT_SLASH_REPORTER_BPS: u32 =
    derive_keccak256_id!("DEFAULT_SLASH_REPORTER_BPS()");
// 0x5d887462
pub const SIG_MAX_ACTIVE_VALIDATORS: u32 = derive_keccak256_id!("MAX_ACTIVE_VALIDATORS()");
// 0x2bc2fec4
pub const SIG_MAX_BLEND_STIPEND_PER_EPOCH: u32 =
    derive_keccak256_id!("MAX_BLEND_STIPEND_PER_EPOCH()");
// 0x9dbdf12b
pub const SIG_MAX_PARTICIPATION_FLOOR_BPS: u32 =
    derive_keccak256_id!("MAX_PARTICIPATION_FLOOR_BPS()");
// 0x0a3a6183
pub const SIG_MAX_SLASH_REPORTER_BPS: u32 = derive_keccak256_id!("MAX_SLASH_REPORTER_BPS()");
// 0x23b872dd
pub const SIG_ERC20_TRANSFER_FROM: u32 =
    derive_keccak256_id!("transferFrom(address,address,uint256)");
// 0xa9059cbb
pub const SIG_ERC20_TRANSFER: u32 = derive_keccak256_id!("transfer(address,uint256)");
// 0xbe199738
pub const SIG_GET_FELONY_THRESHOLD: u32 = derive_keccak256_id!("getFelonyThreshold()");
// 0xfcd6cb3e
pub const SIG_SET_FELONY_THRESHOLD: u32 = derive_keccak256_id!("setFelonyThreshold(uint32)");
// 0x6cbe6cd8
pub const SIG_GET_VALIDATOR_JAIL_EPOCH_LENGTH: u32 =
    derive_keccak256_id!("getValidatorJailEpochLength()");
// 0xc8652bd5
pub const SIG_SET_VALIDATOR_JAIL_EPOCH_LENGTH: u32 =
    derive_keccak256_id!("setValidatorJailEpochLength(uint32)");
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
// 0x4baffdc4
pub const SIG_GET_PARTICIPATION_FLOOR_BPS: u32 = derive_keccak256_id!("getParticipationFloorBps()");
// 0xd0a01007
pub const SIG_SET_PARTICIPATION_FLOOR_BPS: u32 =
    derive_keccak256_id!("setParticipationFloorBps(uint32)");
// 0x485fd959
pub const SIG_GET_PARTICIPATION_JAIL_DISABLED: u32 =
    derive_keccak256_id!("getParticipationJailDisabled()");
// 0x8664f2e7
pub const SIG_SET_PARTICIPATION_JAIL_DISABLED: u32 =
    derive_keccak256_id!("setParticipationJailDisabled(bool)");
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
// 0x225cba85
pub const SIG_SET_CONSENSUS_KEYS: u32 =
    derive_keccak256_id!("setConsensusKeys(address,bytes,bytes,bytes32)");
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
// 0x73a3dda6
pub const SIG_RELEASE_VALIDATOR_FROM_JAIL: u32 =
    derive_keccak256_id!("releaseValidatorFromJail(address)");
// 0x67d80300
pub const SIG_READMIT_EXPIRED_JAILS: u32 = derive_keccak256_id!("readmitExpiredJails(uint64)");
// 0xc96be4cb
pub const SIG_SLASH: u32 = derive_keccak256_id!("slash(address)");
// 0xa5d2dd22
pub const SIG_BLS_COMPRESS_G2_UNCHECKED: u32 = derive_keccak256_id!("compressG2Unchecked(bytes)");
// 0x8bf26133
pub const SIG_BLS_VERIFY: u32 = derive_keccak256_id!("verify(bytes,bytes,bytes,bytes,bytes)");
// 0x52736f7b
pub const SIG_LAST_FINALIZED_EPOCH_P1: u32 = derive_keccak256_id!("lastFinalizedEpochP1()");
// 0x6a4a209f
pub const SIG_PARTICIPATION: u32 = derive_keccak256_id!("participation(uint64,uint32)");
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
pub const ERR_ONLY_GOVERNANCE: u32 = derive_keccak256_id!("OnlyGovernanceContract()");
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
pub const ERR_ONLY_LIVENESS_SLASHING: u32 = derive_keccak256_id!("OnlyLivenessSlashing()");
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
pub const ERR_PARTICIPATION_FLOOR_BPS_TOO_HIGH: u32 =
    derive_keccak256_id!("ParticipationFloorBpsTooHigh(uint32,uint32)");
pub const ERR_BLEND_STIPEND_PER_EPOCH_TOO_HIGH: u32 =
    derive_keccak256_id!("BlendStipendPerEpochTooHigh(uint256,uint256)");
pub const ERR_INVALID_CLAIM_EPOCH: u32 = derive_keccak256_id!("InvalidClaimEpoch()");
pub const ERR_CONSENSUS_KEYS_ALREADY_SET: u32 =
    derive_keccak256_id!("ConsensusKeysAlreadySet(address)");
pub const ERR_PEER_PUBKEY_ALREADY_IN_USE: u32 =
    derive_keccak256_id!("PeerPubkeyAlreadyInUse(bytes32)");
pub const ERR_CONSENSUS_KEYS_NOT_SET: u32 = derive_keccak256_id!("ConsensusKeysNotSet(address)");
pub const ERR_EPOCH_COMMITTEE_NOT_COMMITTED: u32 =
    derive_keccak256_id!("EpochCommitteeNotCommitted(uint64)");
pub const ERR_SIGNER_INDEX_OUT_OF_RANGE: u32 =
    derive_keccak256_id!("SignerIndexOutOfRange(uint64,uint32,uint256)");
pub const ERR_COMMITTEE_LENGTH_MISMATCH: u32 =
    derive_keccak256_id!("CommitteeLengthMismatch(uint256,uint256)");
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
pub const ERR_VALIDATOR_NOT_IN_JAIL: u32 = derive_keccak256_id!("ValidatorNotInJail(address)");
pub const ERR_STILL_IN_JAIL: u32 = derive_keccak256_id!("StillInJail(address)");

pub const BALANCE_COMPACT_PRECISION: U256 = U256::from_limbs([10_000_000_000, 0, 0, 0]);
pub const COMMISSION_RATE_MAX: u16 = 3_000;
pub const DEFAULT_EPOCH_BLOCK_INTERVAL: u64 = 200;
pub const DEFAULT_ACTIVE_VALIDATORS_LENGTH: u64 = 21;
pub const MAX_ACTIVE_VALIDATORS_LENGTH: u64 = 51;
pub const DEFAULT_UNDELEGATE_PERIOD: u64 = 7;
pub const WARMUP_DELAY: u64 = 2;
pub const MAX_EPOCHS_PER_CLAIM: u64 = 1_000;
pub const MAX_SETTLE_CATCHUP: u64 = 32;
pub const EPOCH_COMMITTEE_RETENTION_MARGIN: u64 = 8;
pub const BLS_PUBKEY_UNCOMPRESSED_LENGTH: usize = 256;
pub const BLS_POP_UNCOMPRESSED_LENGTH: usize = 128;
pub const BLS_PUBKEY_LENGTH: usize = 96;

pub const EQUIVOCATION_PROOF_KIND_NOTARIZE: u8 = 0;
pub const EQUIVOCATION_PROOF_KIND_FINALIZE: u8 = 1;
pub const EQUIVOCATION_PROOF_KIND_NULLIFY_FINALIZE: u8 = 2;
pub const EQUIVOCATION_PROOF_KIND_COUNT: u8 = 3;

pub const DEFAULT_FELONY_THRESHOLD: u32 = 1;
pub const DEFAULT_VALIDATOR_JAIL_EPOCH_LENGTH: u32 = 1;
pub const DEFAULT_SLASH_REPORTER_REWARD_BPS: u32 = 3_000;
pub const MAX_SLASH_REPORTER_REWARD_BPS: u32 = 5_000;
pub const DEFAULT_PARTICIPATION_FLOOR_BPS: u32 = 1_500;
pub const MAX_PARTICIPATION_FLOOR_BPS: u32 = 2_000;
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
