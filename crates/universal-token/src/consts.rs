use fluentbase_sdk::{
    derive::{derive_evm_error, derive_keccak256, derive_keccak256_id},
    EvmExitCode, U256,
};

pub const ERR_UNKNOWN_METHOD: EvmExitCode = derive_evm_error!("UnknownMethod()");
pub const ERR_INSUFFICIENT_BALANCE: EvmExitCode = derive_evm_error!("InsufficientBalance()");
pub const ERR_INSUFFICIENT_ALLOWANCE: EvmExitCode = derive_evm_error!("InsufficientAllowance()");
pub const ERR_CONTRACT_NOT_MINTABLE: EvmExitCode = derive_evm_error!("ContractNotMintable()");
pub const ERR_CONTRACT_NOT_PAUSABLE: EvmExitCode = derive_evm_error!("ContractNotPausable()");
pub const ERR_ALREADY_PAUSED: EvmExitCode = derive_evm_error!("AlreadyPaused()");
pub const ERR_MINTING_PAUSED: EvmExitCode = derive_evm_error!("MintingPaused()");
pub const ERR_ALREADY_UNPAUSED: EvmExitCode = derive_evm_error!("AlreadyUnpaused()");
pub const ERR_INVALID_MINTER: EvmExitCode = derive_evm_error!("InvalidMinter()");
pub const ERR_PAUSER_MISMATCH: EvmExitCode = derive_evm_error!("PauserMismatch()");
pub const ERR_INVALID_RECIPIENT: EvmExitCode = derive_evm_error!("InvalidRecipient()");
pub const ERR_INTEGER_OVERFLOW: EvmExitCode = derive_evm_error!("Overflow()");

pub const SIG_SYMBOL: EvmExitCode = derive_keccak256_id!("symbol()");
pub const SIG_NAME: EvmExitCode = derive_keccak256_id!("name()");
pub const SIG_DECIMALS: EvmExitCode = derive_keccak256_id!("decimals()");
pub const SIG_TOTAL_SUPPLY: EvmExitCode = derive_keccak256_id!("totalSupply()");
pub const SIG_BALANCE: EvmExitCode = derive_keccak256_id!("balance()");
pub const SIG_BALANCE_OF: EvmExitCode = derive_keccak256_id!("balanceOf(address)");
pub const SIG_TRANSFER: EvmExitCode = derive_keccak256_id!("transfer(address,uint256)");
pub const SIG_TRANSFER_FROM: EvmExitCode =
    derive_keccak256_id!("transferFrom(address,address,uint256)");
pub const SIG_ALLOWANCE: EvmExitCode = derive_keccak256_id!("allowance(address)");
pub const SIG_APPROVE: EvmExitCode = derive_keccak256_id!("approve(address,uint256)");
pub const SIG_MINT: EvmExitCode = derive_keccak256_id!("mint(address,uint256)");
pub const SIG_PAUSE: EvmExitCode = derive_keccak256_id!("pause()");
pub const SIG_UNPAUSE: EvmExitCode = derive_keccak256_id!("unpause()");
pub const SIG_TOKEN2022: EvmExitCode = derive_keccak256_id!("token2022()");

pub const TOTAL_SUPPLY_STORAGE_SLOT: U256 =
    U256::from_le_bytes(derive_keccak256!(total_supply_slot));
pub const MINTER_STORAGE_SLOT: U256 =
    U256::from_le_bytes(derive_keccak256!(total_supply_slotminter_slot));
pub const PAUSER_STORAGE_SLOT: U256 = U256::from_le_bytes(derive_keccak256!(pauser_slot));
pub const CONTRACT_FROZEN_STORAGE_SLOT: U256 =
    U256::from_le_bytes(derive_keccak256!(contract_frozen_slot));
pub const SYMBOL_STORAGE_SLOT: U256 = U256::from_le_bytes(derive_keccak256!(symbol_slot));
pub const NAME_STORAGE_SLOT: U256 = U256::from_le_bytes(derive_keccak256!(name_slot));
pub const DECIMALS_STORAGE_SLOT: U256 = U256::from_le_bytes(derive_keccak256!(decimals_slot));
pub const FLAGS_STORAGE_SLOT: U256 = U256::from_le_bytes(derive_keccak256!(flags_slot));
pub const ALLOWANCE_STORAGE_SLOT: U256 = U256::from_le_bytes(derive_keccak256!(allowance_slot));
pub const BALANCE_STORAGE_SLOT: U256 = U256::from_le_bytes(derive_keccak256!(balance_slot));

#[allow(unused)]
const fn assert_unique_u32<const N: usize>(values: [u32; N]) {
    let mut i = 0;
    while i < N {
        let mut j = i + 1;
        while j < N {
            if values[i] == values[j] {
                panic!("duplicate u32 constant detected");
            }
            j += 1;
        }
        i += 1;
    }
}

const _: () = assert_unique_u32([
    ERR_UNKNOWN_METHOD,
    ERR_INSUFFICIENT_BALANCE,
    ERR_INSUFFICIENT_ALLOWANCE,
    ERR_CONTRACT_NOT_MINTABLE,
    ERR_CONTRACT_NOT_PAUSABLE,
    ERR_ALREADY_PAUSED,
    ERR_MINTING_PAUSED,
    ERR_ALREADY_UNPAUSED,
    ERR_INVALID_MINTER,
    ERR_PAUSER_MISMATCH,
    ERR_INVALID_RECIPIENT,
    ERR_INTEGER_OVERFLOW,
]);

const _: () = assert_unique_u32([
    SIG_SYMBOL,
    SIG_NAME,
    SIG_DECIMALS,
    SIG_TOTAL_SUPPLY,
    SIG_BALANCE,
    SIG_BALANCE_OF,
    SIG_TRANSFER,
    SIG_TRANSFER_FROM,
    SIG_ALLOWANCE,
    SIG_APPROVE,
    SIG_MINT,
    SIG_PAUSE,
    SIG_UNPAUSE,
    SIG_TOKEN2022,
]);
