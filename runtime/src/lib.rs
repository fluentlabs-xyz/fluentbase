#![cfg_attr(not(feature = "main"), no_std)]
#![allow(dead_code, unreachable_patterns, unused_macros)]

#[cfg(feature = "main")]
pub mod instruction;
#[cfg(feature = "main")]
mod macros;
#[cfg(feature = "main")]
mod platform;
#[cfg(feature = "main")]
pub use platform::*;
#[cfg(feature = "main")]
mod runtime;
#[cfg(feature = "main")]
pub use runtime::*;
#[cfg(feature = "main")]
mod complex_types;
#[cfg(feature = "main")]
mod consts;
#[cfg(feature = "main")]
mod storage;
#[cfg(test)]
mod tests;
#[cfg(any(feature = "main", feature = "types"))]
mod types;
#[cfg(feature = "main")]
mod types_impls;

#[cfg(any(feature = "main", feature = "types"))]
pub use types::*;
