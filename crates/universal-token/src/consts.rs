use fluentbase_sdk::{
    derive::{derive_evm_error, derive_keccak256_id, erc7201_slot},
    EvmExitCode, U256,
};

// Custom UST (Universal Token Standard) error codes
pub const ERR_UST_UNKNOWN_METHOD: EvmExitCode = derive_evm_error!("USTUnknownMethod(bytes4)");
pub const ERR_UST_NOT_PAUSABLE: EvmExitCode = derive_evm_error!("USTNotPausable()");
pub const ERR_UST_PAUSER_MISMATCH: EvmExitCode = derive_evm_error!("USTPauserMismatch(address)");
pub const ERR_UST_NOT_MINTABLE: EvmExitCode = derive_evm_error!("USTNotMintable()");
pub const ERR_UST_MINTER_MISMATCH: EvmExitCode = derive_evm_error!("USTMinterMismatch(address)");

// These errors are compliant with: @openzeppelin-contracts/contracts/interfaces/draft-IERC6093.sol
pub const ERR_ERC20_INSUFFICIENT_BALANCE: EvmExitCode =
    derive_evm_error!("ERC20InsufficientBalance(address,uint256,uint256)");
pub const ERR_ERC20_INVALID_SENDER: EvmExitCode = derive_evm_error!("ERC20InvalidSender(address)");
pub const ERR_ERC20_INVALID_RECEIVER: EvmExitCode =
    derive_evm_error!("ERC20InvalidReceiver(address)");
pub const ERR_ERC20_INSUFFICIENT_ALLOWANCE: EvmExitCode =
    derive_evm_error!("ERC20InsufficientAllowance(address,uint256,uint256)");
pub const ERR_ERC20_INVALID_APPROVER: EvmExitCode =
    derive_evm_error!("ERC20InvalidApprover(address)");
pub const ERR_ERC20_INVALID_SPENDER: EvmExitCode =
    derive_evm_error!("ERC20InvalidSpender(address)");

// These errors are compliant with: @openzeppelin-contracts/contracts/token/ERC20/extensions/ERC20Pausable.sol
pub const ERR_PAUSABLE_ENFORCED_PAUSE: EvmExitCode = derive_evm_error!("EnforcedPause()");
pub const ERR_PAUSABLE_EXPECTED_PAUSE: EvmExitCode = derive_evm_error!("ExpectedPause()");

// These signatures are compliant with: @openzeppelin-contracts/contracts/token/ERC20/IERC20.sol
pub const SIG_ERC20_SYMBOL: u32 = derive_keccak256_id!("symbol()");
pub const SIG_ERC20_NAME: u32 = derive_keccak256_id!("name()");
pub const SIG_ERC20_DECIMALS: u32 = derive_keccak256_id!("decimals()");
pub const SIG_ERC20_TOTAL_SUPPLY: u32 = derive_keccak256_id!("totalSupply()");
pub const SIG_ERC20_BALANCE: u32 = derive_keccak256_id!("balance()");
pub const SIG_ERC20_BALANCE_OF: u32 = derive_keccak256_id!("balanceOf(address)");
pub const SIG_ERC20_TRANSFER: u32 = derive_keccak256_id!("transfer(address,uint256)");
pub const SIG_ERC20_TRANSFER_FROM: u32 =
    derive_keccak256_id!("transferFrom(address,address,uint256)");
pub const SIG_ERC20_ALLOWANCE: u32 = derive_keccak256_id!("allowance(address,address)");
pub const SIG_ERC20_APPROVE: u32 = derive_keccak256_id!("approve(address,uint256)");
pub const SIG_ERC20_MINT: u32 = derive_keccak256_id!("mint(address,uint256)");
pub const SIG_ERC20_BURN: u32 = derive_keccak256_id!("burn(address,uint256)");
pub const SIG_ERC20_PAUSE: u32 = derive_keccak256_id!("pause()");
pub const SIG_ERC20_UNPAUSE: u32 = derive_keccak256_id!("unpause()");

// Not in use, reserved for future use
pub const SIG_TOKEN2022: u32 = derive_keccak256_id!("token2022()");

// Storage slots (all ERC7201 complaint)
pub const TOTAL_SUPPLY_STORAGE_SLOT: U256 = erc7201_slot!("universal-token.total-supply");
pub const MINTER_STORAGE_SLOT: U256 = erc7201_slot!("universal-token.minter");
pub const PAUSER_STORAGE_SLOT: U256 = erc7201_slot!("universal-token.pauser");
pub const CONTRACT_FROZEN_STORAGE_SLOT: U256 = erc7201_slot!("universal-token.contract-frozen");
pub const SYMBOL_STORAGE_SLOT: U256 = erc7201_slot!("universal-token.symbol");
pub const NAME_STORAGE_SLOT: U256 = erc7201_slot!("universal-token.name");
pub const DECIMALS_STORAGE_SLOT: U256 = erc7201_slot!("universal-token.decimals");
pub const ALLOWANCE_STORAGE_SLOT: U256 = erc7201_slot!("universal-token.allowance");
pub const BALANCE_STORAGE_SLOT: U256 = erc7201_slot!("universal-token.balance");

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
    ERR_UST_UNKNOWN_METHOD,
    ERR_UST_NOT_PAUSABLE,
    ERR_UST_PAUSER_MISMATCH,
    ERR_UST_NOT_MINTABLE,
    ERR_UST_MINTER_MISMATCH,
    ERR_ERC20_INSUFFICIENT_BALANCE,
    ERR_ERC20_INVALID_SENDER,
    ERR_ERC20_INVALID_RECEIVER,
    ERR_ERC20_INSUFFICIENT_ALLOWANCE,
    ERR_ERC20_INVALID_APPROVER,
    ERR_ERC20_INVALID_SPENDER,
    ERR_PAUSABLE_ENFORCED_PAUSE,
    ERR_PAUSABLE_EXPECTED_PAUSE,
]);

const _: () = assert_unique_u32([
    SIG_ERC20_SYMBOL,
    SIG_ERC20_NAME,
    SIG_ERC20_DECIMALS,
    SIG_ERC20_TOTAL_SUPPLY,
    SIG_ERC20_BALANCE,
    SIG_ERC20_BALANCE_OF,
    SIG_ERC20_TRANSFER,
    SIG_ERC20_TRANSFER_FROM,
    SIG_ERC20_ALLOWANCE,
    SIG_ERC20_APPROVE,
    SIG_ERC20_MINT,
    SIG_ERC20_BURN,
    SIG_ERC20_PAUSE,
    SIG_ERC20_UNPAUSE,
    SIG_TOKEN2022,
]);
