//! Core functionality for the router macro implementation.
//! This crate provides the base functionality used by the proc-macro crate.

pub mod abi;
pub mod attr;
pub mod client;
mod codec;
mod method;
pub mod router;
mod signature;
pub mod sol_input;
pub mod storage;
#[deprecated(
    note = "Use `fluentbase_sdk_derive_core::storage` instead",
    since = "0.4.5-dev"
)]
pub mod storage_legacy;
mod utils;
