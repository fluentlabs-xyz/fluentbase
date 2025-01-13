pub use alloy_genesis::{ChainConfig, Genesis, GenesisAccount};
use fluentbase_types::{address, Address};

mod devnet;
mod macros;
mod utils;

/// Example greeting `keccak256("_example_greeting")[12..]`
pub const EXAMPLE_GREETING_ADDRESS: Address = address!("43799b91fb174261ec2950ebb819c2cff2983bdf");

/// Example fairblock `keccak256("_example_fairblock")[12..]`
pub const EXAMPLE_FAIRBLOCK_ADDRESS: Address = address!("d92adea71798aadff13f526556dea230214e0a30");

/// Example multicall `keccak256("_example_multicall")[12..]`
pub const EXAMPLE_MULTICALL_ADDRESS: Address = address!("9dafdaa2b09260d530ce3ffb304be59ca9613844");

pub use devnet::{
    devnet_chain_config,
    devnet_genesis,
    devnet_genesis_from_file,
    devnet_genesis_v0_1_0_dev1_from_file,
    devnet_genesis_v0_1_0_dev4_from_file,
    devnet_genesis_v0_1_0_dev5_from_file,
    GENESIS_KECCAK_HASH_SLOT,
    GENESIS_POSEIDON_HASH_SLOT,
};
