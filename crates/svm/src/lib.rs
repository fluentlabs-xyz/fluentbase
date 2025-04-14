#![cfg_attr(not(feature = "std"), no_std)]
#![feature(new_range_api)]
#![feature(assert_matches)]
#![feature(liballoc_internals)]
// #![warn(unused_crate_dependencies)]

extern crate alloc;
extern crate core;
// #[macro_use]
// extern crate solana_rbpf;
// #[cfg(target_arch = "wasm32")]
// extern crate fluentbase_sdk;

pub mod account;
pub mod account_utils;
pub mod builtins;
pub mod common;
pub mod compute_budget;
pub mod compute_budget_processor;
pub mod context;
// pub mod ed25519_instruction;
pub mod error;
pub mod feature_set;
pub mod helpers;
// #[cfg(test)]
// mod helpers_tests;
pub mod loaded_programs;
pub mod macros;
pub mod mem_ops;
pub mod message_processor;
#[cfg(test)]
mod message_processor_tests;
pub mod native_loader;
pub mod nonce;
pub mod nonce_account;
pub mod nonce_current;
pub mod precompiles;
#[cfg(test)]
mod process_instruction_tests;
pub mod program_error;
pub mod recent_blockhashes_account;
// #[cfg(test)]
// pub mod secp256k1_instruction;
pub mod fluentbase;
pub mod loaders;
pub mod serialization;
pub mod storage_helpers;
#[cfg(test)]
mod storage_helpers_tests;
pub mod system_processor;
#[cfg(test)]
mod system_processor_tests;
pub mod system_program;
pub mod sysvar_cache;
#[cfg(test)]
pub mod test_helpers;
pub mod types;
// mod test_macroses;

pub use bincode;
pub use solana_program;
