//! Core zero-alloc ABI encoder infrastructure.
//!
//! Two–phase contract for every value **V**:
//!  1. `build_ctx` – prints only “head” words of `V` **and** recursively the heads of its children
//!     (DFS preorder).
//!  2. `encode_tail` – prints only the data/tail part of `V` (and recursively the tails of the
//!     children).
//!
//! A static value (`u32`, `bool`, …) implements `build_ctx = Ok(0)` and
//! writes itself in `encode_tail`.
//! A dynamic value (`Vec<T>`, `String`, …) writes its own header in
//! `build_ctx`, then invokes `build_ctx` on every element; `encode_tail`
//! writes the data of every element in the same order.
//!
//! ABI wrappers decide which passes to call:
//! * `SolidityABI` – head pass **then** tail pass (32-byte words).
//! * `CompactABI` – head pass for the whole tree, then tail pass.
//! * `SolidityPackedABI` – single `encode` call, compile-time limited to non-dynamic types.

use crate::optimized::{counter::ByteCounter, error::CodecError};
use byteorder::{BigEndian, ByteOrder};
use bytes::{Buf, BufMut};
use core::marker::PhantomData;

// TODO: maybe we should use EncodingContext trait instead of struct?
// with methods finalize
pub trait Encoder<B: ByteOrder, const ALIGN: usize, const SOL_MODE: bool>: Sized {
    // For primitive types, this is `()`.
    type Ctx: Default;

    // For static values - actual size.
    // For dynamic values - size of the metadata.
    const HEADER_SIZE: usize;

    // If a value is dynamic, it will be encoded in 3 passes:
    // 1. `build_ctx` – calculates offsets and sizes of dynamic fields.
    // 2. `encode_header` – encodes static fields and offsets for dynamic fields.
    // 3. `encode_tail` – encodes the actual data of dynamic fields.
    const IS_DYNAMIC: bool;



    // Build offset layouts for dynamic fields and calculate actual data sizes
    fn build_ctx(&self, ctx: &mut Self::Ctx) -> Result<(), CodecError> {
        const {
            assert!(!Self::IS_DYNAMIC, "dynamic type must override build_ctx");
        }
        Ok(())
    }

    // Encodes static fields and offsets for the dynamic fields (calculated in `build_ctx`)
    fn encode_header(&self, out: &mut impl BufMut, ctx: &Self::Ctx) -> Result<usize, CodecError>;

    // Encodes dynamic fields like raw data
    fn encode_tail(&self, out: &mut impl BufMut) -> Result<usize, CodecError> {
        if !Self::IS_DYNAMIC {
            // for static values we don't use encode_tail -
            // they are encoded in encode_header
            Ok(0)
        } else {
            unreachable!("Dynamic types should implement encode_tail")
        }
    }

    fn encode(&self, out: &mut impl BufMut) -> Result<usize, CodecError> {
        let mut ctx = Self::Ctx::default();
        self.build_ctx(&mut ctx)?;

        let head = self.encode_header(out, &ctx)?;
        let tail = if Self::IS_DYNAMIC {
            self.encode_tail(out)?
        } else {
            0
        };

        Ok(head + tail)
    }

    // decoding from a buffer at a given offset
    // note: it's responsibility of the caller to ensure that the buffer is clean and usually offset
    // should be 0 there is a known issue with decoding from a non-zero offset, which is not
    // supported yet
    fn decode(buf: &impl Buf, offset: usize) -> Result<Self, CodecError>;

    // TODO(d1r1): (maybe we don't need this method at all?)
    fn tail_size(&self) -> Result<usize, CodecError> {
        let mut counter = ByteCounter::new();
        self.encode_tail(&mut counter)?;
        Ok(counter.count())
    }

    fn len(&self) -> usize {
        1
    }
}

/* ───────────────── Static-only marker for packed ABI ───────────── */
pub trait PackedSafe {
    const ASSERT_STATIC: ();
}
// Blanket implementation with compile-time check
impl<T> PackedSafe for T
where
    T: Encoder<BigEndian, 1, true>,
{
    const ASSERT_STATIC: () = assert!(
        !T::IS_DYNAMIC,
        "SolidityPackedABI does not support dynamic types"
    );
}

/* ──────────────────────── ABI wrappers ─────────────────────────── */
macro_rules! define_abi {
    // generic ABI (head → tail)
    ($name:ident, $byte_order:ty, $align:expr, $sol_mode:expr) => {
        pub struct $name<T>(PhantomData<T>);
        impl<T> $name<T>
        where
            T: Encoder<$byte_order, $align, $sol_mode>,
        {
            #[inline]
            pub fn encode(value: &T, buf: &mut impl BufMut) -> Result<usize, CodecError> {
                T::encode(value, buf)
            }
            #[inline]
            pub fn decode(buf: &impl Buf, offset: usize) -> Result<T, CodecError> {
                T::decode(buf, offset)
            }
        }
    };
    // packed Solidity – static only, single pass
    ($name:ident, $byte_order:ty, $align:expr, $sol_mode:expr, packed) => {
        pub struct $name<T>(PhantomData<T>);

        impl<T> $name<T>
        where
            T: Encoder<$byte_order, $align, $sol_mode> + PackedSafe,
        {
            #[inline]
            pub fn encode(value: &T, buf: &mut impl BufMut) -> Result<usize, CodecError> {
                let _ = <T as PackedSafe>::ASSERT_STATIC;
                value.encode(buf)
            }
            #[inline]
            pub fn decode(buf: &impl Buf, offset: usize) -> Result<T, CodecError> {
                let _ = <T as PackedSafe>::ASSERT_STATIC;
                T::decode(buf, offset)
            }
        }
    };
}

define_abi!(SolidityABI, byteorder::BigEndian, 32, true);
define_abi!(CompactABI, byteorder::LittleEndian, 4, false);
define_abi!(SolidityPackedABI, byteorder::BigEndian, 1, true, packed);

// Usage examples:
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_example_usage() {
        // SolidityABI
        let mut buf = Vec::new();
        let value = 42u8;
        SolidityABI::encode(&value, &mut buf).unwrap();
        println!("buf: {:?}", hex::encode(&buf));

        // SolidityPackedABI
        let mut buf = Vec::new();
        let value = 42u8;
        SolidityPackedABI::encode(&value, &mut buf).unwrap();
        println!("buf: {:?}", hex::encode(&buf));

        // CompactABI
        let mut buf = Vec::new();
        let value = 42u8;
        CompactABI::encode(&value, &mut buf).unwrap();
        println!("buf: {:?}", hex::encode(&buf));
    }
}
