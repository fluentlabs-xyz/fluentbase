#![cfg_attr(not(feature = "std"), no_std)]

extern crate alloc;

mod decode_block;

mod tests;

mod enveloped;
mod header;
mod receipt;
mod result;
mod transaction;
pub mod util;

// Alias for `Vec<u8>`. This type alias is necessary for rlp-derive to work correctly.
type Bytes = alloc::vec::Vec<u8>;

pub use enveloped::*;
// pub use header::{Header, PartialHeader};
// pub use log::Log;
pub use receipt::*;
pub use transaction::*;
