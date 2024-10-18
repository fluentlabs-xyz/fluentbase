pub use alloy_genesis::{ChainConfig, Genesis, GenesisAccount};
use fluentbase_types::{address, Address};

pub mod devnet;
mod macros;
mod utils;

/// Example greeting `keccak256("_example_greeting")[12..]`
pub const EXAMPLE_GREETING_ADDRESS: Address = address!("43799b91fb174261ec2950ebb819c2cff2983bdf");
