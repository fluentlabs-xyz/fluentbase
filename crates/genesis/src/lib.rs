pub use alloy_genesis::{ChainConfig, Genesis, GenesisAccount};
use fluentbase_types::{address, Address};

pub mod devnet;

// example
pub const EXAMPLE_GREETING_ADDRESS: Address = address!("5300000000000000000000000000000000000001");
