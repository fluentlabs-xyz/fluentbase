#![cfg_attr(not(feature = "std"), no_std)]

extern crate alloc;
extern crate core;

pub mod account;
pub mod account_utils;
pub mod builtins;
pub mod common;
pub mod compute_budget;
pub mod compute_budget_processor;
pub mod context;
// pub mod ed25519_instruction;
pub mod error;
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
pub mod nonce_account;
pub mod precompiles;
#[cfg(test)]
mod process_instruction_tests;
pub mod recent_blockhashes_account;
// #[cfg(test)]
// pub mod secp256k1_instruction;
// pub mod bpf_loader;
pub mod epoch_rewards;
pub mod epoch_schedule;
pub mod fluentbase;
pub mod hash;
pub mod loaders;
pub mod serialization;
pub mod solana_program;
pub mod system_instruction;
pub mod system_processor;
#[cfg(test)]
mod system_processor_tests;
pub mod system_program;
pub mod sysvar_cache;
#[cfg(test)]
pub mod test_helpers;
pub mod word_size;

pub use bincode;
pub use solana_account_info::{self as account_info, debug_account_data};
pub use solana_bincode;
pub use solana_clock as clock;
pub use solana_pubkey as pubkey;
pub use solana_rent as rent;
