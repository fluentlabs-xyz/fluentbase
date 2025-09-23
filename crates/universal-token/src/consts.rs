use fluentbase_sdk::derive::{derive_evm_error, derive_keccak256_id};

pub const ERR_MALFORMED_INPUT: u32 = derive_evm_error!("MalformedInput()");
pub const ERR_INSUFFICIENT_BALANCE: u32 = derive_evm_error!("InsufficientBalance()");
pub const ERR_INSUFFICIENT_ALLOWANCE: u32 = derive_evm_error!("InsufficientAllowance()");
pub const ERR_INDEX_OUT_OF_BOUNDS: u32 = derive_evm_error!("IndexOutOfBounds()");
pub const ERR_MINTABLE_PLUGIN_NOT_ACTIVE: u32 = derive_evm_error!("MintablePluginNotActive()");
pub const ERR_PAUSABLE_PLUGIN_NOT_ACTIVE: u32 = derive_evm_error!("PausablePluginNotActive()");
pub const ERR_ALREADY_PAUSED: u32 = derive_evm_error!("AlreadyPaused()");
pub const ERR_ALREADY_UNPAUSED: u32 = derive_evm_error!("AlreadyUnpaused()");
pub const ERR_INVALID_MINTER: u32 = derive_evm_error!("InvalidMinter()");
pub const ERR_INVALID_PAUSER: u32 = derive_evm_error!("InvalidPauser()");
pub const ERR_INVALID_RECIPIENT: u32 = derive_evm_error!("InvalidRecipient()");
pub const ERR_OVERFLOW: u32 = derive_evm_error!("Overflow()");
pub const ERR_UNINIT: u32 = derive_evm_error!("UninitError()");
pub const ERR_CONVERSION: u32 = derive_evm_error!("ConversionError()");

pub const SIG_DECIMALS: u32 = derive_keccak256_id!("decimals(pubkey)"); // mint
pub const SIG_TOTAL_SUPPLY: u32 = derive_keccak256_id!("totalSupply()");
pub const SIG_BALANCE: u32 = derive_keccak256_id!("balance()");
pub const SIG_BALANCE_OF: u32 = derive_keccak256_id!("balanceOf(pubkey)"); // account
pub const SIG_TRANSFER: u32 = derive_keccak256_id!("transfer(pubkey,pubkey,u64)"); // to, authority, amount
pub const SIG_TRANSFER_FROM: u32 = derive_keccak256_id!("transferFrom(pubkey,pubkey,pubkey,u64)"); // from, to, authority, amount
pub const SIG_INITIALIZE_MINT: u32 = derive_keccak256_id!("initializeMint(pubkey,pubkey)"); // mint, owner, freeze, decimals (u8)
pub const SIG_INITIALIZE_ACCOUNT: u32 =
    derive_keccak256_id!("initializeAccount(pubkey,pubkey,pubkey)"); // account, mint, owner
pub const SIG_MINT_TO: u32 = derive_keccak256_id!("initializeAccount(pubkey,pubkey,pubkey,u64)"); // mint, account, owner, amount
pub const SIG_ALLOWANCE: u32 = derive_keccak256_id!("allowance(pubkey,pubkey)"); // delegate, account
pub const SIG_APPROVE: u32 = derive_keccak256_id!("approve(pubkey,pubkey,pubkey,u64)"); // source, delegate, owner, amount
pub const SIG_APPROVE_CHECKED: u32 =
    derive_keccak256_id!("approveChecked(pubkey,pubkey,pubkey,pubkey,u64,u8)"); // source, mint, delegate, owner, amount, decimals
pub const SIG_REVOKE: u32 = derive_keccak256_id!("revoke(pubkey,pubkey)"); // source, owner
pub const SIG_SET_AUTHORITY: u32 = derive_keccak256_id!("setAuthority(pubkey,pubkey,u8,pubkey)"); // owned, new_authority, authority_type, owner
pub const SIG_BURN: u32 = derive_keccak256_id!("burn(pubkey,pubkey,pubkey,u64)"); // account,mint,authority,amount
pub const SIG_BURN_CHECKED: u32 = derive_keccak256_id!("burnChecked(pubkey,pubkey,pubkey,u64,u8)"); // account, mint, authority, amount, decimals
pub const SIG_CLOSE_ACCOUNT: u32 = derive_keccak256_id!("closeAccount(pubkey,pubkey,pubkey)"); // account, destination, owner
pub const SIG_FREEZE_ACCOUNT: u32 = derive_keccak256_id!("freezeAccount(pubkey,pubkey,pubkey)"); // account, mint, owner
pub const SIG_THAW_ACCOUNT: u32 = derive_keccak256_id!("thawAccount(pubkey,pubkey,pubkey)"); // account, mint, owner
pub const SIG_GET_ACCOUNT_DATA_SIZE: u32 =
    derive_keccak256_id!("getAccountDataSize(pubkey,extension_types)"); // mint, extension_types
pub const SIG_TOKEN2022: u32 = derive_keccak256_id!("token2022()");
