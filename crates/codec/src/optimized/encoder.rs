use crate::optimized::{counter::ByteCounter, error::CodecError, utils::align_up};
use byteorder::{BigEndian, ByteOrder, LittleEndian};
use bytes::{Buf, BufMut};
use core::marker::PhantomData;
use smallvec::SmallVec;

/// Encoding context for recursive ABI encoding with bounded depth.
/// Tracks current recursion depth for safety.
pub struct EncodingContext {
    depth: usize,
    max_depth: usize,
    element_info: SmallVec<[(usize, usize, usize); 32]>,
}

pub struct ElementInfo {
    depth: usize,
    length: usize, // Length of the element
    size: usize,   // Size of the element in bytes
}

impl EncodingContext {
    pub fn new() -> Self {
        Self {
            depth: 0,
            max_depth: 32,
            element_info: SmallVec::new(),
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
    pub fn push_element_info(&mut self, length: usize, offset: usize, size: usize) {
        self.element_info.push((length, offset, size));
    }

    pub fn pop_element_info(&mut self) -> Option<(usize, usize, usize)> {
        self.element_info.pop()
    }
    pub fn get_element_info(&self, index: usize) -> Option<(usize, usize, usize)> {
        self.element_info.get(index).copied()
    }

    pub fn element_info_len(&self) -> usize {
        self.element_info.len()
    }

    pub fn truncate_element_info(&mut self, len: usize) {
        self.element_info.truncate(len);
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

    // what is the best solution - this one or encode data method - that would allow to encode only
    // data? this would allow us to separate encoding for compact mode - where we should caclulate
    // meta and only after that actually write data. The realisation should be pretty
    // streightforward - as shown bellow
    fn data_size(&self, ctx: &mut EncodingContext) -> Result<usize, CodecError> {
        if Self::IS_DYNAMIC {
            let mut counter = ByteCounter::new();
            self.encode(&mut counter, Some(ctx))?;
            Ok(counter.count() - Self::HEADER_SIZE)
        } else {
            Ok(Self::HEADER_SIZE)
        }
    }

    #[inline(always)]
    fn encode_data(
        &self,
        buf: &mut impl BufMut,
        ctx: Option<&mut EncodingContext>,
    ) -> Result<usize, CodecError> {
        // For primitive types - encode as is
        if !Self::IS_DYNAMIC {
            return self.encode(buf, ctx);
        }
        // Динамические типы переопределяют метод,
        // сюда управление не должно попасть
        unreachable!("encode_data must be specialised for dynamic types")
    }

    fn len(&self) -> usize {
        1
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

// [3, 12, 20, 24, 16, 36, 24, 2, 12, 8, 1, 2, 1, 12, 4, 3, 3, 12, 12, 4, 5, 6]

// let expected_encoded = hex::decode(concat!(
// // Main array header
// "03000000", // length = 3 vectors
// "0c000000", // offset = 12 (to first element header)
// "3C000000", // size = 60 (36 bytes headers + 24 bytes data)
// // Nested vector headers
// // vec[0] = [1, 2]
// "02000000", // length = 2
// "24000000", // offset = 36 (from start of this header to its data)
// "08000000", // size = 8 bytes
// // vec[1] = [3]
// "01000000", // length = 1
// "2c000000", // offset = 44 (from start of this header to its data)
// "04000000", // size = 4 bytes
// // vec[2] = [4, 5, 6]
// "03000000", // length = 3
// "30000000", // offset = 48 (from start of this header to its data)
// "0c000000", // size = 12 bytes
// // Data sections
// "01000000", // 1
// "02000000", // 2
// "03000000", // 3
// "04000000", // 4
// "05000000", // 5
// "06000000"  // 6
// ))
// .unwrap();
