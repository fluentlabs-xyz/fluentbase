use crate::optimized::{error::CodecError, utils::align_up};
use byteorder::{BigEndian, ByteOrder, LittleEndian};
use bytes::{Buf, BufMut};
use core::marker::PhantomData;

/// Encoding context for recursive ABI encoding with bounded depth.
/// Tracks current recursion depth for safety.
pub struct EncodingContext {
    depth: usize,
    max_depth: usize,
}

impl EncodingContext {
    pub fn new() -> Self {
        Self {
            depth: 0,
            max_depth: 32,
        }
    }

    #[inline]
    pub fn enter(&mut self) -> Result<(), CodecError> {
        if self.depth >= self.max_depth {
            return Err(CodecError::InvalidData("max encoding depth exceeded"));
        }
        self.depth += 1;
        Ok(())
    }

    #[inline]
    pub fn exit(&mut self) {
        self.depth = self.depth.saturating_sub(1);
    }

    #[inline]
    pub fn depth(&self) -> usize {
        self.depth
    }
}

/// Core encoder trait for serialization/deserialization
pub trait Encoder<B: ByteOrder, const ALIGN: usize, const SOL_MODE: bool>: Sized {
    /// Size of the fixed part in bytes
    /// - For static types (u32, bool): the complete size
    /// - For dynamic types (Vec, String): the header/offset size
    const HEADER_SIZE: usize;

    /// Whether this type has dynamic size (e.g., Vec, String)
    const IS_DYNAMIC: bool;

    /// Encodes the value into the buffer
    ///
    /// # Arguments
    /// * `buf` - Target buffer
    /// * `ctx` - Encoding context (None for top-level, Some for nested dynamic types)
    fn encode(
        &self,
        buf: &mut impl BufMut,
        ctx: Option<&mut EncodingContext>,
    ) -> Result<usize, CodecError>;

    /// Decodes a value from the buffer
    ///
    /// # Arguments
    /// * `buf` - Source buffer
    /// * `offset` - Starting position in the buffer
    fn decode(buf: &impl Buf, offset: usize) -> Result<Self, CodecError>;

    fn partial_decode(_buf: &impl Buf, _offset: usize) -> Result<Self, CodecError> {
        todo!()
    }
}

/// Marker trait for packed encoding validation
pub trait PackedValidation {
    const ASSERT_STATIC: ();
}

// Blanket implementation with compile-time check
impl<T> PackedValidation for T
where
    T: Encoder<BigEndian, 1, true>,
{
    const ASSERT_STATIC: () = assert!(
        !T::IS_DYNAMIC,
        "SolidityPackedABI does not support dynamic types"
    );
}

macro_rules! define_abi_codec {
    ($name:ident, $byte_order:ty, $align:expr, $sol_mode:expr) => {
        pub struct $name<T>(PhantomData<T>);

        impl<T> $name<T>
        where
            T: Encoder<$byte_order, $align, $sol_mode>,
        {
            #[inline]
            pub fn is_dynamic() -> bool {
                T::IS_DYNAMIC
            }

            #[inline]
            pub fn encode(value: &T, buf: &mut impl BufMut) -> Result<usize, CodecError> {
                value.encode(buf, None)
            }

            #[inline]
            pub fn decode(buf: &impl Buf, offset: usize) -> Result<T, CodecError> {
                T::decode(buf, offset)
            }
        }
    };

    ($name:ident, $byte_order:ty, $align:expr, $sol_mode:expr, static_only) => {
        pub struct $name<T>(PhantomData<T>);

        impl<T> $name<T>
        where
            T: Encoder<$byte_order, $align, $sol_mode> + PackedValidation,
        {
            #[inline]
            pub fn is_dynamic() -> bool {
                T::IS_DYNAMIC
            }

            #[inline]
            pub fn encode(value: &T, buf: &mut impl BufMut) -> Result<usize, CodecError> {
                let _ = <T as PackedValidation>::ASSERT_STATIC;
                value.encode(buf, None)
            }

            #[inline]
            pub fn decode(buf: &impl Buf, offset: usize) -> Result<T, CodecError> {
                let _ = <T as PackedValidation>::ASSERT_STATIC;
                T::decode(buf, offset)
            }
        }
    };
}
define_abi_codec!(SolidityABI, BigEndian, 32, true);
define_abi_codec!(CompactABI, LittleEndian, 4, false);
define_abi_codec!(SolidityPackedABI, BigEndian, 1, true, static_only);

// Usage examples:
#[cfg(test)]
mod tests {
    use super::*;
    use crate::optimized::error::CodecError;

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

        // // CompactABI
        // let value = vec![1, 2, 3];
        // CompactABI::encode(&value, &mut buf)?;

        // // SolidityPackedABI - only static types
        // let value = 42u32;
        // SolidityPackedABI::encode(&value, &mut buf)?;

        // This won't compile:
        // let vec = vec![1, 2, 3];
        // SolidityPackedABI::encode(&vec, &mut buf)?;
        // Error: SolidityPackedABI does not support dynamic types
    }
}
