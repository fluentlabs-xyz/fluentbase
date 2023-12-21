#[cfg(feature = "host_basefee")]
mod basefee;
#[cfg(feature = "host_blockhash")]
mod blockhash;
#[cfg(feature = "host_chainid")]
mod chainid;
#[cfg(feature = "host_coinbase")]
mod coinbase;
#[cfg(feature = "host_gaslimit")]
mod gaslimit;
#[cfg(feature = "host_number")]
mod number;
#[cfg(feature = "host_sload")]
pub mod sload;
#[cfg(feature = "host_sstore")]
pub mod sstore;
#[cfg(feature = "host_timestamp")]
mod timestamp;
