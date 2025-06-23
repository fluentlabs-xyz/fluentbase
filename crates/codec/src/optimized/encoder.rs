use crate::optimized::{error::CodecError, utils::align_up};
use byteorder::{BigEndian, ByteOrder, LittleEndian};
use bytes::{Buf, BufMut};
use core::marker::PhantomData;

/// Encoding context for complex types requiring offset calculation
pub struct EncodingContext {
    stack_offsets: [usize; 32],
    heap_offsets: Vec<usize>,
}

impl EncodingContext {
    pub fn new() -> Self {
        Self {
            stack_offsets: [0; 32],
            heap_offsets: Vec::new(),
        }
    }

    pub fn temp_offsets(&mut self, len: usize) -> &mut [usize] {
        if len <= self.stack_offsets.len() {
            &mut self.stack_offsets[..len]
        } else {
            self.heap_offsets.clear();
            self.heap_offsets.resize(len, 0);
            &mut self.heap_offsets
        }
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

    /// Returns the exact encoded size of this value
    ///
    /// For dynamic types, this includes all nested data.
    /// Used for offset calculations in complex structures.
    fn encoded_size(&self) -> usize {
        // Default implementation for static types
        align_up::<ALIGN>(Self::HEADER_SIZE)
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

            #[inline]
            pub fn encoded_size(value: &T) -> usize {
                value.encoded_size()
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

            #[inline]
            pub fn encoded_size(value: &T) -> usize {
                let _ = <T as PackedValidation>::ASSERT_STATIC;
                value.encoded_size()
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
