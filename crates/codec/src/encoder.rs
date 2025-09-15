use crate::func::FunctionArgs;
use crate::{alloc::string::ToString, error::CodecError};
use byteorder::{ByteOrder, BE, LE};
use bytes::{Buf, BytesMut};
use core::marker::PhantomData;
// TODO: @d1r1 Investigate whether decoding the result into an uninitialized memory (e.g., using
// `MaybeUninit`) would be more efficient than initializing with `Default`.
// This could potentially reduce unnecessary memory initialization overhead in cases where
// the default value is not required before the actual decoding takes place.
// Consider benchmarking both approaches to measure performance differences.

/// Trait for encoding and decoding values with specific byte order, alignment, and mode.
///
/// # Type Parameters
/// - `B`: The byte order used for encoding/decoding.
/// - `ALIGN`: The alignment requirement for the encoded data.
/// - `SOL_MODE`: A boolean flag indicating whether Solidity-compatible mode is enabled.
/// - `IS_STATIC`: A boolean flag indicating whether the encoded data is static (used for
///   SolidityPackedABI).
pub trait Encoder<B: ByteOrder, const ALIGN: usize, const SOL_MODE: bool, const IS_STATIC: bool>:
    Sized
{
    /// Returns the header size for this encoder.
    const HEADER_SIZE: usize;
    const IS_DYNAMIC: bool;

    /// Encodes the value into the given buffer at the specified offset.
    ///
    /// # Arguments
    /// * `buf` - The buffer to encode into.
    /// * `offset` - The starting offset in the buffer for encoding.
    ///
    /// # Returns
    /// `Ok(())` if encoding was successful, or an error if encoding failed.
    fn encode(&self, buf: &mut BytesMut, offset: usize) -> Result<(), CodecError>;

    /// Decodes a value from the given buffer starting at the specified offset.
    ///
    /// # Arguments
    /// * `buf` - The buffer to decode from.
    /// * `offset` - The starting offset in the buffer for decoding.
    ///
    /// # Returns
    /// The decoded value if successful, or an error if decoding failed.
    fn decode(buf: &impl Buf, offset: usize) -> Result<Self, CodecError>;

    /// Partially decodes the header to determine the length and offset of the encoded data.
    ///
    /// # Arguments
    /// * `buf` - The buffer to decode from.
    /// * `offset` - The starting offset in the buffer for decoding.
    ///
    /// # Returns
    /// A tuple `(data_offset, data_length)` if successful, or an error if decoding failed.
    fn partial_decode(buf: &impl Buf, offset: usize) -> Result<(usize, usize), CodecError>;

    /// Calculates the number of bytes needed to encode the value.
    ///
    /// This includes the header size and any additional space needed for alignment.
    /// The default implementation aligns the header size to the specified alignment.
    fn size_hint(&self) -> usize {
        align_up::<ALIGN>(Self::HEADER_SIZE)
    }
}

macro_rules! define_encoder_mode {
    ($name:ident, $byte_order:ty, $align:expr, $sol_mode:expr) => {
        pub struct $name<T>(PhantomData<T>);

        impl<T> $name<T>
        where
            T: Encoder<$byte_order, $align, $sol_mode, false>,
        {
            pub fn is_dynamic() -> bool {
                <T as Encoder<$byte_order, $align, $sol_mode, false>>::IS_DYNAMIC
            }

            pub fn encode(value: &T, buf: &mut BytesMut, offset: usize) -> Result<(), CodecError> {
                value.encode(buf, offset)
            }

            pub fn decode(buf: &impl Buf, offset: usize) -> Result<T, CodecError> {
                T::decode(buf, offset)
            }

            pub fn partial_decode(
                buf: &impl Buf,
                offset: usize,
            ) -> Result<(usize, usize), CodecError> {
                T::partial_decode(buf, offset)
            }

            pub fn size_hint(value: &T) -> usize {
                value.size_hint()
            }
        }
    };
    ($name:ident, $byte_order:ty, $align:expr, $sol_mode:expr, static_only) => {
        pub struct $name<T>(PhantomData<T>);

        impl<T> $name<T>
        where
            T: Encoder<$byte_order, $align, $sol_mode, true>,
        {
            pub fn is_dynamic() -> bool {
                T::IS_DYNAMIC
            }

            pub fn encode(value: &T, buf: &mut BytesMut, offset: usize) -> Result<(), CodecError> {
                value.encode(buf, offset)
            }

            pub fn decode(buf: &impl Buf, offset: usize) -> Result<T, CodecError> {
                T::decode(buf, offset)
            }

            pub fn partial_decode(
                buf: &impl Buf,
                offset: usize,
            ) -> Result<(usize, usize), CodecError> {
                T::partial_decode(buf, offset)
            }

            pub fn size_hint(value: &T) -> usize {
                value.size_hint()
            }
        }
    };
}

define_encoder_mode!(SolidityABI, BE, 32, true);
define_encoder_mode!(CompactABI, LE, 4, false);

// SolidityPackedABI works only for static types
define_encoder_mode!(SolidityPackedABI, BE, 1, true, static_only);

impl<T> SolidityABI<T> {
    pub fn encode_function_args(value: &T, buf: &mut BytesMut) -> Result<(), CodecError>
    where
        T: FunctionArgs<BE, 32, true, false>,
    {
        value.encode_as_args(buf)
    }

    pub fn decode_function_args(buf: &impl Buf) -> Result<T, CodecError>
    where
        T: FunctionArgs<BE, 32, true, false>,
    {
        T::decode_as_args(buf)
    }
}

impl<T> CompactABI<T> {
    pub fn encode_function_args(value: &T, buf: &mut BytesMut) -> Result<(), CodecError>
    where
        T: FunctionArgs<LE, 4, false, false>,
    {
        value.encode_as_args(buf)
    }

    pub fn decode_function_args(buf: &impl Buf) -> Result<T, CodecError>
    where
        T: FunctionArgs<LE, 4, false, false>,
    {
        T::decode_as_args(buf)
    }
}
pub trait SolidityEncoder: Encoder<BE, 32, true, false> {
    const SOLIDITY_HEADER_SIZE: usize = <Self as Encoder<BE, 32, true, false>>::HEADER_SIZE;
}

impl<T> SolidityEncoder for T where T: Encoder<BE, 32, true, false> {}

pub trait SolidityPackedEncoder: Encoder<BE, 1, true, true> {
    const SOLIDITY_PACKED_HEADER_SIZE: usize = <Self as Encoder<BE, 1, true, true>>::HEADER_SIZE;
}

impl<T> SolidityPackedEncoder for T where T: Encoder<BE, 1, true, true> {}

pub trait FluentEncoder: Encoder<LE, 4, false, false> {
    const FLUENT_HEADER_SIZE: usize = <Self as Encoder<LE, 4, false, false>>::HEADER_SIZE;
}

impl<T> FluentEncoder for T where T: Encoder<LE, 4, false, false> {}

/// Checks if the given byte order is big-endian.
pub fn is_big_endian<B: ByteOrder>() -> bool {
    B::read_u16(&[0x12, 0x34]) == 0x1234
}

/// Rounds up the given offset to the nearest multiple of ALIGN.
/// ALIGN must be a power of two.
#[inline]
pub const fn align_up<const ALIGN: usize>(offset: usize) -> usize {
    (offset + ALIGN - 1) & !(ALIGN - 1)
}

/// Checks if the given type is dynamic.
pub fn is_dynamic<
    T: Encoder<B, ALIGN, SOL_MODE, IS_STATIC>,
    B: ByteOrder,
    const ALIGN: usize,
    const SOL_MODE: bool,
    const IS_STATIC: bool,
>() -> bool {
    T::IS_DYNAMIC
}

pub fn write_u32_aligned<B: ByteOrder, const ALIGN: usize>(
    buf: &mut BytesMut,
    offset: usize,
    value: u32,
) {
    let aligned_value_size = align_up::<ALIGN>(4);

    ensure_buf_size(buf, offset + aligned_value_size);

    if is_big_endian::<B>() {
        // For big-endian, copy to the end of the aligned array
        let start = offset + aligned_value_size - 4;
        B::write_u32(&mut buf[start..], value);
    } else {
        // For little-endian, copy to the start of the aligned array
        B::write_u32(&mut buf[offset..offset + 4], value);
    }
}

pub fn read_u32_aligned<B: ByteOrder, const ALIGN: usize>(
    buf: &impl Buf,
    offset: usize,
) -> Result<u32, CodecError> {
    let aligned_value_size = align_up::<ALIGN>(4);

    // Check for overflow
    let end_offset = offset.checked_add(aligned_value_size).ok_or_else(|| {
        CodecError::Decoding(crate::error::DecodingError::BufferOverflow {
            msg: "Overflow occurred when calculating end offset while reading aligned u32"
                .to_string(),
        })
    })?;

    if buf.remaining() < end_offset {
        return Err(CodecError::Decoding(
            crate::error::DecodingError::BufferTooSmall {
                expected: end_offset,
                found: buf.remaining(),
                msg: "Buffer underflow occurred while reading aligned u32".to_string(),
            },
        ));
    }

    if is_big_endian::<B>() {
        Ok(B::read_u32(&buf.chunk()[end_offset - 4..end_offset]))
    } else {
        Ok(B::read_u32(&buf.chunk()[offset..offset + 4]))
    }
}

/// Returns a mutable slice of the buffer at the specified offset, aligned to the specified
/// alignment. This slice is guaranteed to be large enough to hold the value of value_size.
pub(crate) fn get_aligned_slice<B: ByteOrder, const ALIGN: usize>(
    buf: &mut BytesMut,
    offset: usize,
    value_size: usize,
) -> &mut [u8] {
    let aligned_offset = align_up::<ALIGN>(offset);
    let word_size = align_up::<ALIGN>(ALIGN.max(value_size));

    // Ensure the buffer is large enough
    ensure_buf_size(buf, aligned_offset + word_size);

    let write_offset = if is_big_endian::<B>() {
        // For big-endian, return slice at the end of the aligned space
        aligned_offset + word_size - value_size
    } else {
        // For little-endian, return a slice at the beginning of the aligned space
        aligned_offset
    };

    &mut buf[write_offset..write_offset + value_size]
}

pub(crate) fn get_aligned_indices<B: ByteOrder, const ALIGN: usize>(
    offset: usize,
    value_size: usize,
) -> (usize, usize) {
    let aligned_offset = align_up::<ALIGN>(offset);
    let word_size = align_up::<ALIGN>(ALIGN.max(value_size));

    let write_offset = if is_big_endian::<B>() {
        // For big-endian, return indices at the end of the aligned space
        aligned_offset + word_size - value_size
    } else {
        // For little-endian, return indices at the beginning of the aligned space
        aligned_offset
    };

    (write_offset, write_offset + value_size)
}

/// Ensure the buffer is large enough to hold the data
pub fn ensure_buf_size(buf: &mut BytesMut, required_size: usize) {
    if buf.len() < required_size {
        buf.resize(required_size, 0);
    }
}
