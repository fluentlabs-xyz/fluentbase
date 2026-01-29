//! Optimism-specific constants, types, and helpers.
#![cfg_attr(not(test), warn(unused_crate_dependencies))]
#![cfg_attr(not(feature = "std"), no_std)]

#[cfg(not(feature = "std"))]
extern crate alloc as std;

mod api;
mod eip2935;
mod evm;
mod executor;
mod handler;
mod inspector;
mod precompiles;
mod result;
mod spec;
mod syscall;
#[cfg(test)]
mod tests;
mod types;
#[cfg(feature = "fluent-testnet")]
mod upgrade;

pub use api::*;
pub use evm::RwasmEvm;
pub use handler::*;
pub use precompiles::*;
pub use result::*;
use rwasm as _;
pub use spec::*;
