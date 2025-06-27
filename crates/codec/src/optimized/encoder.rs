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

use crate::optimized::{counter::ByteCounter, ctx::EncodingContext, error::CodecError};
use byteorder::{BigEndian, ByteOrder};
use bytes::{Buf, BufMut};
use core::marker::PhantomData;

/// ABI Encoder trait. Encodes a type into ABI-compliant bytes in a zero-allocation, phase-based
/// manner.
pub trait Encoder<B: ByteOrder, const ALIGN: usize, const SOL_MODE: bool>: Sized {
    /// Context type for multi-phase encoding (stack state, offsets, etc).
    type Ctx: Default;

    /// For static values - actual size; for dynamic values - metadata size.
    const HEADER_SIZE: usize;

    /// If a value is dynamic, it is encoded in three phases:
    /// 1. `header_size` – calculates offsets and sizes of dynamic fields.
    /// 2. `encode_header` – encodes static fields and offsets for dynamic fields.
    /// 3. `encode_tail` – encodes the actual data of dynamic fields.
    const IS_DYNAMIC: bool;

    /// Build offset layouts for dynamic fields and calculate actual data sizes.
    fn header_size(&self, ctx: &mut Self::Ctx) -> Result<(), CodecError> {
        const {
            assert!(!Self::IS_DYNAMIC, "dynamic type must override header_size");
        }
        Ok(())
    }

    /// Encodes static fields and offsets for dynamic fields (calculated in `header_size`).
    fn encode_header(
        &self,
        out: &mut impl BufMut,
        ctx: &mut Self::Ctx,
    ) -> Result<usize, CodecError>;

    /// Encodes dynamic fields (payload data).
    fn encode_tail(&self, out: &mut impl BufMut, ctx: &mut Self::Ctx) -> Result<usize, CodecError> {
        if !Self::IS_DYNAMIC {
            // For static values, no tail is encoded.
            Ok(0)
        } else {
            unreachable!("Dynamic types must override encode_tail")
        }
    }

    /// High-level encoding: performs header sizing, encodes header and tail.
    fn encode(&self, out: &mut impl BufMut, ctx: &mut Self::Ctx) -> Result<usize, CodecError> {
        self.header_size(ctx)?;
        let head = self.encode_header(out, ctx)?;
        let tail = if Self::IS_DYNAMIC {
            self.encode_tail(out, ctx)?
        } else {
            0
        };
        Ok(head + tail)
    }

    /// Decodes value from buffer at the given offset.
    fn decode(buf: &impl Buf, offset: usize) -> Result<Self, CodecError>;

    /// Calculates tail (data section) size for this value.
    fn tail_size(&self, ctx: &mut Self::Ctx) -> Result<usize, CodecError> {
        let mut counter = ByteCounter::new();
        self.encode_tail(&mut counter, ctx)?;
        Ok(counter.count())
    }

    /// Logical element count for this value (default is 1).
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
    // Generic ABI (header + tail).
    ($name:ident, $byte_order:ty, $align:expr, $sol_mode:expr) => {
        pub struct $name<T>(PhantomData<T>);
        impl<T> $name<T>
        where
            T: Encoder<$byte_order, $align, $sol_mode>,
        {
            #[inline]
            pub fn encode(
                value: &T,
                buf: &mut impl BufMut,
                ctx: &mut T::Ctx,
            ) -> Result<usize, CodecError> {
                T::encode(value, buf, ctx)
            }
            #[inline]
            pub fn decode(buf: &impl Buf, offset: usize) -> Result<T, CodecError> {
                T::decode(buf, offset)
            }
        }
    };
    // Packed Solidity ABI (static types, single pass).
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
                ctx: &mut T::Ctx,
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
