use fluentbase_sdk::{derive::derive_keccak256_id, derive::erc7201_slot, U256};

pub const SIG_LEN_BYTES: usize = 4;

// Existing Solidity ABI selectors. Keeping these explicit makes accidental ABI
// drift visible in review.
pub const SIG_INITIALIZE: u32 = 0x01f6_bb50;
pub const SIG_CURRENT_EPOCH: u32 = 0x7667_1808;
pub const SIG_NEXT_EPOCH: u32 = 0xaea0_e78b;
pub const SIG_OWNER: u32 = 0x8da5_cb5b;
pub const SIG_IS_VALIDATOR: u32 = 0xfacd_743b;
pub const SIG_IS_VALIDATOR_ACTIVE: u32 = 0x42ad_55ac;
pub const SIG_GET_VALIDATOR_STATUS: u32 = 0xa310_624f;
pub const SIG_GET_VALIDATOR_BY_OWNER: u32 = 0x3010_8c22;
pub const SIG_GET_VALIDATORS: u32 = 0xb7ab_4db5;
pub const SIG_ADD_VALIDATOR: u32 = 0x4d23_8c8e;
pub const SIG_REMOVE_VALIDATOR: u32 = 0x40a1_41ff;
pub const SIG_ACTIVATE_VALIDATOR: u32 = 0xb46e_5520;
pub const SIG_DISABLE_VALIDATOR: u32 = 0x1fe9_7684;
pub const SIG_CHANGE_VALIDATOR_COMMISSION_RATE: u32 = 0x14f8_649f;
pub const SIG_CHANGE_VALIDATOR_OWNER: u32 = 0x0052_c9e1;

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
pub const ERR_UNKNOWN_METHOD: u32 = derive_keccak256_id!("UnknownMethod()");

pub const BALANCE_COMPACT_PRECISION: U256 = U256::from_limbs([10_000_000_000, 0, 0, 0]);
pub const COMMISSION_RATE_MAX: u16 = 3_000;
pub const DEFAULT_EPOCH_BLOCK_INTERVAL: u64 = 200;
pub const DEFAULT_ACTIVE_VALIDATORS_LENGTH: u64 = 21;

pub const INITIALIZED_SLOT: U256 = erc7201_slot!("Fluent.storage.Staking.initialized");
pub const OWNER_SLOT: U256 = erc7201_slot!("Fluent.storage.Staking.owner");
pub const ACTIVATION_BLOCK_SLOT: U256 = erc7201_slot!("Fluent.storage.Staking.activation-block");
pub const EPOCH_INTERVAL_SLOT: U256 = erc7201_slot!("Fluent.storage.Staking.epoch-interval");
pub const ACTIVE_VALIDATORS_LENGTH_SLOT: U256 =
    erc7201_slot!("Fluent.storage.Staking.active-validators-length");
pub const ACTIVE_VALIDATORS_SLOT: U256 = erc7201_slot!("Fluent.storage.Staking.active-validators");
pub const VALIDATOR_OWNER_SLOT: U256 = erc7201_slot!("Fluent.storage.Staking.validator-owner");
pub const OWNER_VALIDATOR_SLOT: U256 = erc7201_slot!("Fluent.storage.Staking.owner-validator");
pub const VALIDATOR_STATUS_SLOT: U256 = erc7201_slot!("Fluent.storage.Staking.validator-status");
pub const VALIDATOR_TOTAL_DELEGATED_SLOT: U256 =
    erc7201_slot!("Fluent.storage.Staking.validator-total-delegated");
pub const VALIDATOR_SLASHES_SLOT: U256 = erc7201_slot!("Fluent.storage.Staking.validator-slashes");
pub const VALIDATOR_CHANGED_AT_SLOT: U256 =
    erc7201_slot!("Fluent.storage.Staking.validator-changed-at");
pub const VALIDATOR_JAILED_BEFORE_SLOT: U256 =
    erc7201_slot!("Fluent.storage.Staking.validator-jailed-before");
pub const VALIDATOR_CLAIMED_AT_SLOT: U256 =
    erc7201_slot!("Fluent.storage.Staking.validator-claimed-at");
pub const VALIDATOR_COMMISSION_RATE_SLOT: U256 =
    erc7201_slot!("Fluent.storage.Staking.validator-commission-rate");
