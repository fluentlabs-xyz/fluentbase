use fluentbase_sdk::{derive::derive_keccak256_id, derive::erc7201_slot, U256};

pub const SIG_LEN_BYTES: usize = 4;

// ABI selectors are derived from their canonical signatures. The pinned hex
// values remain beside them to make ABI drift visible during review.
// 0x01f6bb50
pub const SIG_INITIALIZE: u32 =
    derive_keccak256_id!("initialize(address,address[],uint256[],uint16)");
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
// 0x40a141ff
pub const SIG_REMOVE_VALIDATOR: u32 = derive_keccak256_id!("removeValidator(address)");
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
// 0x7b1391a6
pub const SIG_GET_STAKING: u32 = derive_keccak256_id!("getStaking()");
// 0x289b3c0d
pub const SIG_GET_GOVERNANCE: u32 = derive_keccak256_id!("getGovernance()");
// 0x606c0c94
pub const SIG_GET_CHAIN_CONFIG: u32 = derive_keccak256_id!("getChainConfig()");
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
// 0xdc871561
pub const SIG_CONFIGURE: u32 =
    derive_keccak256_id!("configure(address,uint64,uint64,uint256,uint256)");
// 0x23b872dd
pub const SIG_ERC20_TRANSFER_FROM: u32 =
    derive_keccak256_id!("transferFrom(address,address,uint256)");

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
pub const ERR_VALIDATOR_HAS_ACTIVE_DELEGATIONS: u32 =
    derive_keccak256_id!("ValidatorHasActiveDelegations(address)");
pub const ERR_ONLY_VALIDATOR_OWNER: u32 = derive_keccak256_id!("OnlyValidatorOwner(address)");
pub const ERR_ONLY_OWNER: u32 = derive_keccak256_id!("OwnableUnauthorizedAccount(address)");
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
pub const ERR_STAKING_TOKEN_CALL_FAILED: u32 = derive_keccak256_id!("StakingTokenCallFailed()");
pub const ERR_UNKNOWN_METHOD: u32 = derive_keccak256_id!("UnknownMethod()");

pub const BALANCE_COMPACT_PRECISION: U256 = U256::from_limbs([10_000_000_000, 0, 0, 0]);
pub const COMMISSION_RATE_MAX: u16 = 3_000;
pub const DEFAULT_EPOCH_BLOCK_INTERVAL: u64 = 200;
pub const DEFAULT_ACTIVE_VALIDATORS_LENGTH: u64 = 21;
pub const MAX_ACTIVE_VALIDATORS_LENGTH: u64 = 51;
pub const DEFAULT_UNDELEGATE_PERIOD: u64 = 7;
pub const WARMUP_DELAY: u64 = 2;
pub const DEFAULT_MIN_VALIDATOR_STAKE: U256 =
    U256::from_limbs([1_000_000_000_000_000_000, 0, 0, 0]);
pub const DEFAULT_MIN_STAKING_AMOUNT: U256 = U256::from_limbs([1_000_000_000_000_000_000, 0, 0, 0]);

pub const STAKING_STORAGE_SLOT: U256 = erc7201_slot!("Fluent.storage.StakingStorage");
