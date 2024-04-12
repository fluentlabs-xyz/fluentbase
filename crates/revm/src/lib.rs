#![doc = "Revm is a Rust EVM implementation."]
#![warn(rustdoc::all, unreachable_pub)]
#![allow(rustdoc::bare_urls)]
#![cfg_attr(not(test), warn(unused_crate_dependencies))]
#![deny(unused_must_use, rust_2018_idioms)]
#![cfg_attr(not(feature = "std"), no_std)]

#[macro_use]
#[cfg(not(feature = "std"))]
extern crate alloc as std;

// Define modules.

mod builder;
mod context;

#[cfg(any(test, feature = "test-utils"))]
pub mod test_utils;

pub mod db;
mod evm;
mod frame;
pub mod gas;
pub mod handler;
mod inspector;
mod journaled_state;
#[cfg(feature = "optimism")]
pub mod optimism;
#[cfg(test)]
mod test;
mod types;

// Export items.

pub use builder::EvmBuilder;
pub use context::{
    Context, ContextPrecompile, ContextPrecompiles, ContextStatefulPrecompile,
    ContextStatefulPrecompileArc, ContextStatefulPrecompileBox, ContextStatefulPrecompileMut,
    ContextWithHandlerCfg, EvmContext, InnerEvmContext,
};
pub use db::{
    CacheState, DBBox, State, StateBuilder, StateDBBox, TransitionAccount, TransitionState,
};
pub use db::{Database, DatabaseCommit, DatabaseRef, InMemoryDB};
pub use evm::{Evm, CALL_STACK_LIMIT};
pub use frame::{CallFrame, CreateFrame, Frame, FrameData, FrameOrResult, FrameResult};
pub use handler::Handler;
pub use inspector::{inspector_handle_register, inspectors, GetInspector, Inspector};
pub use journaled_state::{JournalCheckpoint, JournalEntry, JournaledState};
// export Optimism types, helpers, and constants
#[cfg(feature = "optimism")]
pub use optimism::{L1BlockInfo, BASE_FEE_RECIPIENT, L1_BLOCK_CONTRACT, L1_FEE_RECIPIENT};

// Reexport libraries

pub mod interpreter {
    pub use super::types::*;
}

#[doc(inline)]
pub use revm_precompile as precompile;
#[doc(inline)]
pub use revm_primitives as primitives;
