//! Interruptible EVM interpreter integration used by Fluentbase execution environments.
#![cfg_attr(not(feature = "std"), no_std)]
#![allow(dead_code)]

extern crate alloc;
extern crate core;

pub mod bytecode;

mod evm;
pub mod host;
mod metadata;
pub mod opcodes;
pub mod types;
mod utils;

pub use bytecode::AnalyzedBytecode;
pub use evm::EthVM;
pub use metadata::EthereumMetadata;
pub use revm_interpreter::{gas, InterpreterAction, InterpreterResult};
pub use types::{ExecutionResult, InterruptingInterpreter};
pub use utils::evm_gas_params;
