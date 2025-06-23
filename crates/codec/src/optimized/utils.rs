use crate::{
    alloc::string::ToString,
    optimized::{encoder::Encoder, error::CodecError},
};
use byteorder::{ByteOrder, BE, LE};
use bytes::{Buf, BufMut, BytesMut};
use core::marker::PhantomData;

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
    T: Encoder<B, ALIGN, SOL_MODE>,
    B: ByteOrder,
    const ALIGN: usize,
    const SOL_MODE: bool,
>() -> bool {
    T::IS_DYNAMIC
}

pub fn write_u32_aligned<B: ByteOrder, const ALIGN: usize>(buf: &mut impl BufMut, value: u32) {
    let aligned_value_size = align_up::<ALIGN>(4);

    if is_big_endian::<B>() {
        // For big-endian, add padding first, then write value
        let padding = aligned_value_size - 4;
        if padding > 0 {
            buf.put_bytes(0, padding);
        }
        buf.put_u32(value);
    } else {
        // For little-endian, write value first, then add padding
        buf.put_u32_le(value);
        let padding = aligned_value_size - 4;
        if padding > 0 {
            buf.put_bytes(0, padding);
        }
    }
}
pub fn read_u32_aligned<B: ByteOrder, const ALIGN: usize>(
    buf: &impl Buf,
    offset: usize,
) -> Result<u32, CodecError> {
    let word_size = align_up::<ALIGN>(4);


    let end = offset + word_size ;

    if buf.remaining() < end {
        println!("read_u32_aligned>>>Buffer too small : expected {}, actual {}", end, buf.remaining());
        return Err(CodecError::BufferTooSmallMsg {
            expected: end,
            actual: buf.remaining(),
            message: "read_u32_aligned",
        });
    }
    if is_big_endian::<B>() {
        Ok(B::read_u32(&buf.chunk()[end - 4..end]))
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
        // For little-endian, return slice at the beginning of the aligned space
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
