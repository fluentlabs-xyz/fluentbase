#![cfg_attr(not(feature = "std"), no_std)]
// #![warn(unused_crate_dependencies)]

extern crate alloc;
extern crate core;

pub mod blended;
pub mod helpers;
mod helpers_svm;
pub mod svm;
mod types;
