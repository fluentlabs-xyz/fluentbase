//! Rwasm API types.

pub mod builder;
pub mod default_ctx;
pub mod exec;
mod frame;

pub use builder::RwasmBuilder;
pub use default_ctx::{fluent_cfg, DefaultRwasm, RwasmContext};
pub use exec::{RwasmContextTr, RwasmError};
pub use frame::RwasmFrame;
use revm::primitives::hardfork::SpecId;

pub type RwasmSpecId = SpecId;
