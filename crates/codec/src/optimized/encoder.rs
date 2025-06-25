//! Core zero-alloc ABI encoder infrastructure.
//!
//! Two–phase contract for every value **V**:
//!  1. `encode_head` – prints only “head” words of `V` **and** recursively the heads of its
//!     children (DFS preorder).
//!  2. `encode_tail` – prints only the data/tail part of `V` (and recursively the tails of the
//!     children).
//!
//! A static value (`u32`, `bool`, …) implements `encode_head = Ok(0)` and
//! writes itself in `encode_tail`.
//! A dynamic value (`Vec<T>`, `String`, …) writes its own header in
//! `encode_head`, then invokes `encode_head` on every element; `encode_tail`
//! writes the data of every element in the same order.
//!
//! ABI wrappers decide which passes to call:
//! * `SolidityABI`          – head pass **then** tail pass (32-byte words).
//! * `CompactABI`           – head pass for the whole tree, then tail pass.
//! * `SolidityPackedABI`    – single `encode` call, compile-time limited to non-dynamic types.

use crate::optimized::{counter::ByteCounter, error::CodecError};
use byteorder::{BigEndian, ByteOrder};
use bytes::{Buf, BufMut};
use core::marker::PhantomData;
/* ──────────────────────── EncodingContext ──────────────────────── */
use smallvec::SmallVec;

/// Metadata collected during pass 1 of encoding.
/// Each `NodeMeta` represents a node in the pre-order tree of dynamic values.
#[derive(Debug)]
pub struct NodeMeta {
    pub len: usize,               // number of children (0 = leaf/static)
    pub tail: usize,             // size of data only (no headers)
}


/// Runtime state shared by all recursive `encode_*` calls.
///
/// * `depth`     – current recursion depth (DoS guard)
/// * `max_depth` – hard limit, panic-safe error if exceeded
/// * `nodes`     – collected node metadata for two-pass flat encoding
#[derive(Debug)]
pub struct EncodingContext {
    pub depth:     u32,
    pub max_depth: u32,
    pub nodes:     SmallVec<[NodeMeta; 32]>,
}

impl EncodingContext {
    #[inline]
    pub fn new() -> Self {
        Self {
            depth: 0,
            max_depth: 32,
            nodes: SmallVec::new(),
        }
    }

    #[inline]
    pub fn enter(&mut self) -> Result<(), CodecError> {
        if self.depth == self.max_depth {
            return Err(CodecError::InvalidData("max depth exceeded"));
        }
        self.depth += 1;
        Ok(())
    }

    #[inline]
    pub fn exit(&mut self) {
        self.depth -= 1;
    }

    #[inline]
    pub fn clear_for_encode(&mut self) {
        self.depth = 0;
        self.nodes.clear();
    }
}



/* ───────────────────────── Encoder trait ───────────────────────── */
pub trait Encoder<B: ByteOrder, const ALIGN: usize, const SOL_MODE: bool>: Sized {
    const IS_DYNAMIC: bool;


    const HEADER_SIZE: usize;

    /* pass-1: head */
    fn encode_head(
        &self,
        out: &mut impl BufMut,
        ctx: &mut EncodingContext,
    ) -> Result<usize, CodecError>;

    /* pass-2: tail / data */
    fn encode_tail(
        &self,
        out: &mut impl BufMut,
        ctx: &mut EncodingContext,
    ) -> Result<usize, CodecError>;

    /* helper: single-call encode (head + tail) */
    #[inline(always)]
    fn encode(
        &self,
        out: &mut impl BufMut,
        ctx: &mut EncodingContext,
    ) -> Result<usize, CodecError> {
        let mut n = self.encode_head(out, ctx)?;
        n += self.encode_tail(out, ctx)?;
        Ok(n)
    }

    /* dry-run size of tail */
    fn tail_size(&self, ctx: &mut EncodingContext) -> Result<usize, CodecError> {
        let mut counter = ByteCounter::new();
        self.encode_tail(&mut counter, ctx)?;
        Ok(counter.count())
    }

    fn len(&self) -> usize {
        1
    }

    /* mandatory decode */
    fn decode(buf: &impl Buf, offset: usize) -> Result<Self, CodecError>;
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
                let mut ctx = EncodingContext::new();
                value.encode_head(buf, &mut ctx)?;
                value.encode_tail(buf, &mut ctx)
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
                let mut ctx = EncodingContext::new();
                value.encode(buf, &mut ctx)
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
