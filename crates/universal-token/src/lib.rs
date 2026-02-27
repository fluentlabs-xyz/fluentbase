//! Universal Token Standard (UST) primitives shared across execution environments.
#![cfg_attr(not(feature = "std"), no_std)]
#![forbid(unused_results)]

extern crate alloc;
extern crate core;

pub mod command;
pub mod consts;
pub mod events;
pub mod storage;
