//! ABI-like encoding and decoding primitives used across Fluentbase runtimes and contracts.
#![cfg_attr(not(feature = "std"), no_std)]
#![allow(unused_imports)]
extern crate alloc;
extern crate core;

pub mod bytes_codec;
pub mod encoder;
mod error;
mod evm;
mod func;
mod hash;
mod primitive;
mod tuple;
mod vec;

#[cfg(test)]
mod test_utils;
#[cfg(test)]
mod tests;

pub use ::byteorder;
pub use ::bytes;
pub use encoder::*;
pub use error::*;
#[cfg(feature = "derive")]
pub use fluentbase_codec_derive::Codec;
