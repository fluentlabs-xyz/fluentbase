use crate::{
    alloc::{string::ToString, vec::Vec},
    error::CodecError,
};
use byteorder::{ByteOrder, BE, LE};
use bytes::{Buf, Bytes, BytesMut};
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
pub trait Encoder<B: ByteOrder, const ALIGN: usize, const SOL_MODE: bool>: Sized {
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
            T: Encoder<$byte_order, $align, $sol_mode>,
        {
            pub fn is_dynamic() -> bool {
                <T as Encoder<$byte_order, $align, $sol_mode>>::IS_DYNAMIC
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
    // Add variant with extra trait bound
    ($name:ident, $byte_order:ty, $align:expr, $sol_mode:expr, $extra_bound:tt) => {
        pub struct $name<T>(PhantomData<T>);

        impl<T> $name<T>
        where
            T: Encoder<$byte_order, $align, $sol_mode> + $extra_bound,
        {
            pub fn is_dynamic() -> bool {
                <T as Encoder<$byte_order, $align, $sol_mode>>::IS_DYNAMIC
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
pub trait IsStatic {}

define_encoder_mode!(SolidityABI, BE, 32, true);
define_encoder_mode!(SolidityPackedABI, BE, 1, false, IsStatic);
define_encoder_mode!(FluentABI, LE, 4, false);

pub trait SolidityEncoder: Encoder<BE, 32, true> {
    const SOLIDITY_HEADER_SIZE: usize = <Self as Encoder<BE, 32, true>>::HEADER_SIZE;
}

impl<T> SolidityEncoder for T where T: Encoder<BE, 32, true> {}

pub trait SolidityPackedEncoder: Encoder<BE, 1, false> {
    const SOLIDITY_PACKED_HEADER_SIZE: usize = <Self as Encoder<BE, 1, false>>::HEADER_SIZE;
}

impl<T> SolidityPackedEncoder for T where T: Encoder<BE, 1, false> {}

pub trait FluentEncoder: Encoder<LE, 4, false> {
    const FLUENT_HEADER_SIZE: usize = <Self as Encoder<LE, 4, false>>::HEADER_SIZE;
}

impl<T> FluentEncoder for T where T: Encoder<LE, 4, false> {}

// TODO: move functions bellow to the utils module

// TODO: d1r1 is it possible to make this fn const?
pub fn is_big_endian<B: ByteOrder>() -> bool {
    B::read_u16(&[0x12, 0x34]) == 0x1234
}

/// Rounds up the given offset to the nearest multiple of ALIGN.
/// ALIGN must be a power of two.
#[inline]
pub const fn align_up<const ALIGN: usize>(offset: usize) -> usize {
    (offset + ALIGN - 1) & !(ALIGN - 1)
}

/// Aligns the source bytes to the specified alignment.
pub fn align<B: ByteOrder, const ALIGN: usize, const SOLIDITY_COMP: bool>(src: &[u8]) -> Bytes {
    let aligned_src_len = align_up::<ALIGN>(src.len());
    let aligned_total_size = aligned_src_len.max(ALIGN);
    let mut aligned = BytesMut::zeroed(aligned_total_size);

    if is_big_endian::<B>() {
        // For big-endian, copy to the end of the aligned array
        let start = aligned_total_size - src.len();
        aligned[start..].copy_from_slice(src);
    } else {
        // For little-endian, copy to the start of the aligned array
        aligned[..src.len()].copy_from_slice(src);
    }

    aligned.freeze()
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

pub fn read_u32_aligned1<B: ByteOrder, const ALIGN: usize>(
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
pub fn get_aligned_slice<B: ByteOrder, const ALIGN: usize>(
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
        // For little-endian, return slice at the beginning of the aligned space
        aligned_offset
    };

    &mut buf[write_offset..write_offset + value_size]
}

pub fn get_aligned_indices<B: ByteOrder, const ALIGN: usize>(
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

pub fn is_dynamic<
    T: Encoder<B, ALIGN, SOL_MODE>,
    B: ByteOrder,
    const ALIGN: usize,
    const SOL_MODE: bool,
>() -> bool {
    T::IS_DYNAMIC
}
