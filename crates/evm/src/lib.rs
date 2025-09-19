#![cfg_attr(not(feature = "std"), no_std)]
#![allow(dead_code)]

extern crate alloc;
extern crate core;

pub mod bytecode;

mod evm;
mod host;
mod opcodes;
mod types;
mod utils;

pub use bytecode::AnalyzedBytecode;
pub use evm::EthVM;
pub use revm_interpreter::gas;
pub use types::ExecutionResult;
