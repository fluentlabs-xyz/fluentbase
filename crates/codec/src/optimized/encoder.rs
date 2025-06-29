//! Core zero-alloc ABI encoder infrastructure.
//!
//! Two–phase contract for every value **V**:
//!  1. `build_ctx` – computes and accumulates only “head” words of `V` and recursively the heads of
//!     its children (DFS preorder).
//!  2. `encode_tail` – encodes only the data/tail part of `V` and recursively the tails of the
//!     children.
//!
//! A static value (`u32`, `bool`, …) implements `build_ctx = Ok(())` and writes itself in
//! `encode_header`. A dynamic value (`Vec<T>`, `String`, …) writes its own header in `build_ctx`,
//! then invokes `build_ctx` on every element; `encode_tail` writes the data of every element in the
//! same order.
//!
//! ABI wrappers decide which passes to call:
//! * `SolidityABI` – head pass **then** tail pass (32-byte words).
//! * `CompactABI` – head pass for the whole tree, then tail pass.
//! * `SolidityPackedABI` – single `encode` call, compile-time limited to non-dynamic types.

use crate::optimized::{
    counter::ByteCounter,
    ctx::EncodingContext,
    error::CodecError,
    utils::align_up,
};
use byteorder::{BigEndian, ByteOrder};
use bytes::{Buf, BufMut};
use core::marker::PhantomData;

/// ABI Encoder trait. Encodes a type into ABI-compliant bytes in a zero-allocation, phase-based
/// manner.
pub trait Encoder<B: ByteOrder, const ALIGN: usize, const SOL_MODE: bool>: Sized {
    const HEADER_SIZE: usize;
    const IS_DYNAMIC: bool;

    fn header_size(&self) -> usize {
        const {
            assert!(!Self::IS_DYNAMIC, "dynamic type must override header_size");
        }
        align_up::<ALIGN>(Self::HEADER_SIZE)
    }

    fn encode_header(
        &self,
        out: &mut impl BufMut,
        ctx: &mut EncodingContext,
    ) -> Result<usize, CodecError>;

    fn encode_tail(
        &self,
        out: &mut impl BufMut,
        ctx: &mut EncodingContext,
    ) -> Result<usize, CodecError> {
        if !Self::IS_DYNAMIC {
            Ok(0)
        } else {
            unreachable!("Dynamic types must override encode_tail")
        }
    }

    fn encode(
        &self,
        out: &mut impl BufMut,
        ctx: &mut EncodingContext,
    ) -> Result<usize, CodecError> {
        ctx.hdr_size = self.header_size() as u32;
        let head = self.encode_header(out, ctx)?;
        let tail = if Self::IS_DYNAMIC {
            self.encode_tail(out, ctx)?
        } else {
            0
        };
        Ok(head + tail)
    }

    fn decode(buf: &impl Buf, offset: usize) -> Result<Self, CodecError>;

    fn tail_size(&self, ctx: &mut EncodingContext) -> Result<usize, CodecError> {
        let mut counter = ByteCounter::new();
        self.encode_tail(&mut counter, ctx)?;
        Ok(counter.count())
    }

    fn len(&self) -> usize {
        1
    }
}

/// Marker trait for SolidityPackedABI: only static types are allowed.
pub trait PackedSafe {
    const ASSERT_STATIC: ();
}

impl<T> PackedSafe for T
where
    T: Encoder<BigEndian, 1, true>,
{
    const ASSERT_STATIC: () = assert!(
        !T::IS_DYNAMIC,
        "SolidityPackedABI does not support dynamic types"
    );
}

/// ABI wrapper macro for different ABI flavors (SolidityABI, CompactABI, SolidityPackedABI).
macro_rules! define_abi {
    // Generic ABI (header_size, header, tail encoding separately)
    ($name:ident, $byte_order:ty, $align:expr, $sol_mode:expr) => {
        pub struct $name<T>(PhantomData<T>);

        impl<T> $name<T>
        where
            T: Encoder<$byte_order, $align, $sol_mode>,
        {
            #[inline]
            pub fn header_size(value: &T) -> usize {
                value.header_size()
            }

            #[inline]
            pub fn encode_header(
                value: &T,
                buf: &mut impl BufMut,
                ctx: &mut EncodingContext,
            ) -> Result<usize, CodecError> {
                value.encode_header(buf, ctx)
            }

            #[inline]
            pub fn encode_tail(
                value: &T,
                buf: &mut impl BufMut,
                ctx: &mut EncodingContext,
            ) -> Result<usize, CodecError> {
                value.encode_tail(buf, ctx)
            }

            #[inline]
            pub fn encode(
                value: &T,
                buf: &mut impl BufMut,
                ctx: &mut EncodingContext,
            ) -> Result<usize, CodecError> {
                let header_bytes = Self::encode_header(value, buf, ctx)?;
                let tail_bytes = if T::IS_DYNAMIC {
                    Self::encode_tail(value, buf, ctx)?
                } else {
                    0
                };
                Ok(header_bytes + tail_bytes)
            }

            #[inline]
            pub fn decode(buf: &impl Buf, offset: usize) -> Result<T, CodecError> {
                T::decode(buf, offset)
            }
        }
    };

    // Packed Solidity ABI (static types, single pass)
    ($name:ident, $byte_order:ty, $align:expr, $sol_mode:expr, packed) => {
        pub struct $name<T>(PhantomData<T>);
        impl<T> $name<T>
        where
            T: Encoder<$byte_order, $align, $sol_mode> + PackedSafe,
        {
            #[inline]
            pub fn encode(
                value: &T,
                buf: &mut impl BufMut,
                ctx: &mut EncodingContext,
            ) -> Result<usize, CodecError> {
                let _ = <T as PackedSafe>::ASSERT_STATIC;
                value.encode(buf, ctx)
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::optimized::ctx::EncodingContext;

    #[test]
    fn test_example_usage() {
        // SolidityABI
        let mut buf = Vec::new();
        let mut ctx = EncodingContext::default();
        let value = 42u8;
        // TODO: remove ctx usage from SolidityABI and CompactABI and SolidityPackedABI
        SolidityABI::encode(&value, &mut buf, &mut ctx).unwrap();
        println!("buf: {:?}", hex::encode(&buf));

        // SolidityPackedABI
        let mut buf = Vec::new();
        let mut ctx = EncodingContext::default();
        let value = 42u8;
        SolidityPackedABI::encode(&value, &mut buf, &mut ctx).unwrap();
        println!("buf: {:?}", hex::encode(&buf));

        // CompactABI
        let mut buf = Vec::new();
        let mut ctx = EncodingContext::default();
        let value = 42u8;
        CompactABI::encode(&value, &mut buf, &mut ctx).unwrap();
        println!("buf: {:?}", hex::encode(&buf));
    }
}
