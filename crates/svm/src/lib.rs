#![cfg_attr(not(feature = "std"), no_std)]
#![feature(new_range_api)]
#![feature(assert_matches)]
// #![feature(lazy_type_alias)]
#![feature(trait_alias)]

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
// pub mod feature_set;
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
// pub mod nonce;
pub mod nonce_account;
// pub mod nonce_current;
pub mod precompiles;
#[cfg(test)]
mod process_instruction_tests;
pub mod program_error;
pub mod recent_blockhashes_account;
// #[cfg(test)]
// pub mod secp256k1_instruction;
pub mod bpf_loader;
pub mod bpf_loader_deprecated;
pub mod epoch_rewards;
pub mod epoch_schedule;
// pub mod epoch_stake;
mod bincode_helpers;
pub mod fluentbase;
pub mod hash;
pub mod loaders;
// pub mod mem_ops_original;
pub mod serialization;
pub mod solana_program;
pub mod storage_helpers;
#[cfg(test)]
mod storage_helpers_tests;
pub mod system_instruction;
pub mod system_processor;
#[cfg(test)]
mod system_processor_tests;
pub mod system_program;
pub mod sysvar_cache;
#[cfg(test)]
pub mod test_helpers;
pub mod types;
pub mod word_size;
// mod test_macroses;

pub use bincode;
pub use {
    solana_account_info::{self as account_info, debug_account_data},
    solana_bincode,
    solana_clock as clock,
    // solana_msg::msg,
    // solana_native_token as native_token,
    // solana_program_entrypoint::{
    //     self as entrypoint,
    //     custom_heap_default,
    //     custom_panic_default,
    //     entrypoint,
    //     entrypoint_no_alloc,
    // },
    // solana_program_option as program_option,
    solana_pubkey as pubkey,
    solana_rent as rent,
};
