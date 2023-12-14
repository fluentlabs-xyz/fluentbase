#![cfg_attr(not(feature = "std"), no_std)]

extern crate alloc;
extern crate core;

pub use crate::{
    buffer::{BufferDecoder, BufferEncoder},
    encoder::Encoder,
};

mod buffer;
mod encoder;
mod hash;
mod primitive;
mod serde;
#[cfg(test)]
mod tests;
mod vec;
