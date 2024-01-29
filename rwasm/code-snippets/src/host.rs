#[cfg(feature = "host_basefee")]
mod basefee;
#[cfg(feature = "host_blockhash")]
mod blockhash;
#[cfg(feature = "host_call")]
mod call;
#[cfg(feature = "host_chainid")]
mod chainid;
#[cfg(feature = "host_coinbase")]
mod coinbase;
#[cfg(feature = "host_gaslimit")]
mod gaslimit;
#[cfg(feature = "host_number")]
mod number;
#[cfg(feature = "host_sload")]
mod sload;
#[cfg(feature = "host_sstore")]
mod sstore;
#[cfg(feature = "host_staticcall")]
mod staticcall;
#[cfg(feature = "host_timestamp")]
mod timestamp;
#[cfg(feature = "host_tload")]
mod tload;
#[cfg(feature = "host_tstore")]
mod tstore;
