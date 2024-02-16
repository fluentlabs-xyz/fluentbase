#![allow(dead_code, unreachable_patterns, unused_macros)]
#![warn(unused_crate_dependencies)]

// #[cfg(feature = "rwasm")]
// pub(crate) use rwasm;
// #[cfg(not(feature = "rwasm"))]
// pub(crate) use wasmi as rwasm;

pub mod instruction;
mod macros;
mod runtime;

pub use runtime::*;

mod storage;

pub use storage::*;

mod journal;

pub use journal::*;

#[cfg(test)]
mod tests;
pub mod types;
pub mod zktrie;
